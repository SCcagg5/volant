// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

/// A password prompt gets the file's text and a newline; a question that is not for a
/// password gets nothing.
#[rstest::rstest]
#[case::password("user@host's password: ", Some("sek ret \n"))]
#[case::keyboard_interactive("Password: ", Some("sek ret \n"))]
#[case::host_key("Are you sure you want to continue connecting (yes/no)? ", None)]
#[case::passphrase("Enter passphrase for key '/k': ", None)]
#[case::passphrase_of_a_password_key("Enter passphrase for key '/x/password.pem': ", None)]
#[case::host_key_of_a_password_host(
    "The authenticity of host 'password-db' can't be established. Are you sure you want to continue connecting (yes/no)? ",
    None
)]
#[case::user_password(
    "(user@host) Password: ",
    Some(
        "sek ret 
"
    )
)]
fn the_askpass_answers_password_prompts_only(#[case] prompt: &str, #[case] want: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pw");
    std::fs::write(&file, "sek ret ").unwrap();
    assert_eq!(
        reply(&file, prompt),
        want.map(|text| text.as_bytes().to_vec())
    );
}

#[test]
fn a_missing_file_gives_no_answer() {
    assert_eq!(
        reply(Path::new("/nonexistent/volant/pw"), "password: "),
        None
    );
}

/// A variable's password gets a private file, one per password, in a 0700 directory that goes
/// with the run's guard; an empty one offers nothing.
#[cfg(unix)]
#[test]
fn a_variable_password_gets_a_private_file_that_goes_with_the_run() {
    use std::os::unix::fs::PermissionsExt as _;
    let guard = install(None).unwrap();
    let password = "variable-pw-only-this-test";
    let path_of = |password| {
        let env = environment(Source::Variable(password)).unwrap();
        assert!(
            env.iter().all(|(key, value)| !key.contains(password)
                && !value.to_string_lossy().contains(password)),
            "the password is in the environment: {env:?}"
        );
        env.iter()
            .find(|(key, _)| *key == FILE_ENV)
            .map(|(_, value)| PathBuf::from(value))
    };
    let path = path_of(password).expect("an askpass file");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), password);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        path_of(password),
        Some(path.clone()),
        "one file for one password"
    );
    assert_ne!(path_of("another"), Some(path.clone()));
    assert!(offers(Source::Variable(password)));
    assert!(!offers(Source::Variable("")));
    assert!(environment(Source::Variable("")).unwrap().is_empty());
    let dir = path.parent().unwrap().to_path_buf();
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    drop(guard);
    assert!(!path.exists(), "the password file outlived the run");
    assert!(!dir.exists(), "the password directory outlived the run");
}

#[cfg(unix)]
/// The process number is the first field after the prefix, a real one.
#[rstest::rstest]
#[case::a_run("volant-askpass-4242-aB3xYz", Some(4242))]
#[case::no_random_part("volant-askpass-4242", None)]
#[case::an_older_name("volant-askpass-aB3xYz", None)]
#[case::zero("volant-askpass-0-aB3xYz", None)]
#[case::negative("volant-askpass--5-aB3xYz", None)]
#[case::another_name("volant-cm-4242-aB3xYz", None)]
fn the_process_number_is_read_from_the_name(#[case] name: &str, #[case] want: Option<i32>) {
    assert_eq!(owner_pid(name), want);
}

#[cfg(unix)]
/// A process that has ended, so its number is free.
fn dead_pid() -> i32 {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id() as i32;
    child.wait().unwrap();
    pid
}

#[cfg(unix)]
fn make_dir(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir(path).unwrap();
    std::fs::write(path.join("password"), "sek ret").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(unix)]
/// What a start removes: the private directory of a process that is gone, and nothing else.
#[rstest::rstest]
#[case::a_dead_run("dead", true)]
#[case::a_live_run("live", false)]
#[case::the_parent_of_this_run("parent", false)]
#[case::a_directory_open_to_the_group("open", false)]
#[case::another_owner("owner", false)]
#[case::a_link_to_a_directory("link", false)]
#[case::a_file("file", false)]
#[case::an_older_name("older", false)]
fn a_start_removes_only_what_a_dead_run_left(#[case] kind: &str, #[case] removed: bool) {
    let base = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let dead = dead_pid();
    let mine = effective_uid();
    let (name, uid) = match kind {
        "live" => (format!("volant-askpass-{}-abc", std::process::id()), mine),
        "parent" => (
            format!("volant-askpass-{}-abc", std::os::unix::process::parent_id()),
            mine,
        ),
        "older" => ("volant-askpass-abc123".to_string(), mine),
        // What the sweep treats as the owner it runs for: a directory of this user is, for
        // another user's sweep, someone else's.
        "owner" => (format!("volant-askpass-{dead}-abc"), mine + 1),
        _ => (format!("volant-askpass-{dead}-abc"), mine),
    };
    let path = base.path().join(&name);
    let target = elsewhere.path().join("target");
    match kind {
        "open" => make_dir(&path, 0o750),
        "link" => {
            make_dir(&target, 0o700);
            std::os::unix::fs::symlink(&target, &path).unwrap();
        }
        "file" => std::fs::write(&path, "sek ret").unwrap(),
        _ => make_dir(&path, 0o700),
    }
    remove_stale_in(base.path(), uid);
    assert_eq!(!path.exists() && path.symlink_metadata().is_err(), removed);
    if kind == "link" {
        assert!(
            target.join("password").exists(),
            "what a link points at stays"
        );
    }
}

#[cfg(unix)]
/// A directory whose lock another run holds is that run's own, whatever its name says; one whose
/// holder is gone is stale.
#[test]
fn a_directory_that_a_live_run_holds_the_lock_of_is_kept() {
    let base = tempfile::tempdir().unwrap();
    let path = base
        .path()
        .join(format!("volant-askpass-{}-abc", dead_pid()));
    make_dir(&path, 0o700);
    let holder = std::fs::File::open(&path).unwrap();
    holder.try_lock().unwrap();
    remove_stale_in(base.path(), effective_uid());
    assert!(path.exists(), "a directory under a lock was removed");
    drop(holder);
    remove_stale_in(base.path(), effective_uid());
    assert!(!path.exists(), "the directory of a gone holder stayed");
}
