// SPDX-License-Identifier: GPL-3.0-or-later
//! What a play's `hosts` renders against: the play, and no host.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Map, Value};

use super::load::extend;
use super::{Scope, VarStore};
use crate::template::Vars;

/// The variables a render with no host reads. Owned, because the render borrows all of it at once.
#[derive(Debug)]
pub(crate) struct PlayView {
    pub vars: Map<String, Value>,
    /// `groups` and the three host lists.
    pub shared: Arc<Map<String, Value>>,
    /// Every host's view, once the run has a play behind it.
    pub hostvars: Option<Arc<Map<String, Value>>>,
    pub untrusted_hosts: Arc<BTreeSet<String>>,
}

impl PlayView {
    pub fn vars(&self) -> Vars<'_> {
        Vars {
            map: &self.vars,
            hostvars: self.hostvars.as_ref(),
            shared: Some(&self.shared),
            facts: None,
            untrusted: None,
            untrusted_hosts: Some(&self.untrusted_hosts),
            base: None,
        }
    }
}

impl VarStore {
    /// The view the reference builds with a play and no host, which is where it renders a play's
    /// `hosts`. Lowest first: a role's defaults, the play's variables, its `vars_files`, what the
    /// roles export, the extra variables, then the magic variables that need no host.
    ///
    /// A host's inventory variables, its `group_vars`, its facts and its registered results are
    /// not in it, and neither is `inventory_hostname`. `scope` names the host lists
    /// `ansible_play_hosts_all` and the others answer with, which are every host the run could
    /// reach while the pattern is not yet known. `hostvars` is there only when `with_hostvars`
    /// says a play has run: the reference builds it when its first play starts.
    pub fn play_view(&mut self, scope: &Scope, with_hostvars: bool) -> PlayView {
        let mut vars = Map::new();
        extend(&mut vars, &scope.role_defaults);
        extend(&mut vars, &scope.play_vars);
        for file in &scope.vars_files {
            extend(&mut vars, file);
        }
        extend(&mut vars, &scope.role_vars);
        extend(&mut vars, &self.extra);
        extend(&mut vars, &self.play_magic());
        PlayView {
            vars,
            shared: self.shared_values(scope),
            hostvars: with_hostvars.then(|| self.hostvars_base()),
            untrusted_hosts: self.untrusted_hosts(),
        }
    }
}
