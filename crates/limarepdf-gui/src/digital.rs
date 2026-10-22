//! Digital IDs and signing with them in the app: the Digital IDs window (create, import,
//! export, remove, choose the default) and the signing window. Making keys, opening IDs
//! with their passwords and signing all run on a background thread. The work itself is
//! limarepdf-core's (`IdStore`, `sign_document`); see docs/signatures.md.
//!
//! Passwords are typed into the windows, handed to the background thread and overwritten
//! there once used; they are never saved or logged. A wrong password, or any failure,
//! means the password has to be typed again.

use crate::dialogs::{self, Btn, Shell};
use crate::signing::Saved;
use eframe::egui::{self, Context, RichText, Ui};
use limarepdf_core::{
    display_line, DigitalId, DisallowedChanges, IdList, IdStore, NewId, Pdf, Permissions,
    SignPlace, SignRequest,
};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

/// The largest file read as a digital ID to import (limarepdf-core's own limit).
const MAX_ID_FILE: u64 = 4 << 20;
const WIDTH: f32 = 460.0;

fn clean(s: &str) -> String {
    display_line(s, 80)
}

/// What an ID is called in lists: its certificate's name.
fn label(id: &DigitalId) -> String {
    display_line(id.cert.display_name(), 60)
}

/// Why LimarePDF won't sign with the ID today, if it won't.
fn unusable(id: &DigitalId) -> Option<String> {
    if id.cert.expired() {
        Some(format!(
            "This ID expired on {}; LimarePDF doesn't sign with expired IDs.",
            id.cert.not_after.format("%Y-%m-%d")
        ))
    } else if !id.valid_now() {
        Some(format!(
            "This ID isn't valid until {}.",
            id.cert.not_before.format("%Y-%m-%d")
        ))
    } else {
        None
    }
}

// ------------------------------------------------------------------ background work

/// Runs `work` on a background thread; its result arrives on the receiver.
fn spawn<T: Send + 'static>(
    ctx: &Context,
    work: impl FnOnce() -> limarepdf_core::Result<T> + Send + 'static,
) -> Receiver<limarepdf_core::Result<T>> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(crate::guard(work));
        ctx.request_repaint();
    });
    rx
}

/// The result, once the work is done.
fn poll<T>(rx: &Receiver<limarepdf_core::Result<T>>) -> Option<limarepdf_core::Result<T>> {
    match rx.try_recv() {
        Ok(r) => Some(r),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!(
            "the background work stopped without a result"
        ))),
    }
}

/// A password being typed. Overwritten in memory when dropped. egui may hold copies of
/// typed text for a while (to draw and edit it); none is ever saved.
#[derive(Default)]
pub struct Secret(String);

impl Secret {
    fn edit(&mut self, ui: &mut Ui, hint: &str) -> egui::Response {
        ui.add(
            egui::TextEdit::singleline(&mut self.0)
                .password(true)
                .hint_text(hint)
                .desired_width(240.0),
        )
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Overwrites the string's bytes with zeros, keeping its length.
fn wipe(s: &mut String) {
    // SAFETY: zero bytes are valid UTF-8, so the string stays valid. Volatile writes, so
    // the compiler can't leave them out when the string is dropped next.
    unsafe {
        for b in s.as_mut_vec().iter_mut() {
            std::ptr::write_volatile(b, 0);
        }
    }
}

fn store() -> Result<IdStore, String> {
    IdStore::default_dir().map(IdStore::new).ok_or_else(|| {
        "LimarePDF can't find its settings folder (HOME or APPDATA isn't set), so it has nowhere \
         to keep digital IDs."
            .to_string()
    })
}

fn list_of(store: &Result<IdStore, String>) -> IdList {
    store.as_ref().map(IdStore::list).unwrap_or_default()
}

fn row(ui: &mut Ui, name: &str, value: impl Into<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{name}:")).weak());
        ui.label(value.into());
    });
}

/// An ID's certificate: who it names, who issued it, its key and dates.
fn id_ui(ui: &mut Ui, id: &DigitalId) {
    let c = &id.cert;
    ui.add(egui::Label::new(clean(&c.subject)).wrap());
    let issuer = if c.self_issued {
        "itself (a self-made digital ID)".to_string()
    } else {
        clean(&c.issuer)
    };
    row(ui, "Issued by", issuer);
    if !id.chain.is_empty() {
        row(
            ui,
            "With",
            format!("{} more certificate(s): its authorities", id.chain.len()),
        );
    }
    row(ui, "Key", c.key.clone());
    row(
        ui,
        "Valid",
        format!(
            "{} to {}",
            c.not_before.format("%Y-%m-%d"),
            c.not_after.format("%Y-%m-%d")
        ),
    );
    row(ui, "SHA-256", c.sha256.clone());
    if let Some(why) = unusable(id) {
        ui.colored_label(ui.visuals().error_fg_color, why);
    }
}

fn error(ui: &mut Ui, e: &Option<String>) {
    if let Some(e) = e {
        ui.add(egui::Label::new(RichText::new(e).color(ui.visuals().error_fg_color)).wrap());
    }
}

/// A row of a form's grid: `label`, then the field `add` draws, named by the label; and
/// under it, `error` when it is about this `field`.
fn form_row(
    ui: &mut Ui,
    label: &str,
    field: Field,
    error: &Option<(Field, String)>,
    add: impl FnOnce(&mut Ui) -> egui::Response,
) {
    ui.label(label);
    dialogs::set_name(&add(ui), label);
    ui.end_row();
    if let Some((_, e)) = error.as_ref().filter(|(f, _)| *f == field) {
        ui.label("");
        let text = RichText::new(e).color(ui.visuals().error_fg_color);
        ui.add(egui::Label::new(text).wrap());
        ui.end_row();
    }
}

