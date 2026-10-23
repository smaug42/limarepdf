//! Digital signatures in the app: checking a signed document in the background when it
//! opens (and again after it is saved), the banner, the Signatures side tab, the Details
//! window, and trusting signers and certificate authorities. The checking itself is
//! limarepdf-core's [`limarepdf_core::verify_signatures`]; see docs/signatures.md.
//!
//! Everything shown here that comes from the file (names, reasons, field names) is
//! cleaned for display first.

use crate::banners::{self, Banner};
use crate::dialogs::{self, fact, Btn, Outcome, Shell};
use crate::theme::{self, Tone};
use eframe::egui::{self, Context, RichText, Ui};
use egui_phosphor::regular as icon;
use limarepdf_core::{
    display_line, CertInfo, DigitalSignature, FieldLock, LockAction, SigKind, SigStatus,
    SignatureReport, TimeSource, Trust, TrustList, TrustRole,
};
use std::path::PathBuf;
use std::sync::Arc;

/// How times are shown: 2026-10-16 14:23:05 -04:00, with the offset the time was given in.
const TIME: &str = "%Y-%m-%d %H:%M:%S %:z";
pub const TRUST_NOTE: &str = "Trust settings affect LimarePDF only.";
/// Changes listed under a signature in the side tab; Details lists them all.
const PANEL_CHANGES: usize = 5;

fn clean(s: &str) -> String {
    display_line(s, 80)
}

fn clean_long(s: &str) -> String {
    display_line(s, 300)
}

// ------------------------------------------------------------------ checking

/// A signed document's signature check, kept with the tab.
pub struct SigState {
    /// The file as opened or last saved: what was checked, and where View signed version
    /// takes its bytes from.
    pub bytes: Arc<[u8]>,
    /// The password the file was opened with, for an encrypted file.
    pub password: Option<String>,
    pub check: Check,
    /// Counts the checks started for the tab, so a late result for an older file is
    /// dropped.
    pub generation: u64,
}

pub enum Check {
    Pending,
    Done(Box<Checked>),
    /// The file couldn't be checked at all, with why.
    Failed(String),
}

pub struct Checked {
    pub report: SignatureReport,
    pub trust: TrustStatus,
}

/// Whether LimarePDF's trust list could be used, and entries in it that couldn't.
#[derive(Clone, Default)]
pub struct TrustStatus {
    /// The list couldn't be read (or other users could change it), so nobody is trusted.
    pub error: Option<String>,
    /// Entries left out, with why.
    pub problems: Vec<String>,
}

/// LimarePDF's trust list from its settings folder, read now so changes made with `limarepdf-cli trust`
/// count; an empty list when it can't be used, with why.
pub fn load_trust() -> (TrustList, TrustStatus) {
    let Some(path) = TrustList::default_path() else {
        let error = "LimarePDF can't find its settings folder, so it has no trust list".to_string();
        return (
            TrustList::default(),
            TrustStatus {
                error: Some(error),
                problems: Vec::new(),
            },
        );
    };
    match TrustList::load(&path) {
        Ok(list) => {
            let problems = list.problems.clone();
            (
                list,
                TrustStatus {
                    error: None,
                    problems,
                },
            )
        }
        Err(e) => (
            TrustList::default(),
            TrustStatus {
                error: Some(format!("{e:#}")),
                problems: Vec::new(),
            },
        ),
    }
}

/// Checks the signatures in `bytes` (a whole file); run off the UI thread.
pub fn run(bytes: &[u8], password: Option<&str>) -> Check {
    let (list, trust) = load_trust();
    match crate::guard(|| limarepdf_core::verify_signatures(bytes, password, &list)) {
        Ok(report) => Check::Done(Box::new(Checked { report, trust })),
        Err(e) => Check::Failed(format!("{e:#}")),
    }
}

// ------------------------------------------------------------------ verdicts

/// How a signature checked, from best to worst.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    /// Valid, only allowed changes since, a trusted signer and no warnings.
    Good,
    /// Valid with only allowed changes, but the signer isn't confirmed or there are
    /// warnings.
    Caution,
    /// LimarePDF can't check it.
    CantCheck,
    /// Broken, changed in ways the signer didn't allow, or a trusted certificate that
    /// fails a check.
    Bad,
}

impl Level {
    pub fn tone(self) -> Tone {
        match self {
            Level::Good => Tone::Good,
            Level::Caution => Tone::Caution,
            Level::CantCheck => Tone::Neutral,
            Level::Bad => Tone::Bad,
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Level::Good => icon::SEAL_CHECK,
            Level::Caution => icon::SEAL_QUESTION,
            Level::CantCheck => icon::SEAL,
            Level::Bad => icon::SEAL_WARNING,
        }
    }
}

pub fn level(s: &DigitalSignature) -> Level {
    match &s.status {
        SigStatus::Broken(_) => Level::Bad,
        SigStatus::CantCheck(_) => Level::CantCheck,
        SigStatus::Valid if !s.changes_allowed() => Level::Bad,
        SigStatus::Valid => match &s.trust {
            Some(Trust::Failed(_)) => Level::Bad,
            Some(t) if t.is_trusted() && s.warnings.is_empty() => Level::Good,
            _ => Level::Caution,
        },
    }
}

/// The signer's (or timestamp authority's) name, cleaned for display.
fn who(s: &DigitalSignature) -> String {
    s.signer
        .as_ref()
        .map_or_else(|| "the signer".into(), |c| clean(c.display_name()))
}

/// One or two sentences on how a signature checked, as the design's status table words
/// them.
pub fn summary(s: &DigitalSignature) -> String {
    let who = who(s);
    let stamp = s.kind == SigKind::DocumentTimestamp;
    let mut text = match &s.status {
        SigStatus::Broken(why) => format!("This signature is broken: {}.", clean_long(why)),
        SigStatus::CantCheck(why) => {
            format!("LimarePDF can't check this signature: {}.", clean_long(why))
        }
        SigStatus::Valid => {
            if let Some(c) = s.changes.iter().find(|c| !c.allowed()) {
                let what = clean_long(&c.to_string());
                return if stamp {
                    format!("Changed after the timestamp in ways that aren't allowed: {what}.")
                } else {
                    format!("Changed after signing in ways the signer didn't allow: {what}.")
                };
            }
            let since = if s.changes.is_empty() {
                "Not changed"
            } else {
                "Only allowed changes"
            };
            if stamp {
                let when = s
                    .time
                    .map(|t| format!(" at {}", t.time.format(TIME)))
                    .unwrap_or_default();
                match &s.trust {
                    Some(t) if t.is_trusted() => format!(
                        "Document timestamp by {who}: the document existed like this{when}. \
                         {since} since."
                    ),
                    Some(Trust::Failed(why)) => format!(
                        "Document timestamp by {who}, but LimarePDF doesn't trust the authority's \
                         certificate: {}.",
                        clean_long(why)
                    ),
                    _ => format!(
                        "Document timestamp by {who}{when}. {since} since. LimarePDF can't confirm \
                         this timestamp authority."
                    ),
                }
            } else {
                match &s.trust {
                    Some(t) if t.is_trusted() => {
                        let since = if s.changes.is_empty() {
                            "No changes"
                        } else {
                            "Only allowed changes"
                        };
                        format!("Signed by {who}. {since} since.")
                    }
                    Some(Trust::Failed(why)) => format!(
                        "{since} since {who} signed it, but LimarePDF doesn't trust the certificate: {}.",
                        clean_long(why)
                    ),
                    _ => format!("{since} since {who} signed it. LimarePDF can't confirm who {who} is."),
                }
            }
        }
    };
    if s.status == SigStatus::Valid && !s.warnings.is_empty() {
        text.push_str(if s.warnings.len() == 1 {
            " There is a warning."
        } else {
            " There are warnings."
        });
    }
    text
}

