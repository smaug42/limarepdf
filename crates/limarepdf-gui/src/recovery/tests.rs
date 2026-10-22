//! The store's tests (the design's section 9, stage R1).

use super::model::{Faulty, Kind, Model};
use super::*;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{RwLock, RwLockReadGuard};

const T1: TabId = TabId(1);
const T2: TabId = TabId(2);
const T3: TabId = TabId(3);

/// A clock the test sets, in seconds.
fn clock(at: u64) -> (Clock, Arc<AtomicU64>) {
    let t = Arc::new(AtomicU64::new(at));
    let c = t.clone();
    (
        Arc::new(move || UNIX_EPOCH + Duration::from_secs(c.load(Ordering::SeqCst))),
        t,
    )
}

const DAY: u64 = 24 * 60 * 60;
const T0: u64 = 1_800_000_000;

fn root() -> PathBuf {
    PathBuf::from("/state/limarepdf/recovery")
}

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(T0)
}

fn start(fs: Arc<dyn Fs>, root: &Path) -> Arc<Store> {
    Arc::new(Store::start(fs, root, clock(T0).0).unwrap())
}

/// A store on a fresh model disk.
fn model_store() -> (Model, Arc<Store>) {
    let m = Model::new();
    let s = start(Arc::new(m.clone()), &root());
    (m, s)
}

fn info(title: &str) -> DocInfo {
    DocInfo {
        title: title.into(),
        pages: 2,
        ..DocInfo::default()
    }
}

/// A made-up copy: the bytes only need to be told apart.
fn pdf(n: u8) -> Vec<u8> {
    let mut v = b"%PDF-1.7 made up ".to_vec();
    v.extend(std::iter::repeat_n(n, 100 + n as usize));
    v
}

fn write(s: &Store, tab: TabId, rev: u64, bytes: &[u8]) -> Result<Published, WriteError> {
    let t = s.ticket(tab, rev).expect("a ticket");
    s.publish(&t, &info(&format!("tab {}", tab.0)), bytes)
}

fn written(s: &Store, tab: TabId, rev: u64, bytes: &[u8]) {
    assert_eq!(write(s, tab, rev, bytes).unwrap(), Published::Written(rev));
}

/// What a start would offer on `fs`: each copy's tab and bytes, or why it is damaged.
fn offered(fs: Arc<dyn Fs>, root: &Path) -> Vec<(TabId, Result<Vec<u8>, String>)> {
    let mut out = Vec::new();
    for c in find_crashed(&fs, root, now()) {
        for (i, d) in c.docs.iter().enumerate() {
            out.push((d.tab, c.read(i)));
        }
    }
    out
}

fn set_hook(s: &Store, f: impl Fn(Point) + Send + Sync + 'static) {
    *lock(&s.hook) = Some(Arc::new(f));
}

fn clear_hook(s: &Store) {
    *lock(&s.hook) = None;
}

/// Stops the thread that reaches `at` until the test sends on the returned sender; the
/// receiver hears when it got there.
fn gate(s: &Store, at: Point) -> (Receiver<()>, Sender<()>) {
    let (reached_tx, reached) = mpsc::channel();
    let (go, go_rx) = mpsc::channel::<()>();
    let go_rx = Mutex::new(go_rx);
    set_hook(s, move |p| {
        if p == at {
            reached_tx.send(()).unwrap();
            go_rx.lock().unwrap().recv().unwrap();
        }
    });
    (reached, go)
}

/// The files in the session folder that are a tab's copies or descriptions (not the lock).
fn copies(m: &Model, s: &Store) -> Vec<String> {
    m.names(s.dir())
        .into_iter()
        .filter(|n| n != "lock")
        .collect()
}

/// A process being started shares every file this one has open, session locks included,
/// until it runs its program; a test checking a lock on the real disk meanwhile finds it
/// held. Tests that hold locks on the real disk read this for as long as they run, and the
/// one that starts a process writes it while it does.
static SPAWN: RwLock<()> = RwLock::new(());

fn real_disk() -> RwLockReadGuard<'static, ()> {
    SPAWN.read().unwrap_or_else(|e| e.into_inner())
}

