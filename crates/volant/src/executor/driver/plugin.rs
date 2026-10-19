// SPDX-License-Identifier: GPL-3.0-or-later
//! The batch a task backed by an action plugin makes.

use crate::action_plugins::Kind;
use crate::vars::HostVars;

/// The one step of a batch an action plugin backs, or the controller error that ends the host
/// when the batch holds anything else: the plugin branch runs one step, so a second one would be
/// reported by nobody and never run.
pub(super) fn plugin_step<T>(batch: &[(usize, T)]) -> Result<&(usize, T), String> {
    match batch {
        [only] => Ok(only),
        _ => Err(format!(
            "a task backed by an action plugin has to be alone in its batch, and this one holds steps {:?}",
            batch.iter().map(|(index, _)| index).collect::<Vec<_>>()
        )),
    }
}

/// What a batch whose one task an action plugin backs needs to run it.
pub(super) struct PluginBatch {
    pub(super) kind: Kind,
    /// The interpreter the host the module runs on asked for, read off its variables.
    pub(super) asked: Option<String>,
    /// That host's name and variables, when it is a delegate.
    pub(super) delegate: Option<(String, HostVars)>,
}
