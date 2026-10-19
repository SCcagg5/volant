// SPDX-License-Identifier: GPL-3.0-or-later
//! The program as `ssh` runs it for `SSH_ASKPASS`: the password on stdout for a password
//! prompt, nothing and a failure for any other question.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn the_binary_answers_as_an_askpass() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pw");
    std::fs::write(&file, "sek ret ").unwrap();
    let ask = |prompt: &str| {
        Command::new(env!("CARGO_BIN_EXE_volant"))
            .arg(prompt)
            .env("VOLANT_ASKPASS_FILE", &file)
            .output()
            .unwrap()
    };
    let answered = ask("user@host's password: ");
    assert!(answered.status.success());
    assert_eq!(answered.stdout, b"sek ret \n");
    let refused = ask("Are you sure you want to continue connecting (yes/no)? ");
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
}

/// What `ssh` does with an askpass, as a script: it runs the program `SSH_ASKPASS` names with the
/// password prompt, takes the answer from its standard output, and then either waits (a host
/// that connects and keeps the run busy) or gives up (a host that cannot be reached).
const FAKE_SSH: &str = r#"#!/bin/sh
answer=$("$SSH_ASKPASS" "box@host's password: ") || exit 255
printf '%s' "$answer" > "$HERE/answer.tmp" && mv "$HERE/answer.tmp" "$HERE/answer"
if [ "$ENDING" = unreachable ]; then exit 255; fi
echo $$ > "$HERE/pid.tmp" && mv "$HERE/pid.tmp" "$HERE/pid"
exec sleep 10
"#;

const PASSWORD: &str = "sek ret";

/// A directory per test with the fake `ssh`, a runtime directory and a temporary directory of
/// the test's own, so that what a run leaves is only what this test's run left.
struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        for name in ["bin", "run", "tmp", "home"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        std::fs::set_permissions(
            root.path().join("run"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let ssh = root.path().join("bin/ssh");
        std::fs::write(&ssh, FAKE_SSH).unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(root.path().join("password"), PASSWORD).unwrap();
        std::fs::write(
            root.path().join("inventory.ini"),
            "box ansible_host=192.0.2.1\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("play.yml"),
            "- hosts: box\n  gather_facts: false\n  tasks:\n    - ping:\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("local.yml"),
            "- hosts: localhost\n  connection: local\n  gather_facts: false\n  tasks:\n    - debug: msg=done\n",
        )
        .unwrap();
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// A run of the controller against the host the fake `ssh` stands for.
    fn start(&self, play: &str, ending: &str) -> std::process::Child {
        let path = format!("{}:/usr/bin:/bin", self.path("bin").display());
        Command::new(env!("CARGO_BIN_EXE_volant"))
            .args(["playbook", "-i"])
            .arg(self.path("inventory.ini"))
            .arg("--connection-password-file")
            .arg(self.path("password"))
            .arg(self.path(play))
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.path("home"))
            .env("XDG_RUNTIME_DIR", self.path("run"))
            .env("TMPDIR", self.path("tmp"))
            .env("HERE", self.root.path())
            .env("ENDING", ending)
            .env("NO_COLOR", "1")
            .env("ANSIBLE_HOST_KEY_CHECKING", "False")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Waits until `ssh` has been answered, so the password file has been read.
    fn wait_for(&self, name: &str) {
        let file = self.path(name);
        for _ in 0..1200 {
            if file.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("{name} never appeared");
    }

    /// The names under the runtime directory and the temporary directory, which are the two
    /// places the run may keep its password.
    fn left(&self) -> Vec<String> {
        let mut names = Vec::new();
        for dir in ["run", "tmp"] {
            for entry in std::fs::read_dir(self.path(dir)).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("volant-askpass-") {
                    names.push(format!("{dir}/{name}"));
                }
            }
        }
        names.sort();
        names
    }
}

fn finish(mut child: std::process::Child) -> Option<i32> {
    for _ in 0..1200 {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = child.kill();
    panic!("the run did not end");
}

fn send(signal: &str, pid: u32) {
    let sent = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .unwrap();
    assert!(sent.success());
}

/// The run that is stopped at a point where `ssh` has the password leaves nothing behind, and the
/// file it had was in a private directory under the runtime directory.
#[rstest::rstest]
#[case::interrupt("INT")]
#[case::terminate("TERM")]
#[case::hang_up("HUP")]
fn a_stopped_run_removes_its_password_file(#[case] signal: &str) {
    use std::os::unix::fs::MetadataExt as _;
    let sandbox = Sandbox::new();
    let child = sandbox.start("play.yml", "hang");
    sandbox.wait_for("pid");
    assert_eq!(
        std::fs::read_to_string(sandbox.path("answer")).unwrap(),
        PASSWORD,
        "the askpass gives ssh the password"
    );
    let kept = sandbox.left();
    assert_eq!(
        kept.len(),
        1,
        "one private directory while the run holds the password: {kept:?}"
    );
    assert!(
        kept[0].starts_with(&format!("run/volant-askpass-{}-", child.id())),
        "{kept:?}"
    );
    let dir = sandbox.path(&kept[0]);
    assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o7777, 0o700);
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    assert_eq!(
        files.len(),
        1,
        "the password file alone is in the directory"
    );
    assert_eq!(files[0].metadata().unwrap().mode() & 0o7777, 0o600);
    send(signal, child.id());
    assert_eq!(finish(child), Some(99));
    assert_eq!(sandbox.left(), Vec::<String>::new());
    let survivor = std::fs::read_to_string(sandbox.path("pid")).unwrap();
    let _ = Command::new("kill").arg(survivor.trim()).status();
}

/// A host that cannot be reached ends the run with code 4 and no file.
#[test]
fn an_unreachable_host_leaves_no_password_file() {
    let sandbox = Sandbox::new();
    let child = sandbox.start("play.yml", "unreachable");
    assert_eq!(finish(child), Some(4));
    assert!(sandbox.path("answer").exists(), "ssh was answered once");
    assert_eq!(sandbox.left(), Vec::<String>::new());
}

/// A run that goes to its end leaves no file either.
#[test]
fn a_finished_run_leaves_no_password_file() {
    let sandbox = Sandbox::new();
    let child = sandbox.start("local.yml", "hang");
    assert_eq!(finish(child), Some(0));
    assert_eq!(sandbox.left(), Vec::<String>::new());
}

/// SIGKILL runs no code, so its directory stays, private and named after the dead process. The
/// next run of the same user removes it.
#[test]
fn a_killed_run_leaves_a_private_directory_the_next_run_removes() {
    use std::os::unix::fs::MetadataExt as _;
    let sandbox = Sandbox::new();
    let mut child = sandbox.start("play.yml", "hang");
    sandbox.wait_for("pid");
    let dead = child.id();
    child.kill().unwrap();
    child.wait().unwrap();
    let survivor = std::fs::read_to_string(sandbox.path("pid")).unwrap();
    let _ = Command::new("kill").arg(survivor.trim()).status();
    let kept = sandbox.left();
    assert_eq!(kept.len(), 1, "{kept:?}");
    let name = kept[0]
        .strip_prefix("run/")
        .expect("under the runtime directory");
    assert!(
        name.starts_with(&format!("volant-askpass-{dead}-")),
        "{name}"
    );
    assert_eq!(
        std::fs::metadata(sandbox.path(&kept[0])).unwrap().mode() & 0o7777,
        0o700
    );
    let next = sandbox.start("local.yml", "hang");
    assert_eq!(finish(next), Some(0));
    assert_eq!(sandbox.left(), Vec::<String>::new());
}