/// A fresh, empty folder on the real disk for one test.
fn folder(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("limarepdf-recovery-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
fn mode(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
}

#[test]
fn descriptions_round_trip() {
    let mut i = info("Line one\nline two \\ \u{7} café");
    i.repaired = true;
    i.scrubbed = true;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        i.original = Some(PathBuf::from(std::ffi::OsString::from_vec(
            b"/home/x/odd\nname\xff.pdf".to_vec(),
        )));
    }
    #[cfg(not(unix))]
    {
        i.original = Some(PathBuf::from(r"C:\Users\x\odd name.pdf"));
    }
    let d = Description {
        info: i,
        time: UNIX_EPOCH + Duration::from_secs(T0),
        pdf: "t7-3-0.pdf".into(),
        len: 42,
        sha256: [0xab; 32],
    };
    let text = d.encode();
    assert_eq!(text.lines().count(), 9, "{text}");
    assert_eq!(
        Description::decode(TabId(7), text.as_bytes()),
        Ok(Some(d.clone()))
    );
    assert_eq!(
        Description::decode(TabId(7), withdrawn_text().as_bytes()),
        Ok(None)
    );
    // A description may only name its own tab's copy, in the same folder.
    for bad in [
        "t8-3-0.pdf",
        "../t7-3-0.pdf",
        "t7-3.pdf",
        "t7-3-0.pdf.x",
        "t7--0.pdf",
    ] {
        let t = text.replace("t7-3-0.pdf", bad);
        assert!(
            Description::decode(TabId(7), t.as_bytes()).is_err(),
            "{bad}"
        );
    }
    assert!(Description::decode(TabId(7), b"something else\n").is_err());
    let cut = text.replace("pages 2\n", "");
    assert!(Description::decode(TabId(7), cut.as_bytes()).is_err());
}

#[test]
fn file_names() {
    assert!(T1.owns_pdf("t1-5-0.pdf"));
    assert!(!T1.owns_pdf("t12-5-0.pdf"));
    assert!(!T1.owns_pdf("t1-5-0.pdf.tmp"));
    assert!(T1.owns_temp(".t1-5-0.pdf.limarepdf-1-2-ab.tmp"));
    assert!(T1.owns_temp(".t1.txt.m3.tmp"));
    assert!(!T1.owns_temp(".t12.txt.m3.tmp"));
    assert_eq!(description_tab("t12.txt"), Some(TabId(12)));
    assert_eq!(description_tab("t.txt"), None);
    assert_eq!(description_tab("lock"), None);
    assert_eq!(
        session_start("session-42-1800000000-3"),
        Some(UNIX_EPOCH + Duration::from_secs(T0))
    );
}

#[test]
fn folder_judgement() {
    use files::{judge_folder, Verdict};
    assert_eq!(judge_folder(1000, 0o40700, 1000), Verdict::Fine);
    assert_eq!(judge_folder(1000, 0o40755, 1000), Verdict::Tighten);
    assert_eq!(judge_folder(1000, 0o40500, 1000), Verdict::Tighten);
    assert_eq!(judge_folder(0, 0o40700, 1000), Verdict::NotOurs);
    assert_eq!(judge_folder(1001, 0o40777, 1000), Verdict::NotOurs);
}

/// Write, find and read back; a second write replaces the first and leaves one PDF.
#[test]
fn write_find_and_read_back() {
    let (m, s) = model_store();
    written(&s, T1, 1, &pdf(1));
    written(&s, T1, 2, &pdf(2));
    written(&s, T2, 1, &pdf(3));
    let names = copies(&m, &s);
    assert_eq!(names, ["t1-2-1.pdf", "t1.txt", "t2-1-2.pdf", "t2.txt"]);
    // The running app's session is never offered.
    assert!(offered(Arc::new(m.clone()), &root()).is_empty());
    drop(s);
    let fs: Arc<dyn Fs> = Arc::new(m.clone());
    let found = find_crashed(&fs, &root(), now());
    assert_eq!(found.len(), 1);
    let c = &found[0];
    assert_eq!(c.keep_until, now() + KEEP);
    let d = c.docs[0].state.as_ref().unwrap();
    assert_eq!(d.info, info("tab 1"));
    assert_eq!(d.time, now());
    assert_eq!(c.read(0).unwrap(), pdf(2));
    assert_eq!(c.read(1).unwrap(), pdf(3));
    // While this start holds the session, another start doesn't offer it.
    assert!(find_crashed(&fs, &root(), now()).is_empty());
}

/// A normal exit leaves nothing; saving or closing a tab removes its copy.
#[test]
fn removal_and_end() {
    let (m, s) = model_store();
    written(&s, T1, 1, &pdf(1));
    written(&s, T2, 1, &pdf(2));
    s.remove(T1).unwrap();
    assert_eq!(copies(&m, &s), ["t2-1-1.pdf", "t2.txt"]);
    s.remove(T3).unwrap();
    s.end().unwrap();
    assert!(m.names(&root()).is_empty());
    assert!(s.ticket(T1, 2).is_none());
}

/// Checks that a start on `fs` offers one whole copy of `T1`, one of `allowed`, or none if
/// `none_ok`.
fn whole(fs: Arc<dyn Fs>, root: &Path, allowed: &[&[u8]], none_ok: bool, when: &str) -> bool {
    let o = offered(fs, root);
    match o.as_slice() {
        [] => none_ok,
        [(t, Ok(b))] => *t == T1 && allowed.contains(&b.as_slice()),
        _ => {
            eprintln!("{when}: {o:?}");
            false
        }
    }
}

const PUBLISH_POINTS: [Point; 8] = [
    Point::TempsWritten,
    Point::Locked,
    Point::Checked,
    Point::PdfRenamed,
    Point::PdfSynced,
    Point::DescriptionRenamed,
    Point::DescriptionSynced,
    Point::PreviousRemoved,
];

/// Copies the session folder as it is into a new root, as an app crash would leave it.
fn copy_session(from: &Path, to_root: &Path) {
    let to = to_root.join(from.file_name().unwrap());
    std::fs::create_dir_all(&to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
}

/// An app crash at each step of a publish, on the real disk: the pair found is always whole.
#[test]
fn app_crash_at_each_publish_step() {
    let _disk = real_disk();
    let base = folder("app-crash");
    for first in [true, false] {
        let root = base.join(format!("root-{first}"));
        let s = start(Arc::new(RealFs), &root);
        if !first {
            written(&s, T1, 1, &pdf(1));
        }
        let images = Arc::new(Mutex::new(Vec::new()));
        let (dir, im) = (s.dir().to_path_buf(), images.clone());
        let b = base.clone();
        set_hook(&s, move |p| {
            let mut im = im.lock().unwrap();
            let r = b.join(format!("image-{first}-{}", im.len()));
            copy_session(&dir, &r);
            im.push((p, r));
        });
        written(&s, T1, 2, &pdf(2));
        clear_hook(&s);
        let images = images.lock().unwrap();
        assert_eq!(images.len(), PUBLISH_POINTS.len());
        for (p, r) in images.iter() {
            let allowed: &[&[u8]] = if first {
                &[&pdf(2)]
            } else {
                &[&pdf(1), &pdf(2)]
            };
            assert!(
                whole(Arc::new(RealFs), r, allowed, first, &format!("{p:?}")),
                "{p:?}"
            );
            // Leftover temporary files and unnamed copies are gone after the start.
            if let Some(sess) = std::fs::read_dir(r).unwrap().next() {
                for f in std::fs::read_dir(sess.unwrap().path()).unwrap() {
                    let n = f.unwrap().file_name().into_string().unwrap();
                    assert!(!n.ends_with(".tmp"), "{p:?}: {n}");
                }
            }
        }
        s.end().unwrap();
    }
    std::fs::remove_dir_all(base).unwrap();
}

/// Runs a publish of `T1` on a model disk, collecting the disk as every power cut could leave
/// it after each step; returns whether every one of them is a whole pair.
fn power_cuts_whole(first: bool, wrong_order: bool) -> bool {
    let (m, s) = model_store();
    if !first {
        written(&s, T1, 1, &pdf(1));
    }
    s.wrong_order.store(wrong_order, Ordering::SeqCst);
    let images = Arc::new(Mutex::new(Vec::new()));
    let (mm, im) = (m.clone(), images.clone());
    set_hook(&s, move |p| {
        for cut in mm.power_cuts() {
            im.lock().unwrap().push((p, cut));
        }
    });
    written(&s, T1, 2, &pdf(2));
    clear_hook(&s);
    let mut all = images.lock().unwrap().split_off(0);
    all.extend(
        m.power_cuts()
            .into_iter()
            .map(|c| (Point::PreviousRemoved, c)),
    );
    let allowed: &[&[u8]] = if first {
        &[&pdf(2)]
    } else {
        &[&pdf(1), &pdf(2)]
    };
    let mut ok = true;
    for (p, cut) in all {
        if !whole(Arc::new(cut), &root(), allowed, first, &format!("{p:?}")) {
            ok = false;
        }
    }
    ok
}

/// A power cut at each step of a publish, on the model disk: during a first write, no copy
/// or the new pair; with a previous copy, always a whole pair.
#[test]
fn power_cut_at_each_publish_step() {
    assert!(power_cuts_whole(true, false));
    assert!(power_cuts_whole(false, false));
}

/// The same with the previous copy removed before the second sync fails, so the test can
/// catch a wrong order.
#[test]
fn power_cut_catches_the_wrong_order() {
    assert!(!power_cuts_whole(false, true));
}

/// A power cut after each step of a withdrawal: before its sync the copy can come back; after
/// it, nothing is offered. A failing sync is reported as not confirmed.
#[test]
fn power_cut_during_a_withdrawal() {
    let (m, s) = model_store();
    written(&s, T1, 1, &pdf(1));
    let images = Arc::new(Mutex::new(Vec::new()));
    let (mm, im) = (m.clone(), images.clone());
    set_hook(&s, move |p| im.lock().unwrap().push((p, mm.power_cuts())));
    assert_eq!(s.withdraw(T1), Withdrawal::Confirmed);
    clear_hook(&s);
    let images = images.lock().unwrap();
    let points: Vec<Point> = images.iter().map(|(p, _)| *p).collect();
    assert_eq!(
        points,
        [
            Point::WithdrewDescription,
            Point::WithdrewFiles,
            Point::WithdrewSynced
        ]
    );
    for (p, cuts) in images.iter() {
        let offers = cuts
            .iter()
            .filter(|c| !offered(Arc::new((*c).clone()), &root()).is_empty())
            .count();
        if *p == Point::WithdrewSynced {
            assert_eq!(offers, 0, "{p:?}");
        } else {
            assert!(
                offers > 0,
                "{p:?}: the removals can't come back before the sync"
            );
        }
    }

    // The same with the sync failing: the withdrawal says so.
    let (m, _) = model_store();
    let f = Faulty::new(Arc::new(m.clone()));
    let s = start(f.clone(), &PathBuf::from("/other"));
    written(&s, T1, 1, &pdf(1));
    f.fail(Kind::Sync, "session-");
    let w = s.withdraw(T1);
    assert!(
        matches!(&w, Withdrawal::NotConfirmed(NotConfirmed::SyncFailed { folder, .. }) if folder == s.dir()),
        "{w:?}"
    );
    assert!(!w.may_proceed());
    assert!(m
        .power_cuts()
        .iter()
        .any(|c| !offered(Arc::new(c.clone()), &PathBuf::from("/other")).is_empty()));
    // No new copies until a withdrawal is confirmed.
    assert!(s.ticket(T1, 2).is_none());
    f.clear();
    assert_eq!(s.withdraw(T1), Withdrawal::Confirmed);
    assert!(s.ticket(T1, 2).is_some());
}

/// A failure of each step of a publish: the write counts as failed and the copy found
/// afterwards is the previous one or the new one, whole.
#[test]
fn a_failure_at_each_publish_step() {
    // (what fails, when the failure is armed)
    let cases: [(Kind, &str, Option<Point>); 5] = [
        (Kind::WriteTemp, ".pdf", None),
        (Kind::Rename, ".pdf", None),
        (Kind::Sync, "session-", None),
        (Kind::Rename, ".txt", None),
        (Kind::Sync, "session-", Some(Point::PdfSynced)),
    ];
    for (kind, what, armed) in cases {
        let m = Model::new();
        let f = Faulty::new(Arc::new(m.clone()));
        let s = start(f.clone(), &root());
        written(&s, T1, 1, &pdf(1));
        match armed {
            None => f.fail_once(kind, what),
            Some(at) => {
                let f = f.clone();
                set_hook(&s, move |p| {
                    if p == at {
                        f.fail_once(kind, what)
                    }
                })
            }
        }
        let e = write(&s, T1, 2, &pdf(2)).unwrap_err();
        assert!(matches!(e, WriteError::Failed(_)), "{kind:?} {what}: {e}");
        clear_hook(&s);
        for cut in std::iter::once(m.crash()).chain(m.power_cuts()) {
            assert!(
                whole(Arc::new(cut), &root(), &[&pdf(1), &pdf(2)], false, what),
                "{kind:?} {what} {armed:?}"
            );
        }
        // The next write works and leaves one copy.
        written(&s, T1, 3, &pdf(3));
        let names = copies(&m, &s);
        assert_eq!(names.len(), 2, "{kind:?} {what}: {names:?}");
    }
}

/// The previous copy's removal failing leaves the new copy written; the old file is named by
/// no description, never offered, and goes with the next publish.
#[test]
fn the_previous_copy_left_behind() {
    let m = Model::new();
    let f = Faulty::new(Arc::new(m.clone()));
    let s = start(f.clone(), &root());
    written(&s, T1, 1, &pdf(1));
    f.fail_once(Kind::Remove, "t1-1-0.pdf");
    written(&s, T1, 2, &pdf(2));
    assert_eq!(copies(&m, &s), ["t1-1-0.pdf", "t1-2-1.pdf", "t1.txt"]);
    let crash = m.crash();
    assert!(whole(
        Arc::new(crash.clone()),
        &root(),
        &[&pdf(2)],
        false,
        "left"
    ));
    assert!(!crash.names(s.dir()).contains(&"t1-1-0.pdf".into()));
    written(&s, T1, 3, &pdf(3));
    assert_eq!(copies(&m, &s), ["t1-3-2.pdf", "t1.txt"]);
}

/// A description whose copy is missing, cut short or changed is reported, not offered as
/// good, and can be shown and discarded.
#[test]
fn damaged_copies() {
    let _disk = real_disk();
    let base = folder("damaged");
    let root = base.join("recovery");
    let s = start(Arc::new(RealFs), &root);
    for t in [T1, T2, T3] {
        written(&s, t, 1, &pdf(t.0 as u8));
    }
    written(&s, TabId(4), 1, &pdf(4));
    let dir = s.dir().to_path_buf();
    drop(s);
    std::fs::remove_file(dir.join("t1-1-0.pdf")).unwrap();
    std::fs::write(dir.join("t2-1-1.pdf"), &pdf(2)[..50]).unwrap();
    let mut changed = pdf(3);
    changed[20] ^= 1;
    std::fs::write(dir.join("t3-1-2.pdf"), changed).unwrap();
    let fs: Arc<dyn Fs> = Arc::new(RealFs);
    let mut found = find_crashed(&fs, &root, now());
    let c = &mut found[0];
    let states: Vec<_> = c
        .docs
        .iter()
        .map(|d| (d.tab, d.state.as_ref().err().cloned()))
        .collect();
    assert_eq!(
        states,
        [
            (T1, Some("the copy is missing".to_string())),
            (T2, Some("the copy is cut short or changed".to_string())),
            (T3, Some("the copy has changed".to_string())),
            (TabId(4), None),
        ]
    );
    for i in 0..3 {
        assert!(c.read(i).is_err());
    }
    assert_eq!(c.file(0), Some(dir.join("t1.txt")));
    assert_eq!(c.read(3).unwrap(), pdf(4));
    // Damaged ones are kept until discarded: never deleted silently.
    drop(found);
    let mut found = find_crashed(&fs, &root, now());
    assert_eq!(found[0].docs.len(), 4);
    for _ in 0..4 {
        found[0].discard(0).unwrap();
    }
    assert!(!dir.exists());
    std::fs::remove_dir_all(base).unwrap();
}

/// Times on disk too large to hold, or past the year 9999, never stop a start (E12): a
/// description with one is damaged, and a session folder named with one counts as old.
#[test]
fn damaged_times_never_stop_a_start() {
    for time in [u64::MAX, 10_000_000_000_000, 253_402_300_800] {
        let (m, s) = model_store();
        written(&s, T1, 1, &pdf(1));
        let desc = s.dir().join(T1.description());
        drop(s);
        let text = String::from_utf8(m.read(&desc).unwrap()).unwrap();
        let bad = text.replace(&format!("time {T0}\n"), &format!("time {time}\n"));
        assert_ne!(bad, text);
        let tmp = m.write_temp(&desc, bad.as_bytes()).unwrap();
        m.rename(&tmp, &desc).unwrap();
        let fs: Arc<dyn Fs> = Arc::new(m.clone());
        let found = find_crashed(&fs, &root(), now());
        assert_eq!(
            found[0].docs[0].state,
            Err("its description has a bad time line".into()),
            "{time}"
        );
    }
    // The last second of 9999 is still a time.
    assert!(Description::decode(
        T1,
        format!(
            "{HEADER}\npdf t1-1-0.pdf\nlength 1\nsha256 {}\ntime 253402300799\npages 1\n",
            "00".repeat(32)
        )
        .as_bytes()
    )
    .is_ok());
    for time in [u64::MAX, 10_000_000_000_000] {
        let m = Model::new();
        let dir = root().join(format!("session-1-{time}"));
        m.create_dir_all(&dir).unwrap();
        let fs: Arc<dyn Fs> = Arc::new(m.clone());
        assert!(find_crashed(&fs, &root(), now()).is_empty());
        assert!(!m.exists(&dir), "{time}");
    }
}

/// Withdrawing a crashed session's copy (a restored tab's, before a redaction, a removal of
/// hidden information or a password): section 6's outcomes, as for this session's copies.
#[test]
fn withdrawing_a_crashed_copy() {
    // Copies of tabs 1 and 2 left by a crash, on a disk that fails as told.
    let setup = || {
        let m = Model::new();
        let s = start(Arc::new(m.clone()), &root());
        written(&s, T1, 1, &pdf(1));
        written(&s, T2, 1, &pdf(2));
        let disk = m.crash();
        let f = Faulty::new(Arc::new(disk.clone()));
        let fs: Arc<dyn Fs> = f.clone();
        let c = find_crashed(&fs, &root(), now()).pop().unwrap();
        (disk, f, c)
    };
    let next_start = |m: &Model| offered(Arc::new(m.crash()), &root());

    // Both withdrawn: nothing left, and the session goes.
    let (m, _, mut c) = setup();
    assert_eq!(c.withdraw(T1), Withdrawal::Confirmed);
    assert_eq!(next_start(&m), [(T2, Ok(pdf(2)))]);
    assert_eq!(c.withdraw(T2), Withdrawal::Confirmed);
    assert!(!m.exists(&c.dir));
    assert!(c.docs.is_empty());

    // The PDF can't be removed: invalidated, not offered, reported; a retry removes it.
    let (m, f, mut c) = setup();
    f.fail(Kind::Remove, "t1-");
    let pdf_path = c.dir.join("t1-1-0.pdf");
    assert_eq!(c.withdraw(T1), Withdrawal::Leftover(vec![pdf_path.clone()]));
    assert_eq!(next_start(&m), [(T2, Ok(pdf(2)))]);
    f.clear();
    c.retry_leftovers();
    assert!(!m.exists(&pdf_path));

    // The folder sync fails: after a power cut the copy could come back.
    let (_, f, mut c) = setup();
    f.fail(Kind::Sync, "session-");
    let w = c.withdraw(T1);
    assert!(
        matches!(&w, Withdrawal::NotConfirmed(NotConfirmed::SyncFailed { folder, .. }) if *folder == c.dir),
        "{w:?}"
    );

    // The description can be neither removed nor replaced: it could be offered.
    let (m, f, mut c) = setup();
    f.fail(Kind::Remove, "t1.txt");
    f.fail(Kind::Rename, "t1.txt");
    let w = c.withdraw(T1);
    assert!(
        matches!(&w, Withdrawal::NotConfirmed(NotConfirmed::Kept { file, .. }) if *file == c.dir.join("t1.txt")),
        "{w:?}"
    );
    assert_eq!(c.docs.len(), 2);
    assert!(m.exists(&c.dir.join("t1.txt")));
}

/// A crashed session's copy whose withdrawal couldn't be confirmed (the folder sync failed)
/// is no longer listed to restore, but stays unconfirmed at every try until a sync succeeds,
/// and its session folder stays meanwhile; files it couldn't delete are tracked throughout
/// and retried. Whenever a withdrawal lets the command go ahead, no crash or power cut can
/// bring the copy back.
#[test]
fn a_crashed_withdrawal_stays_unconfirmed_until_a_sync_succeeds() {
    let m = Model::new();
    let s = start(Arc::new(m.clone()), &root());
    written(&s, T1, 1, &pdf(1));
    written(&s, T2, 1, &pdf(2));
    let disk = m.crash();
    let f = Faulty::new(Arc::new(disk.clone()));
    let fs: Arc<dyn Fs> = f.clone();
    let mut c = find_crashed(&fs, &root(), now()).pop().unwrap();
    let dir = c.dir.clone();
    let t1_pdf = dir.join("t1-1-0.pdf");
    let offers = |tab: TabId| {
        std::iter::once(disk.crash())
            .chain(disk.power_cuts())
            .any(|cut| {
                offered(Arc::new(cut), &root())
                    .iter()
                    .any(|(t, _)| *t == tab)
            })
    };

    // The sync fails, and the PDF can't be deleted either: not confirmed, at every try.
    f.fail(Kind::Sync, "session-");
    f.fail(Kind::Remove, "t1-");
    for _ in 0..3 {
        let w = c.withdraw(T1);
        assert!(
            matches!(&w, Withdrawal::NotConfirmed(NotConfirmed::SyncFailed { folder, .. }) if *folder == dir),
            "{w:?}"
        );
        assert!(!w.may_proceed());
        assert_eq!(c.docs.len(), 1, "no longer listed to restore");
        assert!(offers(T1), "a power cut could bring it back");
        assert!(c.leftovers.iter().eq([&t1_pdf]), "{:?}", c.leftovers);
    }

    // The sync works again, the PDF still can't be deleted: invalidated, not offered.
    f.clear();
    f.fail(Kind::Remove, "t1-");
    assert_eq!(c.withdraw(T1), Withdrawal::Leftover(vec![t1_pdf.clone()]));
    assert!(!offers(T1));
    assert_eq!(c.withdraw(T1), Withdrawal::Confirmed);
    c.retry_leftovers();
    assert!(disk.exists(&t1_pdf));
    f.clear();
    c.retry_leftovers();
    assert!(!disk.exists(&t1_pdf));
    assert!(c.leftovers.is_empty());
    assert!(!offers(T1));
    assert!(offers(T2));

    // The last copy, not confirmed: the folder stays until it is, then goes.
    f.fail(Kind::Sync, "session-");
    assert!(!c.withdraw(T2).may_proceed());
    assert!(c.docs.is_empty() && disk.exists(&dir));
    assert!(!c.withdraw(T2).may_proceed());
    f.clear();
    assert_eq!(c.withdraw(T2), Withdrawal::Confirmed);
    assert!(!disk.exists(&dir));
    assert!(!offers(T1) && !offers(T2));
}

/// The environment variable that makes [`child_session`] act.
const CHILD: &str = "LIMAREPDF_RECOVERY_TEST_CHILD";

/// Run as a child by [`a_running_session_is_not_offered`]: starts a session with one copy,
/// says so, and stops abnormally when told to.
#[test]
fn child_session() {
    let Some(root) = std::env::var_os(CHILD) else {
        return;
    };
    let s = start(Arc::new(RealFs), Path::new(&root));
    written(&s, T1, 1, &pdf(1));
    println!("ready");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    std::process::abort();
}

/// A session whose lock another process holds isn't offered; once that process stops
/// abnormally, it is.
#[test]
fn a_running_session_is_not_offered() {
    use std::io::{BufRead, Write};
    let base = folder("child");
    let spawning = SPAWN.write().unwrap_or_else(|e| e.into_inner());
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "recovery::tests::child_session",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, &base)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    drop(spawning);
    let out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut lines = out.lines();
    // The test harness may print the test's name on the same line first.
    assert!(
        lines.any(|l| l.unwrap().ends_with("ready")),
        "the child didn't start"
    );
    let fs: Arc<dyn Fs> = Arc::new(RealFs);
    assert!(find_crashed(&fs, &base, now()).is_empty());
    assert_eq!(std::fs::read_dir(&base).unwrap().count(), 1);
    writeln!(child.stdin.take().unwrap()).unwrap();
    assert!(!child.wait().unwrap().success());
    let found = find_crashed(&fs, &base, now());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].read(0).unwrap(), pdf(1));
    drop(found);
    std::fs::remove_dir_all(base).unwrap();
}