/// What the signature is, with its number and who made it, for headings.
fn title(i: usize, s: &DigitalSignature) -> String {
    let who = who(s);
    match s.kind {
        SigKind::Approval => format!("{}. {who}", i + 1),
        SigKind::Certification(_) => format!("{}. Certified by {who}", i + 1),
        SigKind::DocumentTimestamp => format!("{}. Timestamp by {who}", i + 1),
    }
}

/// The banner for a check: `None` when the document has no signatures (empty signature
/// fields alone get none).
pub fn banner(check: &Check) -> Option<Banner> {
    let (tone, text) = match check {
        Check::Pending => (
            Tone::Neutral,
            "Checking the digital signatures…".to_string(),
        ),
        Check::Failed(e) => (
            Tone::Neutral,
            format!(
                "LimarePDF can't check the digital signatures: {}.",
                clean_long(e)
            ),
        ),
        Check::Done(c) => {
            let sigs = &c.report.signatures;
            let worst = sigs.iter().map(level).max()?;
            let n = sigs.len();
            let count = |l: Level| sigs.iter().filter(|s| level(s) == l).count();
            let body = if n == 1 {
                summary(&sigs[0])
            } else {
                match worst {
                    Level::Good => "Signed, and all signatures are valid.".to_string(),
                    Level::Caution
                        if sigs
                            .iter()
                            .any(|s| !s.trust.as_ref().is_some_and(Trust::is_trusted)) =>
                    {
                        format!(
                            "None of the {n} signatures is broken and nothing changed that the \
                             signers didn't allow, but LimarePDF can't confirm who all the signers are."
                        )
                    }
                    Level::Caution => {
                        "Signed, and all signatures are valid, with warnings.".to_string()
                    }
                    Level::CantCheck => format!(
                        "LimarePDF can't check {} of the {n} signatures.",
                        count(Level::CantCheck)
                    ),
                    Level::Bad => {
                        let broken = sigs
                            .iter()
                            .filter(|s| matches!(s.status, SigStatus::Broken(_)))
                            .count();
                        if broken > 0 {
                            format!("{broken} of the {n} signatures are broken.")
                        } else if sigs.iter().any(|s| !s.changes_allowed()) {
                            "Changed after signing in ways a signer didn't allow.".to_string()
                        } else {
                            "A signer's certificate fails LimarePDF's checks, so it isn't trusted."
                                .to_string()
                        }
                    }
                }
            };
            let text = match c.report.certification() {
                Some((s, p)) => format!("Certified by {} ({p}). {body}", who(s)),
                None => body,
            };
            (worst.tone(), text)
        }
    };
    Some(banners::signatures(tone, text))
}

/// Whether a signer is trusted, and why, in one line (as `limarepdf-cli verify` puts it).
fn trust_text(t: &Trust) -> String {
    match t {
        Trust::Signer { label } => format!("Trusted: you trust \"{}\" in LimarePDF.", clean(label)),
        Trust::Authority { label, chain } => {
            let path: Vec<String> = chain.iter().map(|c| clean(c.display_name())).collect();
            format!(
                "Trusted: issued under \"{}\", a certificate authority you trust in LimarePDF ({}).",
                clean(label),
                path.join(" < ")
            )
        }
        Trust::Unknown(why) => format!("Not confirmed: {}.", clean_long(why)),
        Trust::Failed(why) => format!("NOT TRUSTED: {}.", clean_long(why)),
    }
}

fn lock_text(l: &FieldLock) -> String {
    let names = || {
        l.fields
            .iter()
            .map(|f| format!("\"{}\"", clean(f)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let fields = match l.action {
        LockAction::All => "every form field".into(),
        LockAction::Include => format!("the fields {}", names()),
        LockAction::Exclude => format!("every form field except {}", names()),
    };
    match l.permissions {
        Some(p) => format!("{fields}; afterwards {p}"),
        None => fields,
    }
}

fn change_line(c: &limarepdf_core::Change) -> String {
    let what = clean_long(&c.to_string());
    match &c.refused {
        None => format!("Allowed: {what}"),
        Some(r) => format!("Not allowed: {what} ({r})"),
    }
}

/// A coloured icon for a tone.
fn tone_icon(ui: &Ui, glyph: &str, tone: Tone) -> RichText {
    let t = theme::icon(glyph).size(16.0);
    match theme::tone(ui.visuals().dark_mode, tone) {
        Some(c) => t.color(c),
        None => t,
    }
}

/// The certificates that could be trusted as an authority from a signature: those it
/// carries that are certificate authorities.
pub fn authorities(s: &DigitalSignature) -> Vec<CertInfo> {
    s.certificates
        .iter()
        .filter(|c| limarepdf_core::is_authority(c))
        .cloned()
        .collect()
}

// ------------------------------------------------------------------ side tab

/// What a click in the Signatures tab asks for.
pub enum PanelAct {
    Details(usize),
    SignedVersion(usize),
    /// Show this page, this far down (points).
    GoTo(usize, f32),
    Trusted,
}

/// The Signatures side tab. `dirty`: the document has unsaved changes, which the check
/// (of the file) doesn't include.
pub fn panel(ui: &mut Ui, state: Option<&SigState>, dirty: bool) -> Option<PanelAct> {
    let mut out = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match state.map(|s| &s.check) {
                None => {
                    ui.weak("This document has no digital signatures.");
                }
                Some(Check::Pending) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Checking the digital signatures…");
                    });
                }
                Some(Check::Failed(e)) => {
                    ui.add(
                        egui::Label::new(format!(
                            "LimarePDF can't check the digital signatures: {}.",
                            clean_long(e)
                        ))
                        .wrap(),
                    );
                }
                Some(Check::Done(c)) => out = report_ui(ui, c, dirty),
            }
            ui.add_space(8.0);
            if ui
                .button("Trusted certificates…")
                .on_hover_text("The signers and certificate authorities you trust in LimarePDF.")
                .clicked()
            {
                out = Some(PanelAct::Trusted);
            }
        });
    out
}