fn window<'a>(title: &'a str, id: &str) -> Shell<'a> {
    Shell::new(title).id(id).width(WIDTH + 40.0)
}

fn busy_ui(ui: &mut Ui, what: &str) {
    dialogs::content(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(what);
        });
    });
}

// ------------------------------------------------------------------ Digital IDs window

/// The field of a form an error is shown beside.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Field {
    Name,
    Organisation,
    Email,
    Password,
    NewPassword,
    Again,
}

struct CreateForm {
    name: String,
    organisation: String,
    email: String,
    password: Secret,
    again: Secret,
    error: Option<(Field, String)>,
    /// Just opened: the keyboard goes to its first field.
    opened: bool,
}

impl CreateForm {
    /// What is wrong with the form, if anything: the first problem, by its field.
    fn problem(&self) -> Option<(Field, String)> {
        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        let new = |organisation, email| NewId {
            name: self.name.trim().to_string(),
            organisation,
            email,
        };
        let wrong = |id: NewId| id.check().err().map(|e| format!("{e:#}"));
        let organisation = opt(&self.organisation);
        wrong(new(None, None))
            .map(|e| (Field::Name, e))
            .or_else(|| wrong(new(organisation.clone(), None)).map(|e| (Field::Organisation, e)))
            .or_else(|| wrong(new(organisation, opt(&self.email))).map(|e| (Field::Email, e)))
            .or_else(|| {
                limarepdf_core::password_problem(&self.password.0).map(|e| (Field::Password, e))
            })
            .or_else(|| {
                let same = self.password.0 == self.again.0;
                (!same).then(|| (Field::Again, "The two passwords aren't the same.".into()))
            })
            .map(|(field, e)| (field, capital(&e)))
    }
}

struct ImportForm {
    file: PathBuf,
    password: Secret,
    error: Option<String>,
    /// Just opened: the keyboard goes to the password.
    opened: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum ExportKind {
    Key,
    Certificate,
}

struct ExportForm {
    id: DigitalId,
    kind: ExportKind,
    password: Secret,
    /// Protect the exported file with a new password instead of the ID's own.
    fresh: bool,
    new_password: Secret,
    again: Secret,
    error: Option<(Field, String)>,
}

enum Page {
    List,
    Create(CreateForm),
    Import(ImportForm),
    Export(Box<ExportForm>),
}

/// The Digital IDs window.
pub struct IdsWindow {
    store: Result<IdStore, String>,
    list: IdList,
    pick: usize,
    page: Page,
    /// Work running: what it is, and its result (a sentence for the window).
    task: Option<(&'static str, Receiver<limarepdf_core::Result<String>>)>,
    /// The last thing done (`true`) or why it failed.
    note: Option<(bool, String)>,
    /// Remove was clicked once; the second click removes.
    remove_armed: bool,
}

/// What the Digital IDs window did this frame.
#[derive(Default)]
pub struct IdsOut {
    pub close: bool,
    /// IDs were made, imported, removed or the default changed.
    pub changed: bool,
}

impl IdsWindow {
    pub fn new() -> Self {
        let store = store();
        let list = list_of(&store);
        let pick = list.ids.iter().position(|i| i.default).unwrap_or(0);
        Self {
            store,
            list,
            pick,
            page: Page::List,
            task: None,
            note: None,
            remove_armed: false,
        }
    }

    fn reload(&mut self, pick: Option<&str>) {
        self.list = list_of(&self.store);
        self.pick = pick
            .and_then(|n| self.list.ids.iter().position(|i| i.name == n))
            .or_else(|| self.list.ids.iter().position(|i| i.default))
            .unwrap_or(0);
        self.remove_armed = false;
    }

    pub fn ui(&mut self, ctx: &Context, author: &str) -> IdsOut {
        let mut out = IdsOut::default();
        if let Some((_, rx)) = &self.task {
            if let Some(r) = poll(rx) {
                self.task = None;
                out.changed = true;
                self.reload(None);
                self.note = Some(match r {
                    Ok(text) => (true, text),
                    Err(e) => (false, format!("{e:#}")),
                });
            }
        }
        // Esc closes the list, not a form being filled in.
        let listed = matches!(self.page, Page::List) && self.task.is_none();
        let (_, esc) = window("Digital IDs", "digital ids")
            .esc(listed)
            .show(ctx, |ui| {
                if let Some((what, _)) = &self.task {
                    busy_ui(ui, what);
                    return;
                }
                match std::mem::replace(&mut self.page, Page::List) {
                    Page::List => self.list_ui(ui, author, &mut out),
                    Page::Create(f) => self.create_ui(ui, f),
                    Page::Import(f) => self.import_ui(ui, f),
                    Page::Export(f) => self.export_ui(ui, f),
                }
            });
        // Stays open while work runs, so its result is seen.
        out.close = (out.close || esc) && self.task.is_none();
        out
    }

