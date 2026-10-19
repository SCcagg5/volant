// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use crate::executor::Abort;

/// Ctrl-C and SIGTERM both request a clean stop: running batches are cancelled, the recap is
/// printed, and the process exits with ansible-playbook's code 99. SIGHUP, which ansible-playbook
/// does not catch, still ends the process at once by the signal itself (code 129, no recap), but
/// only once the connection password's directory is removed. A run started with SIGHUP ignored,
/// as `nohup` does, keeps ignoring it.
pub(super) fn spawn_signal_watcher(stop: Arc<Abort>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        // Registered before the task first runs, so a signal that comes at once is not lost to
        // the default action.
        let (Ok(mut int), Ok(mut term)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
        ) else {
            return;
        };
        let mut hup = if hangup_ignored() {
            None
        } else {
            signal(SignalKind::hangup()).ok()
        };
        tokio::spawn(async move {
            let hung_up = tokio::select! {
                _ = int.recv() => false,
                _ = term.recv() => false,
                _ = async {
                    match hup.as_mut() {
                        Some(hup) => hup.recv().await,
                        None => std::future::pending().await,
                    }
                } => true,
            };
            if hung_up {
                crate::askpass::remove_now();
                // SAFETY: the default action back in place, the signal raised again ends the
                // process the way it would have without the handler.
                unsafe {
                    libc::signal(libc::SIGHUP, libc::SIG_DFL);
                    libc::raise(libc::SIGHUP);
                }
            }
            stop.interrupt();
        });
    }
    #[cfg(not(unix))]
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        stop.interrupt();
    });
}

/// Whether SIGHUP is ignored as the process starts, which is what `nohup` arranges.
#[cfg(unix)]
fn hangup_ignored() -> bool {
    // SAFETY: with a null new action `sigaction` only reads the current one into `old`.
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut old) == 0
            && old.sa_sigaction == libc::SIG_IGN
    }
}