/// A write into a folder made read-only fails, leaving the previous copy whole and no
/// temporary file.
#[cfg(unix)]
#[test]
fn a_read_only_folder() {
    use std::os::unix::fs::PermissionsExt;
    if limarepdf_core::disk::user_id() == 0 {
        return; // Permissions don't stop root.
    }
    let _disk = real_disk();
    let base = folder("read-only");
    let s = start(Arc::new(RealFs), &base);
    written(&s, T1, 1, &pdf(1));
    let dir = s.dir().to_path_buf();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let e = write(&s, T1, 2, &pdf(2));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(e, Err(WriteError::Failed(_))), "{e:?}");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["lock", "t1-1-0.pdf", "t1.txt"]);
    drop(s);
    assert!(whole(
        Arc::new(RealFs),
        &base,
        &[&pdf(1)],
        false,
        "read-only"
    ));
    std::fs::remove_dir_all(base).unwrap();
}

/// A write skipped for free space leaves the previous copy and says why; a reading that
/// fails lets the write go ahead.
#[test]
fn free_space() {
    let m = Model::new();
    let f = Faulty::new(Arc::new(m.clone()));
    let s = start(f.clone(), &root());
    written(&s, T1, 1, &pdf(1));
    f.set_free(Some(FREE_MARGIN + 100));
    let e = write(&s, T1, 2, &pdf(2)).unwrap_err();
    assert!(matches!(e, WriteError::LowSpace { .. }));
    assert!(e.to_string().contains("nearly full"), "{e}");
    assert_eq!(copies(&m, &s), ["t1-1-0.pdf", "t1.txt"]);
    f.set_free(Some(FREE_MARGIN + 10_000));
    written(&s, T1, 2, &pdf(2));
    f.set_free(None);
    written(&s, T1, 3, &pdf(3));
    assert_eq!(copies(&m, &s), ["t1-3-3.pdf", "t1.txt"]);
}

