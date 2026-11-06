//! Outside programs run with a deadline (RC-12): CUPS's commands in production (`print.rs`),
//! and every program and child process the tests start, through [`child_seam`].
//!
//! On Unix one loop on the calling thread does all of it, with no thread of its own: our ends
//! of the pipes are non-blocking, it waits on them with `poll(2)`, writes the input in pieces
//! as the program takes it, keeps at most [`CAP`] of each output, and checks between times
//! whether the program has exited. It ends once the program has exited and both outputs are
//! closed. At the deadline it stops the program's process group, even when the program itself
//! has already exited (something it started may still hold the pipes open), and closes its own
//! ends of the pipes. A process that left the group (`setsid`, as a daemon does) keeps running,
//! but holds nothing of the caller's: its writes fail, and no thread, handle or buffer of the
//! call outlives it.
//!
//! Production starts programs only on Linux and macOS. On Windows [`run`] exists only for the
//! tests, on threads of its own, and doesn't stop what the program started.

use std::io;
#[cfg(unix)]
use std::process::Child;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The most of each output kept: the rest is read and dropped, and [`Ran::truncated`] says so.
pub const CAP: usize = 16 << 20;

/// A program run by [`run`].
pub struct Ran {
    /// How it ended: for one stopped at the deadline, how it was stopped, unless it had
    /// already exited.
    pub status: ExitStatus,
    /// Its standard output, at most [`CAP`] of it.
    pub stdout: Vec<u8>,
    /// Its standard error, at most [`CAP`] of it.
    pub stderr: Vec<u8>,
    /// Whether it took all of its input, or why not.
    pub written: io::Result<()>,
    /// How many bytes of its input it took.
    #[cfg_attr(not(any(test, feature = "test-seams")), allow(dead_code))]
    pub sent: usize,
    /// It, or something it started, kept going past the deadline, and was stopped.
    pub timed_out: bool,
    /// An output was longer than [`CAP`].
    pub truncated: bool,
}

/// Run `cmd`, writing `input` to its standard input (none gives it no input at all), for at
/// most `limit` from its start and again from its taking all of the input. An error means it
/// couldn't start, or waiting for it failed (it is stopped then too).
pub fn run(cmd: &mut Command, input: Option<&[u8]>, limit: Duration) -> io::Result<Ran> {
    run_to(cmd, input, Stdio::piped(), limit)
}

/// As [`run`], with its standard output sent to `stdout`: [`Ran::stdout`] is empty unless
/// that is a pipe.
#[cfg(unix)]
pub fn run_to(
    cmd: &mut Command,
    input: Option<&[u8]>,
    stdout: Stdio,
    limit: Duration,
) -> io::Result<Ran> {
    use std::os::fd::OwnedFd;
    // A group of its own, so that whatever it starts is stopped with it.
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    let mut child = cmd
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take().map(OwnedFd::from);
    let stdout = child.stdout.take().map(OwnedFd::from);
    let stderr = child.stderr.take().map(OwnedFd::from);
    // The pipes are closed inside, before the child is reaped here.
    let tended = tend(
        &child,
        stdin,
        stdout,
        stderr,
        input.unwrap_or_default(),
        limit,
    );
    if !matches!(
        tended,
        Ok(Tended {
            timed_out: false,
            ..
        })
    ) {
        stop(&child);
    }
    let status = child.wait();
    let t = tended?;
    Ok(Ran {
        status: status?,
        stdout: t.stdout.kept,
        stderr: t.stderr.kept,
        written: t.written,
        sent: t.sent,
        timed_out: t.timed_out,
        truncated: t.stdout.truncated || t.stderr.truncated,
    })
}

/// What [`tend`] got of a program.
#[cfg(unix)]
struct Tended {
    stdout: Output,
    stderr: Output,
    written: io::Result<()>,
    sent: usize,
    timed_out: bool,
}

/// How long [`tend`] waits on the pipes at most before it checks whether the program exited,
/// which `poll` doesn't say.
#[cfg(unix)]
const STEP: Duration = Duration::from_millis(20);

/// The most read from one output, or written to the input, before the loop goes on to the
/// other pipes and the deadline: a program that writes without pause can't keep it there.
#[cfg(unix)]
const TURN: usize = 1 << 20;