    fn list_ui(&mut self, ui: &mut Ui, author: &str, out: &mut IdsOut) {
        dialogs::content(ui, |ui| self.list_body(ui, out));
        let ok = self.store.is_ok();
        let hit = dialogs::footer(
            ui,
            None,
            &[
                Btn::plain("Create…")
                    .tip("Make a new self-signed digital ID")
                    .enabled(ok),
                Btn::plain("Import…")
                    .tip(
                        "Add an ID from a .p12 or .pfx file (from Acrobat, Windows or a \
                         certificate authority)",
                    )
                    .enabled(ok),
                Btn::plain("Close"),
            ],
        );
        match hit {
            Some(0) => {
                self.page = Page::Create(CreateForm {
                    name: author.to_string(),
                    organisation: String::new(),
                    email: String::new(),
                    password: Secret::default(),
                    again: Secret::default(),
                    error: None,
                    opened: true,
                });
                self.note = None;
            }
            Some(1) => {
                if let Some(file) = rfd::FileDialog::new()
                    .add_filter("Digital IDs", &["p12", "pfx"])
                    .pick_file()
                {
                    self.page = Page::Import(ImportForm {
                        file,
                        password: Secret::default(),
                        error: None,
                        opened: true,
                    });
                    self.note = None;
                }
            }
            Some(_) => out.close = true,
            None => {}
        }
    }

