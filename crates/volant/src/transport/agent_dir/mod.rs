// SPDX-License-Identifier: GPL-3.0-or-later
//! The directory that holds the cached agent on a host, and the shell that checks it.
//!
//! The directory is `<remote_tmp>/volant-agent-<version>-<uid>`, with the numeric id of the user
//! the agent runs as. A host can share one `remote_tmp` between the login user and a `become`
//! user, and a user must never run a program that another user owns or can write to. A name
//! per user keeps two users from meeting in one directory, and the checks below keep a third
//! user, who made the directory first, from being trusted.
//!
//! Every command that runs the agent, or puts it in place, runs [`CHECKS`] on the host first. A
//! path is accepted only when it is not a symbolic link, belongs to the user the command runs
//! as, and has no group or other write bit. `remote_tmp` itself is accepted only when other
//! users cannot rename an entry in it: it belongs to that user, to root, or to the user whom
//! `sudo` reports in `SUDO_UID` (the login user, who can already act as root), and it is closed
//! to group and other writes or has the sticky bit.
//!
//! The checks use `ls -ldn`, which POSIX specifies and GNU, BusyBox and BSD `ls` all print the
//! same way, so no `stat` option that a host may lack is needed.

use super::bootstrap::EXIT_AGENT_DIR_UNSAFE;
use super::defaults::SshTarget;
use super::shell::shell_word;

/// Shell functions, defined once per command line.
///
/// `st` reads the mode and the owner of a path into `m` and `o`, with the path not followed when
/// it is a link, because `ls -l` shows the link itself. `fit TYPE PATH` succeeds when the path
/// has the type (`d` or `-`), belongs to `$u` and has no group or other write bit. `shared DIR`
/// is the same test for `remote_tmp`, which may instead be sticky. `say` names a refused path.
/// `verify` checks `remote_tmp`, the directory and the program, the way every start does.
/// `prepare` makes the directory for an upload. `sweep` removes the other versions' directories
/// that belong to this user.
const CHECKS: &str = concat!(
    r#"st() { set -f; set -- $(ls -ldn -- "$1" 2>/dev/null); set +f; m=$1; o=$3; }; "#,
    r#"fit() { st "$2"; case "$m" in "$1"????[!w]??[!w]*) [ "$o" = "$u" ]; return;; esac; return 1; }; "#,
    r#"shared() { st "$1/"; case "$m" in d????[!w]??[!w]*|d????????[tT]*) [ "$o" = "$u" ] || [ "$o" = 0 ] || [ "$o" = "${SUDO_UID:-x}" ]; return;; esac; return 1; }; "#,
    r#"say() { printf '%s\n' "$1: permissions ${m:-unknown}, owner uid ${o:-unknown}: $2" >&2; }; "#,
    r#"bad_tmp() { say "$rt" "other users could replace the agent directory in it; it must belong to uid $u, to root or to the user who ran sudo, and be closed to group and other writes or have the sticky bit"; }; "#,
    r#"bad_dir() { say "$d" "it must be a real directory of uid $u with no group or other write bit"; }; "#,
    r#"bad_file() { say "$a" "it must be a regular file of uid $u with no group or other write bit"; }; "#,
    r#"verify() { shared "$rt" || { bad_tmp; return 1; }; fit d "$d" || { bad_dir; return 1; }; fit - "$a" || { bad_file; return 1; }; }; "#,
    r#"prepare() { mkdir -p "$rt" || return 1; shared "$rt" || { bad_tmp; return @UNSAFE@; }; "#,
    r#"if [ -L "$d" ] || [ -e "$d" ]; then if ! fit d "$d"; then "#,
    r#"case "$m" in d*) [ "$o" = "$u" ] || { bad_dir; return @UNSAFE@; };; *) bad_dir; return @UNSAFE@;; esac; "#,
    r#"rm -f "$d/volant-agent" "$d"/volant-agent.tmp.* "$d/agent.stderr"; "#,
    r#"rmdir "$d" 2>/dev/null || { bad_dir; return @UNSAFE@; }; mkdir "$d" || return 1; fi; "#,
    r#"else mkdir "$d" || return 1; fi; fit d "$d" || { bad_dir; return @UNSAFE@; }; }; "#,
    r#"sweep() { for old in "$rt"/volant-agent-*; do [ "$old" = "$d" ] && continue; st "$old"; "#,
    r#"case "$m" in d*) [ "$o" = "$u" ] && rm -rf "$old";; esac; done; return 0; }; "#,
);

impl SshTarget {
    /// The assignments every agent command starts with: the id of the user it runs as, `remote_tmp`
    /// (`rt`), the directory (`d`) and the program (`a`). The tilde of a `remote_tmp` is expanded
    /// by the shell that runs this, which is the target user's own.
    pub(super) fn agent_vars(&self) -> String {
        format!(
            "u=$(id -u) || exit 1; rt={}; d=\"$rt/volant-agent-{}-$u\"; a=\"$d/volant-agent\"; ",
            shell_word(&self.remote_tmp),
            env!("CARGO_PKG_VERSION")
        )
    }

    /// The functions, for a command that checks or makes the directory.
    pub(super) fn agent_checks(&self) -> String {
        format!(
            "{}{}",
            self.agent_vars(),
            CHECKS.replace("@UNSAFE@", &EXIT_AGENT_DIR_UNSAFE.to_string())
        )
    }

    /// The agent's directory for a message: the numeric id is only known on the host.
    pub(super) fn dir_hint(&self) -> String {
        format!(
            "{}/volant-agent-{}-<uid>",
            self.remote_tmp,
            env!("CARGO_PKG_VERSION")
        )
    }

    /// Starts the agent, once `remote_tmp`, the directory and the program have passed the checks
    /// again. The probe checked them earlier, in another `ssh` session, so this closes the gap
    /// of a run that took its time to upload; what is left is the time between the last check
    /// and `exec`. `redirect` is appended to the `exec` command, for the stderr of a link
    /// through `su`.
    pub(super) fn exec_command(&self, redirect: &str) -> String {
        format!(
            "{}verify || exit {EXIT_AGENT_DIR_UNSAFE}; exec \"$a\"{redirect}",
            self.agent_checks()
        )
    }
}

#[cfg(test)]
mod tests;