/// After a content removal: the old copy is gone before the new one is written, a failing
/// new write leaves no copy, and a write whose ticket came first leaves none either.
#[test]
fn after_a_content_removal() {
    let m = Model::new();
    let f = Faulty::new(Arc::new(m.clone()));
    let s = start(f.clone(), &root());
    written(&s, T1, 1, &pdf(1));
    let early = s.ticket(T1, 2).unwrap();
    assert_eq!(s.withdraw(T1), Withdrawal::Confirmed);
    assert!(copies(&m, &s).is_empty());
    assert_eq!(
        s.publish(&early, &info("early"), &pdf(2)).unwrap(),
        Published::Refused
    );
    assert!(copies(&m, &s).is_empty());
    f.fail_once(Kind::Rename, ".txt");
    assert!(write(&s, T1, 3, &pdf(3)).is_err());
    for cut in std::iter::once(m.crash()).chain(m.power_cuts()) {
        assert!(offered(Arc::new(cut), &root()).is_empty());
    }
    written(&s, T1, 4, &pdf(4));
    assert_eq!(copies(&m, &s), ["t1-4-3.pdf", "t1.txt"]);
}

/// Starts a write of `tab` on another thread, stopped at `at`.
fn stopped_writer(
    s: &Arc<Store>,
    tab: TabId,
    at: Point,
) -> (std::thread::JoinHandle<Published>, Sender<()>) {
    let (reached, go) = gate(s, at);
    let t = s.ticket(tab, 9).unwrap();
    let s2 = s.clone();
    let h = std::thread::spawn(move || s2.publish(&t, &info("racing"), &pdf(9)).unwrap());
    reached.recv().unwrap();
    (h, go)
}