/// What the Signatures tab lists for a document restored from a recovery copy: each signed
/// field, never a verdict, since the signatures cover the original file, not the copy.
pub fn recovered_rows(pdf: &limarepdf_core::Pdf) -> Vec<String> {
    let empty: Vec<String> = pdf
        .empty_signature_fields()
        .into_iter()
        .map(|f| f.name)
        .collect();
    pdf.form_fields()
        .into_iter()
        .filter(|f| f.kind == limarepdf_core::FieldKind::Signature && !empty.contains(&f.name))
        .map(|f| format!("{}: Not checked: recovered copy", clean(&f.name)))
        .collect()
}

/// The Signatures tab of a restored copy, listing `rows` ([`recovered_rows`]).
pub fn recovered_panel(ui: &mut Ui, rows: &[String]) -> Option<PanelAct> {
    let mut out = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.weak("This document has no digital signatures.");
            } else {
                ui.add(
                    egui::Label::new(
                        "This is a recovered copy. Its digital signatures cover the original \
                         file, not this copy, so they aren't checked here. Open the original to \
                         check them.",
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
                for r in rows {
                    ui.horizontal(|ui| {
                        ui.label(theme::icon(icon::SEAL_QUESTION));
                        ui.add(egui::Label::new(r).wrap());
                    });
                }
            }
            ui.add_space(8.0);
            if ui.button("Trusted certificates…").clicked() {
                out = Some(PanelAct::Trusted);
            }
        });
    out
}

fn report_ui(ui: &mut Ui, c: &Checked, dirty: bool) -> Option<PanelAct> {
    let mut out = None;
    let r = &c.report;
    if r.signatures.is_empty() && r.empty_fields.is_empty() {
        ui.weak("This document has no digital signatures.");
        return None;
    }
    if dirty {
        ui.add(
            egui::Label::new(
                RichText::new(
                    "This shows the file as last saved: your unsaved changes aren't checked.",
                )
                .weak(),
            )
            .wrap(),
        );
        ui.add_space(4.0);
    }
    trust_status_ui(ui, &c.trust);
    if let Some((s, p)) = r.certification() {
        ui.horizontal_top(|ui| {
            ui.label(tone_icon(ui, icon::CERTIFICATE, level(s).tone()));
            ui.add(egui::Label::new(format!("Certified by {}: {p}.", who(s))).wrap());
        });
        ui.add_space(4.0);
    }
    for (i, s) in r.signatures.iter().enumerate() {
        ui.add_space(4.0);
        let lv = level(s);
        ui.horizontal_top(|ui| {
            ui.label(tone_icon(ui, lv.icon(), lv.tone()));
            ui.vertical(|ui| {
                let head =
                    egui::Button::selectable(false, RichText::new(title(i, s)).strong()).wrap();
                let tip = match s.page {
                    Some(p) => format!("Go to the signature on page {}", p + 1),
                    None => "Show the details".to_string(),
                };
                if ui.add(head).on_hover_text(tip).clicked() {
                    out = Some(match (s.page, s.bbox) {
                        (Some(p), Some(b)) => PanelAct::GoTo(p, b[1]),
                        _ => PanelAct::Details(i),
                    });
                }
                ui.add(egui::Label::new(summary(s)).wrap());
                if !s.changes.is_empty() {
                    ui.add_space(2.0);
                    ui.label(RichText::new("Changes after signing:").small().strong());
                    for ch in s.changes.iter().take(PANEL_CHANGES) {
                        let mut t = RichText::new(change_line(ch)).small();
                        if !ch.allowed() {
                            if let Some(col) = theme::tone(ui.visuals().dark_mode, Tone::Bad) {
                                t = t.color(col);
                            }
                        }
                        ui.add(egui::Label::new(t).wrap());
                    }
                    if s.changes.len() > PANEL_CHANGES {
                        ui.label(
                            RichText::new(format!(
                                "and {} more (see Details)",
                                s.changes.len() - PANEL_CHANGES
                            ))
                            .small()
                            .weak(),
                        );
                    }
                }
                for w in &s.warnings {
                    let w = clean_long(&w.to_string());
                    ui.add(
                        egui::Label::new(RichText::new(format!("Warning: {w}.")).small()).wrap(),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Details…").clicked() {
                        out = Some(PanelAct::Details(i));
                    }
                    if let Some(why) = signed_version_blocked(s) {
                        ui.add_enabled(false, egui::Button::new("Signed version").small())
                            .on_disabled_hover_text(why);
                    } else if ui
                        .small_button("Signed version")
                        .on_hover_text("Open the document as it was when this was signed.")
                        .clicked()
                    {
                        out = Some(PanelAct::SignedVersion(i));
                    }
                });
            });
        });
    }
    if !r.empty_fields.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Empty signature fields").small().strong());
        for f in &r.empty_fields {
            let page = f
                .page
                .map(|p| format!(" on page {}", p + 1))
                .unwrap_or_default();
            let row =
                egui::Button::selectable(false, format!("\"{}\"{page}", clean(&f.name))).wrap();
            let mut resp = ui.add(row);
            if let Some(l) = &f.lock {
                resp = resp.on_hover_text(format!("Signing it locks {}.", lock_text(l)));
            }
            if resp.clicked() {
                if let (Some(p), Some(b)) = (f.page, f.bbox) {
                    out = Some(PanelAct::GoTo(p, b[1]));
                }
            }
        }
    }
    out
}

/// Why View signed version isn't offered for `s`, if it isn't.
fn signed_version_blocked(s: &DigitalSignature) -> Option<&'static str> {
    match (&s.status, s.signed_len) {
        (SigStatus::Broken(_), _) => {
            Some("The signature is broken, so it has no genuine signed version.")
        }
        (_, None) => Some("LimarePDF can't tell which version this signed."),
        _ => None,
    }
}

/// The trust list's problems, when there are any.
fn trust_status_ui(ui: &mut Ui, t: &TrustStatus) {
    let caution = theme::tone(ui.visuals().dark_mode, Tone::Caution);
    let say = |ui: &mut Ui, text: String| {
        let mut t = RichText::new(text);
        if let Some(c) = caution {
            t = t.color(c);
        }
        ui.add(egui::Label::new(t).wrap());
    };
    if let Some(e) = &t.error {
        say(
            ui,
            format!(
                "LimarePDF isn't using its trust list, so no signer is trusted: {}.",
                clean_long(e)
            ),
        );
        ui.add_space(4.0);
    }
    for p in &t.problems {
        say(ui, format!("Trust list entry left out: {}.", clean_long(p)));
    }
}

// ------------------------------------------------------------------ details

