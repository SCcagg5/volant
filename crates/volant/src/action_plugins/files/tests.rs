// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

/// The order the reference searches in, with a role and without one.
#[test]
fn a_file_is_searched_where_the_reference_searches() {
    let role = Origin {
        file_dir: PathBuf::from("/pb/roles/r/tasks"),
        role_dir: Some(PathBuf::from("/pb/roles/r")),
        parent_roles: Vec::new(),
        depth: 0,
        inherited: None,
    };
    let pb = Path::new("/pb");
    assert_eq!(
        search_paths(&role, pb, "files", "a.txt"),
        [
            "/pb/roles/r/files/a.txt",
            "/pb/roles/r/a.txt",
            "/pb/roles/r/tasks/files/a.txt",
            "/pb/roles/r/tasks/a.txt",
            "/pb/files/a.txt",
            "/pb/a.txt",
        ]
        .map(PathBuf::from)
    );
    let play = Origin {
        file_dir: PathBuf::from("/pb"),
        ..Origin::default()
    };
    assert_eq!(
        search_paths(&play, pb, "files", "files/a.txt"),
        ["/pb/files/a.txt", "/pb/files/a.txt"].map(PathBuf::from)
    );
    assert_eq!(
        search_paths(&play, pb, "files", "/etc/a.txt"),
        [PathBuf::from("/etc/a.txt")]
    );
    assert_eq!(
        not_found("a.txt", &search_paths(&play, pb, "files", "a.txt")),
        "Could not find or access 'a.txt'\nSearched in:\n\t/pb/files/a.txt\n\t/pb/a.txt\n\t/pb/files/a.txt\n\t/pb/a.txt on the Ansible Controller."
    );
}

/// A file too big for one frame is refused before anything is encoded, naming both sizes.
#[test]
fn a_file_bigger_than_a_frame_is_refused_naming_both_sizes() {
    FRAME_LIMIT.set(1024);
    let big = vec![0u8; 1025];
    let err = blob_of("big.bin", &big).expect_err("1025 bytes do not fit");
    assert_eq!(err, "big.bin is 1025 bytes; one frame carries at most 1024");
    assert!(blob_of("fits.bin", &big[..1024]).is_ok());
    let blob = blob_of("x", b"x").expect("one byte fits");
    assert_eq!(blob.b64, "eA==");
    assert_eq!(blob.hash, blake3::hash(b"x").to_hex().to_string());
}

/// What each mode does with a name a managed host chose, and with one it did not.
#[rstest::rstest]
#[case::allow_takes_any_path(HostNamedPaths::Allow, "/etc/passwd", Ok(()))]
#[case::allow_takes_a_climb(HostNamedPaths::Allow, "../x", Ok(()))]
#[case::refuse_takes_none(
    HostNamedPaths::Refuse,
    "a.txt",
    Err("a controller file a host chose is never sent")
)]
#[case::contained_plain(HostNamedPaths::Contained, "a.txt", Ok(()))]
#[case::contained_nested(HostNamedPaths::Contained, "sub/dir/a.txt", Ok(()))]
#[case::contained_dot(HostNamedPaths::Contained, "./a.txt", Ok(()))]
#[case::contained_in_role(HostNamedPaths::Contained, "/pb/roles/r/files/a.txt", Ok(()))]
#[case::contained_in_tasks_directory(HostNamedPaths::Contained, "/pb/roles/r/tasks/a.txt", Ok(()))]
#[case::contained_in_playbook(HostNamedPaths::Contained, "/pb/a.txt", Ok(()))]
#[case::contained_climbs(
    HostNamedPaths::Contained,
    "../a.txt",
    Err("is not inside the directories")
)]
#[case::contained_climbs_from_below(
    HostNamedPaths::Contained,
    "sub/../a.txt",
    Err("is not inside the directories")
)]
#[case::contained_absolute_outside(
    HostNamedPaths::Contained,
    "/etc/passwd",
    Err("is not inside the directories")
)]
#[case::contained_absolute_climbs(
    HostNamedPaths::Contained,
    "/pb/roles/r/../../../etc/passwd",
    Err("is not inside the directories")
)]
#[case::contained_sibling_of_the_playbook(
    HostNamedPaths::Contained,
    "/pb2/a.txt",
    Err("is not inside the directories")
)]
fn a_name_a_host_chose_is_read_as_the_mode_says(
    #[case] mode: HostNamedPaths,
    #[case] name: &str,
    #[case] expected: Result<(), &str>,
) {
    let origin = Origin {
        file_dir: PathBuf::from("/pb/roles/r/tasks"),
        role_dir: Some(PathBuf::from("/pb/roles/r")),
        parent_roles: Vec::new(),
        depth: 0,
        inherited: None,
    };
    let untrusted: BTreeSet<String> = ["src".to_string()].into();
    let got = check_host_named(mode, &untrusted, "src", name, &origin, Path::new("/pb"));
    match (got, expected) {
        (Ok(()), Ok(())) => {}
        (Err(msg), Err(says)) => {
            assert!(
                msg.starts_with("the 'src' of this task was named by a managed host, and "),
                "{msg}"
            );
            assert!(msg.contains(says), "{msg}");
        }
        (got, expected) => panic!("{name}: got {got:?}, expected {expected:?}"),
    }
    // A name no host chose is the author's, whatever the mode.
    assert_eq!(
        check_host_named(
            mode,
            &BTreeSet::new(),
            "src",
            name,
            &origin,
            Path::new("/pb")
        ),
        Ok(())
    );
}

/// A refusal of the contained mode names the argument and the value, so the operator can find
/// the task.
#[test]
fn the_contained_refusal_names_the_value() {
    let untrusted: BTreeSet<String> = ["_raw_params".to_string()].into();
    assert_eq!(
        check_host_named(
            HostNamedPaths::Contained,
            &untrusted,
            "_raw_params",
            "../up.sh",
            &Origin::default(),
            Path::new("/pb"),
        ),
        Err("the '_raw_params' of this task was named by a managed host, and '../up.sh' is not inside the directories of the playbook or the role (host_named_paths = contained)".into())
    );
}
