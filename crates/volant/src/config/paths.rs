// SPDX-License-Identifier: GPL-3.0-or-later
//! The directories a setting names: the defaults, the home directory, and path lists.

use std::path::{Path, PathBuf};

use crate::locations;

/// A home-relative directory first, then the system ones: the reference's own default for
/// `roles_path`, read off the list a missing role prints.
pub(super) fn default_roles_path() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home() {
        paths.push(home.join(".ansible").join("roles"));
    }
    paths.extend(locations::system_content_dirs("roles"));
    paths
}

pub(super) fn default_collections_path() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home() {
        paths.push(home.join(".ansible").join("collections"));
    }
    paths.extend(locations::system_content_dirs("collections"));
    paths
}

/// The user's home directory, as [`locations::home`] has it.
pub(super) fn home() -> Option<PathBuf> {
    locations::home()
}

/// `path` with a leading `~` or `~/` replaced by the home directory, anchored on `base` when it
/// is still relative.
pub(super) fn expand_home(path: &str, base: &Path) -> PathBuf {
    match (locations::after_tilde(path), home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => base.join(path),
    }
}

/// A list of directories, as every Ansible path setting is written: colon-separated, and
/// separated by [`locations::LIST_SEPARATOR`] on Windows. Relative entries are anchored against
/// `base`, which is the configuration file's directory for a value read from one and the working
/// directory for one read from the environment.
pub(super) fn path_list(value: &str, base: &Path) -> Vec<PathBuf> {
    value
        .split(locations::LIST_SEPARATOR)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| base.join(p))
        .collect()
}