/// Writes `data` to `stdin`, reads `stdout` and `stderr`, and waits for `child` to exit and
/// both outputs to close, or the deadline. The pipes are closed when it returns.
#[cfg(unix)]
fn tend(
    child: &Child,
    mut stdin: Option<std::os::fd::OwnedFd>,
    stdout: Option<std::os::fd::OwnedFd>,
    stderr: Option<std::os::fd::OwnedFd>,
    data: &[u8],
    limit: Duration,
) -> io::Result<Tended> {
    for fd in [&stdin, &stdout, &stderr].into_iter().flatten() {
        let flags = rustix::fs::fcntl_getfl(fd)?;
        rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
    }
    let (mut stdout, mut stderr) = (Output::new(stdout), Output::new(stderr));
    let mut buf = vec![0; 64 << 10];
    let (mut sent, mut written) = (0, Ok(()));
    let mut deadline = Instant::now() + limit;
    let mut exited = false;
    loop {
        if let Some(fd) = &stdin {
            if let Err(e) = feed(fd, data, &mut sent) {
                written = Err(e);
            }
            if sent == data.len() || written.is_err() {
                stdin = None;
                deadline = Instant::now() + limit;
            }
        }
        stdout.read(&mut buf);
        stderr.read(&mut buf);
        if !exited {
            exited = has_exited(child)?;
        }
        let open = stdin.is_some() || stdout.fd.is_some() || stderr.fd.is_some();
        let timed_out = Instant::now() >= deadline;
        if (exited && !open) || timed_out {
            if timed_out && stdin.is_some() {
                written = Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("it took {sent} of {} bytes", data.len()),
                ));
            }
            return Ok(Tended {
                stdout,
                stderr,
                written,
                sent,
                timed_out: timed_out && !(exited && !open),
            });
        }
        wait_on(&stdin, &stdout, &stderr, deadline)?;
    }
}

