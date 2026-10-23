//! Digital IDs and signing with them in the app: the Digital IDs window (create, import,
//! export, remove, choose the default) and the signing window. Making keys, opening IDs
//! with their passwords and signing all run on a background thread. The work itself is
//! limarepdf-core's (`IdStore`, `sign_document`); see docs/signatures.md.
//!
//! Passwords are typed into the windows, handed to the background thread and overwritten
//! there once used; they are never saved or logged. A wrong password, or any failure,
//! means the password has to be typed again: a form comes back with what else was typed
//! and the error, its passwords empty.

use crate::dialogs::{self, Btn, Shell};
use crate::signing::Saved;
use crate::theme;
use eframe::egui::{self, Context, RichText, Ui};
use limarepdf_core::{
    display_line, DigitalId, DisallowedChanges, IdList, IdStore, NewId, Pdf, Permissions,
    SignPlace, SignRequest, WrongPassword,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

/// The largest file read as a digital ID to import (limarepdf-core's own limit).
const MAX_ID_FILE: u64 = 4 << 20;
const WIDTH: f32 = 460.0;
/// The narrowest a signature's Reason and Location are beside each other.
const REASON_WIDTH: f32 = 180.0;

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
        ui.add(theme::field(
            egui::TextEdit::singleline(&mut self.0)
                .password(true)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        ))
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

/// An ID's certificate: who it names, who issued it, its key and dates.
fn id_ui(ui: &mut Ui, id: &DigitalId) {
    let c = &id.cert;
    dialogs::fact(ui, "Certificate", clean(&c.subject));
    let issuer = if c.self_issued {
        "itself (a self-made digital ID)".to_string()
    } else {
        clean(&c.issuer)
    };
    dialogs::fact(ui, "Issued by", issuer);
    if !id.chain.is_empty() {
        dialogs::fact(
            ui,
            "With",
            format!("{} more certificate(s): its authorities", id.chain.len()),
        );
    }
    dialogs::fact(ui, "Key", c.key.clone());
    dialogs::fact(
        ui,
        "Valid",
        format!(
            "{} to {}",
            c.not_before.format("%Y-%m-%d"),
            c.not_after.format("%Y-%m-%d")
        ),
    );
    dialogs::fact(ui, "SHA-256", c.sha256.clone());
    if let Some(why) = unusable(id) {
        ui.colored_label(ui.visuals().error_fg_color, why);
    }
}

fn error(ui: &mut Ui, e: &Option<String>) {
    if let Some(e) = e {
        ui.add(egui::Label::new(RichText::new(e).color(ui.visuals().error_fg_color)).wrap());
    }
}

/// A field of a form, with `label` above it ([`dialogs::field`]) and the field `add`
/// draws; and under it, `error` when it is about this `field`, also given to the field as
/// its description, so a screen reader says it with the field. The keyboard goes to the
/// field when `focus` is this `field`.
fn form_row(
    ui: &mut Ui,
    label: &str,
    field: Field,
    error: &Option<(Field, String)>,
    focus: &mut Option<Field>,
    add: impl FnOnce(&mut Ui) -> egui::Response,
) {
    dialogs::space_above(ui, dialogs::FIELD_GAP);
    let r = dialogs::field(ui, label, add);
    if *focus == Some(field) {
        *focus = None;
        r.request_focus();
    }
    if let Some((_, e)) = error.as_ref().filter(|(f, _)| *f == field) {
        r.ctx
            .accesskit_node_builder(r.id, |n| n.set_description(e.as_str()));
        let text = RichText::new(e).color(ui.visuals().error_fg_color);
        ui.add(egui::Label::new(text).wrap());
    }
}

/// A field in a form that may be left empty.
fn optional(ui: &mut Ui, text: &mut String) -> egui::Response {
    let edit = egui::TextEdit::singleline(text)
        .hint_text("Optional")
        .desired_width(f32::INFINITY);
    ui.add(theme::field(edit))
}

/// A form's error about no one field, shown at the form's end.
fn form_error(ui: &mut Ui, error: &Option<(Field, String)>) {
    let e = error.as_ref().filter(|(f, _)| *f == Field::Form);
    self::error(ui, &e.map(|(_, e)| e.clone()));
}

/// `e` as a sentence.
fn sentence(e: &anyhow::Error) -> String {
    format!("{}.", capital(format!("{e:#}").trim_end_matches('.')))
}

fn window<'a>(title: &'a str, id: &str) -> Shell<'a> {
    Shell::new(title).id(id).width(WIDTH + 2.0 * dialogs::SIDE)
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
    Reason,
    Location,
    /// No one field (the work failed): shown at the form's end.
    Form,
}

/// Whether `e` says the password was wrong (or the ID's file damaged: PKCS#12 can't
/// tell which).
fn wrong_password(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<WrongPassword>())
}

struct CreateForm {
    name: String,
    organisation: String,
    email: String,
    password: Secret,
    again: Secret,
    error: Option<(Field, String)>,
    /// The field the keyboard goes to when next drawn: the first one when the form opens
    /// (from a button that is gone), the password when it comes back after a failure.
    focus: Option<Field>,
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
    error: Option<(Field, String)>,
    /// The field the keyboard goes to when next drawn: the password.
    focus: Option<Field>,
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
    /// The field the keyboard goes to when next drawn: the ID's password after a failure.
    focus: Option<Field>,
}

enum Page {
    List,
    Create(CreateForm),
    Import(ImportForm),
    Export(Box<ExportForm>),
}

impl Page {
    /// The window's title while the page is shown: the task.
    fn title(&self) -> String {
        match self {
            Page::List => "Digital IDs".into(),
            Page::Create(_) => "Create a digital ID".into(),
            Page::Import(_) => "Import a digital ID".into(),
            Page::Export(f) => format!("Export \"{}\"", label(&f.id)),
        }
    }