/// What the Details window asks for.
#[derive(Default)]
pub struct DetailsOut {
    pub close: bool,
    pub trust_signer: bool,
    pub trust_authority: bool,
    pub signed_version: bool,
    pub go_to: Option<(usize, f32)>,
}

/// The details of signature `i` of `c`'s report, as a window.
pub fn details(ctx: &Context, doc: &str, c: &Checked, i: usize) -> DetailsOut {
    let mut out = DetailsOut::default();
    let Some(s) = c.report.signatures.get(i) else {
        out.close = true;
        return out;
    };
    let n = c.report.signatures.len();
    let which = format!("{doc}: signature {} of {n}", i + 1);
    let shell = Shell::new("Signature details")
        .id("signature details")
        .file(Some(&which))
        .width(540.0);
    let (_, esc) = shell.show(ctx, |ui| {
        dialogs::content(ui, |ui| {
            let lv = level(s);
            ui.horizontal_top(|ui| {
                ui.label(tone_icon(ui, lv.icon(), lv.tone()));
                ui.add(egui::Label::new(RichText::new(summary(s)).strong()).wrap());
            });
            details_body(ui, c, s);
            // What can be done with it, after a rule, as Advanced is in Print.
            ui.add_space(20.0);
            ui.add(egui::Separator::default().spacing(0.0));
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                let valid = s.status == SigStatus::Valid;
                let trusted_alone = matches!(s.trust, Some(Trust::Signer { .. }));
                let who = if s.kind == SigKind::DocumentTimestamp {
                    "Trust this authority…"
                } else {
                    "Trust this signer…"
                };
                if s.signer.is_some() && !trusted_alone {
                    let b = ui
                        .add_enabled(valid, egui::Button::new(who))
                        .on_hover_text("Trust the certificate this was signed with, in LimarePDF.")
                        .on_disabled_hover_text(
                            "LimarePDF only trusts the signer of a valid signature.",
                        );
                    if b.clicked() {
                        out.trust_signer = true;
                    }
                }
                if !authorities(s).is_empty() {
                    let b = ui
                        .add_enabled(valid, egui::Button::new("Trust an authority…"))
                        .on_hover_text(
                            "Trust a certificate authority from this signature's certificates, \
                             so everyone it issued certificates to is trusted in LimarePDF.",
                        )
                        .on_disabled_hover_text(
                            "LimarePDF only trusts authorities from a valid signature.",
                        );
                    if b.clicked() {
                        out.trust_authority = true;
                    }
                }
                match signed_version_blocked(s) {
                    Some(why) => {
                        ui.add_enabled(false, egui::Button::new("View signed version"))
                            .on_disabled_hover_text(why);
                    }
                    None => {
                        if ui
                            .button("View signed version")
                            .on_hover_text("Open the document as it was when this was signed.")
                            .clicked()
                        {
                            out.signed_version = true;
                        }
                    }
                }
                if let (Some(p), Some(b)) = (s.page, s.bbox) {
                    // Closes the window, which would hide the field.
                    if ui.button("Go to field").clicked() {
                        out.go_to = Some((p, b[1]));
                        out.close = true;
                    }
                }
            });
        });
        if dialogs::footer(ui, None, &[Btn::plain("Close")]).is_some() {
            out.close = true;
        }
    });
    if esc {
        out.close = true;
    }
    out
}

fn details_body(ui: &mut Ui, c: &Checked, s: &DigitalSignature) {
    trust_status_ui(ui, &c.trust);
    let kind = match s.kind {
        SigKind::Approval => "Signature".to_string(),
        SigKind::Certification(p) => format!("Certification: {p}"),
        SigKind::DocumentTimestamp => {
            "Document timestamp (a timestamp authority's proof that the document existed then)"
                .to_string()
        }
    };
    dialogs::group(ui, "Signature", |ui| signature_facts(ui, c, s, kind));
    signer_facts(ui, s);
    time_facts(ui, s);
    let claims: Vec<(&str, String)> = [
        ("Name", &s.name),
        ("Reason", &s.reason),
        ("Location", &s.location),
        ("Contact", &s.contact),
    ]
    .into_iter()
    .filter_map(|(l, v)| {
        v.as_deref()
            .filter(|v| !v.trim().is_empty())
            .map(|v| (l, clean_long(v)))
    })
    .collect();
    if !claims.is_empty() {
        dialogs::group(ui, "In the signer's own words", |ui| {
            ui.label(RichText::new("Nobody checks these.").small().weak());
            for (l, v) in claims {
                fact(ui, l, v);
            }
        });
    }
    if !s.changes.is_empty() {
        dialogs::group(ui, "Changes after signing", |ui| {
            let bad = theme::tone(ui.visuals().dark_mode, Tone::Bad);
            for ch in &s.changes {
                let mut t = RichText::new(change_line(ch));
                if let (false, Some(col)) = (ch.allowed(), bad) {
                    t = t.color(col);
                }
                ui.add(egui::Label::new(t).wrap());
            }
        });
    }
    if !s.warnings.is_empty() {
        dialogs::group(ui, "Warnings", |ui| {
            for w in &s.warnings {
                let w = clean_long(&w.to_string());
                ui.add(egui::Label::new(format!("The signature {w}.")).wrap());
            }
        });
    }
    if !s.certificates.is_empty() {
        ui.add_space(dialogs::FIELD_GAP);
        egui::CollapsingHeader::new(format!(
            "Certificates in the signature ({})",
            s.certificates.len()
        ))
        .id_salt("sig certs")
        .show(ui, |ui| {
            for cert in &s.certificates {
                cert_ui(ui, cert);
                ui.add_space(8.0);
            }
        });
    }
    ui.add_space(dialogs::FIELD_GAP);
    ui.add(
        egui::Label::new(
            RichText::new(
                "LimarePDF doesn't check whether certificates were revoked (withdrawn by their \
                 issuer), which Acrobat may do online.",
            )
            .small()
            .weak(),
        )
        .wrap(),
    );
}

/// What the signature is, where, whether its signer is trusted, and what it covers.
fn signature_facts(ui: &mut Ui, c: &Checked, s: &DigitalSignature, kind: String) {
    let stamp = s.kind == SigKind::DocumentTimestamp;
    fact(ui, "Kind", kind);
    let fields: Vec<String> = s
        .fields
        .iter()
        .map(|f| format!("\"{}\"", clean(f)))
        .collect();
    let page = s
        .page
        .map(|p| format!(" on page {}", p + 1))
        .unwrap_or_default();
    fact(ui, "Field", format!("{}{page}", fields.join(", ")));
    if let Some(t) = &s.trust {
        let mut line = trust_text(t);
        if matches!(t, Trust::Unknown(_)) {
            let self_made = s.signer.as_ref().is_some_and(|c| c.self_issued);
            line.push_str(if stamp {
                " Trust the authority to confirm its timestamps."
            } else if self_made {
                " To trust this signer, compare the SHA-256 fingerprint below with them in a \
                 way you trust (in person, by phone), then choose Trust this signer."
            } else {
                " To trust the authority that issued the certificate, choose Trust an \
                 authority; or trust this signer alone."
            });
        }
        fact(ui, "Trust", line);
    }
    if let Some(len) = s.signed_len {
        let what = if s.covers_whole_file {
            "the whole file".to_string()
        } else {
            format!(
                "{len} of {} bytes; the rest was added later",
                c.report.file_len
            )
        };
        fact(ui, "Signed part", what);
    }
    if let Some(l) = &s.locks {
        fact(ui, "Locked", lock_text(l));
    }
}