    /// The IDs, the chosen one's details and buttons, and the last thing done.
    fn list_body(&mut self, ui: &mut Ui, out: &mut IdsOut) {
        let store = match &self.store {
            Ok(s) => s.clone(),
            Err(e) => {
                error(ui, &Some(e.clone()));
                return;
            }
        };
        ui.add(
            egui::Label::new(
                "A digital ID signs documents so that others can check who signed and whether \
                 anything changed afterwards. The ID's key is kept in a password-protected \
                 file in LimarePDF's settings folder.",
            )
            .wrap(),
        );
        ui.add_space(6.0);
        if self.list.ids.is_empty() {
            ui.label("You have no digital IDs yet.");
        }
        for (i, id) in self.list.ids.iter().enumerate() {
            let mut text = label(id);
            if id.default {
                text.push_str("  (default)");
            }
            if unusable(id).is_some() {
                text.push_str("  (can't sign)");
            }
            if ui.selectable_label(self.pick == i, text).clicked() && self.pick != i {
                self.pick = i;
                self.remove_armed = false;
            }
        }
        for p in &self.list.problems {
            ui.label(RichText::new(display_line(p, 200)).small().weak());
        }
        if let Some(id) = self.list.ids.get(self.pick).cloned() {
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                id_ui(ui, &id);
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button("Export…")
                        .on_hover_text(
                            "Save its certificate for others to trust, or the ID with its key \
                             (a backup, or for other apps)",
                        )
                        .clicked()
                    {
                        self.page = Page::Export(Box::new(ExportForm {
                            id: id.clone(),
                            kind: ExportKind::Certificate,
                            password: Secret::default(),
                            fresh: false,
                            new_password: Secret::default(),
                            again: Secret::default(),
                            error: None,
                        }));
                        self.note = None;
                    }
                    if ui
                        .add_enabled(!id.default, egui::Button::new("Make default"))
                        .on_hover_text("Sign with this ID unless you choose another")
                        .clicked()
                    {
                        self.note = Some(match store.set_default(&id) {
                            Ok(()) => (true, format!("\"{}\" is now the default.", label(&id))),
                            Err(e) => (false, format!("{e:#}")),
                        });
                        out.changed = true;
                        self.reload(Some(&id.name));
                    }
                    if !self.remove_armed {
                        if ui.button("Remove…").clicked() {
                            self.remove_armed = true;
                        }
                    } else {
                        let red = ui.visuals().error_fg_color;
                        if ui
                            .button(RichText::new("Remove for good").color(red))
                            .clicked()
                        {
                            self.note = Some(match store.remove(&id) {
                                Ok(()) => (true, format!("Removed \"{}\".", label(&id))),
                                Err(e) => (false, format!("{e:#}")),
                            });
                            out.changed = true;
                            self.reload(None);
                        }
                        if ui.button("Keep it").clicked() {
                            self.remove_armed = false;
                        }
                    }
                });
                if self.remove_armed {
                    ui.add(
                        egui::Label::new(
                            "Removing deletes the ID's file. It can't sign again unless you \
                             have an exported copy. Documents already signed stay valid.",
                        )
                        .wrap(),
                    );
                }
            });
        }
        if let Some((ok, text)) = &self.note {
            ui.add_space(6.0);
            let text = RichText::new(text);
            let text = if *ok {
                text
            } else {
                text.color(ui.visuals().error_fg_color)
            };
            ui.add(egui::Label::new(text).wrap());
        }
    }

    fn create_ui(&mut self, ui: &mut Ui, mut f: CreateForm) {
        dialogs::content(ui, |ui| {
            ui.strong("Create a digital ID");
            ui.add(
                egui::Label::new(
                    "LimarePDF makes a self-signed ID (RSA 3072-bit, valid for five years). The \
                     name is what signatures show. Others see you as an unknown signer until \
                     they trust your certificate.",
                )
                .wrap(),
            );
            ui.add_space(6.0);
            egui::Grid::new("new id").num_columns(2).show(ui, |ui| {
                let e = &f.error;
                form_row(ui, "Name", Field::Name, e, |ui| {
                    let r = ui.text_edit_singleline(&mut f.name);
                    // The keyboard starts in the form, as it opens from a button that is gone.
                    if std::mem::take(&mut f.opened) {
                        r.request_focus();
                    }
                    r
                });
                form_row(ui, "Organisation", Field::Organisation, e, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.organisation).hint_text("Optional"))
                });
                form_row(ui, "Email", Field::Email, e, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut f.email).hint_text("Optional"))
                });
                let at_least = format!("At least {} characters", limarepdf_core::MIN_PASSWORD);
                form_row(ui, "Password", Field::Password, e, |ui| {
                    f.password.edit(ui, &at_least)
                });
                form_row(ui, "Again", Field::Again, e, |ui| {
                    f.again.edit(ui, "Type it again")
                });
            });
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    "Keep the password safe: LimarePDF can't recover it, and without it the ID \
                     can't sign. Keep a copy of the ID too (Export, with its key): if this \
                     computer is lost, so is the ID.",
                )
                .wrap(),
            );
        });
        let hit = dialogs::footer(ui, None, &[Btn::primary("Create"), Btn::plain("Back")]);
        let (go, back) = (hit == Some(0), hit == Some(1));
        if back {
            return;
        }
        if go {
            f.error = f.problem();
        }
        if go && f.error.is_none() {
            if let Ok(store) = self.store.clone() {
                let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
                let new = NewId {
                    name: f.name.trim().to_string(),
                    organisation: opt(&f.organisation),
                    email: opt(&f.email),
                };
                let password = std::mem::take(&mut f.password);
                let ctx = ui.ctx().clone();
                self.task = Some((
                    "Making the digital ID…",
                    spawn(&ctx, move || {
                        let id = store.create(&new, &password.0)?;
                        Ok(format!(
                            "Made the digital ID \"{}\". It is the default for signing. Keep a \
                             copy: Export saves it with its key, for a backup or other apps.",
                            label(&id)
                        ))
                    }),
                ));
                return;
            }
        }
        self.page = Page::Create(f);
    }

    fn import_ui(&mut self, ui: &mut Ui, mut f: ImportForm) {
        dialogs::content(ui, |ui| {
            ui.strong("Import a digital ID");
            row(ui, "File", f.file.display().to_string());
            ui.horizontal(|ui| {
                ui.label("Password");
                let r = f.password.edit(ui, "The file's password");
                dialogs::set_name(&r, "Password");
                if std::mem::take(&mut f.opened) {
                    r.request_focus();
                }
            });
            ui.add(
                egui::Label::new(
                    "LimarePDF keeps its own copy, protected by the same password. The key must \
                     be RSA (2048 to 4096 bits) or ECDSA P-256 or P-384, and the certificate \
                     usable for signing.",
                )
                .wrap(),
            );
            error(ui, &f.error);
        });
        let hit = dialogs::footer(ui, None, &[Btn::primary("Import"), Btn::plain("Back")]);
        let (mut go, back) = (hit == Some(0), hit == Some(1));
        go |= ui.input(|i| i.key_pressed(egui::Key::Enter));
        if back {
            return;
        }
        if go {
            if let Ok(store) = self.store.clone() {
                let password = std::mem::take(&mut f.password);
                let file = f.file.clone();
                let ctx = ui.ctx().clone();
                self.task = Some((
                    "Importing the digital ID…",
                    spawn(&ctx, move || {
                        use anyhow::Context as _;
                        let size = std::fs::metadata(&file)
                            .with_context(|| format!("reading {}", file.display()))?
                            .len();
                        if size > MAX_ID_FILE {
                            anyhow::bail!("{} is too large to be a digital ID", file.display());
                        }
                        let bytes = std::fs::read(&file)
                            .with_context(|| format!("reading {}", file.display()))?;
                        let id = store.import(&bytes, &password.0, None)?;
                        Ok(format!(
                            "Imported \"{}\". It is the default for signing. {} still holds \
                             the ID: keep it safe, or delete it if LimarePDF's copy is enough.",
                            label(&id),
                            file.display()
                        ))
                    }),
                ));
                return;
            }
        }
        self.page = Page::Import(f);
    }

    fn export_ui(&mut self, ui: &mut Ui, mut f: Box<ExportForm>) {
        dialogs::content(ui, |ui| {
            ui.strong(format!("Export \"{}\"", label(&f.id)));
            ui.radio_value(
                &mut f.kind,
                ExportKind::Certificate,
                "Certificate only (.cer): for others to trust your signatures",
            );
            ui.radio_value(
                &mut f.kind,
                ExportKind::Key,
                "With its key (.p12): a backup, or to sign in other apps",
            );
            ui.add_space(4.0);
            match f.kind {
                ExportKind::Certificate => {
                    ui.add(
                        egui::Label::new(
                            "The certificate holds no key, so it is safe to share. Tell \
                             recipients its SHA-256 fingerprint another way, such as by phone, \
                             so they can check it is yours:",
                        )
                        .wrap(),
                    );
                    ui.label(RichText::new(&f.id.cert.sha256).monospace().small());
                }
                ExportKind::Key => {
                    let e = &f.error;
                    egui::Grid::new("export id").num_columns(2).show(ui, |ui| {
                        form_row(ui, "ID's password", Field::Password, e, |ui| {
                            f.password.edit(ui, "Password")
                        });
                    });
                    ui.checkbox(&mut f.fresh, "Protect the file with a new password");
                    if f.fresh {
                        egui::Grid::new("export new").num_columns(2).show(ui, |ui| {
                            let at_least =
                                format!("At least {} characters", limarepdf_core::MIN_PASSWORD);
                            form_row(ui, "New password", Field::NewPassword, e, |ui| {
                                f.new_password.edit(ui, &at_least)
                            });
                            form_row(ui, "Again", Field::Again, e, |ui| {
                                f.again.edit(ui, "Type it again")
                            });
                        });
                    }
                    ui.add(
                        egui::Label::new(
                            "Anyone with this file and its password can sign as you: keep it \
                             safe.",
                        )
                        .wrap(),
                    );
                }
            }
        });
        let hit = dialogs::footer(ui, None, &[Btn::primary("Export…"), Btn::plain("Back")]);
        let (go, back) = (hit == Some(0), hit == Some(1));
        if back {
            return;
        }
        if go {
            match f.kind {
                ExportKind::Certificate => {
                    if let Some(path) = save_path(&f.id.name, "cer", "Certificate") {
                        self.note = Some(match limarepdf_core::write_file(&path, &f.id.cert.der) {
                            Ok(()) => (
                                true,
                                format!(
                                    "Saved the certificate of \"{}\" to {}. Recipients trust \
                                     it in LimarePDF with Edit > Trusted certificates > Add, or in \
                                     their PDF app's trusted certificates, after checking the \
                                     fingerprint with you.",
                                    label(&f.id),
                                    path.display()
                                ),
                            ),
                            Err(e) => (false, format!("{e:#}")),
                        });
                        return;
                    }
                }
                ExportKind::Key => {
                    let problem = if f.fresh {
                        limarepdf_core::password_problem(&f.new_password.0)
                            .map(|e| (Field::NewPassword, e))
                            .or_else(|| {
                                (f.new_password.0 != f.again.0).then(|| {
                                    let e = "The two new passwords aren't the same.";
                                    (Field::Again, e.to_string())
                                })
                            })
                    } else if limarepdf_core::password_problem(&f.password.0).is_some() {
                        let e = format!(
                            "The ID's password is shorter than {} characters: protect the \
                             file with a new password.",
                            limarepdf_core::MIN_PASSWORD
                        );
                        Some((Field::Password, e))
                    } else {
                        None
                    };
                    if let Some((field, why)) = problem {
                        f.error = Some((field, capital(&why)));
                    } else if let (Ok(store), Some(path)) = (
                        self.store.clone(),
                        save_path(&f.id.name, "p12", "Digital ID"),
                    ) {
                        let password = std::mem::take(&mut f.password);
                        let new_password = std::mem::take(&mut f.new_password);
                        let (id, fresh) = (f.id.clone(), f.fresh);
                        let ctx = ui.ctx().clone();
                        self.task = Some((
                            "Exporting the digital ID…",
                            spawn(&ctx, move || {
                                let unlocked = store.unlock(&id, &password.0)?;
                                let file_password = if fresh { &new_password } else { &password };
                                let bytes = unlocked.to_p12(&file_password.0)?;
                                limarepdf_core::write_private(&path, &bytes)?;
                                Ok(format!(
                                    "Saved \"{}\" with its key to {}, protected by {}. Keep \
                                     it safe.",
                                    label(&id),
                                    path.display(),
                                    if fresh {
                                        "the new password"
                                    } else {
                                        "the ID's password"
                                    }
                                ))
                            }),
                        ));
                        return;
                    }
                }
            }
        }
        self.page = Page::Export(f);
    }
}

