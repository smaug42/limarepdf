use crate::dialogs::{self as dlg, Outcome};
use crate::tab::{Side, Tab};
use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use mpe_core::{parse_page_range, Bates, Encryption, HeaderFooter, Pdf, Watermark};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

enum Loaded {
    Ok { path: PathBuf, pdf: Pdf, page: Option<usize> },
    NeedPassword { path: PathBuf, error: Option<String> },
    Err(String),
}

enum Dialog {
    None,
    Watermark(dlg::Wm),
    HeaderFooter(dlg::Hf),
    Meta(dlg::Meta),
    Security(dlg::Sec),
    Range(dlg::Range),
    Blank(dlg::Blank),
    Password(dlg::Password),
    Message(String, String),
    ConfirmClose(usize),
    ConfirmQuit,
    About,
}

#[derive(Clone, Copy)]
enum Act {
    Open,
    Save,
    SaveAs,
    CloseTab,
    Quit,
    Undo,
    Redo,
    Find,
    ZoomIn,
    ZoomOut,
    FitWidth,
    Actual,
    RotL,
    RotR,
    Delete,
    Extract,
    Blank,
    Merge,
    Split,
    HeaderFooter,
    Watermark,
    Props,
    Security,
    About,
    Theme(egui::ThemePreference),
}