/// Who signed, or which authority stamped: their certificate and how it signed.
fn signer_facts(ui: &mut Ui, s: &DigitalSignature) {
    let stamp = s.kind == SigKind::DocumentTimestamp;
    let mut alg: Vec<String> = Vec::new();
    if let Some(c) = &s.signer {
        alg.push(c.key.clone());
    }
    alg.extend(s.algorithm.map(String::from));
    alg.extend(s.digest.map(String::from));
    if !s.sub_filter.is_empty() {
        alg.push(clean(&s.sub_filter));
    }
    if s.signer.is_none() && alg.is_empty() {
        return;
    }
    dialogs::group(ui, if stamp { "Authority" } else { "Signer" }, |ui| {
        if let Some(cert) = &s.signer {
            signer_cert_facts(ui, cert, stamp);
        }
        if !alg.is_empty() {
            fact(ui, "Algorithm", alg.join(", "));
        }
    });
}

/// The signer's certificate: who it names, who issued it, and which it is.
fn signer_cert_facts(ui: &mut Ui, cert: &CertInfo, stamp: bool) {
    let mut name = clean(cert.display_name());
    if let Some(e) = cert
        .email
        .as_deref()
        .filter(|e| Some(*e) != cert.common_name.as_deref())
    {
        name.push_str(&format!(" <{}>", clean(e)));
    }
    if let Some(o) = cert
        .organisation
        .as_deref()
        .filter(|o| Some(*o) != cert.common_name.as_deref())
    {
        name.push_str(&format!(", {}", clean(o)));
    }
    fact(ui, if stamp { "Authority" } else { "Signed by" }, name);
    let issuer = if cert.self_issued {
        "itself (a self-made digital ID)".into()
    } else {
        clean(cert.issuer_common_name.as_deref().unwrap_or(&cert.issuer))
    };
    fact(ui, "Issued by", issuer);
    fact(
        ui,
        "Certificate",
        format!(
            "serial {}, valid {} to {}",
            cert.serial,
            cert.not_before.format("%Y-%m-%d"),
            cert.not_after.format("%Y-%m-%d")
        ),
    );
    fact(ui, "SHA-256", cert.sha256.clone());
}

/// When it was signed, and the timestamp that says so.
fn time_facts(ui: &mut Ui, s: &DigitalSignature) {
    let stamp = s.kind == SigKind::DocumentTimestamp;
    let ts = s.timestamp.as_ref().filter(|_| !stamp);
    if s.time.is_none() && ts.is_none() {
        return;
    }
    dialogs::group(ui, "Time", |ui| {
        if let Some(t) = &s.time {
            let from = match t.source {
                TimeSource::Timestamp => "from a timestamp authority",
                TimeSource::SignerClock | TimeSource::Dictionary => "from the signer's computer",
            };
            fact(
                ui,
                if stamp { "Time" } else { "Signed at" },
                format!("{} ({from})", t.time.format(TIME)),
            );
        }
        if let Some(ts) = ts {
            let trusted = match &ts.trust {
                Some(t) if t.is_trusted() => " (trusted)".to_string(),
                Some(Trust::Failed(why)) => format!(" (NOT TRUSTED: {})", clean_long(why)),
                _ => " (LimarePDF can't confirm this authority)".to_string(),
            };
            fact(
                ui,
                "Timestamp",
                format!(
                    "{} by {}{trusted}",
                    ts.time.format(TIME),
                    clean(ts.authority.display_name())
                ),
            );
        }
    });
}

/// A certificate's subject, issuer, validity and fingerprint.
fn cert_ui(ui: &mut Ui, c: &CertInfo) {
    let mut name = clean(&c.subject);
    if limarepdf_core::is_authority(c) {
        name.push_str(" (certificate authority)");
    }
    ui.add(egui::Label::new(RichText::new(name).strong()).wrap());
    let issuer = if c.self_issued {
        "itself".to_string()
    } else {
        clean(&c.issuer)
    };
    fact(ui, "Issued by", issuer);
    fact(
        ui,
        "Valid",
        format!(
            "{} to {}{}",
            c.not_before.format("%Y-%m-%d"),
            c.not_after.format("%Y-%m-%d"),
            if c.valid_now() {
                ""
            } else {
                " (not valid today)"
            }
        ),
    );
    fact(ui, "Key", c.key.clone());
    fact(ui, "SHA-256", c.sha256.clone());
}

// ------------------------------------------------------------------ trusting

/// Asking before trusting a certificate.
pub struct TrustAsk {
    pub role: TrustRole,
    /// The certificates to choose from: one, unless an authority is picked from a
    /// signature's certificates or a file holds several.
    pub choices: Vec<CertInfo>,
    pub pick: usize,
    /// The name to keep it under; follows the pick until edited.
    pub label: String,
    label_edited: bool,
    /// The role can be chosen (a certificate file, rather than a signature).
    pub choose_role: bool,
    pub error: Option<String>,
}

impl TrustAsk {
    pub fn new(role: TrustRole, choices: Vec<CertInfo>, choose_role: bool) -> Self {
        let label = choices
            .first()
            .map(|c| clean(c.display_name()))
            .unwrap_or_default();
        Self {
            role,
            choices,
            pick: 0,
            label,
            label_edited: false,
            choose_role,
            error: None,
        }
    }

    pub fn cert(&self) -> Option<&CertInfo> {
        self.choices.get(self.pick)
    }

    /// The window; `Apply` means trust [`TrustAsk::cert`] as [`TrustAsk::role`].
    pub fn ui(&mut self, ctx: &Context) -> Outcome {
        let title = match self.role {
            TrustRole::Signer => "Trust a signer",
            TrustRole::Authority => "Trust a certificate authority",
        };
        let shell = Shell::new(title)
            .id("trust ask")
            .width(480.0)
            .esc(!self.label_edited);
        let (out, esc) = shell.show(ctx, |ui| {
            dialogs::content(ui, |ui| self.body(ui));
            match dialogs::footer(ui, None, &[Btn::primary("Trust"), Btn::cancel()]) {
                Some(0) => Outcome::Apply,
                Some(_) => Outcome::Cancel,
                None => Outcome::Open,
            }
        });
        if esc {
            return Outcome::Cancel;
        }
        out
    }