/// Asks where to save an exported file, suggesting `stem.ext`.
fn save_path(stem: &str, ext: &str, kind: &str) -> Option<PathBuf> {
    let mut path = rfd::FileDialog::new()
        .add_filter(kind, &[ext])
        .set_file_name(format!("{stem}.{ext}"))
        .save_file()?;
    if path.extension().is_none() {
        path.set_extension(ext);
    }
    Some(path)
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

// ------------------------------------------------------------------ signing window

/// What signing needs from the tab, gathered by the app once the signer clicks Sign.
pub struct Input {
    /// The file as opened (or as written out in full, for a document never signed).
    pub base: Arc<[u8]>,
    /// The document as edited since, when there are edits to sign with it.
    pub edited: Option<Pdf>,
    /// The signer's saved fill-and-sign signature and its picture's bytes, for the look.
    pub art: Option<(Saved, Vec<u8>)>,
    /// Where the signed document is written.
    pub out: PathBuf,
}

/// A document signed and written.
pub struct Signed {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub field: String,
    pub finished_items: usize,
    pub rewritten: bool,
    /// The ID's name, and whether it is self-made.
    pub signer: String,
    pub self_issued: bool,
    pub certified: bool,
    /// Whether the signed file was confirmed on disk (decision D7).
    pub confirmed: bool,
}

/// What the signing window did this frame.
pub enum SignOut {
    Open,
    Cancel,
    /// Signing was asked for: the app gathers the [`Input`] and calls
    /// [`SignWindow::start`].
    Go,
    Done(Box<Signed>),
    /// Open the Digital IDs window.
    ManageIds,
}

/// The signing window: which ID, its password, the reason and place, and for a
/// certification what others may change.
pub struct SignWindow {
    /// The tab, and its revision when the place was chosen.
    pub tab: u64,
    pub revision: u64,
    pub place: SignPlace,
    where_text: String,
    pub certify: bool,
    permissions: Permissions,
    store: Result<IdStore, String>,
    list: IdList,
    pick: usize,
    password: Secret,
    reason: String,
    location: String,
    /// A saved fill-and-sign signature exists, and whether to show it in the look.
    pub has_saved: bool,
    pub use_saved: bool,
    /// The signer agreed that earlier signatures will report disallowed changes.
    accept_disallowed: bool,
    /// Signing was refused for that reason: the question shown.
    disallowed: Option<String>,
    pub error: Option<String>,
    task: Option<Receiver<limarepdf_core::Result<Signed>>>,
    focus: bool,
}

impl SignWindow {
    pub fn new(
        tab: u64,
        revision: u64,
        place: SignPlace,
        where_text: String,
        certify: bool,
        has_saved: bool,
    ) -> Self {
        let store = store();
        let list = list_of(&store);
        let mut w = Self {
            tab,
            revision,
            place,
            where_text,
            certify,
            permissions: Permissions::FormFilling,
            store,
            list,
            pick: 0,
            password: Secret::default(),
            reason: String::new(),
            location: String::new(),
            has_saved,
            use_saved: has_saved,
            accept_disallowed: false,
            disallowed: None,
            error: None,
            task: None,
            focus: true,
        };
        w.pick_default();
        w
    }

    /// The default ID if it can sign, else the first that can.
    fn pick_default(&mut self) {
        let ids = &self.list.ids;
        self.pick = ids
            .iter()
            .position(|i| i.default && unusable(i).is_none())
            .or_else(|| ids.iter().position(|i| unusable(i).is_none()))
            .unwrap_or(0);
    }

    /// After the Digital IDs window changed the IDs.
    pub fn reload(&mut self) {
        let name = self.list.ids.get(self.pick).map(|i| i.name.clone());
        self.list = list_of(&self.store);
        match name.and_then(|n| self.list.ids.iter().position(|i| i.name == n)) {
            Some(i) => self.pick = i,
            None => self.pick_default(),
        }
    }

    pub fn busy(&self) -> bool {
        self.task.is_some()
    }

    /// Signs in the background with what the window holds. The password is used up:
    /// after a failure it is typed again.
    pub fn start(&mut self, ctx: &Context, input: Input) {
        let (Ok(store), Some(id)) = (self.store.clone(), self.list.ids.get(self.pick).cloned())
        else {
            return;
        };
        let password = std::mem::take(&mut self.password);
        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        let (reason, location) = (opt(&self.reason), opt(&self.location));
        let certify = self.certify.then_some(self.permissions);
        let place = self.place.clone();
        let accept_disallowed = self.accept_disallowed;
        let art = if self.use_saved { input.art } else { None };
        self.error = None;
        self.task = Some(spawn(ctx, move || {
            let unlocked = store.unlock(&id, &password.0)?;
            drop(password);
            let signed = limarepdf_core::sign_document(
                &input.base,
                input.edited.as_ref(),
                &SignRequest {
                    id: &unlocked,
                    place,
                    reason,
                    location,
                    certify,
                    art: art.as_ref().map(|(s, b)| s.art(b)),
                    accept_disallowed,
                },
            )?;
            let confirmed = match limarepdf_core::write_file(&input.out, &signed.bytes) {
                Ok(()) => true,
                // The signed file is in place all the same (decision D7).
                Err(e) if crate::app::not_confirmed(&e) => false,
                Err(e) => return Err(e),
            };
            // The ID just used becomes the default; failing to note that doesn't undo the
            // signing.
            let _ = store.set_default(&id);
            Ok(Signed {
                path: input.out,
                bytes: signed.bytes,
                field: signed.field,
                finished_items: signed.finished_items,
                rewritten: signed.rewritten,
                signer: label(&id),
                self_issued: id.cert.self_issued,
                certified: certify.is_some(),
                confirmed,
            })
        }));
    }

    pub fn ui(&mut self, ctx: &Context) -> SignOut {
        if let Some(rx) = &self.task {
            if let Some(r) = poll(rx) {
                self.task = None;
                match r {
                    Ok(s) => return SignOut::Done(Box::new(s)),
                    Err(e) => match e.downcast_ref::<DisallowedChanges>() {
                        Some(d) => self.disallowed = Some(disallowed_text(d)),
                        None => self.error = Some(capital(&format!("{e:#}"))),
                    },
                }
                self.focus = true;
            }
        }
        let mut out = SignOut::Open;
        let title = if self.certify {
            "Certify with digital ID"
        } else {
            "Sign with digital ID"
        };
        // Esc closes it until something is typed into it.
        let empty =
            self.password.0.is_empty() && self.reason.is_empty() && self.location.is_empty();
        let (_, esc) = window(title, "digital sign")
            .esc(empty && self.task.is_none())
            .show(ctx, |ui| {
                if self.task.is_some() {
                    busy_ui(
                        ui,
                        if self.certify {
                            "Certifying…"
                        } else {
                            "Signing…"
                        },
                    );
                    return;
                }
                if let Some(q) = self.disallowed.clone() {
                    dialogs::content(ui, |ui| ui.add(egui::Label::new(q).wrap()));
                    match dialogs::footer(ui, None, &[Btn::danger("Sign anyway"), Btn::cancel()]) {
                        Some(0) => {
                            self.accept_disallowed = true;
                            self.disallowed = None;
                            self.error = Some("Type the ID's password again to sign.".into());
                            self.focus = true;
                        }
                        Some(_) => out = SignOut::Cancel,
                        None => {}
                    }
                    return;
                }
                out = self.form_ui(ui);
            });
        if esc {
            out = SignOut::Cancel;
        }
        out
    }

    fn form_ui(&mut self, ui: &mut Ui) -> SignOut {
        let mut out = SignOut::Open;
        dialogs::content(ui, |ui| self.form_body(ui, &mut out));
        let can = self
            .list
            .ids
            .get(self.pick)
            .is_some_and(|i| unusable(i).is_none());
        let go = if self.certify {
            "Certify…"
        } else {
            "Sign…"
        };
        let buttons = [
            Btn::primary(go)
                .enabled(can)
                .tip("Choose where to save the signed document, then sign"),
            Btn::plain("Digital IDs…"),
            Btn::cancel(),
        ];
        match dialogs::footer(ui, None, &buttons) {
            Some(0) => out = SignOut::Go,
            Some(1) => out = SignOut::ManageIds,
            Some(_) => out = SignOut::Cancel,
            None => {}
        }
        if matches!(out, SignOut::Go) {
            if !can {
                return SignOut::Open;
            }
            if self.password.0.is_empty() {
                self.error = Some("Type the ID's password.".into());
                self.focus = true;
                return SignOut::Open;
            }
        }
        out
    }

    /// The signing form: where, the ID, its password, and the options; `out` becomes Go
    /// on Enter in the password.
    fn form_body(&mut self, ui: &mut Ui, out: &mut SignOut) {
        row(ui, "Where", self.where_text.clone());
        ui.add_space(6.0);
        if let Err(e) = &self.store {
            error(ui, &Some(e.clone()));
        } else if self.list.ids.is_empty() {
            ui.add(
                egui::Label::new(
                    "You need a digital ID to sign. Create one, or import one from Acrobat, \
                     Windows or a certificate authority, in the Digital IDs window.",
                )
                .wrap(),
            );
        } else {
            let ids = &self.list.ids;
            let shown = ids.get(self.pick).map(label).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label("Digital ID");
                egui::ComboBox::from_id_salt("sign id")
                    .selected_text(shown)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for (i, id) in ids.iter().enumerate() {
                            let mut text = label(id);
                            if unusable(id).is_some() {
                                text.push_str(" (can't sign)");
                            }
                            ui.selectable_value(&mut self.pick, i, text);
                        }
                    });
            });
            if let Some(id) = ids.get(self.pick) {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    id_ui(ui, id);
                    if id.cert.self_issued {
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "Others see you as an unknown signer until they trust your \
                                     certificate: send it to them from Digital IDs (Export, \
                                     certificate only).",
                                )
                                .small()
                                .weak(),
                            )
                            .wrap(),
                        );
                    }
                });
            }
            ui.add_space(4.0);
            egui::Grid::new("sign form").num_columns(2).show(ui, |ui| {
                ui.label("Password");
                let r = self.password.edit(ui, "The ID's password");
                if std::mem::take(&mut self.focus) {
                    r.request_focus();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    *out = SignOut::Go;
                }
                ui.end_row();
                ui.label("Reason");
                ui.add(egui::TextEdit::singleline(&mut self.reason).hint_text("Optional"));
                ui.end_row();
                ui.label("Location");
                ui.add(egui::TextEdit::singleline(&mut self.location).hint_text("Optional"));
                ui.end_row();
            });
        }
        if self.certify {
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    "Certifying marks you as the document's author. Readers are told of any \
                     change beyond what you allow. After certifying, others may:",
                )
                .wrap(),
            );
            for (p, text) in [
                (Permissions::NoChanges, "Make no changes"),
                (Permissions::FormFilling, "Fill in forms and sign"),
                (
                    Permissions::FormFillingAndComments,
                    "Fill in forms, sign and add comments",
                ),
            ] {
                ui.radio_value(&mut self.permissions, p, text);
            }
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(self.has_saved, |ui| {
            ui.checkbox(
                &mut self.use_saved,
                "Show my saved signature beside the text",
            )
            .on_disabled_hover_text(
                "Create a signature in Fill and sign first; meanwhile your name is written \
                     in handwriting.",
            );
        });
        ui.add(
            egui::Label::new(
                RichText::new(
                    "The signature covers this document with every change made to it so far. \
                     It is saved as a new file, which then opens in place of this one.",
                )
                .small()
                .weak(),
            )
            .wrap(),
        );
        error(ui, &self.error);
    }
}