/// Runs `f` on another thread; returns its handle and whether it finished within a moment.
fn finishes_alone<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> (std::thread::JoinHandle<T>, bool) {
    let (tx, rx) = mpsc::channel();
    let h = std::thread::spawn(move || {
        let r = f();
        let _ = tx.send(());
        r
    });
    let done = rx.recv_timeout(Duration::from_millis(300)).is_ok();
    (h, done)
}

/// The race between a write and a withdrawal (a content removal or a password added: the
/// same call), the writer stopped at each point.
#[test]
fn a_write_racing_a_withdrawal() {
    for at in PUBLISH_POINTS {
        let (m, s) = model_store();
        written(&s, T1, 1, &pdf(1));
        let (writer, go) = stopped_writer(&s, T1, at);
        let s2 = s.clone();
        let (withdrawal, done) = finishes_alone(move || s2.withdraw(T1));
        let before_lock = at == Point::TempsWritten;
        // The withdrawal waits for a writer holding the lock (after its last step too, until
        // it returns); one that hasn't taken it yet doesn't hold it up.
        assert_eq!(done, before_lock, "{at:?}");
        go.send(()).unwrap();
        let p = writer.join().unwrap();
        assert_eq!(
            p,
            if before_lock {
                Published::Refused
            } else {
                Published::Written(9)
            },
            "{at:?}"
        );
        assert_eq!(withdrawal.join().unwrap(), Withdrawal::Confirmed, "{at:?}");
        clear_hook(&s);
        assert!(copies(&m, &s).is_empty(), "{at:?}: {:?}", copies(&m, &s));
        for cut in std::iter::once(m.crash()).chain(m.power_cuts()) {
            assert!(offered(Arc::new(cut), &root()).is_empty(), "{at:?}");
        }
    }
}