    /// What is trusted and how, the certificate, and the name it gets in the list.
    fn body(&mut self, ui: &mut Ui) {
        if self.choose_role {
            let ca = self.cert().is_some_and(limarepdf_core::is_authority);
            dialogs::group(ui, "Trust it as", |ui| {
                ui.radio_value(
                    &mut self.role,
                    TrustRole::Signer,
                    "A signer: signatures made with this certificate",
                );
                ui.add_enabled_ui(ca, |ui| {
                    ui.radio_value(
                        &mut self.role,
                        TrustRole::Authority,
                        "A certificate authority: also everyone it issued certificates to",
                    )
                    .on_disabled_hover_text("This certificate isn't a certificate authority.");
                });
            });
            if !ca {
                self.role = TrustRole::Signer;
            }
        }
        dialogs::group(ui, "Certificate", |ui| {
            if self.choices.len() > 1 {
                let before = self.pick;
                for (k, c) in self.choices.iter().enumerate() {
                    ui.radio_value(&mut self.pick, k, clean(&c.subject));
                }
                if self.pick != before && !self.label_edited {
                    self.label = self
                        .cert()
                        .map(|c| clean(c.display_name()))
                        .unwrap_or_default();
                }
                ui.add_space(dialogs::FIELD_GAP);
            }
            if let Some(c) = self.cert() {
                cert_ui(ui, c);
            }
        });
        ui.add_space(dialogs::FIELD_GAP);
        ui.add(
            egui::Label::new(
                "Anyone can make a certificate with any name. Before trusting it, check this \
                 SHA-256 fingerprint with its owner in a way you trust, such as in person or by \
                 phone.",
            )
            .wrap(),
        );
        if self.role == TrustRole::Authority {
            ui.add(
                egui::Label::new(
                    "Everyone this authority issued a certificate to, directly or through \
                     authorities it allowed to, will show as trusted.",
                )
                .wrap(),
            );
        }
        if self.cert().is_some_and(|c| !c.valid_now()) {
            ui.add(
                egui::Label::new(
                    "The certificate isn't valid today: only signatures made while it was valid \
                     will be trusted.",
                )
                .wrap(),
            );
        }
        ui.add_space(20.0);
        let r = dialogs::field(ui, "Name in your list", |ui| {
            let edit = egui::TextEdit::singleline(&mut self.label).desired_width(f32::INFINITY);
            ui.add(theme::field(edit))
        });
        if r.changed() {
            self.label_edited = true;
        }
        if let Some(e) = &self.error {
            ui.add(egui::Label::new(RichText::new(e).color(ui.visuals().error_fg_color)).wrap());
        }
        ui.add_space(dialogs::FIELD_GAP);
        ui.label(RichText::new(TRUST_NOTE).small().weak());
    }
}

/// The Trusted certificates window: LimarePDF's trust list, to look at and change.
pub struct TrustedWindow {
    pub list: TrustList,
    pub status: TrustStatus,
    pub path: Option<PathBuf>,
    /// The entry whose Remove was clicked, waiting for a second click.
    removing: Option<usize>,
}

pub enum TrustedOut {
    Open,
    Close,
    /// Add a certificate from a file.
    Add,
    Remove(usize),
}

impl TrustedWindow {
    pub fn load() -> Self {
        let (list, status) = load_trust();
        Self {
            list,
            status,
            path: TrustList::default_path(),
            removing: None,
        }
    }

    pub fn ui(&mut self, ctx: &Context) -> TrustedOut {
        let mut out = TrustedOut::Open;
        let shell = Shell::new("Trusted certificates")
            .id("trusted certificates")
            .width(520.0);
        let (_, esc) = shell.show(ctx, |ui| {
            dialogs::content(ui, |ui| self.body(ui, &mut out));
            let usable = self.status.error.is_none();
            let add = Btn::plain("Add certificate file…").enabled(usable).tip(
                "Trust a certificate from a file (.cer, .crt, .pem, .der, .p7c, .p7b), such as \
                 one a signer or an authority sent you.",
            );
            match dialogs::footer(ui, None, &[add, Btn::plain("Close")]) {
                Some(0) => out = TrustedOut::Add,
                Some(_) => out = TrustedOut::Close,
                None => {}
            }
        });
        if esc {
            out = TrustedOut::Close;
        }
        out
    }