/// Each signer of `d` with the changes their signature would report, one line each
/// (each line starts with a line break).
pub fn disallowed_list(d: &DisallowedChanges) -> String {
    let mut s = String::new();
    for (who, what) in &d.signers {
        let what: Vec<String> = what.iter().take(5).map(|w| display_line(w, 120)).collect();
        s.push_str(&format!(
            "\n• {}: {}",
            capital(&display_line(who, 80)),
            what.join("; ")
        ));
    }
    s
}

/// The question asked when signing would make earlier signatures report disallowed
/// changes.
fn disallowed_text(d: &DisallowedChanges) -> String {
    let mut s = String::from(
        "This document was changed in ways its earlier signatures don't allow. If you sign \
         now, anyone who checks them will be told:\n",
    );
    s.push_str(&disallowed_list(d));
    s.push_str(
        "\n\nThe earlier signatures stay valid for the version they signed. Sign anyway? You \
         will be asked for the password again.",
    );
    s
}

/// Where a signature goes, in words: "the signature field "Approver" on page 2".
pub fn where_text(pdf: &Pdf, place: &SignPlace) -> String {
    match place {
        SignPlace::Field(name) => {
            let page = pdf
                .empty_signature_fields()
                .into_iter()
                .find(|f| &f.name == name)
                .and_then(|f| f.widgets.first().map(|w| w.page));
            let name = display_line(name, 60);
            match page {
                Some(p) => format!("the signature field \"{name}\" on page {}", p + 1),
                None => format!("the signature field \"{name}\""),
            }
        }
        SignPlace::NewBox { page, .. } => format!("a new signature box on page {}", page + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const PASSWORD: &str = "correct horse battery";

    /// A signing window on a store of its own, holding one new ID.
    fn window(name: &str) -> (SignWindow, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("limarepdf-digital-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = IdStore::new(dir.join("ids"));
        let new = NewId {
            name: "Jane Example".into(),
            ..NewId::default()
        };
        store.create(&new, PASSWORD).unwrap();
        let place = SignPlace::NewBox {
            page: 0,
            rect: [72.0, 72.0, 272.0, 132.0],
        };
        let mut w = SignWindow::new(0, 0, place, "a box".into(), false, false);
        w.store = Ok(store);
        w.reload();
        (w, dir)
    }

    #[test]
    fn a_new_id_s_problem_is_shown_beside_its_field() {
        let form = |name: &str, organisation: &str, email: &str, again: &str| CreateForm {
            name: name.into(),
            organisation: organisation.into(),
            email: email.into(),
            password: Secret(PASSWORD.into()),
            again: Secret(again.into()),
            error: None,
            opened: false,
        };
        let field = |f: CreateForm| f.problem().map(|(field, _)| field);
        assert_eq!(field(form("Jane", "", "", PASSWORD)), None);
        assert_eq!(field(form(" ", "", "", PASSWORD)), Some(Field::Name));
        assert_eq!(
            field(form("Jane", "A\nB", "", PASSWORD)),
            Some(Field::Organisation)
        );
        assert_eq!(
            field(form("Jane", "", "jane", PASSWORD)),
            Some(Field::Email)
        );
        assert_eq!(field(form("Jane", "", "", "other")), Some(Field::Again));
        let mut short = form("Jane", "", "", "abc");
        short.password = Secret("abc".into());
        let (field, text) = short.problem().unwrap();
        assert_eq!(field, Field::Password);
        assert!(text.starts_with(char::is_uppercase), "{text}");
    }

    /// Signs as the app does, waiting for the background work.
    fn sign(w: &mut SignWindow, base: &[u8], edited: Option<Pdf>, out: &Path) -> SignOut {
        let ctx = Context::default();
        w.password = Secret(PASSWORD.into());
        let input = Input {
            base: Arc::from(base),
            edited,
            art: None,
            out: out.to_path_buf(),
        };
        w.start(&ctx, input);
        assert!(w.busy());
        assert!(
            w.password.0.is_empty(),
            "the password is handed over, not kept"
        );
        let rx = w.task.take().unwrap();
        let r = rx
            .recv_timeout(std::time::Duration::from_secs(120))
            .unwrap();
        let (tx, again) = channel();
        tx.send(r).unwrap();
        w.task = Some(again);
        let mut out = SignOut::Open;
        crate::theme::install(&ctx);
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| out = w.ui(ui.ctx()));
        // Nothing draws the frame: its font atlas upload is dropped on purpose.
        frame.textures_delta.clear();
        out
    }

    #[test]
    fn signing_writes_a_valid_signed_copy() {
        let (mut w, dir) = window("sign");
        let mut doc = Pdf::from_text("Hello", (612.0, 792.0)).unwrap();
        let base = doc.to_bytes().unwrap();
        let out = dir.join("signed.pdf");
        let SignOut::Done(s) = sign(&mut w, &base, None, &out) else {
            panic!("not signed: {:?}", w.error);
        };
        assert_eq!(s.signer, "Jane Example");
        assert!(s.self_issued && !s.certified);
        let written = std::fs::read(&out).unwrap();
        assert_eq!(written, s.bytes);
        let rep = limarepdf_core::verify_signatures(
            &written,
            None,
            &limarepdf_core::TrustList::default(),
        )
        .unwrap();
        assert_eq!(rep.signatures.len(), 1);
        assert_eq!(rep.signatures[0].status, limarepdf_core::SigStatus::Valid);
        assert!(rep.signatures[0].covers_whole_file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_passwords_and_disallowed_changes_are_said() {
        let (mut w, dir) = window("refused");
        let mut doc = Pdf::from_text("Hello", (612.0, 792.0)).unwrap();
        let base = doc.to_bytes().unwrap();
        let first = dir.join("first.pdf");
        let SignOut::Done(s) = sign(&mut w, &base, None, &first) else {
            panic!("not signed: {:?}", w.error);
        };

        // A wrong password: said, nothing written.
        let ctx = Context::default();
        w.password = Secret("not the password".into());
        let out = dir.join("second.pdf");
        w.start(
            &ctx,
            Input {
                base: Arc::from(s.bytes.as_slice()),
                edited: None,
                art: None,
                out: out.clone(),
            },
        );
        let r = w.task.take().unwrap().recv().unwrap();
        assert!(r.is_err());
        assert!(!out.exists());

        // A turned page breaks the first signature's rules: asked, then signed on consent.
        let mut turned = Pdf::from_bytes(&s.bytes, None).unwrap();
        turned.rotate_pages(&[0], 90).unwrap();
        w.place = SignPlace::NewBox {
            page: 0,
            rect: [72.0, 300.0, 272.0, 360.0],
        };
        let asked = sign(&mut w, &s.bytes, Some(turned.clone()), &out);
        assert!(matches!(asked, SignOut::Open));
        let q = w.disallowed.clone().expect("the question");
        assert!(q.contains("\"Jane Example\""), "{q}");
        assert!(!out.exists());
        w.disallowed = None;
        w.accept_disallowed = true;
        let SignOut::Done(s2) = sign(&mut w, &s.bytes, Some(turned), &out) else {
            panic!("not signed: {:?}", w.error);
        };
        let rep = limarepdf_core::verify_signatures(
            &s2.bytes,
            None,
            &limarepdf_core::TrustList::default(),
        )
        .unwrap();
        assert_eq!(rep.signatures.len(), 2);
        assert!(!rep.signatures[0].changes_allowed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn passwords_are_overwritten() {
        let mut s = String::from("correct horse");
        wipe(&mut s);
        assert_eq!(s.len(), 13);
        assert!(s.bytes().all(|b| b == 0));
    }

    #[test]
    fn the_question_names_each_earlier_signer() {
        let d = DisallowedChanges {
            signers: vec![
                ("\"Jane Doe\"".into(), vec!["page 1 turned".into()]),
                ("an earlier signer".into(), vec!["a page added".into()]),
            ],
        };
        let q = disallowed_text(&d);
        assert!(q.contains("\"Jane Doe\": page 1 turned"));
        assert!(q.contains("An earlier signer: a page added"));
        assert!(q.contains("password again"));
    }
}