/// The same for recovery turned off, with several tabs and one mid-write.
#[test]
fn a_write_racing_turning_off() {
    for at in [
        Point::TempsWritten,
        Point::Checked,
        Point::DescriptionRenamed,
    ] {
        let (m, s) = model_store();
        for t in [T1, T2, T3] {
            written(&s, t, 1, &pdf(t.0 as u8));
        }
        let (writer, go) = stopped_writer(&s, T2, at);
        let s2 = s.clone();
        let (off, done) = finishes_alone(move || s2.turn_off());
        assert_eq!(done, at == Point::TempsWritten, "{at:?}");
        assert!(!s.is_on());
        assert!(s.ticket(T1, 5).is_none());
        go.send(()).unwrap();
        writer.join().unwrap();
        let off = off.join().unwrap();
        assert_eq!(
            off,
            [
                (T1, Withdrawal::Confirmed),
                (T2, Withdrawal::Confirmed),
                (T3, Withdrawal::Confirmed)
            ]
        );
        clear_hook(&s);
        assert!(copies(&m, &s).is_empty(), "{at:?}");
        s.turn_on();
        written(&s, T1, 6, &pdf(6));
    }
}

/// Withdrawal failures, one per row of section 6's table, and turning recovery off with each.
#[test]
fn withdrawal_failures() {
    let setup = || {
        let m = Model::new();
        let f = Faulty::new(Arc::new(m.clone()));
        let s = start(f.clone(), &root());
        written(&s, T1, 1, &pdf(1));
        (m, f, s)
    };
    let nothing_offered = |m: &Model| {
        std::iter::once(m.crash())
            .chain(m.power_cuts())
            .all(|c| offered(Arc::new(c), &root()).is_empty())
    };

    // The PDF can't be removed: invalidated, not offered, reported; a retry removes it.
    let (m, f, s) = setup();
    f.fail(Kind::Remove, ".pdf");
    let w = s.withdraw(T1);
    let pdf_path = s.dir().join("t1-1-0.pdf");
    assert_eq!(w, Withdrawal::Leftover(vec![pdf_path.clone()]));
    assert!(w.may_proceed());
    assert!(nothing_offered(&m));
    assert!(
        s.ticket(T1, 2).is_none(),
        "no copies for the rest of the session"
    );
    assert_eq!(s.retry_leftovers(), std::slice::from_ref(&pdf_path));
    f.clear();
    assert!(s.retry_leftovers().is_empty());
    assert!(copies(&m, &s).is_empty());

    // The same with a delete left pending (the name stays): a failure.
    let (m, f, s) = setup();
    f.pending_delete(".pdf");
    assert!(matches!(s.withdraw(T1), Withdrawal::Leftover(_)));
    assert!(nothing_offered(&m));

    // The description can't be removed: replaced with a withdrawn one.
    let (m, f, s) = setup();
    f.fail(Kind::Remove, ".txt");
    assert_eq!(s.withdraw(T1), Withdrawal::Confirmed);
    assert!(nothing_offered(&m));
    assert_eq!(copies(&m, &s), ["t1.txt"]);
    assert_eq!(
        m.read(&s.dir().join("t1.txt")).unwrap(),
        withdrawn_text().as_bytes()
    );
    f.clear();
    written(&s, T1, 2, &pdf(2));

    // Neither removed nor replaced: it could be offered.
    let (m, f, s) = setup();
    f.fail(Kind::Remove, ".txt");
    f.fail(Kind::Rename, ".txt");
    let w = s.withdraw(T1);
    assert!(
        matches!(&w, Withdrawal::NotConfirmed(NotConfirmed::Kept { file, .. }) if *file == s.dir().join("t1.txt")),
        "{w:?}"
    );
    assert!(!w.may_proceed());
    // A pending delete of the description, with its replacement failing, is the same.
    let (_, f2, s2) = setup();
    f2.pending_delete(".txt");
    f2.fail(Kind::Rename, ".txt");
    assert!(!s2.withdraw(T1).may_proceed());
    // Turning recovery off still stops new copies.
    let off = s.turn_off();
    assert!(!off[0].1.may_proceed());
    assert!(s.ticket(T2, 1).is_none());
    assert!(!m.names(s.dir()).is_empty());

    // Removals done, folder sync failing: covered by power_cut_during_a_withdrawal; turning
    // off with it still stops new copies.
    let (_, f, s) = setup();
    f.fail(Kind::Sync, "session-");
    assert!(!s.turn_off()[0].1.may_proceed());
    assert!(!s.is_on());
    assert!(s.ticket(T2, 1).is_none());
}