    /// The form again after its work failed with `e`: the error beside the password when
    /// the password was wrong, otherwise at the form's end; the keyboard goes to the
    /// password, which has to be typed again.
    fn failed(mut self, e: &anyhow::Error) -> Page {
        let text = sentence(e);
        let wrong = wrong_password(e);
        let general = |again: &str| {
            let text = format!("{text} Type the {again} again to try again.");
            (Field::Form, text)
        };
        match &mut self {
            Page::List => {}
            Page::Create(f) => {
                // Making an ID opens no file of the signer's: its error is never the
                // password's, even when the ID read back wrong.
                f.error = Some(general("passwords"));
                f.focus = Some(Field::Password);
            }
            Page::Import(f) => {
                let password = (Field::Password, text.clone());
                f.error = Some(if wrong { password } else { general("password") });
                f.focus = Some(Field::Password);
            }
            Page::Export(f) => {
                let password = (Field::Password, text.clone());
                let again = if f.fresh { "passwords" } else { "password" };
                f.error = Some(if wrong { password } else { general(again) });
                f.focus = Some(Field::Password);
            }
        }
        self
    }
}

/// Work running in the Digital IDs window.
struct Task {
    /// What it is, for the window while it runs.
    what: &'static str,
    /// The form it was started from, without its passwords, shown again if it fails.
    form: Page,
    /// Its result: a sentence for the window.
    rx: Receiver<limarepdf_core::Result<String>>,
}

