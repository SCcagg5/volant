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

/// A password a host variable gave is read from a file of its own that only this user can read,
/// one file for one password, and the file is gone once the run's guard drops. An empty one
/// offers nothing, and neither the program nor its arguments hold the text.
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
    drop(guard);
    assert!(!path.exists(), "the password file outlived the run");
}
