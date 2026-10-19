//! Crash recovery copies: a copy of each document with unsaved changes, kept in a private
//! folder so that the next start can offer it when LimarePDF stopped without closing
//! normally. The user's own files are never written here. The design, with the reasons, is
//! `limarepdf-recovery-design.md` (outside the repository); section numbers below are its.
//!
//! This is the store: sessions, publishing a tab's copy, removing and withdrawing it,
//! finding crashed sessions and the one thread that does the writing; [`schedule`] says
//! when a tab's copy is due. The app's use of them (the restore window, the notices, the
//! setting) is in `app/recover.rs`.
//!
//! On disk, under [`recovery_root`]:
//!
//! - `session-<pid>-<start>/`, one per running app, with a `lock` file held while it runs.
//! - In it, per tab: `t<id>.txt`, the description of its copy, and `t<id>-<revision>-<n>.pdf`,
//!   the copy it names. Only a PDF that a description names is ever offered.
//! - Temporary files (`.t<id>…tmp`) while a copy is written.
//!
//! The promise (section 1): once [`Store::withdraw`] reports [`Withdrawal::Confirmed`] or
//! [`Withdrawal::Leftover`], no copy published before it can be offered, after a crash or
//! otherwise.

mod files;
#[cfg(test)]
pub mod model;
pub mod schedule;
#[cfg(test)]
mod tests;

pub use files::{Fs, Held, RealFs};

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Copies from a crashed session are kept this long after the crash (section 5).
pub const KEEP: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// A write is skipped when the disk would be left with less than this free (section 5).
pub const FREE_MARGIN: u64 = 1 << 30;

/// A session folder with no lock file yet, younger than this, is still being made.
const YOUNG: Duration = Duration::from_secs(60);

/// The first line of every description.
const HEADER: &str = "LimarePDF recovery copy 1";

/// How many session folder names are tried when one is taken.
const TRIES: u32 = 16;

/// The clock, passed in so that tests can set it.
pub type Clock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// The system clock.
pub fn system_clock() -> Clock {
    Arc::new(SystemTime::now)
}

/// The recovery folder, from the environment; `None` when it can't be found:
///
/// - Linux and other Unix: `$XDG_STATE_HOME/limarepdf/recovery/`, else
///   `~/.local/state/limarepdf/recovery/`
/// - macOS: `~/Library/Application Support/limarepdf/recovery/`
/// - Windows: `%LOCALAPPDATA%\limarepdf\recovery\` (not `APPDATA`, which roams)
pub fn recovery_root() -> Option<PathBuf> {
    let env = |k: &str| {
        let v = PathBuf::from(std::env::var_os(k)?);
        v.is_absolute().then_some(v)
    };
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")?
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support")
    } else {
        env("XDG_STATE_HOME").or_else(|| Some(env("HOME")?.join(".local/state")))?
    };
    Some(base.join("limarepdf").join("recovery"))
}

/// A tab, as the app numbers them. Its files are named after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabId(pub u64);

impl TabId {
    fn stem(self) -> String {
        format!("t{}", self.0)
    }

    fn description(self) -> String {
        format!("t{}.txt", self.0)
    }

    /// Whether `name` is one of this tab's copies.
    fn owns_pdf(self, name: &str) -> bool {
        name.strip_prefix(&format!("t{}-", self.0))
            .and_then(|r| r.strip_suffix(".pdf"))
            .is_some_and(|r| {
                let mut parts = r.split('-');
                let ok = |p: Option<&str>| {
                    p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                };
                ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
            })
    }

    /// Whether `name` is one of this tab's temporary files.
    fn owns_temp(self, name: &str) -> bool {
        let pdf = format!(".t{}-", self.0);
        let txt = format!(".t{}.txt.", self.0);
        (name.starts_with(&pdf) || name.starts_with(&txt)) && name.ends_with(".tmp")
    }
}

/// What a copy's description says about its document, besides the copy itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocInfo {
    pub title: String,
    /// The file it was opened from or last saved to, if any.
    pub original: Option<PathBuf>,
    pub pages: usize,
    pub repaired: bool,
    pub signed: bool,
    pub scrubbed: bool,
}

