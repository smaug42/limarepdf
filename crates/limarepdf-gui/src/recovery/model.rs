//! File systems for the tests: a model one that simulates power cuts, and a wrapper that
//! makes chosen operations fail.
//!
//! In the model, file contents written by `write_temp` are synced, so they survive a power
//! cut, but a name created, renamed or removed in a folder becomes durable only when the
//! folder is synced, as on a real disk. After a power cut, any subset of the operations since
//! the folder's last sync may have survived (applied in order): no file system promises more.
//! Folders themselves are durable as soon as they are made or removed.

use super::files::{Fs, Held};
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug)]
enum Node {
    Dir,
    File(Arc<Vec<u8>>),
}

#[derive(Clone, Debug)]
enum Op {
    Put(PathBuf, Node),
    Rename(PathBuf, PathBuf),
    Remove(PathBuf),
}

impl Op {
    fn dir(&self) -> &Path {
        let p = match self {
            Op::Put(p, _) | Op::Remove(p) => p,
            Op::Rename(_, to) => to,
        };
        p.parent().unwrap_or(Path::new(""))
    }

    /// Applies the operation to `files`; one whose file is gone (an earlier operation that
    /// didn't survive) does nothing.
    fn apply(&self, files: &mut BTreeMap<PathBuf, Node>) {
        match self {
            Op::Put(p, n) => {
                files.insert(p.clone(), n.clone());
            }
            Op::Rename(from, to) => {
                if let Some(n) = files.remove(from) {
                    files.insert(to.clone(), n);
                }
            }
            Op::Remove(p) => {
                files.remove(p);
            }
        }
    }
}

#[derive(Default)]
struct State {
    current: BTreeMap<PathBuf, Node>,
    durable: BTreeMap<PathBuf, Node>,
    pending: Vec<Op>,
    locks: HashSet<PathBuf>,
    next: u64,
}

impl State {
    fn op(&mut self, op: Op) {
        op.apply(&mut self.current);
        self.pending.push(op);
    }
}

#[derive(Clone, Default)]
pub struct Model(Arc<Mutex<State>>);

struct ModelLock(Arc<Mutex<State>>, PathBuf);

impl Drop for ModelLock {
    fn drop(&mut self) {
        lock(&self.0).locks.remove(&self.1);
    }
}

fn lock(s: &Mutex<State>) -> MutexGuard<'_, State> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

fn not_found(p: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{}: not found", p.display()),
    )
}

impl Model {
    pub fn new() -> Model {
        Model::default()
    }

    fn st(&self) -> MutexGuard<'_, State> {
        lock(&self.0)
    }

    /// How many operations aren't durable yet.
    pub fn pending(&self) -> usize {
        self.st().pending.len()
    }

    /// The disk as a power cut now could leave it, in every way: each subset of the
    /// operations not yet durable, applied in order to what is. No locks are held on it.
    pub fn power_cuts(&self) -> Vec<Model> {
        let s = self.st();
        let n = s.pending.len();
        assert!(
            n <= 12,
            "{n} operations pending; the test should sync more often"
        );
        (0..1u32 << n)
            .map(|mask| {
                let mut files = s.durable.clone();
                for (i, op) in s.pending.iter().enumerate() {
                    if mask & (1 << i) != 0 {
                        op.apply(&mut files);
                    }
                }
                Model(Arc::new(Mutex::new(State {
                    current: files.clone(),
                    durable: files,
                    ..State::default()
                })))
            })
            .collect()
    }

    /// The disk as an app crash now leaves it: everything done so far, durable or not.
    pub fn crash(&self) -> Model {
        let s = self.st();
        Model(Arc::new(Mutex::new(State {
            current: s.current.clone(),
            durable: s.current.clone(),
            next: s.next,
            ..State::default()
        })))
    }

    /// The names of the files in `dir` now.
    pub fn names(&self, dir: &Path) -> Vec<String> {
        self.list(dir).unwrap_or_default()
    }
}