pub struct App {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    dialog: Dialog,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    show_side: bool,
    status: String,
    last_title: String,
    quit_ok: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>, page: Option<usize>) -> Self {
        let (tx, rx) = channel();
        let app = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            dialog: Dialog::None,
            tx,
            rx,
            show_side: true,
            status: String::new(),
            last_title: String::new(),
            quit_ok: false,
        };
        for f in files {
            app.spawn_open(&cc.egui_ctx, f, None, page);
        }
        app
    }

    fn spawn_open(&self, ctx: &egui::Context, path: PathBuf, password: Option<String>, page: Option<usize>) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let msg = match Pdf::open(&path, password.as_deref()) {
                Ok(pdf) => Loaded::Ok { path, pdf, page },
                Err(e) => {
                    let s = format!("{e:#}");
                    if s.to_lowercase().contains("password") {
                        Loaded::NeedPassword { path, error: password.map(|_| "Incorrect password.".to_string()) }
                    } else {
                        Loaded::Err(format!("{}: {s}", path.display()))
                    }
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn add_tab(&mut self, ctx: &egui::Context, pdf: Pdf, path: Option<PathBuf>, page: Option<usize>, dirty: bool) {
        match Tab::new(ctx, self.next_id, pdf, path) {
            Ok(mut t) => {
                self.next_id += 1;
                t.dirty = dirty;
                if let Some(p) = page {
                    t.go_to(p.saturating_sub(1));
                }
                self.tabs.push(t);
                self.active = self.tabs.len() - 1;
            }
            Err(e) => self.dialog = Dialog::Message("Cannot open".into(), e),
        }
    }

    fn poll_loads(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Loaded::Ok { path, pdf, page } => self.add_tab(ctx, pdf, Some(path), page, false),
                Loaded::NeedPassword { path, error } => {
                    self.dialog = Dialog::Password(dlg::Password { path, input: String::new(), error });
                }
                Loaded::Err(e) => self.dialog = Dialog::Message("Cannot open file".into(), e),
            }
        }
    }

    fn pdf_dialog() -> rfd::FileDialog {
        rfd::FileDialog::new().add_filter("PDF documents", &["pdf"])
    }

    fn save(&mut self, as_new: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        let path = if as_new || tab.path.is_none() {
            let mut d = Self::pdf_dialog();
            if let Some(n) = tab.path.as_ref().and_then(|p| p.file_name()) {
                d = d.set_file_name(n.to_string_lossy());
            }
            match d.save_file() {
                Some(mut p) => {
                    if p.extension().is_none() {
                        p.set_extension("pdf");
                    }
                    p
                }
                None => return,
            }
        } else {
            tab.path.clone().unwrap()
        };
        tab.pdf.set_encryption(tab.encryption.clone());
        match tab.pdf.save(&path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                tab.path = Some(path);
                tab.dirty = false;
            }
            Err(e) => self.dialog = Dialog::Message("Save failed".into(), format!("{e:#}")),
        }
    }

    fn act(&mut self, ctx: &egui::Context, a: Act) {
        let has = !self.tabs.is_empty();
        match a {
            Act::Open => {
                if let Some(files) = Self::pdf_dialog().pick_files() {
                    for f in files {
                        self.spawn_open(ctx, f, None, None);
                    }
                }
            }
            Act::Save => self.save(false),
            Act::SaveAs => self.save(true),
            Act::CloseTab if has => {
                if self.tabs[self.active].dirty {
                    self.dialog = Dialog::ConfirmClose(self.active);
                } else {
                    self.close_tab(self.active);
                }
            }
            Act::Quit => {
                if self.tabs.iter().any(|t| t.dirty) {
                    self.dialog = Dialog::ConfirmQuit;
                } else {
                    self.quit_ok = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Act::About => self.dialog = Dialog::About,
            Act::Theme(p) => ctx.set_theme(p),
            _ if !has => {}
            Act::Undo => self.tabs[self.active].undo(ctx),
            Act::Redo => self.tabs[self.active].redo(ctx),
            Act::Find => {
                self.show_side = true;
                self.tabs[self.active].side = Side::Search;
            }
            Act::ZoomIn => {
                let t = &mut self.tabs[self.active];
                t.set_zoom(t.zoom * 1.2);
            }
            Act::ZoomOut => {
                let t = &mut self.tabs[self.active];
                t.set_zoom(t.zoom / 1.2);
            }
            Act::Actual => self.tabs[self.active].set_zoom(1.0),
            Act::FitWidth => {
                let w = ctx.content_rect().width() - if self.show_side { 190.0 } else { 0.0 };
                self.tabs[self.active].fit_width(w);
            }
            Act::RotL | Act::RotR => {
                let deg = if matches!(a, Act::RotR) { 90 } else { 270 };
                let t = &mut self.tabs[self.active];
                let idx = t.targets();
                t.edit(ctx, |p| p.rotate_pages(&idx, deg));
            }
            Act::Delete => self.dialog = Dialog::Range(dlg::Range { kind: dlg::RangeKind::Delete, text: self.default_range() }),
            Act::Extract => self.dialog = Dialog::Range(dlg::Range { kind: dlg::RangeKind::Extract, text: self.default_range() }),
            Act::Split => self.dialog = Dialog::Range(dlg::Range { kind: dlg::RangeKind::SplitEvery, text: "1".into() }),
            Act::Blank => {
                let t = &self.tabs[self.active];
                let (w, h) = t.sizes.get(t.current).copied().unwrap_or((612.0, 792.0));
                self.dialog = Dialog::Blank(dlg::Blank { after: t.current + 1, w, h });
            }
            Act::Merge => {
                let Some(files) = Self::pdf_dialog().pick_files() else { return };
                let mut docs = Vec::new();
                for f in &files {
                    match Pdf::open(f, None) {
                        Ok(d) => docs.push(d),
                        Err(e) => {
                            self.dialog = Dialog::Message("Merge failed".into(), format!("{}: {e:#}", f.display()));
                            return;
                        }
                    }
                }
                self.tabs[self.active].edit(ctx, |p| p.append(docs));
            }
            Act::HeaderFooter => self.dialog = Dialog::HeaderFooter(dlg::Hf::default()),
            Act::Watermark => self.dialog = Dialog::Watermark(dlg::Wm::default()),
            Act::Props => self.dialog = Dialog::Meta(dlg::Meta(self.tabs[self.active].pdf.info())),
            Act::Security => {
                let mut s = dlg::Sec::default();
                if let Some(e) = &self.tabs[self.active].encryption {
                    s = dlg::Sec { enabled: true, user: e.user_password.clone(), owner: e.owner_password.clone(), cipher: e.cipher, r: e.restrictions };
                }
                self.dialog = Dialog::Security(s);
            }
            _ => {}
        }
    }

    fn default_range(&self) -> String {
        let t = &self.tabs[self.active];
        let mut v = t.targets();
        v.sort_unstable();
        // Collapse consecutive runs into a-b.
        let mut out: Vec<String> = Vec::new();
        let mut i = 0;
        while i < v.len() {
            let mut j = i;
            while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
                j += 1;
            }
            out.push(if i == j { format!("{}", v[i] + 1) } else { format!("{}-{}", v[i] + 1, v[j] + 1) });
            i = j + 1;
        }
        out.join(",")
    }

    fn close_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.tabs.remove(i);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut dialog = std::mem::replace(&mut self.dialog, Dialog::None);
        let n = self.tabs.get(self.active).map(|t| t.page_count()).unwrap_or(0);
        let mut done = false;
        match &mut dialog {
            Dialog::None => done = true,
            Dialog::About => {
                done = dlg::message(
                    ctx,
                    "About MPE",
                    "MPE - a fast, pure-Rust PDF viewer and editor.\nLicensed under MIT OR Apache-2.0.\nRendering: hayro. Document engine: lopdf. UI: egui.",
                ) != Outcome::Open;
            }
            Dialog::Message(title, text) => done = dlg::message(ctx, title, text) != Outcome::Open,
            Dialog::ConfirmClose(i) => match dlg::confirm(ctx, "Unsaved changes", "This document has unsaved changes. Close it anyway?", "Discard changes") {
                Outcome::Apply => {
                    let i = *i;
                    self.close_tab(i);
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::ConfirmQuit => match dlg::confirm(ctx, "Unsaved changes", "Some documents have unsaved changes. Quit anyway?", "Quit without saving") {
                Outcome::Apply => {
                    self.quit_ok = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Password(s) => match dlg::password(ctx, s) {
                Outcome::Apply => {
                    self.spawn_open(ctx, s.path.clone(), Some(s.input.clone()), None);
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            _ if self.tabs.is_empty() => done = true,
            Dialog::Watermark(s) => match dlg::watermark(ctx, s) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    match parse_page_range(&s.range, t.page_count()) {
                        Ok(idx) => {
                            let wm = Watermark {
                                text: s.text.clone(),
                                font_size: s.size,
                                opacity: s.opacity,
                                rotation: s.rotation,
                                color: (s.color[0], s.color[1], s.color[2]),
                                foreground: s.foreground,
                            };
                            t.edit(ctx, |p| p.add_watermark(&idx, &wm));
                            done = true;
                        }
                        Err(e) => t.error = Some(e.to_string()),
                    }
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::HeaderFooter(s) => match dlg::header_footer(ctx, s) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    match parse_page_range(&s.range, t.page_count()) {
                        Ok(idx) => {
                            let hf = HeaderFooter {
                                header: s.header.clone(),
                                footer: s.footer.clone(),
                                font_size: s.size,
                                margin: s.margin,
                                color: (0.0, 0.0, 0.0),
                                bates: s.bates.then(|| Bates { prefix: s.prefix.clone(), suffix: s.suffix.clone(), digits: s.digits as usize, start: s.start }),
                            };
                            t.edit(ctx, |p| p.add_header_footer(&idx, &hf));
                            done = true;
                        }
                        Err(e) => t.error = Some(e.to_string()),
                    }
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Meta(s) => match dlg::metadata(ctx, s) {
                Outcome::Apply => {
                    let info = s.0.clone();
                    self.tabs[self.active].edit(ctx, |p| p.set_info(&info));
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Security(s) => match dlg::security(ctx, s) {
                Outcome::Apply => {
                    let t = &mut self.tabs[self.active];
                    t.encryption = s.enabled.then(|| Encryption {
                        user_password: s.user.clone(),
                        owner_password: if s.owner.is_empty() { s.user.clone() } else { s.owner.clone() },
                        cipher: s.cipher,
                        restrictions: s.r,
                    });
                    t.dirty = true;
                    self.status = "Security settings apply when you save.".into();
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Blank(s) => match dlg::blank(ctx, s, n) {
                Outcome::Apply => {
                    let (after, w, h) = (s.after, s.w, s.h);
                    self.tabs[self.active].edit(ctx, |p| p.insert_blank(after, w, h));
                    done = true;
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
            Dialog::Range(s) => match dlg::range(ctx, s) {
                Outcome::Apply => {
                    done = self.apply_range(ctx, s.kind, &s.text);
                }
                Outcome::Cancel => done = true,
                Outcome::Open => {}
            },
        }
        if !done {
            self.dialog = dialog;
        }
    }

    /// Returns true when the dialog should close.
    fn apply_range(&mut self, ctx: &egui::Context, kind: dlg::RangeKind, text: &str) -> bool {
        let t = &mut self.tabs[self.active];
        match kind {
            dlg::RangeKind::SplitEvery => {
                let Ok(every) = text.trim().parse::<usize>() else {
                    t.error = Some("Enter a whole number of pages per file.".into());
                    return false;
                };
                let Some(dir) = rfd::FileDialog::new().set_title("Choose output folder").pick_folder() else { return true };
                let stem = t.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "part".into());
                match t.pdf.split_every(every) {
                    Ok(parts) => {
                        let count = parts.len();
                        for (i, mut part) in parts.into_iter().enumerate() {
                            if let Err(e) = part.save(dir.join(format!("{stem}-{:03}.pdf", i + 1))) {
                                self.dialog = Dialog::Message("Split failed".into(), format!("{e:#}"));
                                return true;
                            }
                        }
                        self.status = format!("Wrote {count} files to {}", dir.display());
                    }
                    Err(e) => t.error = Some(format!("{e:#}")),
                }
                true
            }
            _ => {
                let idx = match parse_page_range(text, t.page_count()) {
                    Ok(v) => v,
                    Err(e) => {
                        t.error = Some(e.to_string());
                        return false;
                    }
                };
                if kind == dlg::RangeKind::Delete {
                    t.edit(ctx, |p| p.delete_pages(&idx));
                } else {
                    match t.pdf.extract(&idx) {
                        Ok(p) => self.add_tab(ctx, p, None, None, true),
                        Err(e) => t.error = Some(format!("{e:#}")),
                    }
                }
                true
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let mut acts = Vec::new();
        ctx.input_mut(|i| {
            let c = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
            let cs = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
            for (sc, a) in [
                (c(Key::O), Act::Open),
                (cs(Key::S), Act::SaveAs),
                (c(Key::S), Act::Save),
                (c(Key::W), Act::CloseTab),
                (cs(Key::Z), Act::Redo),
                (c(Key::Z), Act::Undo),
                (c(Key::Y), Act::Redo),
                (c(Key::F), Act::Find),
                (c(Key::Num0), Act::FitWidth),
                (c(Key::Num1), Act::Actual),
                (c(Key::Equals), Act::ZoomIn),
                (c(Key::Minus), Act::ZoomOut),
            ] {
                if i.consume_shortcut(&sc) {
                    acts.push(a);
                }
            }
        });
        for a in acts {
            self.act(ctx, a);
        }
        if ctx.egui_wants_keyboard_input() || self.tabs.is_empty() {
            return;
        }
        let (down, up, home, end) = ctx.input(|i| {
            (i.key_pressed(Key::PageDown), i.key_pressed(Key::PageUp), i.key_pressed(Key::Home), i.key_pressed(Key::End))
        });
        let t = &mut self.tabs[self.active];
        let last = t.page_count().saturating_sub(1);
        if down {
            t.go_to((t.current + 1).min(last));
        }
        if up {
            t.go_to(t.current.saturating_sub(1));
        }
        if home {
            t.go_to(0);
        }
        if end {
            t.go_to(last);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_loads(&ctx);

        // Files dropped onto the window.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            self.spawn_open(&ctx, p, None, None);
        }

        // Guard against closing the window with unsaved work.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_ok && self.tabs.iter().any(|t| t.dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.dialog = Dialog::ConfirmQuit;
        }

        self.shortcuts(&ctx);

        let mut acts: Vec<Act> = Vec::new();
        let has = !self.tabs.is_empty();
        let (can_undo, can_redo) = self.tabs.get(self.active).map(|t| (t.can_undo(), t.can_redo())).unwrap_or((false, false));

        egui::Panel::top("menu").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                let mut item = |ui: &mut egui::Ui, label: &str, key: &str, enabled: bool, act: Act| {
                    let b = egui::Button::new(label).shortcut_text(key);
                    if ui.add_enabled(enabled, b).clicked() {
                        acts.push(act);
                        ui.close();
                    }
                };
                ui.menu_button("File", |ui| {
                    item(ui, "Open…", "Ctrl+O", true, Act::Open);
                    item(ui, "Save", "Ctrl+S", has, Act::Save);
                    item(ui, "Save As…", "Ctrl+Shift+S", has, Act::SaveAs);
                    item(ui, "Close tab", "Ctrl+W", has, Act::CloseTab);
                    ui.separator();
                    item(ui, "Quit", "", true, Act::Quit);
                });
                ui.menu_button("Edit", |ui| {
                    item(ui, "Undo", "Ctrl+Z", can_undo, Act::Undo);
                    item(ui, "Redo", "Ctrl+Y", can_redo, Act::Redo);
                    ui.separator();
                    item(ui, "Find…", "Ctrl+F", has, Act::Find);
                });
                ui.menu_button("View", |ui| {
                    item(ui, "Zoom in", "Ctrl+=", has, Act::ZoomIn);
                    item(ui, "Zoom out", "Ctrl+-", has, Act::ZoomOut);
                    item(ui, "Fit width", "Ctrl+0", has, Act::FitWidth);
                    item(ui, "Actual size", "Ctrl+1", has, Act::Actual);
                    ui.separator();
                    ui.checkbox(&mut self.show_side, "Side panel");
                    ui.menu_button("Theme", |ui| {
                        item(ui, "System", "", true, Act::Theme(egui::ThemePreference::System));
                        item(ui, "Light", "", true, Act::Theme(egui::ThemePreference::Light));
                        item(ui, "Dark", "", true, Act::Theme(egui::ThemePreference::Dark));
                    });
                });
                ui.menu_button("Document", |ui| {
                    item(ui, "Rotate clockwise", "", has, Act::RotR);
                    item(ui, "Rotate counter-clockwise", "", has, Act::RotL);
                    item(ui, "Delete pages…", "", has, Act::Delete);
                    item(ui, "Extract pages to new tab…", "", has, Act::Extract);
                    item(ui, "Insert blank page…", "", has, Act::Blank);
                    item(ui, "Append PDFs…", "", has, Act::Merge);
                    item(ui, "Split into files…", "", has, Act::Split);
                    ui.separator();
                    item(ui, "Headers, footers, Bates…", "", has, Act::HeaderFooter);
                    item(ui, "Watermark…", "", has, Act::Watermark);
                    ui.separator();
                    item(ui, "Properties…", "", has, Act::Props);
                    item(ui, "Password security…", "", has, Act::Security);
                });
                ui.menu_button("Help", |ui| item(ui, "About", "", true, Act::About));
            });
        });

        if has {
            egui::Panel::top("tabs").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut close = None;
                    for i in 0..self.tabs.len() {
                        if ui.selectable_label(i == self.active, self.tabs[i].title()).clicked() {
                            self.active = i;
                        }
                        if ui.small_button("×").clicked() {
                            close = Some(i);
                        }
                        ui.add_space(6.0);
                    }
                    if let Some(i) = close {
                        self.active = i;
                        acts.push(Act::CloseTab);
                    }
                });
            });
        }

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(t) = self.tabs.get(self.active) {
                    ui.label(format!("Page {} of {}", t.current + 1, t.page_count()));
                    ui.separator();
                    ui.label(format!("{:.0}%", t.zoom * 100.0));
                    if t.encryption.is_some() {
                        ui.separator();
                        ui.label("will be encrypted on save");
                    }
                }
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(&self.status);
                }
            });
        });

        if has && self.show_side {
            let active = self.active;
            egui::Panel::left("side").resizable(true).default_size(170.0).show(ui, |ui| {
                let t = &mut self.tabs[active];
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut t.side, Side::Pages, "Pages");
                    ui.selectable_value(&mut t.side, Side::Search, "Search");
                });
                ui.separator();
                match t.side {
                    Side::Pages => t.show_thumbs(ui),
                    Side::Search => {
                        let r = ui.text_edit_singleline(&mut t.query);
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.horizontal(|ui| {
                            if ui.button("Find").clicked() || enter {
                                t.run_search();
                            }
                            if ui.button("◀").clicked() {
                                t.step_hit(-1);
                            }
                            if ui.button("▶").clicked() {
                                t.step_hit(1);
                            }
                        });
                        ui.label(format!("{} match(es)", t.hits.len()));
                        let mut jump = None;
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for (i, h) in t.hits.iter().enumerate() {
                                let text = format!("p{}  {}", h.page + 1, h.snippet);
                                if ui.selectable_label(i == t.hit_idx, text).clicked() {
                                    jump = Some(i);
                                }
                            }
                        });
                        if let Some(i) = jump {
                            t.hit_idx = i;
                            let p = t.hits[i].page;
                            t.go_to(p);
                        }
                    }
                }
            });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(t) = self.tabs.get_mut(self.active) {
                t.show_pages(ui);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Open a PDF with File ▸ Open, Ctrl+O, or drop a file here.");
                });
            }
        });

        // Surface errors raised by tab operations.
        if let Some(t) = self.tabs.get_mut(self.active) {
            if let Some(e) = t.error.take() {
                self.dialog = Dialog::Message("Error".into(), e);
            }
        }

        for a in acts {
            self.act(&ctx, a);
        }
        self.dialogs(&ctx);

        let title = match self.tabs.get(self.active) {
            Some(t) => format!("{} - MPE", t.title()),
            None => "MPE".into(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }
}