/// The Digital IDs window.
pub struct IdsWindow {
    store: Result<IdStore, String>,
    list: IdList,
    pick: usize,
    page: Page,
    task: Option<Task>,
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
        if let Some(r) = self.task.as_ref().and_then(|t| poll(&t.rx)) {
            let task = self.task.take().expect("the task polled");
            out.changed = true;
            self.reload(None);
            match r {
                Ok(text) => self.note = Some((true, text)),
                Err(e) => self.page = task.form.failed(&e),
            }
        }
        // Esc closes the list, not a form being filled in.
        let listed = matches!(self.page, Page::List) && self.task.is_none();
        let title = match &self.task {
            Some(t) => t.form.title(),
            None => self.page.title(),
        };
        let (_, esc) = window(&title, "digital ids").esc(listed).show(ctx, |ui| {
            if let Some(task) = &self.task {
                busy_ui(ui, task.what);
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
                    focus: Some(Field::Name),
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
                        focus: Some(Field::Password),
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
        dialogs::group(ui, "Your digital IDs", |ui| {
            if self.list.ids.is_empty() {
                ui.label("You have no digital IDs yet.");
            } else {
                dialogs::list_box(ui, |ui| {
                    for (i, id) in self.list.ids.iter().enumerate() {
                        let mut text = label(id);
                        if id.default {
                            text.push_str("  (default)");
                        }
                        if unusable(id).is_some() {
                            text.push_str("  (can't sign)");
                        }
                        let r = dialogs::list_pick(ui, self.pick == i, text);
                        if r.clicked() && self.pick != i {
                            self.pick = i;
                            self.remove_armed = false;
                        }
                    }
                });
            }
            for p in &self.list.problems {
                let text = RichText::new(display_line(p, 200)).small().weak();
                ui.add(egui::Label::new(text).wrap());
            }
        });
        if let Some(id) = self.list.ids.get(self.pick).cloned() {
            dialogs::group(ui, "Selected ID", |ui| {
                id_ui(ui, &id);
                ui.add_space(8.0);
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
                            focus: None,
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
            ui.add_space(dialogs::FIELD_GAP);
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
            ui.add(
                egui::Label::new(
                    "LimarePDF makes a self-signed ID (RSA 3072-bit, valid for five years). The \
                     name is what signatures show. Others see you as an unknown signer until \
                     they trust your certificate.",
                )
                .wrap(),
            );
            let (e, focus) = (&f.error, &mut f.focus);
            dialogs::group(ui, "About you", |ui| {
                form_row(ui, "Name", Field::Name, e, focus, |ui| {
                    let edit = egui::TextEdit::singleline(&mut f.name).desired_width(f32::INFINITY);
                    ui.add(theme::field(edit))
                });
                form_row(ui, "Organisation", Field::Organisation, e, focus, |ui| {
                    optional(ui, &mut f.organisation)
                });
                form_row(ui, "Email", Field::Email, e, focus, |ui| {
                    optional(ui, &mut f.email)
                });
            });
            dialogs::group(ui, "Password", |ui| {
                let at_least = format!("At least {} characters", limarepdf_core::MIN_PASSWORD);
                form_row(ui, "Password", Field::Password, e, focus, |ui| {
                    f.password.edit(ui, &at_least)
                });
                form_row(ui, "Confirm password", Field::Again, e, focus, |ui| {
                    f.again.edit(ui, "Type it again")
                });
            });
            ui.add_space(dialogs::FIELD_GAP);
            ui.add(
                egui::Label::new(
                    "Keep the password safe: LimarePDF can't recover it, and without it the ID \
                     can't sign. Keep a copy of the ID too (Export, with its key): if this \
                     computer is lost, so is the ID.",
                )
                .wrap(),
            );
            form_error(ui, &f.error);
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
            self.create(ui.ctx(), f);
            return;
        }
        self.page = Page::Create(f);
    }

    /// Runs `work` on the store in the background, keeping `form` to show again if it
    /// fails.
    fn start(
        &mut self,
        ctx: &Context,
        what: &'static str,
        form: Page,
        work: impl FnOnce(IdStore) -> limarepdf_core::Result<String> + Send + 'static,
    ) {
        match self.store.clone() {
            Ok(store) => {
                let rx = spawn(ctx, move || work(store));
                self.task = Some(Task { what, form, rx });
            }
            Err(e) => self.page = form.failed(&anyhow::anyhow!(e)),
        }
    }

    /// Starts making the ID the form describes.
    fn create(&mut self, ctx: &Context, mut f: CreateForm) {
        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        let new = NewId {
            name: f.name.trim().to_string(),
            organisation: opt(&f.organisation),
            email: opt(&f.email),
        };
        let password = std::mem::take(&mut f.password);
        f.again = Secret::default();
        let form = Page::Create(f);
        self.start(ctx, "Making the digital ID…", form, move |store| {
            let id = store.create(&new, &password.0)?;
            Ok(format!(
                "Made the digital ID \"{}\". It is the default for signing. Keep a copy: \
                 Export saves it with its key, for a backup or other apps.",
                label(&id)
            ))
        });
    }

    fn import_ui(&mut self, ui: &mut Ui, mut f: ImportForm) {
        dialogs::content(ui, |ui| {
            dialogs::fact(ui, "File", f.file.display().to_string());
            let (e, focus) = (&f.error, &mut f.focus);
            form_row(ui, "Password", Field::Password, e, focus, |ui| {
                f.password.edit(ui, "The file's password")
            });
            ui.add_space(dialogs::FIELD_GAP);
            ui.add(
                egui::Label::new(
                    "LimarePDF keeps its own copy, protected by the same password. The key must \
                     be RSA (2048 to 4096 bits) or ECDSA P-256 or P-384, and the certificate \
                     usable for signing.",
                )
                .wrap(),
            );
            form_error(ui, &f.error);
        });
        let hit = dialogs::footer(ui, None, &[Btn::primary("Import"), Btn::plain("Back")]);
        let (mut go, back) = (hit == Some(0), hit == Some(1));
        go |= ui.input(|i| i.key_pressed(egui::Key::Enter));
        if back {
            return;
        }
        if go {
            self.import(ui.ctx(), f);
            return;
        }
        self.page = Page::Import(f);
    }

    /// Starts importing the form's file with its password.
    fn import(&mut self, ctx: &Context, mut f: ImportForm) {
        let password = std::mem::take(&mut f.password);
        let file = f.file.clone();
        let form = Page::Import(f);
        self.start(ctx, "Importing the digital ID…", form, move |store| {
            use anyhow::Context as _;
            let size = std::fs::metadata(&file)
                .with_context(|| format!("reading {}", file.display()))?
                .len();
            if size > MAX_ID_FILE {
                anyhow::bail!("{} is too large to be a digital ID", file.display());
            }
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let id = store.import(&bytes, &password.0, None)?;
            Ok(format!(
                "Imported \"{}\". It is the default for signing. {} still holds the ID: keep \
                 it safe, or delete it if LimarePDF's copy is enough.",
                label(&id),
                file.display()
            ))
        });
    }

    fn export_ui(&mut self, ui: &mut Ui, mut f: Box<ExportForm>) {
        dialogs::content(ui, |ui| {
            dialogs::group(ui, "What to export", |ui| {
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
            });
            match f.kind {
                ExportKind::Certificate => {
                    ui.add_space(dialogs::FIELD_GAP);
                    ui.add(
                        egui::Label::new(
                            "The certificate holds no key, so it is safe to share. Tell \
                             recipients its SHA-256 fingerprint another way, such as by phone, \
                             so they can check it is yours.",
                        )
                        .wrap(),
                    );
                    ui.add_space(dialogs::FIELD_GAP);
                    dialogs::fact(ui, "SHA-256", f.id.cert.sha256.clone());
                }
                ExportKind::Key => {
                    let (e, focus) = (&f.error, &mut f.focus);
                    dialogs::group(ui, "Password", |ui| {
                        form_row(ui, "ID's password", Field::Password, e, focus, |ui| {
                            f.password.edit(ui, "Password")
                        });
                        ui.add_space(dialogs::FIELD_GAP);
                        ui.checkbox(&mut f.fresh, "Protect the file with a new password");
                        if f.fresh {
                            let at_least =
                                format!("At least {} characters", limarepdf_core::MIN_PASSWORD);
                            form_row(ui, "New password", Field::NewPassword, e, focus, |ui| {
                                f.new_password.edit(ui, &at_least)
                            });
                            form_row(ui, "Confirm password", Field::Again, e, focus, |ui| {
                                f.again.edit(ui, "Type it again")
                            });
                        }
                    });
                    ui.add_space(dialogs::FIELD_GAP);
                    ui.add(
                        egui::Label::new(
                            "Anyone with this file and its password can sign as you: keep it \
                             safe.",
                        )
                        .wrap(),
                    );
                }
            }
            form_error(ui, &f.error);
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
                        self.export_certificate(f, &path);
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
                    } else if let Some(path) = save_path(&f.id.name, "p12", "Digital ID") {
                        self.export_key(ui.ctx(), f, path);
                        return;
                    }
                }
            }
        }
        self.page = Page::Export(f);
    }

    /// Saves the form's certificate to `path`. The form stays, with the error, if that
    /// fails.
    fn export_certificate(&mut self, mut f: Box<ExportForm>, path: &Path) {
        match limarepdf_core::write_file(path, &f.id.cert.der) {
            Ok(()) => {
                self.note = Some((
                    true,
                    format!(
                        "Saved the certificate of \"{}\" to {}. Recipients trust it in \
                         LimarePDF with Edit > Trusted certificates > Add, or in their PDF \
                         app's trusted certificates, after checking the fingerprint with you.",
                        label(&f.id),
                        path.display()
                    ),
                ));
            }
            Err(e) => {
                f.error = Some((Field::Form, sentence(&e)));
                self.page = Page::Export(f);
            }
        }
    }

    /// Starts saving the form's ID with its key to `path`.
    fn export_key(&mut self, ctx: &Context, mut f: Box<ExportForm>, path: PathBuf) {
        let password = std::mem::take(&mut f.password);
        let new_password = std::mem::take(&mut f.new_password);
        f.again = Secret::default();
        let (id, fresh) = (f.id.clone(), f.fresh);
        let form = Page::Export(f);
        self.start(ctx, "Exporting the digital ID…", form, move |store| {
            let unlocked = store.unlock(&id, &password.0)?;
            let file_password = if fresh { &new_password } else { &password };
            let bytes = unlocked.to_p12(&file_password.0)?;
            limarepdf_core::write_private(&path, &bytes)?;
            Ok(format!(
                "Saved \"{}\" with its key to {}, protected by {}. Keep it safe.",
                label(&id),
                path.display(),
                if fresh {
                    "the new password"
                } else {
                    "the ID's password"
                }
            ))
        });
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
    /// A problem with the password, shown under it: missing, wrong, or to be typed again.
    field_error: Option<(Field, String)>,
    /// A problem with signing itself, shown at the form's end (the app sets some too).
    pub error: Option<String>,
    task: Option<Receiver<limarepdf_core::Result<Signed>>>,
    /// The field the keyboard goes to when next drawn: the password.
    focus: Option<Field>,
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
            field_error: None,
            error: None,
            task: None,
            focus: Some(Field::Password),
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

    /// The chosen ID can sign.
    fn can_sign(&self) -> bool {
        self.list
            .ids
            .get(self.pick)
            .is_some_and(|i| unusable(i).is_none())
    }

    /// Whether signing can go ahead: an ID that can sign, and its password, which is asked
    /// for beside it when missing.
    fn ready(&mut self) -> bool {
        if !self.can_sign() {
            return false;
        }
        if self.password.0.is_empty() {
            self.field_error = Some((Field::Password, "Type the ID's password.".into()));
            self.focus = Some(Field::Password);
            return false;
        }
        true
    }

    /// The signer agreed to sign although earlier signatures will report disallowed
    /// changes: the password, used up by the refused try, is typed again.
    fn sign_anyway(&mut self) {
        self.accept_disallowed = true;
        self.disallowed = None;
        let again = "Type the ID's password again to sign.";
        self.field_error = Some((Field::Password, again.into()));
        self.focus = Some(Field::Password);
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
        self.field_error = None;
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
                        None if wrong_password(&e) => {
                            self.field_error = Some((Field::Password, sentence(&e)));
                        }
                        None => {
                            let again = "Type the ID's password again to try again.";
                            self.error = Some(format!("{} {again}", sentence(&e)));
                        }
                    },
                }
                self.focus = Some(Field::Password);
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
        // Where the signature goes, under the title.
        let place = capital(&self.where_text);
        let (_, esc) = window(title, "digital sign")
            .file(Some(&place))
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
                        Some(0) => self.sign_anyway(),
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
        let can = self.can_sign();
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
        if matches!(out, SignOut::Go) && !self.ready() {
            return SignOut::Open;
        }
        out
    }

    /// The signing form: the ID, its password, and the options; `out` becomes Go on Enter
    /// in the password.
    fn form_body(&mut self, ui: &mut Ui, out: &mut SignOut) {
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
            let wide = ui.available_width();
            dialogs::field(ui, "Digital ID", |ui| {
                egui::ComboBox::from_id_salt("sign id")
                    .selected_text(shown)
                    .width(wide)
                    .show_ui(ui, |ui| {
                        for (i, id) in ids.iter().enumerate() {
                            let mut text = label(id);
                            if unusable(id).is_some() {
                                text.push_str(" (can't sign)");
                            }
                            ui.selectable_value(&mut self.pick, i, text);
                        }
                    })
                    .response
            });
            if let Some(id) = ids.get(self.pick) {
                ui.add_space(8.0);
                dialogs::list_box(ui, |ui| {
                    egui::Frame::NONE.inner_margin(4).show(ui, |ui| {
                        id_ui(ui, id);
                        if id.cert.self_issued {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(
                                        "Others see you as an unknown signer until they trust \
                                         your certificate: send it to them from Digital IDs \
                                         (Export, certificate only).",
                                    )
                                    .small()
                                    .weak(),
                                )
                                .wrap(),
                            );
                        }
                    });
                });
            }
            let (e, focus) = (&self.field_error, &mut self.focus);
            form_row(ui, "Password", Field::Password, e, focus, |ui| {
                let r = self.password.edit(ui, "The ID's password");
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    *out = SignOut::Go;
                }
                r
            });
            ui.add_space(dialogs::FIELD_GAP);
            let half = (ui.available_width() - dialogs::COLUMN_GAP) / 2.0;
            let mut both = (&mut self.reason, &mut self.location, focus);
            dialogs::columns(
                ui,
                &mut both,
                half,
                REASON_WIDTH,
                |ui, (reason, _, focus), _| {
                    form_row(ui, "Reason", Field::Reason, e, focus, |ui| {
                        optional(ui, reason)
                    });
                },
                |ui, (_, location, focus), _| {
                    form_row(ui, "Location", Field::Location, e, focus, |ui| {
                        optional(ui, location)
                    });
                },
            );
        }
        if self.certify {
            ui.add_space(dialogs::FIELD_GAP);
            ui.add(
                egui::Label::new(
                    "Certifying marks you as the document's author. Readers are told of any \
                     change beyond what you allow.",
                )
                .wrap(),
            );
            dialogs::group(ui, "Others may", |ui| {
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
            });
        }
        ui.add_space(dialogs::FIELD_GAP);
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
            focus: None,
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
        drawn(w).0
    }

    /// Draws the signing window as the app does, once its work (if any) is done; returns
    /// what it did and its widgets.
    fn drawn(w: &mut SignWindow) -> (SignOut, egui::accesskit::TreeUpdate) {
        if let Some(rx) = w.task.take() {
            let r = rx
                .recv_timeout(std::time::Duration::from_secs(120))
                .unwrap();
            let (tx, again) = channel();
            tx.send(r).unwrap();
            w.task = Some(again);
        }
        let ctx = Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let mut out = SignOut::Open;
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| out = w.ui(ui.ctx()));
        // Nothing draws the frame: its font atlas upload is dropped on purpose.
        frame.textures_delta.clear();
        assert!(!w.busy());
        (out, frame.platform_output.accesskit_update.unwrap())
    }

    #[test]
    fn password_problems_are_said_at_the_password() {
        let (mut w, dir) = window("password");
        let mut doc = Pdf::from_text("Hello", (612.0, 792.0)).unwrap();
        let base = doc.to_bytes().unwrap();
        let input = |out: &Path| Input {
            base: Arc::from(base.as_slice()),
            edited: None,
            art: None,
            out: out.to_path_buf(),
        };
        let ctx = Context::default();
        let out = dir.join("signed.pdf");
        w.reason = "Approved".into();
        for certify in [false, true] {
            w.certify = certify;
            // None typed: asked for beside it, with the keyboard there.
            assert!(!w.ready());
            let (_, tree) = drawn(&mut w);
            said_at(&tree, "Password", "Type the ID's password.");
            assert!(w.error.is_none());

            // A wrong one: said there too; it is typed again.
            w.password = Secret("not the password".into());
            assert!(w.ready());
            w.start(&ctx, input(&out));
            let (_, tree) = drawn(&mut w);
            let wrong = "The password is wrong, or the file is damaged.";
            said_at(&tree, "Password", wrong);
            assert!(w.password.0.is_empty() && w.error.is_none() && !out.exists());
            assert_eq!(w.reason, "Approved");
        }

        // A failure that isn't the password's: at the form's end, and the keyboard is in
        // Password, as the password is typed again.
        std::fs::write(dir.join("blocker"), "a file, not a folder").unwrap();
        w.password = Secret(PASSWORD.into());
        w.start(&ctx, input(&dir.join("blocker").join("signed.pdf")));
        let (_, tree) = drawn(&mut w);
        let e = w.error.clone().unwrap();
        assert!(
            e.ends_with("Type the ID's password again to try again."),
            "{e}"
        );
        assert!(w.field_error.is_none());
        let field = |name: &str| {
            tree.nodes
                .iter()
                .find(|(_, n)| n.label() == Some(name) && n.role() != egui::accesskit::Role::Label)
                .map(|(id, n)| (*id, n.description().is_none()))
        };
        assert_eq!(field("Password"), Some((tree.focus, true)));
        for name in ["Digital ID", "Reason", "Location"] {
            assert!(field(name).is_some(), "{name} isn't named");
        }
        let _ = std::fs::remove_dir_all(&dir);
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
        // Sign anyway: the password is asked for again, beside it.
        w.sign_anyway();
        let (_, tree) = drawn(&mut w);
        said_at(&tree, "Password", "Type the ID's password again to sign.");
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

    // ------------------------------------------------------------------ Digital IDs window

    /// A folder of its own for a test, with `blocker`, a file no folder can be made in.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("limarepdf-ids-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("blocker"), "a file, not a folder").unwrap();
        dir
    }

    /// A store in `dir` holding Jane Example's new ID.
    fn store_with_id(dir: &Path) -> (IdStore, DigitalId) {
        let store = IdStore::new(dir.join("ids"));
        let new = NewId {
            name: "Jane Example".into(),
            ..NewId::default()
        };
        let id = store.create(&new, PASSWORD).unwrap();
        (store, id)
    }

    fn ids_window(store: IdStore) -> IdsWindow {
        let mut w = IdsWindow {
            store: Ok(store),
            list: IdList::default(),
            pick: 0,
            page: Page::List,
            task: None,
            note: None,
            remove_armed: false,
        };
        w.reload(None);
        w
    }

    /// The running task's form, which must hold no passwords.
    fn running(w: &IdsWindow) -> &Page {
        &w.task.as_ref().expect("work started").form
    }

    /// Waits for the window's work, then draws it as the app does; returns its widgets.
    fn finish(w: &mut IdsWindow) -> egui::accesskit::TreeUpdate {
        let task = w.task.as_mut().expect("work started");
        let r = task
            .rx
            .recv_timeout(std::time::Duration::from_secs(120))
            .unwrap();
        let (tx, again) = channel();
        tx.send(r).unwrap();
        task.rx = again;
        let ctx = Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            w.ui(ui.ctx(), "");
        });
        frame.textures_delta.clear();
        assert!(w.task.is_none());
        frame.platform_output.accesskit_update.unwrap()
    }

    /// The form's error is shown with `label`'s field, which has the keyboard and says the
    /// error to a screen reader.
    fn said_at(tree: &egui::accesskit::TreeUpdate, label: &str, error: &str) {
        let (id, node) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.description() == Some(error))
            .unwrap_or_else(|| panic!("no field says {error:?}"));
        assert_eq!(node.label(), Some(label));
        assert_eq!(tree.focus, *id, "the keyboard is in {label}");
    }

    /// Draws the window as the app does; returns its widgets.
    fn shown(w: &mut IdsWindow) -> egui::accesskit::TreeUpdate {
        let ctx = Context::default();
        ctx.enable_accesskit();
        crate::theme::install(&ctx);
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            w.ui(ui.ctx(), "");
        });
        frame.textures_delta.clear();
        frame.platform_output.accesskit_update.unwrap()
    }

    /// G06: every control of the signing window, signing or certifying, and of the Digital
    /// IDs window's list and forms says what it is.
    #[test]
    fn digital_id_controls_are_named() {
        let (mut w, dir) = window("names");
        for certify in [false, true] {
            w.certify = certify;
            let (_, tree) = drawn(&mut w);
            let unclear = crate::dialogs::unclear_names(&tree);
            assert_eq!(unclear, [""; 0], "certifying: {certify}");
        }
        let _ = std::fs::remove_dir_all(&dir);

        let dir = scratch("names");
        let (store, id) = store_with_id(&dir);
        let mut w = ids_window(store);
        let create = CreateForm {
            name: "Jane Example".into(),
            organisation: String::new(),
            email: String::new(),
            password: Secret::default(),
            again: Secret::default(),
            error: None,
            focus: None,
        };
        let export = ExportForm {
            id,
            kind: ExportKind::Key,
            password: Secret::default(),
            fresh: true,
            new_password: Secret::default(),
            again: Secret::default(),
            error: None,
            focus: None,
        };
        let import = import_form(&dir.join("jane.p12"), "");
        for (what, page) in [
            ("the list", Page::List),
            ("a new ID", Page::Create(create)),
            ("importing", Page::Import(import)),
            ("exporting", Page::Export(Box::new(export))),
        ] {
            w.page = page;
            let unclear = crate::dialogs::unclear_names(&shown(&mut w));
            assert_eq!(unclear, [""; 0], "{what}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// G08: the Digital IDs window's list and forms, and Sign and Certify, on short and
    /// narrow screens: the window inside the screen with its buttons in view, and every
    /// control clearly named. While work runs, the window says what it is doing, offers no
    /// buttons (the work can't be stopped) and stays open on Esc; Esc closes the list and
    /// an empty signing window, never a form with something typed in.
    #[test]
    fn digital_windows_fit_short_screens_and_say_when_busy() {
        use crate::dialogs::{drawn_window, fits, unclear_names};
        let dir = scratch("short");
        let (store, id) = store_with_id(&dir);
        let mut w = ids_window(store);
        let sizes = [
            egui::vec2(1200.0, 300.0),
            egui::vec2(400.0, 300.0),
            egui::vec2(400.0, 800.0),
        ];
        let create = || CreateForm {
            name: "Jane Example".into(),
            organisation: "Example Ltd".into(),
            email: String::new(),
            password: Secret::default(),
            again: Secret::default(),
            error: Some((Field::Form, "Making the folder failed.".into())),
            focus: None,
        };
        for size in sizes {
            let export = ExportForm {
                id: id.clone(),
                kind: ExportKind::Key,
                password: Secret::default(),
                fresh: true,
                new_password: Secret::default(),
                again: Secret::default(),
                error: None,
                focus: None,
            };
            let import = import_form(&dir.join("jane.p12"), "");
            for (page, buttons) in [
                (Page::List, &["Create…", "Import…", "Close"][..]),
                (Page::Create(create()), &["Create", "Back"]),
                (Page::Import(import), &["Import", "Back"]),
                (Page::Export(Box::new(export)), &["Export…", "Back"]),
            ] {
                w.page = page;
                let (tree, window) = drawn_window(size, |ctx| {
                    w.ui(ctx, "");
                });
                fits(&tree, window, size, buttons).unwrap();
                assert_eq!(unclear_names(&tree), [""; 0], "{size:?}");
            }

            // Work running: what it is, no buttons, Esc ignored.
            let (_tx, rx) = channel();
            w.page = Page::List;
            w.task = Some(Task {
                what: "Making the digital ID…",
                form: Page::Create(create()),
                rx,
            });
            let (tree, window) = drawn_window(size, |ctx| {
                w.ui(ctx, "");
            });
            fits(&tree, window, size, &[]).unwrap();
            let busy = tree
                .nodes
                .iter()
                .any(|(_, n)| n.value() == Some("Making the digital ID…"));
            assert!(busy, "{size:?}: doesn't say what it is doing");
            let buttons = tree
                .nodes
                .iter()
                .filter(|(_, n)| n.role() == egui::accesskit::Role::Button)
                .count();
            assert_eq!(buttons, 0, "{size:?}: buttons while busy");
            assert!(
                !esc_closes(|ctx| w.ui(ctx, "").close),
                "{size:?}: Esc while busy"
            );
            w.task = None;
        }
        // Esc: the list closes; a form doesn't.
        w.page = Page::List;
        assert!(esc_closes(|ctx| w.ui(ctx, "").close));
        w.page = Page::Create(create());
        assert!(!esc_closes(|ctx| w.ui(ctx, "").close));
        let _ = std::fs::remove_dir_all(&dir);

        let (mut w, dir) = window("short");
        for size in sizes {
            for certify in [false, true] {
                w.certify = certify;
                let go = if certify { "Certify…" } else { "Sign…" };
                let (tree, window) = drawn_window(size, |ctx| {
                    w.ui(ctx);
                });
                fits(&tree, window, size, &[go, "Digital IDs…", "Cancel"])
                    .unwrap_or_else(|e| panic!("certify {certify}: {e}"));
                assert_eq!(unclear_names(&tree), [""; 0], "{size:?}");
            }
        }
        let cancelled = |w: &mut SignWindow| esc_closes(|ctx| matches!(w.ui(ctx), SignOut::Cancel));
        assert!(cancelled(&mut w), "Esc kept the empty window");
        w.reason = "Approved".into();
        assert!(
            !cancelled(&mut w),
            "Esc closed a window with a reason typed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Whether the window `show` draws (returning whether it closes) closes on Esc.
    fn esc_closes(mut show: impl FnMut(&Context) -> bool) -> bool {
        let ctx = Context::default();
        crate::theme::install(&ctx);
        let esc = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let mut closed = false;
        for events in [vec![], vec![], vec![], vec![], vec![esc]] {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            };
            ctx.run_ui(raw, |ui| closed = show(ui.ctx()))
                .textures_delta
                .clear();
        }
        closed
    }

    /// Batch 7b: the digital ID forms and the signing window label their fields above
    /// them, as Print does (the user, 2026-10-22: "above for consistency"), wide and
    /// narrow; a signature's Reason and Location side by side when there is room, and
    /// the window titled with the task.
    #[test]
    fn digital_id_forms_have_labels_above_their_fields() {
        use crate::dialogs::{drawn as settled, label_above};
        let dir = scratch("labels");
        let (store, id) = store_with_id(&dir);
        let mut w = ids_window(store);
        for size in [egui::vec2(1200.0, 900.0), egui::vec2(400.0, 900.0)] {
            let create = CreateForm {
                name: "Jane Example".into(),
                organisation: String::new(),
                email: String::new(),
                password: Secret::default(),
                again: Secret::default(),
                error: None,
                focus: None,
            };
            let export = ExportForm {
                id: id.clone(),
                kind: ExportKind::Key,
                password: Secret::default(),
                fresh: true,
                new_password: Secret::default(),
                again: Secret::default(),
                error: None,
                focus: None,
            };
            let import = import_form(&dir.join("jane.p12"), "");
            for (title, page, labels) in [
                (
                    "Create a digital ID",
                    Page::Create(create),
                    &[
                        "Name",
                        "Organisation",
                        "Email",
                        "Password",
                        "Confirm password",
                    ][..],
                ),
                ("Import a digital ID", Page::Import(import), &["Password"]),
                (
                    "Export \"Jane Example\"",
                    Page::Export(Box::new(export)),
                    &["ID's password", "New password", "Confirm password"],
                ),
            ] {
                w.page = page;
                let tree = settled(size, |ctx| {
                    w.ui(ctx, "");
                });
                let shown = tree.nodes.iter().any(|(_, n)| n.value() == Some(title));
                assert!(shown, "{size:?}: no title {title:?}");
                for l in labels {
                    label_above(&tree, l).unwrap_or_else(|e| panic!("{size:?}: {e}"));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);

        let (mut w, dir) = window("labels");
        for (size, beside) in [
            (egui::vec2(1200.0, 900.0), true),
            (egui::vec2(400.0, 900.0), false),
        ] {
            for certify in [false, true] {
                w.certify = certify;
                let tree = settled(size, |ctx| {
                    w.ui(ctx);
                });
                for l in ["Digital ID", "Password", "Reason", "Location"] {
                    label_above(&tree, l).unwrap_or_else(|e| panic!("{size:?}: {e}"));
                }
                let at = |name: &str| {
                    let (_, n) = tree
                        .nodes
                        .iter()
                        .find(|(_, n)| n.label() == Some(name))
                        .unwrap_or_else(|| panic!("no {name}"));
                    let b = n.bounds().unwrap();
                    egui::Rect::from_min_max(
                        egui::pos2(b.x0 as f32, b.y0 as f32),
                        egui::pos2(b.x1 as f32, b.y1 as f32),
                    )
                };
                let (reason, location) = (at("Reason"), at("Location"));
                if beside {
                    assert!((reason.top() - location.top()).abs() < 1.0, "{size:?}");
                    assert!(location.left() > reason.right(), "{size:?}");
                } else {
                    assert!(location.top() > reason.bottom(), "{size:?}");
                }
                let heading = |n: &egui::accesskit::Node| n.value() == Some("Others may");
                let shown = tree.nodes.iter().any(|(_, n)| heading(n));
                assert_eq!(shown, certify, "{size:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn import_form(file: &Path, password: &str) -> ImportForm {
        ImportForm {
            file: file.to_path_buf(),
            password: Secret(password.into()),
            error: None,
            focus: None,
        }
    }

    #[test]
    fn a_failed_import_comes_back_to_its_form() {
        let dir = scratch("import");
        let (store, id) = store_with_id(&dir.join("other"));
        let file = dir.join("jane.p12");
        let p12 = store
            .unlock(&id, PASSWORD)
            .unwrap()
            .to_p12(PASSWORD)
            .unwrap();
        std::fs::write(&file, &p12).unwrap();
        let mut w = ids_window(IdStore::new(dir.join("ids")));
        let ctx = Context::default();

        // A wrong password: said beside the password, which is to be typed again.
        w.import(&ctx, import_form(&file, "not the password"));
        let Page::Import(f) = running(&w) else {
            panic!("not importing")
        };
        assert!(
            f.password.0.is_empty(),
            "the password is handed over, not kept"
        );
        let tree = finish(&mut w);
        let Page::Import(f) = &w.page else {
            panic!("not back at the form")
        };
        assert_eq!(f.file, file);
        assert!(f.password.0.is_empty());
        let (field, text) = f.error.clone().unwrap();
        assert_eq!(field, Field::Password);
        assert!(text.starts_with("The password is wrong"), "{text}");
        said_at(&tree, "Password", &text);
        assert!(w.note.is_none() && w.list.ids.is_empty());

        // The file gone: said at the form's end, with what to do.
        std::fs::remove_file(&file).unwrap();
        w.page = Page::List;
        w.import(&ctx, import_form(&file, PASSWORD));
        finish(&mut w);
        let Page::Import(f) = &w.page else {
            panic!("not back at the form")
        };
        let (field, text) = f.error.clone().unwrap();
        assert_eq!(field, Field::Form);
        assert!(text.starts_with("Reading "), "{text}");
        assert!(
            text.ends_with("Type the password again to try again."),
            "{text}"
        );

        // Tried again: imported, and the list says so.
        std::fs::write(&file, &p12).unwrap();
        w.page = Page::List;
        w.import(&ctx, import_form(&file, PASSWORD));
        finish(&mut w);
        assert!(matches!(w.page, Page::List));
        let (ok, text) = w.note.clone().unwrap();
        assert!(
            ok && text.starts_with("Imported \"Jane Example\""),
            "{text}"
        );
        assert_eq!(w.list.ids.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_export_comes_back_to_its_form() {
        let dir = scratch("export");
        let (store, id) = store_with_id(&dir);
        let mut w = ids_window(store);
        let ctx = Context::default();
        const NEW: &str = "a new file password";
        let form = |kind, password: &str| {
            Box::new(ExportForm {
                id: id.clone(),
                kind,
                password: Secret(password.into()),
                fresh: true,
                new_password: Secret(NEW.into()),
                again: Secret(NEW.into()),
                error: None,
                focus: None,
            })
        };
        let out = dir.join("jane.p12");

        // A wrong password for the ID: said beside it; every password is to be typed again.
        w.export_key(&ctx, form(ExportKind::Key, "not the password"), out.clone());
        let Page::Export(f) = running(&w) else {
            panic!("not exporting")
        };
        assert!(f.password.0.is_empty() && f.new_password.0.is_empty() && f.again.0.is_empty());
        let tree = finish(&mut w);
        let Page::Export(f) = &w.page else {
            panic!("not back at the form")
        };
        assert!(f.kind == ExportKind::Key && f.fresh && f.id.name == id.name);
        assert!(f.password.0.is_empty() && f.new_password.0.is_empty() && f.again.0.is_empty());
        let (field, text) = f.error.clone().unwrap();
        assert_eq!(field, Field::Password);
        said_at(&tree, "ID's password", &text);
        assert!(!out.exists() && w.note.is_none());

        // Nowhere to write it: said at the form's end.
        let nowhere = dir.join("blocker").join("jane.p12");
        w.page = Page::List;
        w.export_key(&ctx, form(ExportKind::Key, PASSWORD), nowhere.clone());
        finish(&mut w);
        let Page::Export(f) = &w.page else {
            panic!("not back at the form")
        };
        let (field, text) = f.error.clone().unwrap();
        assert_eq!(field, Field::Form);
        assert!(
            text.ends_with("Type the passwords again to try again."),
            "{text}"
        );

        // The certificate alone, nowhere to write it: the form stays too.
        w.page = Page::List;
        w.export_certificate(form(ExportKind::Certificate, ""), &nowhere);
        let Page::Export(f) = &w.page else {
            panic!("not back at the form")
        };
        assert_eq!(f.error.as_ref().map(|e| e.0), Some(Field::Form));
        assert!(w.note.is_none());

        // Tried again: saved under the new password.
        w.page = Page::List;
        w.export_key(&ctx, form(ExportKind::Key, PASSWORD), out.clone());
        finish(&mut w);
        assert!(matches!(w.page, Page::List));
        let (ok, text) = w.note.clone().unwrap();
        assert!(ok && text.contains("the new password"), "{text}");
        let back = IdStore::new(dir.join("back"));
        back.import(&std::fs::read(&out).unwrap(), NEW, None)
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_new_id_comes_back_to_its_form() {
        let dir = scratch("create");
        // The store's folder can't be made: the ID is made, then can't be kept.
        let mut w = ids_window(IdStore::new(dir.join("blocker").join("ids")));
        let ctx = Context::default();
        let form = || CreateForm {
            name: "Jane Example".into(),
            organisation: "Example Ltd".into(),
            email: "jane@example.com".into(),
            password: Secret(PASSWORD.into()),
            again: Secret(PASSWORD.into()),
            error: None,
            focus: None,
        };
        w.create(&ctx, form());
        let Page::Create(f) = running(&w) else {
            panic!("not making an ID")
        };
        assert!(f.password.0.is_empty() && f.again.0.is_empty());
        let tree = finish(&mut w);
        let Page::Create(f) = &w.page else {
            panic!("not back at the form")
        };
        assert_eq!(
            (f.name.as_str(), f.organisation.as_str(), f.email.as_str()),
            ("Jane Example", "Example Ltd", "jane@example.com")
        );
        assert!(f.password.0.is_empty() && f.again.0.is_empty());
        let (field, text) = f.error.clone().unwrap();
        assert_eq!(field, Field::Form);
        assert!(text.starts_with("Making the folder "), "{text}");
        assert!(
            text.ends_with("Type the passwords again to try again."),
            "{text}"
        );
        // Shown at the form's end; the keyboard is in Password, to type it again.
        let password = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Password") && n.description().is_none())
            .map(|(id, _)| *id);
        assert_eq!(Some(tree.focus), password);
        assert!(w.note.is_none());

        // Tried again with somewhere to keep it: made.
        w.store = Ok(IdStore::new(dir.join("ids")));
        w.page = Page::List;
        w.create(&ctx, form());
        finish(&mut w);
        assert!(matches!(w.page, Page::List));
        let (ok, text) = w.note.clone().unwrap();
        assert!(ok && text.starts_with("Made the digital ID"), "{text}");
        assert_eq!(w.list.ids.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