impl Fs for Model {
    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        let mut s = self.st();
        for d in dir.ancestors().filter(|d| !d.as_os_str().is_empty()) {
            s.current.insert(d.into(), Node::Dir);
            s.durable.insert(d.into(), Node::Dir);
        }
        Ok(())
    }

    fn create_dir_new(&self, dir: &Path) -> io::Result<()> {
        let mut s = self.st();
        if s.current.contains_key(dir) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        if !matches!(dir.parent().and_then(|p| s.current.get(p)), Some(Node::Dir)) {
            return Err(not_found(dir));
        }
        s.current.insert(dir.into(), Node::Dir);
        s.durable.insert(dir.into(), Node::Dir);
        Ok(())
    }

    fn check_folder(&self, dir: &Path) -> io::Result<()> {
        match self.st().current.get(dir) {
            Some(Node::Dir) => Ok(()),
            _ => Err(not_found(dir)),
        }
    }

    fn write_temp(&self, path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
        let mut s = self.st();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let tmp = loop {
            s.next += 1;
            let t = path.with_file_name(format!(".{name}.m{}.tmp", s.next));
            if !s.current.contains_key(&t) {
                break t;
            }
        };
        if !matches!(tmp.parent().and_then(|p| s.current.get(p)), Some(Node::Dir)) {
            return Err(not_found(&tmp));
        }
        s.op(Op::Put(tmp.clone(), Node::File(Arc::new(bytes.to_vec()))));
        Ok(tmp)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut s = self.st();
        if !s.current.contains_key(from) {
            return Err(not_found(from));
        }
        s.op(Op::Rename(from.into(), to.into()));
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let mut s = self.st();
        match s.current.get(path) {
            Some(Node::File(_)) => {
                s.op(Op::Remove(path.into()));
                Ok(())
            }
            Some(Node::Dir) => Err(io::Error::other("is a folder")),
            None => Err(not_found(path)),
        }
    }

    fn remove_dir_all(&self, dir: &Path) -> io::Result<()> {
        let mut s = self.st();
        if !s.current.contains_key(dir) {
            return Err(not_found(dir));
        }
        s.current.retain(|p, _| !p.starts_with(dir));
        s.durable.retain(|p, _| !p.starts_with(dir));
        s.pending.retain(|op| !op.dir().starts_with(dir));
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.st().current.contains_key(path)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        let mut s = self.st();
        let (now, later): (Vec<Op>, Vec<Op>) = std::mem::take(&mut s.pending)
            .into_iter()
            .partition(|op| op.dir() == dir);
        for op in &now {
            op.apply(&mut s.durable);
        }
        s.pending = later;
        Ok(())
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.st().current.get(path) {
            Some(Node::File(b)) => Ok(b.to_vec()),
            Some(Node::Dir) => Err(io::Error::other("is a folder")),
            None => Err(not_found(path)),
        }
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        let s = self.st();
        if !matches!(s.current.get(dir), Some(Node::Dir)) {
            return Err(not_found(dir));
        }
        Ok(s.current
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect())
    }

    fn lock(&self, path: &Path, create: bool) -> io::Result<Option<Held>> {
        let mut s = self.st();
        if create {
            if s.current.contains_key(path) {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            s.op(Op::Put(path.into(), Node::File(Arc::new(Vec::new()))));
        } else if !s.current.contains_key(path) {
            return Err(not_found(path));
        }
        if !s.locks.insert(path.into()) {
            return Ok(None);
        }
        Ok(Some(Box::new(ModelLock(self.0.clone(), path.into()))))
    }

    fn free_space(&self, _dir: &Path) -> io::Result<u64> {
        Ok(u64::MAX / 2)
    }
}

/// The operations [`Faulty`] can make fail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    WriteTemp,
    Rename,
    Remove,
    Sync,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fault {
    /// The operation fails.
    Fail,
    /// A removal reports success but leaves the name (a Windows delete left pending).
    Pending,
}

struct Rule {
    kind: Kind,
    /// Matches a path containing this.
    matches: String,
    fault: Fault,
    /// How many more times it applies; `None` for always.
    times: Option<usize>,
}