/// Writes what of `data` past `sent` the pipe takes now, adding it to `sent`.
#[cfg(unix)]
fn feed(fd: &std::os::fd::OwnedFd, data: &[u8], sent: &mut usize) -> io::Result<()> {
    let end = data.len().min(*sent + TURN);
    while *sent < end {
        match rustix::io::write(fd, &data[*sent..end]) {
            Ok(n) => *sent += n,
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// One of a program's outputs.
#[cfg(unix)]
struct Output {
    /// Closed once it reached its end.
    fd: Option<std::os::fd::OwnedFd>,
    kept: Vec<u8>,
    truncated: bool,
}

#[cfg(unix)]
impl Output {
    fn new(fd: Option<std::os::fd::OwnedFd>) -> Output {
        Output {
            fd,
            kept: Vec::new(),
            truncated: false,
        }
    }

    /// Reads what is there now, through `buf`; closes it at its end, or when it fails.
    fn read(&mut self, buf: &mut [u8]) {
        let mut read = 0;
        while let Some(fd) = &self.fd {
            if read >= TURN {
                return;
            }
            match rustix::io::read(fd, &mut *buf) {
                Ok(0) => self.fd = None,
                Ok(n) => {
                    read += n;
                    let room = CAP - self.kept.len();
                    self.kept.extend_from_slice(&buf[..n.min(room)]);
                    self.truncated |= n > room;
                }
                Err(rustix::io::Errno::INTR) => {}
                Err(rustix::io::Errno::AGAIN) => return,
                Err(_) => self.fd = None,
            }
        }
    }
}

/// Whether `child` has exited, without reaping it: until it is reaped, its process ID (and so
/// its group's) can't be taken by another process, so stopping the group can't reach one.
#[cfg(unix)]
fn has_exited(child: &Child) -> io::Result<bool> {
    use rustix::process::{waitid, Pid, WaitId, WaitIdOptions};
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
    Ok(waitid(WaitId::Pid(Pid::from_child(child)), options)?.is_some())
}

/// Waits until a pipe is ready, [`STEP`] has passed, or the deadline.
#[cfg(unix)]
fn wait_on(
    stdin: &Option<std::os::fd::OwnedFd>,
    stdout: &Output,
    stderr: &Output,
    deadline: Instant,
) -> io::Result<()> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let mut fds = Vec::with_capacity(3);
    if let Some(fd) = stdin {
        fds.push(PollFd::new(fd, PollFlags::OUT));
    }
    for fd in [&stdout.fd, &stderr.fd].into_iter().flatten() {
        fds.push(PollFd::new(fd, PollFlags::IN));
    }
    let wait = deadline.saturating_duration_since(Instant::now()).min(STEP);
    let wait = Timespec::try_from(wait).map_err(|_| io::Error::other("a wait out of range"))?;
    match poll(&mut fds, Some(&wait)) {
        Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Stops `child` and every process in its group: it may have exited already, but not been
/// reaped, so its group's ID is still its own.
#[cfg(unix)]
fn stop(child: &Child) {
    use rustix::process::{kill_process, kill_process_group, Pid, Signal};
    let pid = Pid::from_child(child);
    let _ = kill_process_group(pid, Signal::KILL);
    let _ = kill_process(pid, Signal::KILL);
}

/// [`run_to`] for the tests on Windows, on threads of its own: what the program started isn't
/// stopped with it, and one of those holding its output open keeps the call waiting.
#[cfg(windows)]
pub fn run_to(
    cmd: &mut Command,
    input: Option<&[u8]>,
    stdout: Stdio,
    limit: Duration,
) -> io::Result<Ran> {
    use std::io::{Read, Write};
    let mut child = cmd
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()?;
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    std::thread::scope(|s| {
        let writer = s.spawn(move || match (stdin, input) {
            (Some(mut w), Some(data)) => w.write_all(data).map(|()| data.len()),
            _ => Ok(0),
        });
        let read = |pipe: Option<Box<dyn Read + Send>>| {
            s.spawn(move || {
                let (mut kept, mut truncated) = (Vec::new(), false);
                if let Some(mut p) = pipe {
                    let mut buf = vec![0; 64 << 10];
                    while let Ok(n @ 1..) = p.read(&mut buf) {
                        let room = CAP - kept.len();
                        kept.extend_from_slice(&buf[..n.min(room)]);
                        truncated |= n > room;
                    }
                }
                (kept, truncated)
            })
        };
        let out = read(stdout.map(|p| Box::new(p) as Box<dyn Read + Send>));
        let err = read(stderr.map(|p| Box::new(p) as Box<dyn Read + Send>));
        let mut deadline = Instant::now() + limit;
        let mut writing = true;
        let mut timed_out = false;
        let status = loop {
            if writing && writer.is_finished() {
                writing = false;
                deadline = Instant::now() + limit;
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {}
                Ok(None) => {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait()?;
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let written = writer
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("panicked")));
        let (stdout, t_out) = out.join().unwrap_or_default();
        let (stderr, t_err) = err.join().unwrap_or_default();
        Ok(Ran {
            status,
            stdout,
            stderr,
            sent: *written.as_ref().unwrap_or(&0),
            written: written.map(|_| ()),
            timed_out,
            truncated: t_out || t_err,
        })
    })
}

/// Outside programs and child processes for the tests of core, the app and the CLI, run as
/// `Command::output` does, but stopped (with anything they started) and the test failed once
/// they take longer than [`TIMEOUT`](child_seam::TIMEOUT), or write more than [`CAP`]. Built
/// only for tests: core's own, and the app's and the CLI's through the `test-seams` feature.
#[cfg(any(test, feature = "test-seams"))]
pub mod child_seam {
    pub use super::{run, run_to, Ran, CAP};
    use std::process::{Command, Output, Stdio};
    use std::time::Duration;

    /// How long one run of a program may take; `LIMAREPDF_TOOL_TIMEOUT` (seconds) changes it.
    pub const TIMEOUT: Duration = Duration::from_secs(120);

    /// [`TIMEOUT`], or what `LIMAREPDF_TOOL_TIMEOUT` says.
    pub fn timeout() -> Duration {
        std::env::var("LIMAREPDF_TOOL_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .map_or(TIMEOUT, Duration::from_secs)
    }

    /// Runs `cmd` as `Command::output` does, but failing the test once it takes longer than
    /// [`timeout`] or writes more than [`CAP`]. An error means the program couldn't start,
    /// most often because it isn't installed.
    pub fn output(cmd: &mut Command) -> std::io::Result<Output> {
        checked(cmd, None, Stdio::piped(), timeout())
    }

    /// As [`output`], with the program's standard output sent to `stdout`: the output's
    /// `stdout` is empty unless that is a pipe.
    pub fn output_to(cmd: &mut Command, stdout: Stdio) -> std::io::Result<Output> {
        checked(cmd, None, stdout, timeout())
    }

    /// As [`output`], with `input` written to the program's standard input.
    pub fn output_with_input(cmd: &mut Command, input: &[u8]) -> std::io::Result<Output> {
        checked(cmd, Some(input), Stdio::piped(), timeout())
    }

    /// As [`output`], stopped after `limit` instead.
    pub fn output_within(cmd: &mut Command, limit: Duration) -> std::io::Result<Output> {
        checked(cmd, None, Stdio::piped(), limit)
    }

    fn checked(
        cmd: &mut Command,
        input: Option<&[u8]>,
        stdout: Stdio,
        limit: Duration,
    ) -> std::io::Result<Output> {
        let program = cmd.get_program().to_string_lossy().into_owned();
        let ran = run_to(cmd, input, stdout, limit)?;
        assert!(
            !ran.timed_out,
            "{program} didn't finish within {} seconds (LIMAREPDF_TOOL_TIMEOUT changes it)",
            limit.as_secs()
        );
        assert!(
            !ran.truncated,
            "{program} wrote more output than the {} MiB kept",
            CAP >> 20
        );
        Ok(Output {
            status: ran.status,
            stdout: ran.stdout,
            stderr: ran.stderr,
        })
    }
}
