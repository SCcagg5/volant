// SPDX-License-Identifier: GPL-3.0-or-later
//! Where a task's controller file is found, and how its bytes travel to the host.

use std::path::{Component, Path, PathBuf};

use volant_protocol::encoding::b64_encode;
use volant_protocol::frame::MAX_FRAME_LEN;

use std::collections::BTreeSet;

use super::FileBlob;
use crate::compile::Origin;
use crate::config::HostNamedPaths;

/// The largest file one blob carries: one frame, which holds the blob's bytes alone, raw or
/// deflated to fewer.
pub(crate) const MAX_FILE_LEN: usize = MAX_FRAME_LEN;

/// Where the reference looks for a task's local file, in its order.
///
/// Measured on ansible-core 2.19.12 from its "Searched in" list: `<role>/<subdir>/`, `<role>/`,
/// `<parent role>/<subdir>/`, `<parent role>/` for each role the role is a dependency of,
/// `<role>/tasks/<subdir>/`, `<role>/tasks/`, `<playbook>/<subdir>/`, `<playbook>/`. The task's
/// own directory stands where `<role>/tasks/` is, so a task outside a role searches its own
/// directory and the playbook's, which for a task written in the playbook is the same one twice,
/// as the reference lists it. A name that already starts with `<subdir>/` is not looked for
/// under `<subdir>/` again, and an absolute one is taken as it is.
pub(crate) fn search_paths(
    origin: &Origin,
    playbook_dir: &Path,
    subdir: &str,
    name: &str,
) -> Vec<PathBuf> {
    let named = Path::new(name);
    if named.is_absolute() {
        return vec![named.to_path_buf()];
    }
    let under = name.split('/').next() != Some(subdir);
    let mut out = Vec::new();
    for base in origin.search_dirs().chain([playbook_dir]) {
        if under {
            out.push(base.join(subdir).join(name));
        }
        out.push(base.join(name));
    }
    out
}

/// The reference's refusal when none of the [`search_paths`] is there.
pub(crate) fn not_found(name: &str, searched: &[PathBuf]) -> String {
    let searched: Vec<String> = searched.iter().map(|p| p.display().to_string()).collect();
    format!(
        "Could not find or access '{name}'\nSearched in:\n\t{} on the Ansible Controller.",
        searched.join("\n\t")
    )
}

/// What `host_named_paths` says about a controller source `name` that `untrusted` lists `arg`
/// as named by a managed host, made before the path is so much as looked up: a lookup that
/// answers "not found" already tells a host whether a controller path exists.
///
/// Measured on ansible-core 2.19.12, the reference sends whatever controller file a registered
/// value or a gathered fact names, `~/.ssh` included, so a host that controls a command's output
/// can have any file the operator can read sent to it. `Allow` does the same. `Refuse` refuses
/// every such name. `Contained` accepts a name that stays inside the directories where the
/// author keeps the files of the task, which are the ones [`search_paths`] looks in: it has no
/// `..` component, and an absolute one lies under one of them. The check reads the text of the
/// path. A link the author put inside those directories is the author's own file and is followed.
pub(crate) fn check_host_named(
    mode: HostNamedPaths,
    untrusted: &BTreeSet<String>,
    arg: &str,
    name: &str,
    origin: &Origin,
    playbook_dir: &Path,
) -> Result<(), String> {
    if !untrusted.contains(arg) {
        return Ok(());
    }
    match mode {
        HostNamedPaths::Allow => Ok(()),
        HostNamedPaths::Refuse => Err(format!(
            "the '{arg}' of this task was named by a managed host, and a controller file a host chose is never sent"
        )),
        HostNamedPaths::Contained => {
            if inside_task_directories(origin, playbook_dir, name) {
                Ok(())
            } else {
                Err(format!(
                    "the '{arg}' of this task was named by a managed host, and '{name}' is not inside the directories of the playbook or the role (host_named_paths = contained)"
                ))
            }
        }
    }
}

/// Whether `name` stays under the directories [`search_paths`] looks in: a relative name is
/// joined to one of them, so it is inside unless it climbs out with `..`.
fn inside_task_directories(origin: &Origin, playbook_dir: &Path, name: &str) -> bool {
    let named = Path::new(name);
    if named.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    !named.is_absolute()
        || origin
            .search_dirs()
            .chain([playbook_dir])
            .any(|base| named.starts_with(base))
}

#[cfg(test)]
thread_local! {
    /// The frame limit the tests of this thread run with, so a test of the refusal does not build
    /// a payload of the real size.
    pub(crate) static FRAME_LIMIT: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_FILE_LEN) };
}

/// The most one frame carries: [`MAX_FILE_LEN`], or the test thread's own limit.
fn frame_limit() -> usize {
    #[cfg(test)]
    return FRAME_LIMIT.get();
    #[cfg(not(test))]
    MAX_FILE_LEN
}

/// Refuses `len` bytes that could not fit in one frame, naming both sizes.
pub(crate) fn fits(name: &str, len: usize) -> Result<(), String> {
    let limit = frame_limit();
    if len > limit {
        return Err(format!(
            "{name} is {len} bytes; one frame carries at most {limit}"
        ));
    }
    Ok(())
}

/// A file's bytes as a blob, refused when they could not fit in one frame.
///
/// Named by the blake3 of the bytes, as the union is: the agent refuses a blob whose bytes hash
/// to anything else.
pub(crate) fn blob_of(name: &str, bytes: &[u8]) -> Result<FileBlob, String> {
    fits(name, bytes.len())?;
    Ok(FileBlob {
        hash: blake3::hash(bytes).to_hex().to_string(),
        b64: b64_encode(bytes),
    })
}

#[cfg(test)]
mod tests;
