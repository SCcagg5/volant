// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use crate::executor::Abort;

/// Ctrl-C, SIGTERM and SIGHUP all request a clean stop: running batches are cancelled, the
/// recap is printed, and the process exits with ansible-playbook's code 99. A hang-up is what a
/// closed terminal sends; it ends the run the way the other two do, so what the run holds, the
/// password file among it, is released.
pub(super) fn spawn_signal_watcher(stop: Arc<Abort>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        // Registered before the task first runs, so a signal that comes at once is not lost to
        // the default action.
        let (Ok(mut int), Ok(mut term), Ok(mut hup)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            return;
        };
        tokio::spawn(async move {
            tokio::select! {
                _ = int.recv() => {}
                _ = term.recv() => {}
                _ = hup.recv() => {}
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
