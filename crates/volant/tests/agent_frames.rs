// SPDX-License-Identifier: GPL-3.0-or-later
//! What the controller does with the frames of an agent that sends more than it should.
//!
//! A stand-in agent (`fixtures/frame_agent.py`) answers the task named `bad` with a frame of the
//! kind and size a row asks for, and every other task with a small result. Two hosts run the same
//! play: the one named `bad` fails with a message that names the limit, the one named `good`
//! completes, and the controller stays under a memory bound that a frame parsed whole would pass
//! by gigabytes.
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use rstest::rstest;

#[path = "common/process.rs"]
#[expect(dead_code, reason = "only the watched run is used here")]
mod process;

const AGENT: &str = include_str!("fixtures/frame_agent.py");

/// What a refused frame may cost the controller. The same frames parsed whole took 600 MB and
/// more in a debug build.
const REFUSED_PEAK_MB: u64 = 250;

/// How long a run that ends may take. A frame refused or read whole ends in a few seconds.
const ENDS: Duration = Duration::from_secs(120);

struct Run {
    code: Option<i32>,
    text: String,
    peak_mb: u64,
}

fn run(name: &str, frame: &str, n: u64, tasks: &str, hosts: &str, deadline: Duration) -> Run {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("volant-frames-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("agents")).unwrap();
    let agent = dir.join("agents/volant-agent");
    std::fs::write(&agent, AGENT).unwrap();
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
    let inventory = hosts
        .split_whitespace()
        .map(|host| {
            format!("{host} ansible_connection=local ansible_python_interpreter=/usr/bin/python3\n")
        })
        .collect::<Vec<_>>()
        .concat();
    std::fs::write(dir.join("inv.ini"), inventory).unwrap();
    std::fs::write(
        dir.join("play.yml"),
        format!("- hosts: all\n  gather_facts: false\n  tasks:\n{tasks}"),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_volant"));
    command
        .args(["playbook", "-i"])
        .arg(dir.join("inv.ini"))
        .arg(dir.join("play.yml"))
        .env("NO_COLOR", "1")
        .env("VOLANT_AGENT_DIR", dir.join("agents"))
        .env("VOLANT_RUNTIME", "system")
        .env("FRAME", frame)
        .env("N", n.to_string());
    let mut sampler = None;
    let out = process::run_within_seen(&mut command, deadline, Some(b""), |pid| {
        sampler = Some(std::thread::spawn(move || peak_mb_of(pid)));
    });
    let peak_mb = sampler.expect("volant started").join().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    println!("{name}: peak {peak_mb} MB, exit {:?}", out.status.code());
    Run {
        code: out.status.code(),
        text,
        peak_mb,
    }
}

/// The high-water mark of the resident set of `pid`, in MB. It only grows, so the last reading
/// before the process goes is its peak.
fn peak_mb_of(pid: u32) -> u64 {
    let status = PathBuf::from(format!("/proc/{pid}/status"));
    let mut peak_kb = 0u64;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(300) {
        let Ok(text) = std::fs::read_to_string(&status) else {
            break;
        };
        if let Some(kb) = text
            .lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))
            .and_then(|l| l.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        {
            peak_kb = peak_kb.max(kb);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    peak_kb / 1024
}

const BAD_TASK: &str = "    - command: \"{{ inventory_hostname }}\"\n";

/// A frame over a limit fails its host alone (exit 2 for a result, 4 for any other message),
/// with the figure and the limit in its message, and the other host finishes.
#[rstest]
#[case::too_many_values("values", 2_000_000, 2, "has 2000013 values, over the limit of 500000")]
#[case::too_many_lines("lines", 3_000_000, 2, "more lines than the limit of 500000")]
#[case::too_deep("depth", 70_000, 2, "70002 levels of nesting, over the limit of 64")]
#[case::a_long_log_line(
    "log",
    2_000_000,
    4,
    "log message of 2000042 bytes, over the 1048576 byte limit"
)]
#[case::a_long_log_line_with_a_junk_field(
    "logjunk",
    60_000_000,
    4,
    "message of 60000054 bytes, over the 1048576 byte limit"
)]
fn a_frame_over_a_limit_fails_its_host_alone(
    #[case] frame: &str,
    #[case] n: u64,
    #[case] code: i32,
    #[case] says: &str,
) {
    let run = run(frame, frame, n, BAD_TASK, "bad good", ENDS);
    assert_eq!(run.code, Some(code), "{}", run.text);
    assert!(run.text.contains(says), "{}", run.text);
    assert!(
        run.text.contains("changed: [good]"),
        "the other host finishes: {}",
        run.text
    );
    assert!(
        run.peak_mb < REFUSED_PEAK_MB,
        "peak {} MB: {}",
        run.peak_mb,
        run.text
    );
}

/// Results that are large and legitimate still arrive whole: ten megabytes of command output,
/// and the package list of a big host.
#[rstest]
#[case::ten_megabytes_of_stdout(
    "stdout",
    10_000_000,
    "    - command: bad\n      register: r\n    - assert:\n        that:\n          - r.stdout | length == 10000000\n          - r.stdout_lines | length == 200000\n"
)]
#[case::a_large_package_list(
    "facts",
    20_000,
    "    - command: bad\n      register: r\n    - assert:\n        that:\n          - r.ansible_facts.packages | length == 20000\n"
)]
fn a_large_result_within_the_limits_arrives_whole(
    #[case] frame: &str,
    #[case] n: u64,
    #[case] tasks: &str,
) {
    let run = run(frame, frame, n, tasks, "only", ENDS);
    assert_eq!(run.code, Some(0), "{}", run.text);
}

/// A refused result no task waits for ends its host, and the agent's 30 s of silence after it
/// does not hold the run: the other host finishes.
#[rstest]
#[case::no_batch("nobatch")]
#[case::no_index("noindex")]
#[case::neither("neither")]
#[case::a_batch_nobody_waits_for("wrongbatch")]
#[case::a_task_nobody_waits_for("wrongindex")]
fn a_result_nobody_waits_for_ends_its_host(#[case] frame: &str) {
    let run = run(
        frame,
        frame,
        1_000_000,
        BAD_TASK,
        "bad good",
        Duration::from_secs(20),
    );
    assert_eq!(run.code, Some(4), "{}", run.text);
    assert!(
        run.text.contains("over the limit of 500000"),
        "{}",
        run.text
    );
    assert!(run.text.contains("changed: [good]"), "{}", run.text);
    assert!(run.peak_mb < REFUSED_PEAK_MB, "peak {} MB", run.peak_mb);
}