/// Another file system, with chosen operations failing and a chosen free-space reading.
pub struct Faulty {
    inner: Arc<dyn Fs>,
    rules: Mutex<Vec<Rule>>,
    /// `Some(None)`: the reading fails.
    free: Mutex<Option<Option<u64>>>,
}

impl Faulty {
    pub fn new(inner: Arc<dyn Fs>) -> Arc<Faulty> {
        Arc::new(Faulty {
            inner,
            rules: Mutex::new(Vec::new()),
            free: Mutex::new(None),
        })
    }

    /// Every `kind` operation on a path containing `matches` fails from now on.
    pub fn fail(&self, kind: Kind, matches: &str) {
        self.add(kind, matches, Fault::Fail, None);
    }

    /// The next `kind` operation on a path containing `matches` fails.
    pub fn fail_once(&self, kind: Kind, matches: &str) {
        self.add(kind, matches, Fault::Fail, Some(1));
    }

    /// Removals of paths containing `matches` report success but leave the name.
    pub fn pending_delete(&self, matches: &str) {
        self.add(Kind::Remove, matches, Fault::Pending, None);
    }

    fn add(&self, kind: Kind, matches: &str, fault: Fault, times: Option<usize>) {
        self.rules.lock().unwrap().push(Rule {
            kind,
            matches: matches.into(),
            fault,
            times,
        });
    }

    pub fn clear(&self) {
        self.rules.lock().unwrap().clear();
    }

    /// The free-space reading: `Some(bytes)`, or `None` for a reading that fails.
    pub fn set_free(&self, free: Option<u64>) {
        *self.free.lock().unwrap() = Some(free);
    }

    fn fault(&self, kind: Kind, path: &Path) -> Option<Fault> {
        let mut rules = self.rules.lock().unwrap();
        let p = path.to_string_lossy();
        let i = rules
            .iter()
            .position(|r| r.kind == kind && p.contains(&r.matches))?;
        let fault = rules[i].fault;
        if let Some(t) = &mut rules[i].times {
            *t -= 1;
            if *t == 0 {
                rules.remove(i);
            }
        }
        Some(fault)
    }

    fn check(&self, kind: Kind, path: &Path) -> io::Result<()> {
        match self.fault(kind, path) {
            Some(Fault::Fail) => Err(io::Error::other(format!(
                "test failure: {kind:?} {}",
                path.display()
            ))),
            _ => Ok(()),
        }
    }
}

impl Fs for Faulty {
    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        self.inner.create_dir_all(dir)
    }

    fn create_dir_new(&self, dir: &Path) -> io::Result<()> {
        self.inner.create_dir_new(dir)
    }

    fn check_folder(&self, dir: &Path) -> io::Result<()> {
        self.inner.check_folder(dir)
    }

    fn write_temp(&self, path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
        self.check(Kind::WriteTemp, path)?;
        self.inner.write_temp(path, bytes)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.check(Kind::Rename, to)?;
        self.inner.rename(from, to)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        match self.fault(Kind::Remove, path) {
            Some(Fault::Fail) => Err(io::Error::other(format!(
                "test failure: removing {}",
                path.display()
            ))),
            Some(Fault::Pending) => Ok(()),
            None => self.inner.remove_file(path),
        }
    }

    fn remove_dir_all(&self, dir: &Path) -> io::Result<()> {
        self.inner.remove_dir_all(dir)
    }

    fn exists(&self, path: &Path) -> bool {
        self.inner.exists(path)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        self.check(Kind::Sync, dir)?;
        self.inner.sync_dir(dir)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.inner.read(path)
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        self.inner.list(dir)
    }

    fn lock(&self, path: &Path, create: bool) -> io::Result<Option<Held>> {
        self.inner.lock(path, create)
    }

    fn free_space(&self, dir: &Path) -> io::Result<u64> {
        match *self.free.lock().unwrap() {
            Some(Some(n)) => Ok(n),
            Some(None) => Err(io::Error::other("test failure: no reading")),
            None => self.inner.free_space(dir),
        }
    }
}
