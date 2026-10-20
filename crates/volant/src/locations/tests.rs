// SPDX-License-Identifier: GPL-3.0-or-later
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use rstest::rstest;

use super::Platform::{Macos, Unix, Windows};
use super::*;

/// A directory that is absolute on the host the test runs on.
fn abs(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

/// An environment of the variables given, where a value written `abs:name` is [`abs`] of it.
fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let vars: Vec<(String, OsString)> = vars
        .iter()
        .map(|(name, value)| {
            let value = match value.strip_prefix("abs:") {
                Some(name) => abs(name).into_os_string(),
                None => OsString::from(value),
            };
            ((*name).to_string(), value)
        })
        .collect();
    move |name| {
        vars.iter()
            .find(|(var, _)| var == name)
            .map(|(_, value)| value.clone())
    }
}

#[rstest]
#[case::unix_home(Unix, &[("HOME", "abs:h")], Some("h"))]
#[case::unix_ignores_userprofile(Unix, &[("USERPROFILE", "abs:u")], None)]
#[case::unix_empty_home(Unix, &[("HOME", "")], None)]
#[case::windows_profile(Windows, &[("USERPROFILE", "abs:u"), ("HOME", "abs:h")], Some("u"))]
#[case::windows_ignores_home(Windows, &[("HOME", "abs:h")], None)]
#[case::windows_empty_profile(Windows, &[("USERPROFILE", ""), ("HOMEDRIVE", "C:"), ("HOMEPATH", "\\Users\\me")], Some("C:\\Users\\me"))]
#[case::windows_drive_alone(Windows, &[("HOMEDRIVE", "C:")], None)]
fn the_home_of_a_platform(
    #[case] platform: Platform,
    #[case] vars: &[(&str, &str)],
    #[case] want: Option<&str>,
) {
    let want = want.map(|name| {
        if name.contains(':') {
            PathBuf::from(name)
        } else {
            abs(name)
        }
    });
    assert_eq!(home_of(platform, &env_of(vars)), want);
}

#[rstest]
#[case::unix_xdg(Unix, &[("XDG_CACHE_HOME", "abs:x"), ("HOME", "abs:h")], Some(abs("x").join("volant")))]
#[case::unix_home(Unix, &[("HOME", "abs:h")], Some(abs("h").join(".cache").join("volant")))]
#[case::macos_like_unix(Macos, &[("HOME", "abs:h")], Some(abs("h").join(".cache").join("volant")))]
#[case::unix_relative_xdg_is_ignored(Unix, &[("XDG_CACHE_HOME", "rel"), ("HOME", "abs:h")], Some(abs("h").join(".cache").join("volant")))]
#[case::unix_nothing(Unix, &[("USERPROFILE", "abs:u")], None)]
#[case::windows_local_app_data(Windows, &[("LOCALAPPDATA", "abs:l"), ("HOME", "abs:h")], Some(abs("l").join("volant").join("cache")))]
#[case::windows_relative_is_ignored(Windows, &[("LOCALAPPDATA", "rel")], None)]
#[case::windows_needs_the_variable(Windows, &[("USERPROFILE", "abs:u")], None)]
fn the_cache_of_a_platform(
    #[case] platform: Platform,
    #[case] vars: &[(&str, &str)],
    #[case] want: Option<PathBuf>,
) {
    assert_eq!(cache_root_of(platform, &env_of(vars)), want);
}

#[rstest]
#[case::unix_xdg(Unix, &[("XDG_DATA_HOME", "abs:x"), ("HOME", "abs:h")], Some(abs("x").join("volant")))]
#[case::unix_home(Unix, &[("HOME", "abs:h")], Some(abs("h").join(".local/share").join("volant")))]
#[case::macos(Macos, &[("XDG_DATA_HOME", "abs:x"), ("HOME", "abs:h")], Some(abs("h").join("Library").join("Application Support").join("volant")))]
#[case::windows(Windows, &[("LOCALAPPDATA", "abs:l"), ("XDG_DATA_HOME", "abs:x")], Some(abs("l").join("volant")))]
#[case::windows_nothing(Windows, &[("HOME", "abs:h")], None)]
fn the_data_directory_of_a_platform(
    #[case] platform: Platform,
    #[case] vars: &[(&str, &str)],
    #[case] want: Option<PathBuf>,
) {
    assert_eq!(data_root_of(platform, &env_of(vars)), want);
}

#[test]
fn the_system_directory_is_etc_ansible_or_program_data() {
    let none = env_of(&[]);
    assert_eq!(system_dir_of(Unix, &none), Path::new("/etc/ansible"));
    assert_eq!(
        system_dir_of(Windows, &env_of(&[("PROGRAMDATA", "abs:p")])),
        abs("p").join("ansible")
    );
    assert_eq!(
        system_dir_of(Windows, &none),
        Path::new(r"C:\ProgramData").join("ansible")
    );
}

#[rstest]
#[case::unix(Unix, &[], &["/usr/share/ansible/roles", "/etc/ansible/roles"])]
#[case::macos(Macos, &[], &["/usr/share/ansible/roles", "/etc/ansible/roles"])]
#[case::windows(Windows, &[("PROGRAMDATA", "abs:p")], &["abs:p"])]
fn the_system_roles_of_a_platform(
    #[case] platform: Platform,
    #[case] vars: &[(&str, &str)],
    #[case] want: &[&str],
) {
    let want: Vec<PathBuf> = want
        .iter()
        .map(|dir| match dir.strip_prefix("abs:") {
            Some(name) => abs(name).join("ansible").join("roles"),
            None => PathBuf::from(dir),
        })
        .collect();
    assert_eq!(
        system_content_dirs_of(platform, &env_of(vars), "roles"),
        want
    );
}

#[rstest]
#[case::bare("~", Unix, Some(""))]
#[case::slash("~/a/b", Unix, Some("a/b"))]
#[case::doubled("~//a", Unix, Some("a"))]
#[case::user("~me/a", Unix, None)]
#[case::not_leading("a/~", Unix, None)]
#[case::unix_backslash_is_a_name("~\\a", Unix, None)]
#[case::windows_backslash("~\\a\\b", Windows, Some("a\\b"))]
#[case::windows_slash("~/a", Windows, Some("a"))]
#[case::windows_user("~me\\a", Windows, None)]
fn a_leading_tilde_names_the_home(
    #[case] path: &str,
    #[case] platform: Platform,
    #[case] want: Option<&str>,
) {
    assert_eq!(after_tilde_on(platform, path), want);
}

#[test]
fn a_list_is_split_where_the_platform_splits_it() {
    assert_eq!(LIST_SEPARATOR, if cfg!(windows) { ';' } else { ':' });
}