/// A crash during a withdrawal (files as after each step): nothing is offered, and a copy
/// left with no description is removed at start.
#[test]
fn a_crash_during_a_withdrawal() {
    let (m, s) = model_store();
    written(&s, T1, 1, &pdf(1));
    written(&s, T2, 1, &pdf(2));
    let images = Arc::new(Mutex::new(Vec::new()));
    let (mm, im) = (m.clone(), images.clone());
    set_hook(&s, move |p| im.lock().unwrap().push((p, mm.crash())));
    assert_eq!(s.withdraw(T1), Withdrawal::Confirmed);
    clear_hook(&s);
    let dir = s.dir().to_path_buf();
    for (p, crash) in images.lock().unwrap().iter() {
        let o = offered(Arc::new(crash.clone()), &root());
        assert_eq!(o.len(), 1, "{p:?}");
        assert_eq!(o[0].0, T2, "{p:?}");
        let names = crash.names(&dir);
        assert!(
            !names.iter().any(|n| n.starts_with("t1")),
            "{p:?}: {names:?}"
        );
    }
}

/// The start-up folder sync failing turns recovery off for the run, and leaves nothing.
#[test]
fn a_start_that_cant_sync() {
    let m = Model::new();
    let f = Faulty::new(Arc::new(m.clone()));
    f.fail(Kind::Sync, "session-");
    let e = Store::start(f, &root(), clock(T0).0).err().unwrap();
    assert!(e.contains("can't confirm changes"), "{e}");
    assert!(m.names(&root()).is_empty());
}

/// Temporary files a write leaves behind (a crash while writing) are removed at start, as are
/// copies no description names; files of a running write are never touched by another.
#[test]
fn leftover_files_at_start() {
    let (m, s) = model_store();
    written(&s, T1, 1, &pdf(1));
    let dir = s.dir().to_path_buf();
    let stray = m.write_temp(&dir.join("t1-7-9.pdf"), b"half").unwrap();
    m.rename(&stray, &dir.join("t1-7-9.pdf")).unwrap();
    m.write_temp(&dir.join("t1.txt"), b"half").unwrap();
    drop(s);
    let fs: Arc<dyn Fs> = Arc::new(m.clone());
    let found = find_crashed(&fs, &root(), now());
    assert_eq!(found[0].docs.len(), 1);
    assert_eq!(m.names(&dir), ["lock", "t1-1-0.pdf", "t1.txt"]);
}

/// A session entry that isn't a real folder of this user's (a symbolic link, another user's)
/// is left untouched at start, and nothing in it is offered (R01).
#[test]
fn only_real_session_folders_are_scanned() {
    let (m, s) = model_store();
    written(&s, T1, 1, &pdf(1));
    let dir = s.dir().to_path_buf();
    let other = m.write_temp(&dir.join("report.pdf"), b"someone's").unwrap();
    m.rename(&other, &dir.join("report.pdf")).unwrap();
    drop(s);
    let before = m.names(&dir);
    let f = Faulty::new(Arc::new(m.clone()));
    f.fail(Kind::Check, dir.file_name().unwrap().to_str().unwrap());
    let fs: Arc<dyn Fs> = f;
    assert!(find_crashed(&fs, &root(), now()).is_empty());
    assert_eq!(m.names(&dir), before);
    // Control: as a real folder, the copy is offered and the stray file removed.
    let fs: Arc<dyn Fs> = Arc::new(m.clone());
    assert_eq!(find_crashed(&fs, &root(), now())[0].docs.len(), 1);
    assert_eq!(m.names(&dir), ["lock", "t1-1-0.pdf", "t1.txt"]);
}

