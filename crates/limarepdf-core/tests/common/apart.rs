//! Corpus checks run a file at a time in a child process: the test runs again for that one
//! file, so a file that takes too long or too much memory is stopped without stopping the
//! others, and nothing of it is left running. The child prints its outcome in lines
//! starting with a marker the parent picks out of its output.

use std::io::Read as _;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How a child process ended.
pub enum Ran {
    /// It ended by itself: its standard output, and how it ended.
    Ended(String, ExitStatus),
    /// It took longer than its time and was stopped, with anything it started.
    TimedOut,
    /// It couldn't start.
    NotStarted(String),
}

/// Runs the ignored test `test` of this test program again, alone, with `env` set, for at most
/// `limit`.
pub fn run(test: &str, env: &[(&str, String)], limit: Duration) -> Ran {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", test, "--ignored"])
        .args(["--nocapture", "--test-threads", "1"])
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // A group of its own, so that whatever it starts is stopped with it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ran::NotStarted(e.to_string()),
    };
    // Read as it comes, so a full pipe can't stop the child.
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() > limit => {
                #[cfg(unix)]
                let _ = rustix::process::kill_process_group(
                    rustix::process::Pid::from_child(&child),
                    rustix::process::Signal::KILL,
                );
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Ran::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Ran::NotStarted(e.to_string());
            }
        }
    };
    Ran::Ended(reader.join().unwrap_or_default(), status)
}

/// The lines after `marker` in a child's output (the first may follow the test harness's
/// "test ... ").
pub fn reports<'a>(text: &'a str, marker: &str) -> Vec<&'a str> {
    text.lines()
        .filter_map(|l| l.split_once(marker).map(|(_, l)| l))
        .collect()
}

/// Why a child that reported no outcome stopped.
pub fn why_stopped(status: ExitStatus) -> String {
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    match (status.code(), signal) {
        (_, Some(6)) => {
            "stopped (signal 6), most likely for needing more memory than its limit".to_string()
        }
        (_, Some(9)) => "killed (signal 9), most likely for using too much memory".to_string(),
        (_, Some(sig)) => format!("killed (signal {sig})"),
        (code, None) => format!("the check stopped with status {}", code.unwrap_or(-1)),
    }
}

/// In the child: a memory limit of `default_mb` MiB, or what `var` says, on Linux. A file
/// needing more stops this check alone, when an allocation fails, before the system runs short.
pub fn limit_memory(var: &str, default_mb: u64) {
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{getrlimit, setrlimit, Resource, Rlimit};
        let mb: u64 = std::env::var(var)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default_mb);
        let hard = getrlimit(Resource::Data).maximum;
        let soft = hard.map_or(mb << 20, |h| h.min(mb << 20));
        setrlimit(
            Resource::Data,
            Rlimit {
                current: Some(soft),
                maximum: hard,
            },
        )
        .expect("a memory limit");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (var, default_mb);
}

/// In the child: `f` on a thread with a large stack, as deeply nested files need; `None` if
/// it panicked.
pub fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .ok()
}

/// A child's outcome line's field, without the characters that separate lines and fields.
pub fn field(s: &str) -> String {
    s.replace(['\n', '\r', '|'], " ")
}

/// How many child processes run at once: `var`, or 2.
pub fn jobs(var: &str) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(2)
}
