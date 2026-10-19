// SPDX-License-Identifier: GPL-3.0-or-later
//! The agent directory on a shared `remote_tmp`: who may own it, and what a planted one does.

use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use rstest::rstest;
use serde_json::json;

use crate::inventory::Host;
use crate::testing::{defaults, scratch};
use crate::transport::{SshTarget, Transport};

use super::super::bootstrap::{Captured, is_this_agent};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The uid the tests hand a directory to, as if another user on the host had made it.
const OTHER_UID: u32 = 4242;
const EXIT_UNSAFE: i32 = 47;
const EXIT_MISSING: i32 = 42;

/// A host as the bootstrap sees it, rooted in a scratch directory the test owns. The shell
/// commands run on this machine, so a file the test makes is a file "on the host".
struct Remote {
    root: tempfile::TempDir,
    target: SshTarget,
    handed_over: bool,
}

impl Remote {
    fn new() -> Self {
        let root = scratch();
        let shared = root.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o700)).unwrap();
        let target = Self::target_at(shared.to_str().unwrap());
        Self {
            root,
            target,
            handed_over: false,
        }
    }

    fn target_at(remote_tmp: &str) -> SshTarget {
        let host = Host {
            name: "web1".into(),
            vars: [("ansible_remote_tmp".to_string(), json!(remote_tmp))]
                .into_iter()
                .collect(),
        };
        let Transport::Ssh(target) = Transport::for_host(&host, &defaults()).unwrap() else {
            panic!("an ssh transport")
        };
        target
    }

    fn shared(&self) -> PathBuf {
        self.root.path().join("shared")
    }

    fn dir(&self) -> PathBuf {
        self.shared()
            .join(format!("volant-agent-{VERSION}-{}", effective_uid()))
    }

    fn agent(&self) -> PathBuf {
        self.dir().join("volant-agent")
    }

    fn marker(&self) -> PathBuf {
        self.root.path().join("marker")
    }

    /// What an attacker would plant: it prints the version line the probe looks for, and leaves
    /// a mark of having run.
    fn fake_agent(&self) -> String {
        format!(
            "#!/bin/sh\necho 'volant-agent {VERSION} planted'\necho ran >> '{}'\n",
            self.marker().display()
        )
    }

    fn plant(&self, dir_mode: u32, agent_mode: u32) {
        std::fs::create_dir(self.dir()).unwrap();
        std::fs::write(self.agent(), self.fake_agent()).unwrap();
        std::fs::set_permissions(self.agent(), std::fs::Permissions::from_mode(agent_mode))
            .unwrap();
        std::fs::set_permissions(self.dir(), std::fs::Permissions::from_mode(dir_mode)).unwrap();
    }

    /// Gives `path` to another user. `false` when this process can neither chown nor use
    /// `sudo` without a password, which skips the test with a message.
    fn hand_over(&mut self, path: &Path) -> bool {
        let mut chown = if effective_uid() == 0 {
            Command::new("chown")
        } else {
            let mut sudo = Command::new("sudo");
            sudo.args(["-n", "chown"]);
            sudo
        };
        let done = chown
            .args(["-R", &OTHER_UID.to_string()])
            .arg(path)
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        self.handed_over |= done;
        if !done {
            eprintln!("skipped: giving a directory to another user needs root or `sudo -n`");
        }
        done
    }

    fn sh(script: &str, stdin: &[u8], home: Option<&Path>) -> Output {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(home) = home {
            command.env("HOME", home);
        }
        let mut child = command.spawn().unwrap();
        // A command that refuses early closes its input; the exit code says why.
        let _ = child.stdin.take().unwrap().write_all(stdin);
        child.wait_with_output().unwrap()
    }

    fn upload(&self, payload: &[u8]) -> Output {
        Self::sh(
            &self.target.upload_command(payload.len() as u64),
            payload,
            None,
        )
    }

    fn probe(&self) -> Output {
        Self::sh(&self.target.probe_command(), b"", None)
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        // What another user owns is beyond the scratch directory's own clean-up.
        if self.handed_over {
            let _ = Command::new("sudo")
                .args(["-n", "rm", "-rf"])
                .arg(self.root.path())
                .status();
        }
    }
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` reads the calling process's own credentials and cannot fail.
    unsafe { libc::geteuid() }
}

fn captured(out: &Output) -> Captured {
    Captured {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// What the cached agent would print for the probe: the version, then a hash of itself.
fn good_agent(hash: &str) -> Vec<u8> {
    format!("#!/bin/sh\necho 'volant-agent {VERSION} {hash}'\n").into_bytes()
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[derive(Clone, Copy, Debug)]
enum Plant {
    /// Another user owns the directory and the program in it.
    ForeignDirAndAgent,
    /// Another user made the directory first and put nothing in it.
    ForeignEmptyDir,
    /// This user owns the directory, and anybody can write to it.
    OpenDir,
    /// This user owns a closed directory, and anybody can write to the program in it.
    OpenAgent,
    /// The directory is a link to another directory that holds a program.
    SymlinkedDir,
    /// The program is a link to a file somewhere else.
    SymlinkedAgent,
    /// A good earlier upload, then anybody can rename entries in `remote_tmp`.
    OpenParent,
}

/// The planted program never runs, and an upload either replaces it or refuses.
#[rstest]
#[case::another_users_directory(Plant::ForeignDirAndAgent, EXIT_UNSAFE, EXIT_UNSAFE, false)]
#[case::another_users_empty_directory(Plant::ForeignEmptyDir, EXIT_MISSING, EXIT_UNSAFE, false)]
#[case::a_directory_anybody_can_write(Plant::OpenDir, EXIT_UNSAFE, 0, true)]
#[case::a_program_anybody_can_write(Plant::OpenAgent, EXIT_UNSAFE, 0, true)]
#[case::a_linked_directory(Plant::SymlinkedDir, EXIT_UNSAFE, EXIT_UNSAFE, false)]
#[case::a_linked_program(Plant::SymlinkedAgent, EXIT_UNSAFE, 0, true)]
#[case::a_shared_remote_tmp_without_the_sticky_bit(
    Plant::OpenParent,
    EXIT_UNSAFE,
    EXIT_UNSAFE,
    false
)]
fn a_planted_agent_directory_is_never_run(
    #[case] plant: Plant,
    #[case] probe_code: i32,
    #[case] upload_code: i32,
    #[case] replaced: bool,
) {
    let mut remote = Remote::new();
    let elsewhere = remote.root.path().join("elsewhere");
    match plant {
        Plant::ForeignDirAndAgent => {
            remote.plant(0o755, 0o755);
            if !remote.hand_over(&remote.dir()) {
                return;
            }
        }
        Plant::ForeignEmptyDir => {
            std::fs::create_dir(remote.dir()).unwrap();
            if !remote.hand_over(&remote.dir()) {
                return;
            }
        }
        Plant::OpenDir => remote.plant(0o777, 0o755),
        Plant::OpenAgent => remote.plant(0o700, 0o777),
        Plant::SymlinkedDir => {
            std::fs::create_dir(&elsewhere).unwrap();
            std::fs::write(elsewhere.join("volant-agent"), remote.fake_agent()).unwrap();
            std::fs::set_permissions(
                elsewhere.join("volant-agent"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            symlink(&elsewhere, remote.dir()).unwrap();
        }
        Plant::SymlinkedAgent => {
            std::fs::create_dir(remote.dir()).unwrap();
            std::fs::set_permissions(remote.dir(), std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(&elsewhere, remote.fake_agent()).unwrap();
            std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o755)).unwrap();
            symlink(&elsewhere, remote.agent()).unwrap();
        }
        Plant::OpenParent => {
            assert_eq!(remote.upload(&good_agent("one")).status.code(), Some(0));
            std::fs::set_permissions(remote.shared(), std::fs::Permissions::from_mode(0o777))
                .unwrap();
        }
    }

    let probe = remote.probe();
    assert_eq!(
        probe.status.code(),
        Some(probe_code),
        "{:?}",
        captured(&probe)
    );
    if probe_code == EXIT_UNSAFE {
        let said = String::from_utf8_lossy(&probe.stderr).into_owned();
        assert!(
            said.contains(remote.shared().to_str().unwrap()),
            "the refusal names the path: {said}"
        );
    }
    let upload = remote.upload(&good_agent("two"));
    assert_eq!(
        upload.status.code(),
        Some(upload_code),
        "{:?}",
        captured(&upload)
    );
    if upload_code == EXIT_UNSAFE {
        let said = String::from_utf8_lossy(&upload.stderr).into_owned();
        assert!(
            said.contains(remote.shared().to_str().unwrap()),
            "the refusal names the path: {said}"
        );
    }
    assert!(
        !remote.marker().exists(),
        "the planted program ran on the host"
    );
    let landed = std::fs::read(remote.agent()).unwrap_or_default();
    if replaced {
        assert_eq!(landed, good_agent("two"), "the upload replaced it");
        let probe = remote.probe();
        assert_eq!(probe.status.code(), Some(0), "{:?}", captured(&probe));
        assert!(is_this_agent(&captured(&probe), "two"));
        assert_eq!(
            mode(&remote.dir()) & 0o077,
            0,
            "the directory is private again"
        );
        assert!(!remote.marker().exists());
    } else if let Plant::OpenParent = plant {
        assert_eq!(
            landed,
            good_agent("one"),
            "nothing was written under an open parent"
        );
    } else {
        assert_ne!(landed, good_agent("two"), "nothing was written into it");
    }
}

/// A normal run: the first upload makes a private directory, later runs reuse the program, a
/// directory from an earlier run is accepted, and clean-up touches only what this user owns.
#[test]
fn an_ordinary_run_still_caches_and_reuses_the_agent() {
    let mut remote = Remote::new();
    let payload = good_agent("abc");
    let upload = remote.upload(&payload);
    assert_eq!(upload.status.code(), Some(0), "{:?}", captured(&upload));
    assert_eq!(mode(&remote.dir()) & 0o077, 0, "a new directory is private");
    assert_eq!(
        mode(&remote.agent()) & 0o022,
        0,
        "nobody else can write the program"
    );
    let probe = remote.probe();
    assert!(
        is_this_agent(&captured(&probe), "abc"),
        "{:?}",
        captured(&probe)
    );
    let inode = std::fs::metadata(remote.agent()).unwrap();
    let again = remote.probe();
    assert!(is_this_agent(&captured(&again), "abc"));
    assert_eq!(
        std::fs::metadata(remote.agent())
            .unwrap()
            .modified()
            .unwrap(),
        inode.modified().unwrap(),
        "a probe changes nothing"
    );

    // `remote_tmp` may be this user's own and open to its group (a umask of 002 makes it so), or
    // sticky and open to everybody, as `/tmp` is.
    for parent in [0o775, 0o1777, 0o755] {
        std::fs::set_permissions(remote.shared(), std::fs::Permissions::from_mode(parent)).unwrap();
        let probe = remote.probe();
        assert!(
            is_this_agent(&captured(&probe), "abc"),
            "{parent:o}: {:?}",
            captured(&probe)
        );
    }
    std::fs::set_permissions(remote.shared(), std::fs::Permissions::from_mode(0o700)).unwrap();

    // A directory an earlier run made with the usual umask is still good.
    std::fs::set_permissions(remote.dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let probe = remote.probe();
    assert!(
        is_this_agent(&captured(&probe), "abc"),
        "{:?}",
        captured(&probe)
    );

    // Another version's directory of this user goes. Another user's stays.
    let old = remote.shared().join("volant-agent-0.0.1");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("volant-agent"), "old").unwrap();
    let theirs = remote.shared().join("volant-agent-0.0.2");
    std::fs::create_dir(&theirs).unwrap();
    std::fs::write(theirs.join("volant-agent"), "theirs").unwrap();
    let handed = remote.hand_over(&theirs);
    let upload = remote.upload(&payload);
    assert_eq!(upload.status.code(), Some(0), "{:?}", captured(&upload));
    assert!(!old.exists(), "an older directory of this user is removed");
    if handed {
        assert!(theirs.exists(), "another user's directory is left alone");
    }
    assert!(is_this_agent(&captured(&remote.probe()), "abc"));
}

/// The default `remote_tmp` is under the home directory, which nobody else can write.
#[test]
fn the_default_remote_tmp_under_home_works() {
    let home = scratch();
    let target = Remote::target_at("~/.ansible/tmp");
    let payload = good_agent("abc");
    let upload = Remote::sh(
        &target.upload_command(payload.len() as u64),
        &payload,
        Some(home.path()),
    );
    assert_eq!(upload.status.code(), Some(0), "{:?}", captured(&upload));
    let probe = Remote::sh(&target.probe_command(), b"", Some(home.path()));
    assert!(
        is_this_agent(&captured(&probe), "abc"),
        "{:?}",
        captured(&probe)
    );
}

/// The start command checks again, so a directory swapped after the probe is not run, and the
/// stderr file of a link through `su` lands in the directory it was checked in.
#[test]
fn the_start_command_checks_the_directory_again() {
    let remote = Remote::new();
    let script = format!(
        "#!/bin/sh\necho started >> '{}'\necho oops >&2\n",
        remote.marker().display()
    );
    assert_eq!(remote.upload(script.as_bytes()).status.code(), Some(0));

    let started = Remote::sh(
        &remote.target.exec_command(" 2>\"$d/agent.stderr\""),
        b"",
        None,
    );
    assert_eq!(started.status.code(), Some(0), "{:?}", captured(&started));
    assert_eq!(
        std::fs::read_to_string(remote.marker()).unwrap(),
        "started\n"
    );
    assert_eq!(
        std::fs::read_to_string(remote.dir().join("agent.stderr")).unwrap(),
        "oops\n"
    );

    // Somebody opens the directory after the probe, and swaps the program.
    std::fs::set_permissions(remote.dir(), std::fs::Permissions::from_mode(0o777)).unwrap();
    std::fs::remove_file(remote.marker()).unwrap();
    let refused = Remote::sh(&remote.target.exec_command(""), b"", None);
    assert_eq!(
        refused.status.code(),
        Some(EXIT_UNSAFE),
        "{:?}",
        captured(&refused)
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains(remote.dir().to_str().unwrap()),
        "{:?}",
        captured(&refused)
    );
    assert!(!remote.marker().exists(), "the program did not start");
}