/// On the real disk (R01): a session entry that is a symbolic link, and a recovery folder
/// that is one, are never followed. The folder they lead to keeps its files, and nothing in
/// it is offered.
#[cfg(unix)]
#[test]
fn symbolic_links_are_not_followed_at_start() {
    let _disk = real_disk();
    let base = folder("links");
    let s = start(Arc::new(RealFs), &base.join("made"));
    written(&s, T1, 1, &pdf(1));
    let name = s.dir().file_name().unwrap().to_owned();
    // A crashed session's files, with an unrelated PDF beside them, in a folder elsewhere.
    let elsewhere = base.join("elsewhere");
    copy_session(s.dir(), &elsewhere);
    drop(s);
    let outside = elsewhere.join(&name);
    std::fs::write(outside.join("report.pdf"), b"someone's").unwrap();
    let listing = |d: &Path| {
        let mut v: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    };
    let before = listing(&outside);
    let fs: Arc<dyn Fs> = Arc::new(RealFs);
    let root = base.join("recovery");
    std::fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(&outside, root.join(&name)).unwrap();
    assert!(find_crashed(&fs, &root, now()).is_empty());
    assert_eq!(listing(&outside), before);
    assert!(std::fs::symlink_metadata(root.join(&name)).is_ok());
    let linked = base.join("linked");
    std::os::unix::fs::symlink(&elsewhere, &linked).unwrap();
    assert!(find_crashed(&fs, &linked, now()).is_empty());
    assert_eq!(listing(&outside), before);
    // Control: the same folder as the recovery folder itself is scanned and tidied.
    let found = find_crashed(&fs, &elsewhere, now());
    assert_eq!(found[0].read(0).unwrap(), pdf(1));
    assert!(!outside.join("report.pdf").exists());
    drop(found);
    std::fs::remove_dir_all(base).unwrap();
}

/// On the real disk: folders 0700, files 0600, a wider recovery folder tightened, and nothing
/// left after a normal exit.
#[cfg(unix)]
#[test]
fn permissions_and_a_normal_exit() {
    use std::os::unix::fs::PermissionsExt;
    let _disk = real_disk();
    let base = folder("modes");
    let root = base.join("recovery");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let s = start(Arc::new(RealFs), &root);
    assert_eq!(mode(&root), 0o700);
    assert_eq!(mode(s.dir()), 0o700);
    written(&s, T1, 1, &pdf(1));
    for e in std::fs::read_dir(s.dir()).unwrap() {
        let p = e.unwrap().path();
        assert_eq!(mode(&p), 0o600, "{}", p.display());
    }
    s.end().unwrap();
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    // A recovery folder that is a symbolic link isn't used.
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::remove_dir(&root).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &root).unwrap();
    assert!(Store::start(Arc::new(RealFs), &root, system_clock()).is_err());
    std::fs::remove_dir_all(base).unwrap();
}

/// Sessions are kept for 30 days after the crash (the last copy's time), then removed.
#[test]
fn expiry() {
    let m = Model::new();
    let (c, t) = clock(T0);
    let s = Store::start(Arc::new(m.clone()), &root(), c).unwrap();
    t.store(T0 + 5 * DAY, Ordering::SeqCst);
    written(&s, T1, 1, &pdf(1));
    drop(s);
    let fs: Arc<dyn Fs> = Arc::new(m.clone());
    let at = |days: u64| UNIX_EPOCH + Duration::from_secs(T0 + days * DAY);
    let found = find_crashed(&fs, &root(), at(34));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].keep_until, at(35));
    drop(found);
    assert!(find_crashed(&fs, &root(), at(36)).is_empty());
    assert!(m.names(&root()).is_empty());
}

/// A session folder without a lock file is left alone while young (its app is making it),
/// and cleaned up when old.
#[test]
fn a_session_folder_being_made() {
    let m = Model::new();
    let dir = root().join(format!("session-77-{T0}"));
    m.create_dir_all(&dir).unwrap();
    let fs: Arc<dyn Fs> = Arc::new(m.clone());
    assert!(find_crashed(&fs, &root(), now() + Duration::from_secs(10)).is_empty());
    assert!(m.exists(&dir));
    assert!(find_crashed(&fs, &root(), now() + Duration::from_secs(3600)).is_empty());
    assert!(!m.exists(&dir));
}

/// The recovery thread: a removal queued after a write runs after it, leaving nothing; a
/// snapshot that fails or panics is reported and leaves the previous copy.
#[test]
fn the_recovery_thread() {
    let (m, s) = model_store();
    let q = Queue::new(s.clone(), Box::new(|| {}));
    let t = s.ticket(T1, 1).unwrap();
    q.write(
        t,
        info("slow"),
        Box::new(|| {
            std::thread::sleep(Duration::from_millis(100));
            Ok(pdf(1))
        }),
    );
    q.remove(T1);
    let t = s.ticket(T2, 1).unwrap();
    q.write(t, info("two"), Box::new(|| Ok(pdf(2))));
    let t = s.ticket(T2, 2).unwrap();
    q.write(
        t,
        info("fails"),
        Box::new(|| anyhow::bail!("out of memory")),
    );
    let t = s.ticket(T2, 3).unwrap();
    q.write(t, info("panics"), Box::new(|| panic!("bad object")));
    q.retry();
    let s = q.finish();
    assert_eq!(copies(&m, &s), ["t2-1-1.pdf", "t2.txt"]);
    s.end().unwrap();
}

/// The thread reports each job, in order, and wakes the app after each.
#[test]
fn the_recovery_thread_reports() {
    let (_, s) = model_store();
    let woken = Arc::new(AtomicU64::new(0));
    let w = woken.clone();
    let q = Queue::new(
        s.clone(),
        Box::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }),
    );
    let outdated = s.ticket(T1, 2).unwrap();
    q.write(s.ticket(T1, 1).unwrap(), info("a"), Box::new(|| Ok(pdf(1))));
    q.write(
        s.ticket(T1, 3).unwrap(),
        info("b"),
        Box::new(|| panic!("bad object")),
    );
    q.remove(T1);
    let mut reports = Vec::new();
    let wait = |reports: &mut Vec<Report>, n| {
        while reports.len() < n {
            reports.extend(q.reports());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    wait(&mut reports, 3);
    // A write whose ticket a withdrawal outdated is refused.
    assert_eq!(s.withdraw(T1), Withdrawal::Confirmed);
    q.write(outdated, info("c"), Box::new(|| Ok(pdf(2))));
    wait(&mut reports, 4);
    q.finish();
    assert_eq!(woken.load(Ordering::SeqCst), 4);
    let got: Vec<String> = reports.iter().map(|r| format!("{:?}", r.outcome)).collect();
    assert_eq!(got[0], "Written(1)");
    assert!(got[1].contains("bad object"), "{}", got[1]);
    assert_eq!(got[2..], ["Removed", "Refused"]);
    assert!(reports.iter().all(|r| r.tab == T1));
}
