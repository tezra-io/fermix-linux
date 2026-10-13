//! A command run within a cap, in a process group of its own: what it wrote
//! arrives whole, and nothing it started outlives it or the cap. Each command
//! here is `sh`, which every host and the SDK have.

use fermix_client::capped::{run, Ended};
use std::fs;
use std::io;
use std::thread;
use std::time::{Duration, Instant};

/// Whether `pid` has ended: no such process, or only its zombie. Looked at
/// every 100 ms for at most 5 s, since a killed orphan is reaped by whoever
/// adopts it.
fn gone(pid: &str) -> bool {
    for _ in 0..50 {
        let ended = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(e) => e.kind() == io::ErrorKind::NotFound,
            // The state follows the command name, which closes with ')'.
            Ok(stat) => stat
                .rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
        };
        if ended {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn pids(output: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(output)
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_command_that_ends_within_the_cap_gives_what_it_wrote() {
    let ended = run(&["sh", "-c", "echo hello"], Duration::from_secs(10)).expect("sh runs");
    assert_eq!(ended, Ended::Exited(b"hello\n".to_vec()));
}

#[test]
fn an_answer_longer_than_a_pipe_holds_arrives_whole() {
    let ended = run(
        &["sh", "-c", "head -c 200000 /dev/zero"],
        Duration::from_secs(10),
    )
    .expect("sh runs");
    assert_eq!(ended, Ended::Exited(vec![0; 200_000]));
}

#[test]
fn the_command_reads_nothing_and_never_waits_on_input() {
    let ended = run(&["sh", "-c", "cat"], Duration::from_secs(10)).expect("sh runs");
    assert_eq!(ended, Ended::Exited(Vec::new()));
}

/// The command and the child it started both outlive the cap unless the whole
/// group is killed: their pids are written where the test can read them, and
/// the child's output goes elsewhere, so only the kill can end it.
#[test]
fn at_the_cap_the_command_and_everything_it_started_are_killed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let file = dir.path().join("pids");
    let script = format!(
        "sleep 60 > /dev/null & echo $$ $! > {}; wait",
        file.display()
    );
    let started = Instant::now();
    let ended = run(&["sh", "-c", &script], Duration::from_secs(1)).expect("sh runs");
    assert_eq!(ended, Ended::Capped);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let pids = pids(&fs::read(&file).expect("the script wrote its pids"));
    assert_eq!(pids.len(), 2, "{pids:?}");
    for pid in &pids {
        assert!(gone(pid), "process {pid} outlived the cap");
    }
}

/// The child a command leaves behind when it ends, by its output.
fn left_behind(script: &str) -> String {
    let started = Instant::now();
    let ended = run(&["sh", "-c", script], Duration::from_secs(30)).expect("sh runs");
    let Ended::Exited(output) = ended else {
        panic!("the command ended by itself: {ended:?}");
    };
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    let pids = pids(&output);
    assert_eq!(pids.len(), 1, "{pids:?}");
    pids[0].clone()
}

#[test]
fn what_a_command_leaves_behind_is_killed_when_it_ends() {
    let child = left_behind("sleep 60 > /dev/null & echo $!");
    assert!(gone(&child), "the child {child} outlived its command");
}

/// A child left holding the command's output would keep the answer open; the
/// run ends at once all the same, since the child goes with the group.
#[test]
fn a_child_left_holding_the_output_does_not_hold_the_run() {
    let child = left_behind("sleep 60 & echo $!");
    assert!(gone(&child), "the child {child} outlived its command");
}

#[test]
fn a_command_that_cannot_start_is_an_error() {
    let error = run(
        &["/nonexistent/fermix", "service", "install"],
        Duration::from_secs(1),
    )
    .expect_err("there is no such program");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
#[should_panic(expected = "names its program")]
fn a_command_names_its_program() {
    let _ = run(&[], Duration::from_secs(1));
}
