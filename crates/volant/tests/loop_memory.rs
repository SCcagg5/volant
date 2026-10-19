// SPDX-License-Identifier: GPL-3.0-or-later
//! The memory of a loop grows with the data of its items, not with the host's variables.

#![cfg(target_os = "linux")]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The peak resident memory, in MB, of a run of the playbook, read from `/proc` while it runs.
fn peak_megabytes(dir: &std::path::Path, playbook: &str) -> u64 {
    std::fs::write(dir.join("p.yml"), playbook).expect("the playbook is written");
    std::fs::write(
        dir.join("hosts.ini"),
        "[all]\nh1 ansible_connection=local\n",
    )
    .expect("the inventory is written");
    let mut child = Command::new(env!("CARGO_BIN_EXE_volant"))
        .args(["playbook", "-i", "hosts.ini", "p.yml"])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("VOLANT_RUNTIME", "system")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("volant starts");
    let started = Instant::now();
    let mut peak_kb = 0;
    loop {
        if let Ok(status) = std::fs::read_to_string(format!("/proc/{}/status", child.id()))
            && let Some(line) = status.lines().find(|l| l.starts_with("VmHWM:"))
        {
            let kb: u64 = line
                .split_whitespace()
                .nth(1)
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            peak_kb = peak_kb.max(kb);
        }
        if let Some(status) = child.try_wait().expect("the child is polled") {
            assert!(status.success(), "the run failed: {status}");
            return peak_kb / 1024;
        }
        if started.elapsed() > Duration::from_secs(300) {
            let _ = child.kill();
            panic!("the run did not end");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The bound is for a debug build, which the tests use, on the machine that wrote it: a release
/// build or a machine with another allocator peaks lower. The runs it fences off are an order of
/// magnitude apart (about 1 GB before the fix, under 100 MB after), so the bound sits between them.
const DEBUG_BUILD_BOUND_MB: u64 = 250;

/// A command prints `LINES` lines and a loop walks them. Every item used to own a copy of the
/// host's variables, `r` and its lines among them, so the memory was the number of items times
/// the size of the result: about 1 GB for these 3,000 lines in a debug build, 70 MB now.
#[test]
fn a_loop_over_a_long_result_does_not_copy_it_for_every_item() {
    const LINES: usize = 3000;
    const BOUND_MB: u64 = DEBUG_BUILD_BOUND_MB;
    let dir = std::env::temp_dir().join(format!("volant-loop-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the directory is made");
    let playbook = format!(
        "- hosts: h1\n  gather_facts: false\n  tasks:\n    - command: seq 1 {LINES}\n      register: r\n    - debug: {{msg: \"{{{{ item }}}}\"}}\n      loop: \"{{{{ r.stdout_lines }}}}\"\n      loop_control: {{label: x}}\n"
    );
    let peak = peak_megabytes(&dir, &playbook);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        peak < BOUND_MB,
        "a loop of {LINES} items over a {LINES}-line result peaked at {peak} MB, the bound is {BOUND_MB} MB"
    );
}

/// `loop_control: {extended: true}` hands every item the list of all the items. It is one list
/// for the loop, not one copy for each item: the memory is what it was without `extended`, and
/// each item still reads the whole list and its own place in it.
#[test]
fn an_extended_loop_over_a_long_result_shares_the_list_of_all_items() {
    const LINES: usize = 3000;
    let dir = std::env::temp_dir().join(format!("volant-loop-extended-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the directory is made");
    let playbook = format!(
        "- hosts: h1\n  gather_facts: false\n  tasks:\n    - command: seq 1 {LINES}\n      register: r\n    - debug: {{msg: \"{{{{ item }}}}\"}}\n      loop: \"{{{{ r.stdout_lines }}}}\"\n      loop_control: {{label: x, extended: true}}\n      failed_when: (ansible_loop.allitems | length) != {LINES} or ansible_loop.allitems[ansible_loop.index0] != item\n"
    );
    let peak = peak_megabytes(&dir, &playbook);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        peak < DEBUG_BUILD_BOUND_MB,
        "an extended loop of {LINES} items peaked at {peak} MB, the bound is {DEBUG_BUILD_BOUND_MB} MB"
    );
}