/// A copy's description (section 5, step 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub info: DocInfo,
    /// When the copy was written.
    pub time: SystemTime,
    /// The copy's file name, in the same folder.
    pdf: String,
    len: u64,
    sha256: [u8; 32],
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Text for a description line: control characters, `\` and bytes that aren't UTF-8 as
/// `\xHH`.
fn escape(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            if c == '\\' || c.is_control() {
                for b in c.encode_utf8(&mut [0; 4]).bytes() {
                    let _ = write!(s, "\\x{b:02x}");
                }
            } else {
                s.push(c);
            }
        }
        for b in chunk.invalid() {
            let _ = write!(s, "\\x{b:02x}");
        }
    }
    s
}

fn unescape(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut it = s.bytes();
    while let Some(b) = it.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        if it.next()? != b'x' {
            return None;
        }
        let h = [it.next()?, it.next()?];
        out.push(u8::from_str_radix(std::str::from_utf8(&h).ok()?, 16).ok()?);
    }
    Some(out)
}

fn path_bytes(p: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        p.as_os_str().as_bytes().to_vec()
    }
    // Windows names that aren't valid Unicode lose those characters; the original path only
    // suggests where to save the recovered copy.
    #[cfg(not(unix))]
    {
        p.to_string_lossy().into_owned().into_bytes()
    }
}

fn bytes_path(b: Vec<u8>) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(b))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(&b).into_owned())
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

impl Description {
    fn encode(&self) -> String {
        let i = &self.info;
        let mut s = format!(
            "{HEADER}\npdf {}\nlength {}\nsha256 {}\ntime {}\npages {}\ntitle {}\n",
            self.pdf,
            self.len,
            hex(&self.sha256),
            secs(self.time),
            i.pages,
            escape(i.title.as_bytes()),
        );
        if let Some(p) = &i.original {
            s += &format!("original {}\n", escape(&path_bytes(p)));
        }
        let flags: Vec<&str> = [
            (i.repaired, "repaired"),
            (i.signed, "signed"),
            (i.scrubbed, "scrubbed"),
        ]
        .into_iter()
        .filter_map(|(on, f)| on.then_some(f))
        .collect();
        if !flags.is_empty() {
            s += &format!("flags {}\n", flags.join(" "));
        }
        s
    }

    /// Reads a description of `tab`'s copy: `Ok(None)` for a withdrawn one, which names no
    /// copy.
    fn decode(tab: TabId, bytes: &[u8]) -> Result<Option<Description>, String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "its description isn't text")?;
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err("its description isn't one LimarePDF wrote".into());
        }
        let mut info = DocInfo::default();
        let (mut pdf, mut len, mut sha256, mut time) = (None, None, None, None);
        let mut pages = None;
        for line in lines {
            let (k, v) = line.split_once(' ').unwrap_or((line, ""));
            let bad = || format!("its description has a bad {k} line");
            match k {
                "withdrawn" => return Ok(None),
                "pdf" => {
                    if !tab.owns_pdf(v) {
                        return Err(bad());
                    }
                    pdf = Some(v.to_string());
                }
                "length" => len = Some(v.parse::<u64>().map_err(|_| bad())?),
                "sha256" => sha256 = Some(unhex(v).ok_or_else(bad)?),
                "time" => time = Some(v.parse::<u64>().map_err(|_| bad())?),
                "pages" => pages = Some(v.parse::<usize>().map_err(|_| bad())?),
                "title" => {
                    info.title = String::from_utf8_lossy(&unescape(v).ok_or_else(bad)?).into()
                }
                "original" => info.original = Some(bytes_path(unescape(v).ok_or_else(bad)?)),
                "flags" => {
                    for f in v.split(' ') {
                        match f {
                            "repaired" => info.repaired = true,
                            "signed" => info.signed = true,
                            "scrubbed" => info.scrubbed = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        let missing = |what: &str| format!("its description has no {what}");
        info.pages = pages.ok_or_else(|| missing("page count"))?;
        Ok(Some(Description {
            info,
            time: UNIX_EPOCH + Duration::from_secs(time.ok_or_else(|| missing("time"))?),
            pdf: pdf.ok_or_else(|| missing("file name"))?,
            len: len.ok_or_else(|| missing("length"))?,
            sha256: sha256.ok_or_else(|| missing("checksum"))?,
        }))
    }
}

/// What a withdrawal leaves in place of a description it couldn't remove: one that names no
/// copy, so nothing is offered.
fn withdrawn_text() -> String {
    format!("{HEADER}\nwithdrawn\n")
}

/// Removes the description `desc`, or failing that replaces it with one that names no copy.
/// `Some` when it can do neither, so its copy could still be offered.
fn withdraw_description(fs: &dyn Fs, desc: &Path) -> Option<NotConfirmed> {
    let e = remove_checked(fs, desc).err()?;
    let replaced = fs
        .write_temp(desc, withdrawn_text().as_bytes())
        .and_then(|tmp| {
            fs.rename(&tmp, desc).inspect_err(|_| {
                let _ = fs.remove_file(&tmp);
            })
        });
    replaced.is_err().then(|| NotConfirmed::Kept {
        file: desc.to_path_buf(),
        why: e.to_string(),
    })
}

/// Removes `path`; gone already counts as done. A name still there afterwards (a Windows
/// delete left pending while another program has the file open) counts as a failure.
fn remove_checked(fs: &dyn Fs, path: &Path) -> io::Result<()> {
    match fs.remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if fs.exists(path) {
        return Err(io::Error::other(format!(
            "{} is still there after it was deleted",
            path.display()
        )));
    }
    Ok(())
}

/// Points in a publish and a withdrawal where tests can stop the thread doing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    /// Both temporary files written and synced, the tab's lock not yet taken.
    TempsWritten,
    /// The tab's lock taken, the withdrawal count not yet compared.
    Locked,
    /// The count compared and found unchanged.
    Checked,
    PdfRenamed,
    PdfSynced,
    DescriptionRenamed,
    DescriptionSynced,
    PreviousRemoved,
    /// A withdrawal has removed (or replaced) the description.
    WithdrewDescription,
    /// A withdrawal has removed the PDFs and temporary files, the folder not yet synced.
    WithdrewFiles,
    /// A withdrawal has synced the folder.
    WithdrewSynced,
}

/// Called by the thread doing a publish or withdrawal at each [`Point`] (tests only).
#[cfg(test)]
type Hook = Arc<dyn Fn(Point) + Send + Sync>;

/// Why a tab gets no new copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Block {
    #[default]
    No,
    /// A withdrawal couldn't be confirmed: until one is (section 6).
    UntilConfirmed,
    /// A withdrawal left a file it couldn't delete: for the rest of the session.
    Session,
}

/// What the store knows about one tab. Its mutex is the tab's lock (section 4).
#[derive(Default)]
struct Rec {
    /// Bumped by each withdrawal; a publish whose ticket has an older count is refused.
    withdrawals: u64,
    /// The copy the latest description names.
    published: Option<String>,
    /// Copies an earlier description may still name until the next publish is durable, or
    /// that no description names; removed by the next publish or withdrawal.
    stale: BTreeSet<String>,
    /// Files a withdrawal invalidated but couldn't delete, retried.
    leftovers: BTreeSet<PathBuf>,
    block: Block,
}

/// What a tab's write is allowed to publish: taken on the UI thread with the document's clone.
#[derive(Clone, Copy, Debug)]
pub struct Ticket {
    pub tab: TabId,
    pub revision: u64,
    withdrawals: u64,
}

/// A publish that did its work.
#[derive(Debug, PartialEq)]
pub enum Published {
    /// The copy is durable, recorded as the ticket's revision.
    Written(u64),
    /// A withdrawal came after the ticket, or recovery was turned off: nothing was left.
    Refused,
}

/// A publish that didn't (section 6). The previous copy, if any, is still whole.
#[derive(Debug)]
pub enum WriteError {
    /// The disk would be left with less than [`FREE_MARGIN`].
    LowSpace {
        free: u64,
    },
    Failed(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LowSpace { free } => write!(
                f,
                "the disk is nearly full ({} MB free)",
                free / (1024 * 1024)
            ),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

/// The result of a withdrawal (section 6's table).
#[derive(Debug, PartialEq)]
pub enum Withdrawal {
    /// Nothing of the tab's copies is left.
    Confirmed,
    /// No earlier copy can be offered, but these files couldn't be deleted; they are retried.
    Leftover(Vec<PathBuf>),
    /// An earlier copy could still be offered: a redaction, removal of hidden information or
    /// password must not take effect.
    NotConfirmed(NotConfirmed),
}

#[derive(Debug, PartialEq)]
pub enum NotConfirmed {
    /// The description couldn't be removed or replaced.
    Kept { file: PathBuf, why: String },
    /// The files were removed, but the folder sync failed, so after a power cut they could
    /// come back.
    SyncFailed { folder: PathBuf, why: String },
}

impl Withdrawal {
    /// Whether a command that removes content may go ahead.
    #[cfg(test)]
    pub fn may_proceed(&self) -> bool {
        !matches!(self, Self::NotConfirmed(_))
    }
}

/// One running app's recovery folder.
pub struct Store {
    fs: Arc<dyn Fs>,
    clock: Clock,
    dir: PathBuf,
    lock: Mutex<Option<Held>>,
    on: AtomicBool,
    /// Numbers each publish, so that no two copies ever share a name.
    seq: AtomicU64,
    tabs: Mutex<HashMap<TabId, Arc<Mutex<Rec>>>>,
    #[cfg(test)]
    hook: Mutex<Option<Hook>>,
    /// Publishes in the wrong order (the previous copy removed before the second sync), to
    /// show that the power-cut test catches it.
    #[cfg(test)]
    wrong_order: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn fail(what: &str, path: &Path, e: io::Error) -> String {
    format!("{what} {}: {e}", path.display())
}

impl Store {
    /// Starts this app's session under `root` (section 5). An error means recovery is off
    /// for the run: the folders can't be made, one belongs to another user, the lock can't
    /// be taken or the disk can't sync folders.
    pub fn start(fs: Arc<dyn Fs>, root: &Path, clock: Clock) -> Result<Store, String> {
        fs.create_dir_all(root)
            .map_err(|e| fail("couldn't make", root, e))?;
        fs.check_folder(root)
            .map_err(|e| fail("can't use", root, e))?;
        let base = format!("session-{}-{}", std::process::id(), secs(clock()));
        let mut dir = root.join(&base);
        let mut n = 0;
        loop {
            match fs.create_dir_new(&dir) {
                Ok(()) => break,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n + 1 < TRIES => {
                    n += 1;
                    dir = root.join(format!("{base}-{n}"));
                }
                Err(e) => return Err(fail("couldn't make", &dir, e)),
            }
        }
        let started = (|| {
            fs.check_folder(&dir)
                .map_err(|e| fail("can't use", &dir, e))?;
            let lock = fs
                .lock(&dir.join("lock"), true)
                .map_err(|e| fail("couldn't lock", &dir, e))?
                .ok_or_else(|| format!("{} is locked by another program", dir.display()))?;
            // A disk that can't sync folders could never confirm a withdrawal, which would
            // stop every redaction; better no recovery at all.
            fs.sync_dir(&dir)
                .and_then(|()| fs.sync_dir(root))
                .map_err(|e| fail("can't confirm changes in", &dir, e))?;
            Ok(lock)
        })();
        let lock = match started {
            Ok(l) => l,
            Err(e) => {
                let _ = fs.remove_dir_all(&dir);
                return Err(e);
            }
        };
        Ok(Store {
            fs,
            clock,
            dir,
            lock: Mutex::new(Some(lock)),
            on: AtomicBool::new(true),
            seq: AtomicU64::new(0),
            tabs: Mutex::new(HashMap::new()),
            #[cfg(test)]
            hook: Mutex::new(None),
            #[cfg(test)]
            wrong_order: AtomicBool::new(false),
        })
    }

    /// This session's folder.
    #[cfg(test)]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn at(&self, _p: Point) {
        #[cfg(test)]
        {
            let h = lock(&self.hook).clone();
            if let Some(h) = h {
                h(_p)
            }
        }
    }

    fn rec(&self, tab: TabId) -> Arc<Mutex<Rec>> {
        lock(&self.tabs).entry(tab).or_default().clone()
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }

    /// A ticket for writing `tab` at `revision`, taken on the UI thread when the document is
    /// cloned; `None` when recovery is off or the tab gets no copies (section 6).
    pub fn ticket(&self, tab: TabId, revision: u64) -> Option<Ticket> {
        if !self.is_on() {
            return None;
        }
        let rec = self.rec(tab);
        let r = lock(&rec);
        (r.block == Block::No).then_some(Ticket {
            tab,
            revision,
            withdrawals: r.withdrawals,
        })
    }

    /// Writes `pdf` as the ticket's tab's copy, replacing its previous one, in the durable
    /// order of section 5, so that an app crash or a power cut at any point leaves a whole
    /// pair. Runs on the recovery thread.
    pub fn publish(&self, t: &Ticket, info: &DocInfo, pdf: &[u8]) -> Result<Published, WriteError> {
        let fs = &*self.fs;
        let tab = t.tab;
        let name = format!(
            "{}-{}-{}.pdf",
            tab.stem(),
            t.revision,
            self.seq.fetch_add(1, Ordering::Relaxed)
        );
        let desc = Description {
            info: info.clone(),
            time: (self.clock)(),
            pdf: name.clone(),
            len: pdf.len() as u64,
            sha256: limarepdf_core::disk::sha256(pdf),
        }
        .encode();
        // Advisory only: a reading that fails lets the write go ahead.
        if let Ok(free) = fs.free_space(&self.dir) {
            let needed = (pdf.len() + desc.len()) as u64;
            if free < needed.saturating_add(FREE_MARGIN) {
                return Err(WriteError::LowSpace { free });
            }
        }
        let pdf_path = self.dir.join(&name);
        let desc_path = self.dir.join(tab.description());
        let failed = |what: &str, p: &Path, e| WriteError::Failed(fail(what, p, e));

        // Step 1, outside the lock.
        let tmp_pdf = fs
            .write_temp(&pdf_path, pdf)
            .map_err(|e| failed("couldn't write a copy in", &self.dir, e))?;
        let tmp_desc = match fs.write_temp(&desc_path, desc.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                let _ = fs.remove_file(&tmp_pdf);
                return Err(failed("couldn't write a copy in", &self.dir, e));
            }
        };
        let drop_temps = || {
            let _ = fs.remove_file(&tmp_pdf);
            let _ = fs.remove_file(&tmp_desc);
        };
        self.at(Point::TempsWritten);

        let rec = self.rec(tab);
        let mut r = lock(&rec);
        self.at(Point::Locked);
        if r.withdrawals != t.withdrawals || !self.is_on() {
            drop_temps();
            return Ok(Published::Refused);
        }
        self.at(Point::Checked);

        // Step 2: the new PDF, durable before any description names it.
        if let Err(e) = fs.rename(&tmp_pdf, &pdf_path) {
            drop_temps();
            return Err(failed("couldn't write", &pdf_path, e));
        }
        self.at(Point::PdfRenamed);
        if let Err(e) = fs.sync_dir(&self.dir) {
            let _ = fs.remove_file(&tmp_desc);
            r.stale.insert(name);
            return Err(failed("couldn't confirm the copy in", &self.dir, e));
        }
        self.at(Point::PdfSynced);

        // Step 3: the description naming it.
        if let Err(e) = fs.rename(&tmp_desc, &desc_path) {
            let _ = fs.remove_file(&tmp_desc);
            r.stale.insert(name);
            return Err(failed("couldn't write", &desc_path, e));
        }
        self.at(Point::DescriptionRenamed);
        let previous: Vec<String> = r
            .published
            .replace(name.clone())
            .into_iter()
            .chain(std::mem::take(&mut r.stale))
            .filter(|p| *p != name)
            .collect();
        #[cfg(test)]
        if self.wrong_order.load(Ordering::SeqCst) {
            for p in &previous {
                let _ = fs.remove_file(&self.dir.join(p));
            }
            self.at(Point::DescriptionRenamed);
        }
        if let Err(e) = fs.sync_dir(&self.dir) {
            // Either description may be the durable one, so both copies stay.
            r.stale.extend(previous);
            return Err(failed("couldn't confirm the copy in", &self.dir, e));
        }
        self.at(Point::DescriptionSynced);

        // Step 4: no description names the previous copies any more.
        for p in previous {
            if remove_checked(fs, &self.dir.join(&p)).is_err() {
                r.stale.insert(p);
            }
        }
        self.at(Point::PreviousRemoved);
        Ok(Published::Written(t.revision))
    }

    /// Removes the tab's copy because it was saved or closed (no promise rides on it, so
    /// failures are only reported). Runs on the recovery thread, after any write asked for
    /// before it.
    pub fn remove(&self, tab: TabId) -> Result<(), String> {
        let fs = &*self.fs;
        let rec = self.rec(tab);
        let mut r = lock(&rec);
        let desc = self.dir.join(tab.description());
        remove_checked(fs, &desc).map_err(|e| fail("couldn't delete", &desc, e))?;
        // So that a power cut can't bring the description back without its copy.
        fs.sync_dir(&self.dir)
            .map_err(|e| fail("couldn't confirm changes in", &self.dir, e))?;
        let mut left = None;
        for p in self.files_of(tab, &r) {
            if let Err(e) = remove_checked(fs, &p) {
                left.get_or_insert(fail("couldn't delete", &p, e));
            }
        }
        r.published = None;
        r.stale.clear();
        left.map_or(Ok(()), Err)
    }

    /// Every PDF and temporary file of `tab` in the folder, and the ones the record names.
    fn files_of(&self, tab: TabId, r: &Rec) -> Vec<PathBuf> {
        let mut names: BTreeSet<String> = r.published.iter().chain(&r.stale).cloned().collect();
        if let Ok(list) = self.fs.list(&self.dir) {
            names.extend(
                list.into_iter()
                    .filter(|n| tab.owns_pdf(n) || tab.owns_temp(n)),
            );
        }
        names.into_iter().map(|n| self.dir.join(n)).collect()
    }

    /// Withdraws the tab's copies before a redaction, removal of hidden information or added
    /// password takes effect (section 4), on the UI thread. Waits for a publish that has
    /// passed its check; any publish not there yet is refused.
    pub fn withdraw(&self, tab: TabId) -> Withdrawal {
        let fs = &*self.fs;
        let rec = self.rec(tab);
        let mut r = lock(&rec);
        r.withdrawals += 1;
        let kept = withdraw_description(fs, &self.dir.join(tab.description()));
        self.at(Point::WithdrewDescription);
        let mut left = Vec::new();
        for p in self.files_of(tab, &r) {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            let temp = tab.owns_temp(name);
            // A temporary file is never offered; it goes where the system allows.
            if remove_checked(fs, &p).is_err() && !temp {
                left.push(p);
            }
        }
        r.published = None;
        r.stale.clear();
        r.leftovers.extend(left.iter().cloned());
        self.at(Point::WithdrewFiles);
        drop(r);
        let synced = fs.sync_dir(&self.dir);
        self.at(Point::WithdrewSynced);
        let result = match (kept, synced) {
            (Some(k), _) => Withdrawal::NotConfirmed(k),
            (None, Err(e)) => Withdrawal::NotConfirmed(NotConfirmed::SyncFailed {
                folder: self.dir.clone(),
                why: e.to_string(),
            }),
            (None, Ok(())) if left.is_empty() => Withdrawal::Confirmed,
            (None, Ok(())) => Withdrawal::Leftover(left),
        };
        let mut r = lock(&rec);
        r.block = match (&result, r.block) {
            (_, Block::Session) | (Withdrawal::Leftover(_), _) => Block::Session,
            (Withdrawal::NotConfirmed(_), _) => Block::UntilConfirmed,
            (Withdrawal::Confirmed, _) => Block::No,
        };
        result
    }

    /// Turns recovery off: no new copy is written from now on, whatever the withdrawals
    /// report (section 4). Returns each tab's withdrawal.
    pub fn turn_off(&self) -> Vec<(TabId, Withdrawal)> {
        self.on.store(false, Ordering::SeqCst);
        let mut tabs: Vec<TabId> = lock(&self.tabs).keys().copied().collect();
        tabs.sort();
        tabs.into_iter().map(|t| (t, self.withdraw(t))).collect()
    }

    pub fn turn_on(&self) {
        self.on.store(true, Ordering::SeqCst);
    }

    /// Whether a file (such as one a withdrawal left) is still there.
    pub fn exists(&self, path: &Path) -> bool {
        self.fs.exists(path)
    }

    /// Tries again to delete the files withdrawals left; returns the ones still there.
    pub fn retry_leftovers(&self) -> Vec<PathBuf> {
        let recs: Vec<_> = lock(&self.tabs).values().cloned().collect();
        let mut still = Vec::new();
        for rec in recs {
            let mut r = lock(&rec);
            r.leftovers
                .retain(|p| remove_checked(&*self.fs, p).is_err());
            still.extend(r.leftovers.iter().cloned());
        }
        still.sort();
        still
    }

    /// Ends the session at a normal exit: every copy and the folder go, and nothing more is
    /// written. An error names what couldn't be deleted; the next start removes it.
    pub fn end(&self) -> Result<(), String> {
        let fs = &*self.fs;
        self.on.store(false, Ordering::SeqCst);
        // Descriptions first, so that whatever is left can't be offered.
        if let Ok(list) = fs.list(&self.dir) {
            for n in list.iter().filter(|n| n.ends_with(".txt")) {
                let _ = remove_checked(fs, &self.dir.join(n));
            }
        }
        let _ = fs.sync_dir(&self.dir);
        drop(lock(&self.lock).take());
        fs.remove_dir_all(&self.dir)
            .map_err(|e| fail("couldn't delete", &self.dir, e))
    }
}

/// A copy found in a crashed session.
#[derive(Debug)]
pub struct Found {
    pub tab: TabId,
    pub state: Result<Description, String>,
}

/// A session that ended without cleaning up, with its copies. Its lock is held while this
/// lives, so another LimarePDF starting meanwhile doesn't offer the same copies.
pub struct Crashed {
    fs: Arc<dyn Fs>,
    pub dir: PathBuf,
    pub docs: Vec<Found>,
    /// When its copies are removed (section 5).
    pub keep_until: SystemTime,
    /// Files withdrawals left; the next start removes them if they are still there.
    leftovers: BTreeSet<PathBuf>,
    /// Copies whose withdrawal removed their files but couldn't confirm it (the folder sync
    /// failed): no longer in `docs`, but a power cut could bring them back, so the next
    /// withdrawal of the tab tries again, and the folder stays until it succeeds.
    unconfirmed: BTreeSet<TabId>,
    _lock: Option<Held>,
}

impl std::fmt::Debug for Crashed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Crashed")
            .field("dir", &self.dir)
            .field("docs", &self.docs)
            .field("keep_until", &self.keep_until)
            .finish()
    }
}

/// The start time in a session folder's name, `session-<pid>-<start>[-<n>]`.
fn session_start(name: &str) -> Option<SystemTime> {
    let mut parts = name.strip_prefix("session-")?.split('-');
    parts.next()?.parse::<u32>().ok()?;
    let s = parts.next()?.parse::<u64>().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(s))
}

/// Checks a copy against its description.
fn check(fs: &dyn Fs, dir: &Path, tab: TabId) -> Result<Option<Description>, String> {
    let bytes = fs
        .read(&dir.join(tab.description()))
        .map_err(|e| format!("its description can't be read ({e})"))?;
    let Some(d) = Description::decode(tab, &bytes)? else {
        return Ok(None);
    };
    let pdf = match fs.read(&dir.join(&d.pdf)) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err("the copy is missing".into()),
        Err(e) => return Err(format!("the copy can't be read ({e})")),
    };
    if pdf.len() as u64 != d.len {
        return Err("the copy is cut short or changed".into());
    }
    if limarepdf_core::disk::sha256(&pdf) != d.sha256 {
        return Err("the copy has changed".into());
    }
    Ok(Some(d))
}

/// The tab a description's name is for.
fn description_tab(name: &str) -> Option<TabId> {
    let id = name.strip_prefix('t')?.strip_suffix(".txt")?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .then(|| id.parse().ok().map(TabId))
        .flatten()
}

/// Finds sessions under `root` that ended without cleaning up (their lock can be taken), at
/// start (sections 5 and 7). Removes expired ones, leftover temporary files and copies no
/// description names, and sessions with nothing left to offer.
pub fn find_crashed(fs: &Arc<dyn Fs>, root: &Path, now: SystemTime) -> Vec<Crashed> {
    let mut found = Vec::new();
    let Ok(names) = fs.list(root) else {
        return found;
    };
    for name in names.iter().filter(|n| n.starts_with("session-")) {
        let dir = root.join(name);
        let started = session_start(name).unwrap_or(UNIX_EPOCH);
        let held = match fs.lock(&dir.join("lock"), false) {
            Ok(Some(h)) => Some(h),
            // Its app is running.
            Ok(None) => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if now.duration_since(started).unwrap_or_default() < YOUNG {
                    // Being made (or its clock is ahead): leave it to its app.
                    continue;
                }
                None
            }
            Err(_) => continue,
        };
        let Ok(files) = fs.list(&dir) else {
            continue;
        };
        let mut docs = Vec::new();
        let mut named = BTreeSet::new();
        for f in &files {
            let Some(tab) = description_tab(f) else {
                continue;
            };
            match check(&**fs, &dir, tab) {
                Ok(Some(d)) => {
                    named.insert(d.pdf.clone());
                    docs.push(Found { tab, state: Ok(d) });
                }
                Ok(None) => {
                    let _ = fs.remove_file(&dir.join(f));
                }
                Err(why) => {
                    // A damaged description may still name a copy: keep both for Show file.
                    if let Ok(b) = fs.read(&dir.join(f)) {
                        if let Ok(t) = std::str::from_utf8(&b) {
                            named.extend(
                                t.lines()
                                    .filter_map(|l| l.strip_prefix("pdf "))
                                    .map(str::to_string),
                            );
                        }
                    }
                    docs.push(Found {
                        tab,
                        state: Err(why),
                    });
                }
            }
        }
        let crashed_at = docs
            .iter()
            .filter_map(|d| d.state.as_ref().ok().map(|d| d.time))
            .chain([started])
            .max()
            .unwrap_or(started);
        let keep_until = crashed_at + KEEP;
        if docs.is_empty() || now > keep_until {
            drop(held);
            let _ = fs.remove_dir_all(&dir);
            continue;
        }
        for f in &files {
            let temp = f.starts_with('.') && f.ends_with(".tmp");
            let unnamed = f.ends_with(".pdf") && !named.contains(f);
            if temp || unnamed {
                let _ = fs.remove_file(&dir.join(f));
            }
        }
        let held = held.or_else(|| fs.lock(&dir.join("lock"), true).ok().flatten());
        docs.sort_by_key(|d| d.tab);
        found.push(Crashed {
            fs: fs.clone(),
            dir,
            docs,
            keep_until,
            leftovers: BTreeSet::new(),
            unconfirmed: BTreeSet::new(),
            _lock: held,
        });
    }
    found
}

impl Crashed {
    /// The bytes of copy `i`, checked against its description again.
    pub fn read(&self, i: usize) -> Result<Vec<u8>, String> {
        let doc = self.docs.get(i).ok_or("no such copy")?;
        let d = doc.state.as_ref().map_err(Clone::clone)?;
        let pdf = self
            .fs
            .read(&self.dir.join(&d.pdf))
            .map_err(|e| format!("the copy can't be read ({e})"))?;
        if pdf.len() as u64 != d.len || limarepdf_core::disk::sha256(&pdf) != d.sha256 {
            return Err("the copy has changed".into());
        }
        Ok(pdf)
    }

    /// The file to show for copy `i`: the copy, or its description when the copy is missing.
    pub fn file(&self, i: usize) -> Option<PathBuf> {
        let doc = self.docs.get(i)?;
        let pdf = doc.state.as_ref().ok().map(|d| self.dir.join(&d.pdf));
        Some(pdf.unwrap_or_else(|| self.dir.join(doc.tab.description())))
    }

    /// Deletes copy `i`: its description first, then the copy. The whole session goes once
    /// nothing is left in it.
    pub fn discard(&mut self, i: usize) -> Result<(), String> {
        let fs = &*self.fs;
        let doc = self.docs.get(i).ok_or("no such copy")?;
        let desc = self.dir.join(doc.tab.description());
        let pdf = doc.state.as_ref().ok().map(|d| self.dir.join(&d.pdf));
        remove_checked(fs, &desc).map_err(|e| fail("couldn't delete", &desc, e))?;
        let _ = fs.sync_dir(&self.dir);
        let tab = doc.tab;
        self.docs.remove(i);
        let mut result = Ok(());
        if let Some(p) = pdf {
            result = remove_checked(fs, &p).map_err(|e| fail("couldn't delete", &p, e));
        }
        // A damaged description's copies, if any.
        if let Ok(list) = fs.list(&self.dir) {
            for n in list.iter().filter(|n| tab.owns_pdf(n)) {
                let _ = remove_checked(fs, &self.dir.join(n));
            }
        }
        if self.docs.is_empty() && self.unconfirmed.is_empty() {
            self._lock = None;
            let _ = fs.remove_dir_all(&self.dir);
        }
        result
    }

    /// Withdraws copy `tab` before a redaction, a removal of hidden information or a
    /// password takes effect in the tab restored from it, with the outcomes of
    /// [`Store::withdraw`] (section 6). One not confirmed is tried again in full at the next
    /// call. The session goes once nothing is left in it.
    pub fn withdraw(&mut self, tab: TabId) -> Withdrawal {
        let fs = &*self.fs;
        let offered = self.docs.iter().position(|d| d.tab == tab);
        if offered.is_none() && !self.unconfirmed.contains(&tab) {
            return Withdrawal::Confirmed;
        }
        let kept = withdraw_description(fs, &self.dir.join(tab.description()));
        let mut left = Vec::new();
        if let Ok(list) = fs.list(&self.dir) {
            for p in list.iter().filter(|n| tab.owns_pdf(n)) {
                let p = self.dir.join(p);
                if remove_checked(fs, &p).is_err() {
                    left.push(p);
                }
            }
        }
        let synced = fs.sync_dir(&self.dir);
        self.leftovers.extend(left.iter().cloned());
        if let Some(k) = kept {
            return Withdrawal::NotConfirmed(k);
        }
        // No longer offered, even if the sync failed: its description is gone here. But
        // until a sync confirms that, a power cut could bring it back.
        if let Some(i) = offered {
            self.docs.remove(i);
        }
        if let Err(e) = synced {
            self.unconfirmed.insert(tab);
            return Withdrawal::NotConfirmed(NotConfirmed::SyncFailed {
                folder: self.dir.clone(),
                why: e.to_string(),
            });
        }
        self.unconfirmed.remove(&tab);
        if self.docs.is_empty() && self.unconfirmed.is_empty() {
            self._lock = None;
            let _ = fs.remove_dir_all(&self.dir);
            left.retain(|p| fs.exists(p));
            self.leftovers.retain(|p| fs.exists(p));
        }
        if left.is_empty() {
            Withdrawal::Confirmed
        } else {
            Withdrawal::Leftover(left)
        }
    }

    /// Tries again to delete the files withdrawals left.
    pub fn retry_leftovers(&mut self) {
        let fs = &*self.fs;
        self.leftovers.retain(|p| remove_checked(fs, p).is_err());
    }
}

/// What the recovery thread reports back to the app.
#[derive(Debug)]
pub struct Report {
    pub tab: TabId,
    pub outcome: Outcome,
}

#[derive(Debug)]
pub enum Outcome {
    Written(u64),
    /// A withdrawal came after the ticket: the tab stays due.
    Refused,
    /// The write was skipped or failed (section 6's notice); the tab stays due.
    Failed(String),
    Removed,
    RemoveFailed(String),
}

/// Makes a copy's bytes on the recovery thread (the document's snapshot).
pub type Make = Box<dyn FnOnce() -> limarepdf_core::Result<Vec<u8>> + Send>;

enum Job {
    Write(Ticket, DocInfo, Make),
    Remove(TabId),
    Retry,
}

/// The recovery thread: every write and removal, one at a time, in the order asked for, so a
/// removal asked for after a write can't be undone by it (section 4).
pub struct Queue {
    store: Arc<Store>,
    tx: Option<mpsc::Sender<Job>>,
    reports: mpsc::Receiver<Report>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Jobs asked for and not yet done.
    pending: Arc<AtomicUsize>,
}

impl Queue {
    /// Starts the thread; `wake` is called after each report (to repaint the window).
    pub fn new(store: Arc<Store>, wake: Box<dyn Fn() + Send>) -> Queue {
        let (tx, rx) = mpsc::channel::<Job>();
        let (report, reports) = mpsc::channel();
        let s = store.clone();
        let pending = Arc::new(AtomicUsize::new(0));
        let left = pending.clone();
        let thread = std::thread::Builder::new()
            .name("recovery".into())
            .spawn(move || {
                for job in rx {
                    let done = || left.fetch_sub(1, Ordering::SeqCst);
                    let (tab, outcome) = match job {
                        Job::Write(t, info, make) => {
                            // A failed or panicking snapshot skips this write (section 4).
                            let o = match crate::guard(make) {
                                Err(e) => Outcome::Failed(format!("{e:#}")),
                                Ok(pdf) => match s.publish(&t, &info, &pdf) {
                                    Ok(Published::Written(r)) => Outcome::Written(r),
                                    Ok(Published::Refused) => Outcome::Refused,
                                    Err(e) => Outcome::Failed(e.to_string()),
                                },
                            };
                            (t.tab, o)
                        }
                        Job::Remove(tab) => match s.remove(tab) {
                            Ok(()) => (tab, Outcome::Removed),
                            Err(e) => (tab, Outcome::RemoveFailed(e)),
                        },
                        Job::Retry => {
                            s.retry_leftovers();
                            done();
                            continue;
                        }
                    };
                    let sent = report.send(Report { tab, outcome });
                    done();
                    if sent.is_err() {
                        break;
                    }
                    wake();
                }
            })
            .expect("starting the recovery thread");
        Queue {
            store,
            tx: Some(tx),
            reports,
            thread: Some(thread),
            pending,
        }
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Whether every job asked for is done (its report waiting in [`Queue::reports`]).
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }

    fn send(&self, job: Job) {
        if let Some(tx) = &self.tx {
            self.pending.fetch_add(1, Ordering::SeqCst);
            if tx.send(job).is_err() {
                self.pending.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// Queues a write of the ticket's tab, its bytes made by `make` on the recovery thread.
    pub fn write(&self, ticket: Ticket, info: DocInfo, make: Make) {
        self.send(Job::Write(ticket, info, make));
    }

    /// Queues the removal of a saved or closed tab's copy.
    pub fn remove(&self, tab: TabId) {
        self.send(Job::Remove(tab));
    }

    /// Queues another try at deleting the files withdrawals left.
    pub fn retry(&self) {
        self.send(Job::Retry);
    }

    /// The reports so far.
    pub fn reports(&self) -> Vec<Report> {
        self.reports.try_iter().collect()
    }

    /// Waits for the queued work to finish and stops the thread.
    pub fn finish(mut self) -> Arc<Store> {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.store.clone()
    }
}
