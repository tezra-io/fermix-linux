//! A command run to its end within a cap. It runs in a process group of its
//! own, and once it ends or the cap runs out the whole group is killed and the
//! command reaped, so nothing it started is left behind: Burrito's wrapper,
//! for one, runs the engine's BEAM as a child.

use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How often the run looks at whether the command has ended.
const POLL: Duration = Duration::from_millis(100);
/// How long the command's output may stay open once its group is gone. Only a
/// process that left the group can still hold it.
const READ_GRACE: Duration = Duration::from_secs(5);

/// How the command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// It exited within the cap, having written this to stdout.
    Exited(Vec<u8>),
    /// The cap ran out first.
    Capped,
}

/// Runs `argv`, blocking, with stdin empty, stdout read and stderr inherited,
/// until it exits or `cap` runs out. Either way its group is then killed and
/// the command reaped. `Err` when it could not be started, waited on or read.
pub fn run(argv: &[&str], cap: Duration) -> io::Result<Ended> {
    let (program, args) = argv.split_first().expect("a command names its program");
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let output = read_in_background(stdout);
    let exited = wait_within(&mut child, cap);
    let killed = kill_group(&child);
    let reaped = child.wait();
    let read = output
        .recv_timeout(READ_GRACE)
        .unwrap_or_else(|_| Err(held_open()));
    let (exited, read) = (exited?, read?);
    killed?;
    reaped?;
    Ok(if exited {
        Ended::Exited(read)
    } else {
        Ended::Capped
    })
}

/// Reads the command's stdout on a thread of its own, so a long answer never
/// fills the pipe and stalls the command. The answer arrives on the channel.
fn read_in_background(mut stdout: ChildStdout) -> mpsc::Receiver<io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut output = Vec::new();
        let read = stdout.read_to_end(&mut output).map(|_| output);
        // Sending fails only once the run has given up on this output and
        // returned its own error, so there is no one left to tell.
        let _ = sender.send(read);
    });
    receiver
}

/// Whether the command exited within `cap`, looking every `POLL`. When the
/// cap runs out it is left unreaped, so its id still names its group.
fn wait_within(child: &mut Child, cap: Duration) -> io::Result<bool> {
    let deadline = Instant::now() + cap;
    while Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
    Ok(child.try_wait()?.is_some())
}

/// Kills the command's whole group. Unreaped, the command holds its id; once
/// reaped, the id stays reserved while any member of its group lives, so the
/// kill reaches that group or finds nothing left.
fn kill_group(child: &Child) -> io::Result<()> {
    let group = libc::pid_t::try_from(child.id()).expect("a pid fits in a pid_t");
    // SAFETY: kill(2) takes two integers and touches no memory of this process.
    // A negative pid names the process group.
    if unsafe { libc::kill(-group, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(()),
        _ => Err(error),
    }
}

fn held_open() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "its output stayed open after it ended",
    )
}
