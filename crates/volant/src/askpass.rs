// SPDX-License-Identifier: GPL-3.0-or-later
//! The connection password, handed to `ssh` through `SSH_ASKPASS`. The run writes the password
//! to a private file; every `ssh` it starts is told to run this program as its askpass, which
//! prints the file's text. The password is never in an argument list, in the environment of
//! the program or in anything sent to the host: only the file's path is.
//!
//! `ssh` closes every descriptor above 2 before it starts its askpass, so a descriptor the run
//! holds cannot carry the password and a path has to. The file sits in a directory of its own,
//! mode 0700, named after the process. The run removes the directory when it ends, when it
//! unwinds from a panic, when it stops on SIGINT or SIGTERM, and before it dies of SIGHUP. A
//! kill by SIGKILL, or by the out-of-memory killer, cannot run any code: the directory stays,
//! and the next run of the same user removes it, because the process in its name is gone and
//! no run holds the lock that the directory's owner took on it.

use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The environment variable that turns this program into an askpass for one `ssh` call.
const FILE_ENV: &str = "VOLANT_ASKPASS_FILE";

/// The start of the name of a run's private directory: `volant-askpass-<pid>-<random>`.
const DIR_PREFIX: &str = "volant-askpass-";

static FILE: OnceLock<PathBuf> = OnceLock::new();

/// Keeps the password directory, the lock on it and the file in it alive. The directory and the
/// file are removed when this is dropped.
pub struct Installed(
    #[expect(dead_code, reason = "held for its Drop")] tempfile::TempDir,
    #[expect(dead_code, reason = "held for its lock")] std::fs::File,
    #[expect(dead_code, reason = "held for its Drop")] tempfile::NamedTempFile,
);

/// Stores the connection password in a file only this user can read, and makes every `ssh`
/// the run starts ask this program for it.
pub fn install(password: &str) -> std::io::Result<Installed> {
    use std::io::Write as _;
    let (dir, lock) = private_dir()?;
    let mut file = tempfile::Builder::new()
        .prefix("password-")
        .tempfile_in(dir.path())?;
    file.write_all(password.as_bytes())?;
    file.flush()?;
    FILE.set(file.path().to_path_buf())
        .map_err(|_| std::io::Error::other("a connection password is already installed"))?;
    Ok(Installed(dir, lock, file))
}

/// The directories a run's own directory can sit in: the user's runtime directory when it is
/// private, then the temporary directory.
fn bases() -> Vec<PathBuf> {
    #[cfg(unix)]
    let runtime = {
        let uid = effective_uid();
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .filter(|dir| {
                std::fs::symlink_metadata(dir)
                    .is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o077 == 0)
            })
    };
    #[cfg(not(unix))]
    let runtime: Option<PathBuf> = None;
    runtime.into_iter().chain([std::env::temp_dir()]).collect()
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` takes no argument and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn effective_uid() -> u32 {
    0
}

/// A new directory, mode 0700 and named after this process, in the first base that takes it, and
/// the open descriptor that holds an exclusive lock on it. The kernel drops the lock when the
/// process ends, whatever ended it, so a directory nobody locks has no run behind it.
fn private_dir() -> std::io::Result<(tempfile::TempDir, std::fs::File)> {
    let prefix = format!("{DIR_PREFIX}{}-", std::process::id());
    let mut last = std::io::Error::other("no directory to keep the password file in");
    for base in bases() {
        let mut builder = tempfile::Builder::new();
        builder.prefix(&prefix);
        // Mode 0700 from the `mkdir` itself: the default is the umask's, 0755 on most accounts.
        #[cfg(unix)]
        builder.permissions(std::fs::Permissions::from_mode(0o700));
        let made = builder.tempdir_in(base);
        match made {
            Ok(dir) => {
                let lock = std::fs::File::open(dir.path())?;
                // A file system with no locks leaves the sweep to the process number alone.
                let _ = lock.try_lock();
                return Ok((dir, lock));
            }
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// Removes the directories that runs of this user left when they were killed: one whose name
/// carries the number of a process that is gone. Called at the start of a run.
pub fn remove_stale() {
    let uid = effective_uid();
    for base in bases() {
        remove_stale_in(&base, uid);
    }
}

/// [`remove_stale`] in one directory. Only a real directory, owned by `uid`, with mode 0700,
/// that no run holds the lock of goes: a symlink, a file, another user's directory, a directory
/// of another mode or a directory of a live run stays. The lock is what tells a live run, since
/// a process number means nothing to a run in another PID namespace on the same directory.
fn remove_stale_in(base: &Path, uid: u32) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let pid = entry.file_name().to_str().and_then(owner_pid);
        if pid.is_none_or(crate::runtime::install::alive) {
            continue;
        }
        let path = entry.path();
        // The lock stays held while the directory goes, so no other sweep judges it meanwhile.
        if let Some(_held) = claim_stale(&path, uid) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// The open, locked directory at `path` when it is a real directory of `uid`, mode 0700, that no
/// run holds the lock of. It is opened without following a link, and judged by what the
/// descriptor says.
#[cfg(unix)]
fn claim_stale(path: &Path, uid: u32) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let Ok(dir) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)
    else {
        return None;
    };
    let mine = dir
        .metadata()
        .is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o7777 == 0o700);
    // A file system with no locks leaves the judgement to the process number alone.
    (mine && !matches!(dir.try_lock(), Err(std::fs::TryLockError::WouldBlock))).then_some(dir)
}

/// Without owners and modes to judge by, nothing is swept.
#[cfg(not(unix))]
fn claim_stale(_: &Path, _: u32) -> Option<std::fs::File> {
    None
}

/// Removes the run's password directory at once. For a stop that runs no destructor: the
/// process is about to die of a signal.
pub fn remove_now() {
    if let Some(dir) = FILE.get().and_then(|file| file.parent()) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The process number in the name of a run's directory.
fn owner_pid(name: &str) -> Option<i32> {
    let (pid, _) = name.strip_prefix(DIR_PREFIX)?.split_once('-')?;
    pid.parse().ok().filter(|pid| *pid > 0)
}

/// Whether the run has a connection password to offer.
pub fn installed() -> bool {
    FILE.get().is_some()
}

/// The environment an `ssh` of this run is started with, empty when there is no password.
pub fn environment() -> Vec<(&'static str, OsString)> {
    let (Some(file), Ok(me)) = (FILE.get(), std::env::current_exe()) else {
        return Vec::new();
    };
    vec![
        ("SSH_ASKPASS", me.into_os_string()),
        ("SSH_ASKPASS_REQUIRE", "force".into()),
        (FILE_ENV, file.clone().into_os_string()),
    ]
}

/// What `ssh` is told when it asks `prompt`: the password for a prompt that ends in
/// `password:`, nothing for any other question. A host-key confirmation or a key passphrase
/// may name a file or a host with "password" in it and must never get the password.
fn reply(file: &Path, prompt: &str) -> Option<Vec<u8>> {
    let lower = prompt.trim().to_ascii_lowercase();
    if lower.contains("passphrase") || lower.contains("yes/no") || !lower.ends_with("password:") {
        return None;
    }
    let mut text = std::fs::read(file).ok()?;
    text.push(b'\n');
    Some(text)
}

/// When `ssh` started this program as its askpass, answers and exits; otherwise returns.
pub fn answer_if_asked() {
    use std::io::Write as _;
    let Some(file) = std::env::var_os(FILE_ENV) else {
        return;
    };
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let code = match reply(Path::new(&file), &prompt) {
        Some(text) if std::io::stdout().write_all(&text).is_ok() => 0,
        _ => 1,
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests;
