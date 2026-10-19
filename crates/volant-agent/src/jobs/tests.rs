// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn a_leading_tilde_is_the_home_directory_and_nothing_else_is_touched() {
    let home = || Some(std::ffi::OsString::from("/home/u"));
    assert_eq!(expand_under("~", home()), PathBuf::from("/home/u"));
    assert_eq!(
        expand_under("~/.ansible_async", home()),
        PathBuf::from("/home/u/.ansible_async")
    );
    assert_eq!(expand_under("~bob/x", home()), PathBuf::from("~bob/x"));
    assert_eq!(
        expand_under("/var/jobs", home()),
        PathBuf::from("/var/jobs")
    );
    assert_eq!(expand_under("~/x", None), PathBuf::from("~/x"));
}

#[rstest::rstest]
#[case(1, 1)]
#[case(2, 2)]
#[case(5, 5)]
#[case(7, 10)]
#[case(10, 10)]
#[case(11, 15)]
#[case(0, 5)]
#[case(3, 3)]
#[case(4, 4)]
#[case(6, 10)]
#[case(101, 105)]
fn a_job_is_killed_where_the_reference_kills_it(#[case] limit: u64, #[case] secs: u64) {
    assert_eq!(kill_after(limit), Duration::from_secs(secs));
}

/// The stepping rule as it was written, one step at a time: the closed form must give the same
/// answer for every limit that this can reach.
fn stepped(limit: u64) -> Duration {
    let limit = Duration::from_secs(limit);
    let mut step = Duration::from_secs(5);
    if !limit.is_zero() && step >= limit {
        step = limit / 2;
    }
    let mut at = step;
    while at < limit {
        at = at.saturating_add(step);
    }
    at
}

#[test]
fn the_kill_moment_is_the_first_step_past_the_limit_for_every_small_limit() {
    for limit in 0..=3000 {
        assert_eq!(kill_after(limit), stepped(limit), "limit {limit}");
    }
}

/// The reference takes any integer as `async`. A limit of a century, of `i64::MAX` or of
/// `u64::MAX` seconds is a job that is not killed, and working out when must not take time
/// that grows with the limit.
#[rstest::rstest]
#[case::century(3_153_600_000)]
#[case::i64_max(i64::MAX as u64)]
#[case::u64_max(u64::MAX)]
fn a_huge_limit_is_worked_out_at_once_and_never_expires(#[case] limit: u64) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send((kill_after(limit), deadline(limit)));
    });
    let (after, at) = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the kill moment was not worked out in five seconds");
    assert!(after >= Duration::from_secs(limit), "{after:?}");
    if limit == u64::MAX {
        assert!(
            at.is_none(),
            "a limit the clock cannot count to never expires"
        );
    }
}

#[test]
fn a_status_is_replaced_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j1.2");
    write_status(&path, &json!({"started": true})).unwrap();
    write_status(&path, &json!({"rc": 0, "cmd": ["a", "b"]})).unwrap();
    let read = std::fs::read_to_string(&path).unwrap();
    assert_eq!(read, r#"{"rc": 0, "cmd": ["a", "b"]}"#);
    assert!(!dir.path().join("j1.2.tmp").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let jobs = dir.path().join("jobs");
        private_dir(&jobs).unwrap();
        let mode = std::fs::metadata(&jobs).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}
