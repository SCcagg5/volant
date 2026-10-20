// SPDX-License-Identifier: GPL-3.0-or-later
//! Where the controller keeps its own files and where it looks for Ansible's, for each platform.
//!
//! Every reader of `HOME`, `XDG_*` and the system directory of Ansible goes through this module,
//! so a platform has one answer for each place. On Unix the places are those of ansible-core.
//! ansible-core does not run on Windows, so the Windows places are Volant's own rule, written on
//! the installation and configuration pages.
//!
//! Each place is a function of the platform and of the environment, and the functions without
//! those two arguments read the running process. That lets a test check the Windows rule on any
//! host.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The platform whose rule decides a place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Unix,
    Macos,
    Windows,
}

/// The platform this controller was built for.
pub(crate) const HOST: Platform = if cfg!(windows) {
    Platform::Windows
} else if cfg!(target_os = "macos") {
    Platform::Macos
} else {
    Platform::Unix
};

/// The separator of the entries in a setting that holds a list of paths: `os.pathsep`, which is
/// `;` on Windows because `C:\x:D:\y` is not a list.
pub(crate) const LIST_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// How an environment variable is read.
pub(crate) type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn process_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The value of `name` when it is set and not empty.
fn set(env: Env<'_>, name: &str) -> Option<PathBuf> {
    env(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The value of `name` when it names an absolute directory: a relative one would put a cache in
/// whatever directory the run started in.
fn absolute(env: Env<'_>, name: &str) -> Option<PathBuf> {
    env(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The home directory of the user: `HOME` on Unix; on Windows `USERPROFILE`, then `HOMEDRIVE`
/// and `HOMEPATH` together. An empty value counts as unset.
pub(crate) fn home() -> Option<PathBuf> {
    home_of(HOST, &process_env)
}

pub(crate) fn home_of(platform: Platform, env: Env<'_>) -> Option<PathBuf> {
    if platform != Platform::Windows {
        return set(env, "HOME");
    }
    set(env, "USERPROFILE").or_else(|| {
        let drive = env("HOMEDRIVE").filter(|v| !v.is_empty())?;
        let path = env("HOMEPATH").filter(|v| !v.is_empty())?;
        let mut joined = drive;
        joined.push(path);
        Some(PathBuf::from(joined))
    })
}

/// The directory of the controller's caches: `$XDG_CACHE_HOME/volant`, else `~/.cache/volant`,
/// and on Windows `%LOCALAPPDATA%\volant\cache`. `None` when no variable names an absolute
/// directory.
pub(crate) fn cache_root() -> Option<PathBuf> {
    cache_root_of(HOST, &process_env)
}

pub(crate) fn cache_root_of(platform: Platform, env: Env<'_>) -> Option<PathBuf> {
    if platform == Platform::Windows {
        return absolute(env, "LOCALAPPDATA").map(|dir| dir.join("volant").join("cache"));
    }
    absolute(env, "XDG_CACHE_HOME")
        .or_else(|| absolute(env, "HOME").map(|home| home.join(".cache")))
        .map(|base| base.join("volant"))
}

/// The variables [`cache_root`] reads, as the message that says there is no cache directory
/// names them.
pub(crate) fn cache_variables() -> &'static str {
    if HOST == Platform::Windows {
        "LOCALAPPDATA"
    } else {
        "neither XDG_CACHE_HOME nor HOME"
    }
}

/// The directory of the state the controller keeps: `$XDG_DATA_HOME/volant`, else
/// `~/.local/share/volant`, `~/Library/Application Support/volant` on macOS and
/// `%LOCALAPPDATA%\volant` on Windows. Relative values are ignored, as for the cache.
pub(crate) fn data_root() -> Option<PathBuf> {
    data_root_of(HOST, &process_env)
}

pub(crate) fn data_root_of(platform: Platform, env: Env<'_>) -> Option<PathBuf> {
    let base = match platform {
        Platform::Windows => absolute(env, "LOCALAPPDATA"),
        Platform::Macos => {
            absolute(env, "HOME").map(|home| home.join("Library").join("Application Support"))
        }
        Platform::Unix => absolute(env, "XDG_DATA_HOME")
            .or_else(|| absolute(env, "HOME").map(|home| home.join(".local/share"))),
    };
    base.map(|base| base.join("volant"))
}

/// The variables [`data_root`] reads, as the message that says there is no data directory names
/// them, and the one to set.
pub(crate) fn data_variables() -> (&'static str, &'static str) {
    if HOST == Platform::Windows {
        ("LOCALAPPDATA", "LOCALAPPDATA")
    } else {
        ("neither XDG_DATA_HOME nor HOME", "HOME")
    }
}

/// The system directory of Ansible's files: `/etc/ansible`, and `%PROGRAMDATA%\ansible` on
/// Windows, where `PROGRAMDATA` falls back to `C:\ProgramData` when the environment has no
/// absolute value.
pub(crate) fn system_dir() -> PathBuf {
    system_dir_of(HOST, &process_env)
}

pub(crate) fn system_dir_of(platform: Platform, env: Env<'_>) -> PathBuf {
    if platform == Platform::Windows {
        absolute(env, "PROGRAMDATA")
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join("ansible")
    } else {
        PathBuf::from("/etc/ansible")
    }
}

/// The system `ansible.cfg`.
pub(crate) fn system_config() -> PathBuf {
    system_dir().join("ansible.cfg")
}

/// The inventory the reference reads when none is given.
pub(crate) fn default_inventory() -> PathBuf {
    system_dir().join("hosts")
}

/// The system directories that hold `kind` (`roles` or `collections`), the shared one first:
/// `/usr/share/ansible/<kind>` and `/etc/ansible/<kind>` on Unix, `%PROGRAMDATA%\ansible\<kind>`
/// on Windows.
pub(crate) fn system_content_dirs(kind: &str) -> Vec<PathBuf> {
    system_content_dirs_of(HOST, &process_env, kind)
}

pub(crate) fn system_content_dirs_of(platform: Platform, env: Env<'_>, kind: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if platform != Platform::Windows {
        dirs.push(Path::new("/usr/share/ansible").join(kind));
    }
    dirs.push(system_dir_of(platform, env).join(kind));
    dirs
}

/// What follows a leading `~` in `path` when it names the home directory: the empty text, or
/// the rest after a separator (`/`, and `\` on Windows) with the separators dropped. `None` for
/// any other path, including `~user`.
pub(crate) fn after_tilde(path: &str) -> Option<&str> {
    after_tilde_on(HOST, path)
}

pub(crate) fn after_tilde_on(platform: Platform, path: &str) -> Option<&str> {
    let rest = path.strip_prefix('~')?;
    let is_separator = |c: char| c == '/' || (platform == Platform::Windows && c == '\\');
    (rest.is_empty() || rest.starts_with(is_separator))
        .then(|| rest.trim_start_matches(is_separator))
}

#[cfg(test)]
mod tests;