    /// The list, each entry with Remove (Stop trusting once clicked), and where it is kept.
    fn body(&mut self, ui: &mut Ui, out: &mut TrustedOut) {
        ui.add(
            egui::Label::new(
                "LimarePDF shows a signature as trusted when its signer is in this list, or when \
                 an authority in this list issued the signer's certificate.",
            )
            .wrap(),
        );
        trust_status_ui(ui, &self.status);
        let usable = self.status.error.is_none();
        ui.add_space(20.0);
        if self.list.entries.is_empty() {
            ui.weak("You don't trust any certificates in LimarePDF yet.");
        }
        let entries = &self.list.entries;
        let removing = &mut self.removing;
        dialogs::list_rows(ui, entries.len(), |ui, i| {
            let e = &entries[i];
            let text = |ui: &mut Ui| {
                let role = match e.role {
                    TrustRole::Signer => "Signer",
                    TrustRole::Authority => "Certificate authority",
                };
                ui.add(egui::Label::new(RichText::new(clean(&e.label)).strong()).wrap());
                let added = e
                    .added
                    .map(|d| format!(", trusted {d}"))
                    .unwrap_or_default();
                ui.label(RichText::new(format!("{role}{added}")).small());
                ui.add(
                    egui::Label::new(RichText::new(clean(&e.cert.subject)).small().weak()).wrap(),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("SHA-256 {}", e.cert.sha256))
                            .small()
                            .weak(),
                    )
                    .wrap(),
                );
            };
            // Named for screen readers with the certificate each acts on.
            let who = clean(&e.label);
            dialogs::list_row(ui, text, |ui| {
                if *removing == Some(i) {
                    let r = ui.button("Stop trusting");
                    dialogs::set_name(&r, &format!("Stop trusting {who}"));
                    if r.clicked() {
                        *out = TrustedOut::Remove(i);
                    }
                    let r = ui.button("Keep");
                    dialogs::set_name(&r, &format!("Keep trusting {who}"));
                    if r.clicked() {
                        *removing = None;
                    }
                } else {
                    let r = ui.add_enabled(usable, egui::Button::new("Remove…"));
                    dialogs::set_name(&r, &format!("Remove {who}…"));
                    if r.clicked() {
                        *removing = Some(i);
                    }
                }
            });
        });
        ui.add_space(dialogs::FIELD_GAP);
        if let Some(p) = &self.path {
            ui.label(
                RichText::new(format!("{TRUST_NOTE} Kept in {}.", p.display()))
                    .small()
                    .weak(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use limarepdf_core::{Change, ChangeKind, Pdf, Refusal, SigWarning};

    /// A committed sample signed file (pyHanko and node-signpdf, MIT).
    fn sample(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../limarepdf-core/tests/data/signatures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn report(name: &str, trust: &TrustList) -> SignatureReport {
        limarepdf_core::verify_signatures(&sample(name), None, trust).unwrap()
    }

    /// The text of the banner for a report.
    fn banner_of(r: SignatureReport) -> Banner {
        banner(&checked(r)).expect("a banner")
    }

    const SELF_MADE: &str = "node-signpdf/signed.pdf";

    #[test]
    fn unknown_signer_is_caution() {
        let r = report(SELF_MADE, &TrustList::default());
        let s = &r.signatures[0];
        assert_eq!(level(s), Level::Caution);
        assert_eq!(
            summary(s),
            "Not changed since signpdf signed it. LimarePDF can't confirm who signpdf is."
        );
        let b = banner_of(r);
        assert_eq!(b.tone, Tone::Caution);
        assert!(
            b.text.starts_with("Not changed since signpdf"),
            "{}",
            b.text
        );
        assert_eq!(
            b.action().map(|a| a.1),
            Some(crate::actions::Act::Signatures)
        );
    }

    #[test]
    fn trusted_signer_is_good_until_warned() {
        let mut r = report(SELF_MADE, &TrustList::default());
        let mut list = TrustList::default();
        let der = r.signatures[0].signer.as_ref().unwrap().der.clone();
        list.add(&der, TrustRole::Signer, Some("Sig")).unwrap();
        r.apply_trust(&list);
        let s = &r.signatures[0];
        assert_eq!(level(s), Level::Good);
        assert_eq!(summary(s), "Signed by signpdf. No changes since.");
        let b = banner_of(r.clone());
        assert_eq!((b.tone, b.icon()), (Tone::Good, icon::SEAL_CHECK));
        // A warning keeps it from green, even when trusted.
        r.signatures[0]
            .warnings
            .push(SigWarning::WeakDigest("SHA-1"));
        assert_eq!(level(&r.signatures[0]), Level::Caution);
        assert!(summary(&r.signatures[0]).ends_with("There is a warning."));
    }

    #[test]
    fn broken_is_bad() {
        let r = report("pyhanko/one-byterange.pdf", &TrustList::default());
        let s = &r.signatures[0];
        assert_eq!(level(s), Level::Bad);
        assert_eq!(
            summary(s),
            "This signature is broken: the byte range must be exactly four whole numbers."
        );
        assert_eq!(banner_of(r).tone, Tone::Bad);
    }

    #[test]
    fn a_changed_byte_breaks_it() {
        let mut bytes = sample(SELF_MADE);
        // A space in the signed part becomes a tab: the file still reads the same.
        let range = bytes.windows(10).position(|w| w == b"/ByteRange").unwrap();
        let at = range + bytes[range..].iter().position(|&b| b == b']').unwrap() + 1;
        assert_eq!(bytes[at], b' ');
        bytes[at] = b'\t';
        let r = limarepdf_core::verify_signatures(&bytes, None, &TrustList::default()).unwrap();
        assert_eq!(level(&r.signatures[0]), Level::Bad);
        assert!(summary(&r.signatures[0]).starts_with("This signature is broken"));
    }

    #[test]
    fn disallowed_changes_and_failed_trust_are_bad() {
        let mut r = report(SELF_MADE, &TrustList::default());
        r.signatures[0].changes.push(Change {
            kind: ChangeKind::PageContent,
            page: Some(0),
            refused: Some(Refusal::Never),
        });
        let s = &r.signatures[0];
        assert_eq!(level(s), Level::Bad);
        assert_eq!(
            summary(s),
            "Changed after signing in ways the signer didn't allow: page content changed on \
             page 1."
        );
        let mut r = report(SELF_MADE, &TrustList::default());
        r.signatures[0].trust = Some(Trust::Failed("its certificate had expired".into()));
        assert_eq!(level(&r.signatures[0]), Level::Bad);
        assert!(summary(&r.signatures[0]).contains("LimarePDF doesn't trust the certificate"));
    }

    #[test]
    fn allowed_changes_are_said() {
        let r = report(
            "pyhanko/minimal-two-fields-signed-twice.pdf",
            &TrustList::default(),
        );
        assert_eq!(r.signatures.len(), 2);
        assert!(summary(&r.signatures[0]).starts_with("Only allowed changes since"));
        let b = banner_of(r);
        assert_eq!(b.tone, Tone::Caution);
        assert!(
            b.text.starts_with("None of the 2 signatures is broken"),
            "{}",
            b.text
        );
    }

    #[test]
    fn certification_heads_the_banner() {
        let r = report(
            "pyhanko/certified-with-indirect-refs-in-dir.pdf",
            &TrustList::default(),
        );
        assert!(r.certification().is_some());
        let b = banner_of(r);
        assert!(
            b.text.starts_with("Certified by Lord Testerino ("),
            "{}",
            b.text
        );
    }

    #[test]
    fn document_timestamps_are_worded_as_such() {
        let r = report(
            "pyhanko/pades-lta-dss-indirect-arrs-test.pdf",
            &TrustList::default(),
        );
        let ts = r
            .signatures
            .iter()
            .find(|s| s.kind == SigKind::DocumentTimestamp)
            .expect("a document timestamp");
        assert!(
            summary(ts).starts_with("Document timestamp by "),
            "{}",
            summary(ts)
        );
        assert!(title(1, ts).starts_with("2. Timestamp by "));
    }

    #[test]
    fn signed_version_only_for_genuine_signatures() {
        let ok = report(SELF_MADE, &TrustList::default());
        assert!(signed_version_blocked(&ok.signatures[0]).is_none());
        let broken = report("pyhanko/one-byterange.pdf", &TrustList::default());
        assert!(signed_version_blocked(&broken.signatures[0]).is_some());
    }

    #[test]
    fn names_from_the_file_are_cleaned() {
        let mut r = report(SELF_MADE, &TrustList::default());
        let c = r.signatures[0].signer.as_mut().unwrap();
        c.common_name = Some("Mallory\u{202e}gpj.exe\nSecond line".into());
        let s = summary(&r.signatures[0]);
        assert!(!s.contains('\u{202e}') && !s.contains('\n'), "{s}");
    }

    /// A report for a document with no signatures.
    fn empty_report() -> SignatureReport {
        let mut p = Pdf::new();
        p.insert_blank(0, 300.0, 300.0).unwrap();
        let bytes = p.to_bytes().unwrap();
        limarepdf_core::verify_signatures(&bytes, None, &TrustList::default()).unwrap()
    }

    fn checked(report: SignatureReport) -> Check {
        Check::Done(Box::new(Checked {
            report,
            trust: TrustStatus::default(),
        }))
    }

    #[test]
    fn no_signatures_no_banner() {
        assert!(banner(&checked(empty_report())).is_none());
    }

    #[test]
    fn pending_and_failed_banners_are_neutral() {
        let b = banner(&Check::Pending).unwrap();
        assert_eq!(b.tone, Tone::Neutral);
        assert_eq!(b.kind, crate::banners::Kind::Signatures);
        let b = banner(&Check::Failed("cannot open PDF\u{202e}x".into())).unwrap();
        assert_eq!(b.tone, Tone::Neutral);
        // Text from outside is cleaned: no bidirectional controls.
        assert!(!b.text.contains('\u{202e}'), "{}", b.text);
    }

    /// Batch 7b: the Trust question in headed groups with "Name in your list" above its
    /// field, and Signature details' facts in headed groups, each name beside its value,
    /// with what can be done after them.
    #[test]
    fn trust_and_details_follow_the_dialog_look() {
        use crate::dialogs::{drawn, label_above};
        use egui::accesskit::{Node, TreeUpdate};
        let at = |tree: &TreeUpdate, f: &dyn Fn(&Node) -> bool| {
            let (_, n) = tree.nodes.iter().find(|(_, n)| f(n))?;
            let b = n.bounds()?;
            Some(egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32, b.y0 as f32),
                egui::pos2(b.x1 as f32, b.y1 as f32),
            ))
        };
        let size = egui::vec2(1200.0, 900.0);
        let r = report(SELF_MADE, &TrustList::default());
        let cert = r.signatures[0].signer.clone().expect("a signer");
        let mut ask = TrustAsk::new(TrustRole::Signer, vec![cert], true);
        let tree = drawn(size, |ctx| {
            ask.ui(ctx);
        });
        label_above(&tree, "Name in your list").unwrap();
        let heading = at(&tree, &|n| n.value() == Some("Trust it as")).expect("a heading");
        let signer = "A signer: signatures made with this certificate";
        let choice = at(&tree, &|n| n.label() == Some(signer)).expect("the choice");
        assert!(heading.bottom() <= choice.top());

        let c = Checked {
            report: r,
            trust: TrustStatus::default(),
        };
        let tree = drawn(size, |ctx| {
            details(ctx, "walrus.pdf", &c, 0);
        });
        let heading = at(&tree, &|n| n.value() == Some("Signer")).expect("Signer");
        let name = at(&tree, &|n| n.value() == Some("Issued by")).expect("Issued by");
        let value = "itself (a self-made digital ID)";
        let value = at(&tree, &|n| n.value() == Some(value)).expect("its value");
        assert!(heading.bottom() <= name.top());
        assert!((name.top() - value.top()).abs() < 1.0 && value.left() > name.right());
        let note = |n: &Node| {
            let start = "LimarePDF doesn't check whether";
            n.value().is_some_and(|v| v.starts_with(start))
        };
        let note = at(&tree, &note).expect("the revocation note");
        let button = at(&tree, &|n| n.label() == Some("View signed version")).unwrap();
        assert!(button.top() > note.bottom());
    }

    /// G08: Trusted certificates, one entry waiting for its second click: each row's
    /// buttons say which certificate they act on, every control is clearly named, and Esc
    /// closes the window (rows of different widths once had it laid out twice each frame,
    /// losing the key).
    #[test]
    fn each_trusted_certificate_s_buttons_say_which_it_is() {
        use crate::dialogs::{drawn, unclear_names};
        let r = report(SELF_MADE, &TrustList::default());
        let cert = r.signatures[0].signer.clone().expect("a signer");
        let mut list = TrustList::default();
        for (label, role) in [
            ("Sam Walrus", TrustRole::Signer),
            ("Walrus Authority", TrustRole::Authority),
        ] {
            list.entries.push(limarepdf_core::TrustEntry {
                role,
                label: label.into(),
                added: None,
                cert: cert.clone(),
            });
        }
        let mut w = TrustedWindow {
            list,
            status: TrustStatus::default(),
            path: None,
            removing: Some(0),
        };
        for size in [egui::vec2(1200.0, 800.0), egui::vec2(400.0, 300.0)] {
            let tree = drawn(size, |ctx| {
                w.ui(ctx);
            });
            assert_eq!(unclear_names(&tree), [""; 0], "{size:?}");
            for name in [
                "Stop trusting Sam Walrus",
                "Keep trusting Sam Walrus",
                "Remove Walrus Authority…",
            ] {
                let found = tree.nodes.iter().any(|(_, n)| n.label() == Some(name));
                assert!(found, "{size:?}: no {name}");
            }

            let ctx = Context::default();
            theme::install(&ctx);
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
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                };
                ctx.run_ui(raw, |_| closed = matches!(w.ui(&ctx), TrustedOut::Close))
                    .textures_delta
                    .clear();
            }
            assert!(closed, "{size:?}: Esc didn't close it");
        }
    }

    /// G08: Signature details and the Trust question on short and narrow screens: the
    /// window inside the screen with its buttons in view and every control clearly named;
    /// Esc closes details, and the question until its name is changed.
    #[test]
    fn signature_windows_fit_short_screens() {
        use crate::dialogs::{drawn_window, fits, unclear_names};
        let r = report(SELF_MADE, &TrustList::default());
        let cert = r.signatures[0].signer.clone().expect("a signer");
        let c = Checked {
            report: r,
            trust: TrustStatus::default(),
        };
        let mut ask = TrustAsk::new(TrustRole::Signer, vec![cert], true);
        for size in [
            egui::vec2(1200.0, 300.0),
            egui::vec2(400.0, 300.0),
            egui::vec2(400.0, 800.0),
        ] {
            let (tree, window) = drawn_window(size, |ctx| {
                details(ctx, "walrus.pdf", &c, 0);
            });
            fits(&tree, window, size, &["Close"]).unwrap();
            assert_eq!(unclear_names(&tree), [""; 0], "{size:?}");
            let (tree, window) = drawn_window(size, |ctx| {
                ask.ui(ctx);
            });
            fits(&tree, window, size, &["Trust", "Cancel"]).unwrap();
            assert_eq!(unclear_names(&tree), [""; 0], "{size:?}");
        }
        let esc_closes = |show: &mut dyn FnMut(&Context) -> bool| {
            let ctx = Context::default();
            theme::install(&ctx);
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
        };
        assert!(esc_closes(
            &mut |ctx| details(ctx, "walrus.pdf", &c, 0).close
        ));
        assert!(esc_closes(&mut |ctx| ask.ui(ctx) == Outcome::Cancel));
        ask.label_edited = true;
        assert!(!esc_closes(&mut |ctx| ask.ui(ctx) == Outcome::Cancel));
    }

    #[test]
    fn levels_order_worst_last() {
        assert!(Level::Good < Level::Caution);
        assert!(Level::Caution < Level::CantCheck);
        assert!(Level::CantCheck < Level::Bad);
    }
}
