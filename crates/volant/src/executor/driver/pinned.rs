// SPDX-License-Identifier: GPL-3.0-or-later
//! The place a `host_pinned` host holds in the queue for forks.

use tokio::sync::watch;

/// A `host_pinned` host's place in the queue for forks. Dropping it hands the place to the next
/// host, so a host that leaves before it queued - interrupted, or panicked - holds nobody up.
pub(super) struct Turn<'a> {
    pub(super) gate: &'a watch::Sender<usize>,
    pub(super) mine: usize,
    pub(super) passed: bool,
}

impl Turn<'_> {
    pub(super) fn pass(&mut self) {
        if !std::mem::replace(&mut self.passed, true) {
            let next = self.mine + 1;
            self.gate
                .send_modify(|reached| *reached = (*reached).max(next));
        }
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.pass();
    }
}
