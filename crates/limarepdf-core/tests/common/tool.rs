//! Outside programs the tests check LimarePDF's output with (poppler, Ghostscript, OpenSSL,
//! CUPS's tools). Each runs with a deadline, so one that hangs fails its test instead of
//! stopping the run, with its output read as it comes, so a full pipe can't stop it. Its files
//! go in a folder of the test's own under Cargo's folder for test files.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long one run of a program may take; `LIMAREPDF_TOOL_TIMEOUT` (seconds) changes it.
pub const TIMEOUT: Duration = Duration::from_secs(120);

fn timeout() -> Duration {
    std::env::var("LIMAREPDF_TOOL_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(TIMEOUT, Duration::from_secs)
}

/// A folder of one test's own in `target/tmp/tools`, its name unique to the run and the call.
/// Removed when dropped, unless the test is failing: then it stays, to look at.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("tools")
            .join(format!("{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, p: impl AsRef<Path>) -> PathBuf {
        self.0.join(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Runs `cmd` as `Command::output` does, but stopped (with anything it started) and the test
/// failed once it takes longer than [`TIMEOUT`]. An error means the program couldn't start,
/// most often because it isn't installed.
pub fn output(cmd: &mut Command) -> std::io::Result<Output> {
    run(cmd, None, timeout())
}

/// As [`output`], with `input` written to the program's standard input.
pub fn output_with_input(cmd: &mut Command, input: &[u8]) -> std::io::Result<Output> {
    run(cmd, Some(input), timeout())
}

/// As [`output`], stopped after `limit` instead.
pub fn output_within(cmd: &mut Command, limit: Duration) -> std::io::Result<Output> {
    run(cmd, None, limit)
}

/// Whether `program` is installed: it runs with `arg` (its version option, say) in time.
pub fn installed(program: &str, arg: &str) -> bool {
    output(Command::new(program).arg(arg)).is_ok()
}

fn run(cmd: &mut Command, input: Option<&[u8]>, limit: Duration) -> std::io::Result<Output> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    // A group of its own, so that whatever it starts is stopped with it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    let mut child = cmd
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    std::thread::scope(|s| {
        // Written and read on threads of their own, so that a full pipe can't block.
        if let (Some(mut w), Some(data)) = (stdin, input) {
            s.spawn(move || {
                let _ = w.write_all(data);
            });
        }
        let read = |pipe: Option<Box<dyn Read + Send>>| {
            s.spawn(move || {
                let mut v = Vec::new();
                if let Some(mut p) = pipe {
                    let _ = p.read_to_end(&mut v);
                }
                v
            })
        };
        let out = read(stdout.map(|p| Box::new(p) as Box<dyn Read + Send>));
        let err = read(stderr.map(|p| Box::new(p) as Box<dyn Read + Send>));
        let deadline = Instant::now() + limit;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Ok(None) => {
                    stop(&mut child);
                    panic!(
                        "{program} didn't finish within {} seconds (LIMAREPDF_TOOL_TIMEOUT changes it)",
                        limit.as_secs()
                    );
                }
                Err(e) => {
                    stop(&mut child);
                    return Err(e);
                }
            }
        };
        Ok(Output {
            status,
            stdout: out.join().unwrap_or_default(),
            stderr: err.join().unwrap_or_default(),
        })
    })
}

/// Stops `child` and its group, and waits for it.
fn stop(child: &mut Child) {
    #[cfg(unix)]
    let _ = rustix::process::kill_process_group(
        rustix::process::Pid::from_child(child),
        rustix::process::Signal::KILL,
    );
    let _ = child.kill();
    let _ = child.wait();
}
