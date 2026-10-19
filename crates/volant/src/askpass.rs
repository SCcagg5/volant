// SPDX-License-Identifier: GPL-3.0-or-later
//! The connection password, handed to `ssh` through `SSH_ASKPASS`. The run writes the password
//! to a private file; every `ssh` it starts is told to run this program as its askpass, which
//! prints the file's text. The password is never in an argument list, in the environment of
//! the program or in anything sent to the host: only the file's path is. A password a host
//! names in `ansible_password` or one of its aliases takes the same way, in a file of its own.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

/// The environment variable that turns this program into an askpass for one `ssh` call.
const FILE_ENV: &str = "VOLANT_ASKPASS_FILE";

/// The file that holds the password of `-k` or `--connection-password-file`.
static FILE: OnceLock<PathBuf> = OnceLock::new();

/// The files that hold the passwords hosts named in a variable, by password. A file is made the
/// first time an `ssh` call needs it and is removed with [`Installed`].
static OWN: Mutex<Option<HashMap<String, tempfile::TempPath>>> = Mutex::new(None);

/// Which password an `ssh` call offers. `Debug` is written by hand so a `{:?}` of a value that
/// holds the password prints a placeholder.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source<'a> {
    /// The one the command line gave, if it gave one.
    Run,
    /// The one a host variable gave. An empty one offers nothing, as in the reference: the
    /// variable is set, so no other password is tried in its place.
    Variable(&'a str),
}

impl std::fmt::Debug for Source<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Run => f.write_str("Run"),
            Source::Variable(_) => f.write_str("Variable(<redacted>)"),
        }
    }
}

/// Keeps the password files alive. They are removed when this is dropped.
pub struct Installed(
    #[expect(dead_code, reason = "held for its Drop")] Option<tempfile::NamedTempFile>,
);

impl Drop for Installed {
    fn drop(&mut self) {
        OWN.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
}

fn private_file(password: &str) -> std::io::Result<tempfile::NamedTempFile> {
    use std::io::Write as _;
    let mut file = tempfile::Builder::new()
        .prefix("volant-askpass-")
        .tempfile()?;
    file.write_all(password.as_bytes())?;
    file.flush()?;
    Ok(file)
}

/// Stores the connection password of the command line, if there is one, in a file only this
/// user can read, and makes every `ssh` the run starts ask this program for it. The value that
/// comes back has to live to the end of the run: it also removes the files of the passwords
/// that hosts name in a variable.
pub fn install(password: Option<&str>) -> std::io::Result<Installed> {
    let Some(password) = password else {
        return Ok(Installed(None));
    };
    let file = private_file(password)?;
    FILE.set(file.path().to_path_buf())
        .map_err(|_| std::io::Error::other("a connection password is already installed"))?;
    Ok(Installed(Some(file)))
}

/// Whether an `ssh` call with this source has a password to offer.
pub fn offers(source: Source<'_>) -> bool {
    match source {
        Source::Run => FILE.get().is_some(),
        Source::Variable(password) => !password.is_empty(),
    }
}

/// The environment an `ssh` call of this run is started with, empty when it has no password.
pub fn environment(source: Source<'_>) -> std::io::Result<Vec<(&'static str, OsString)>> {
    let file = match source {
        Source::Run => FILE.get().cloned(),
        Source::Variable("") => None,
        Source::Variable(password) => Some(own_file(password)?),
    };
    let (Some(file), Ok(me)) = (file, std::env::current_exe()) else {
        return Ok(Vec::new());
    };
    Ok(vec![
        ("SSH_ASKPASS", me.into_os_string()),
        ("SSH_ASKPASS_REQUIRE", "force".into()),
        (FILE_ENV, file.into_os_string()),
    ])
}

/// The file that holds `password`, made on first use.
fn own_file(password: &str) -> std::io::Result<PathBuf> {
    let mut files = OWN.lock().unwrap_or_else(PoisonError::into_inner);
    let files = files.get_or_insert_with(HashMap::new);
    if let Some(path) = files.get(password) {
        return Ok(path.to_path_buf());
    }
    let path = private_file(password)?.into_temp_path();
    let shown = path.to_path_buf();
    files.insert(password.to_string(), path);
    Ok(shown)
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
