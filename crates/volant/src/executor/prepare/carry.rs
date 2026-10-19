// SPDX-License-Identifier: GPL-3.0-or-later
//! The facts a looped `set_fact` leaves for the items after it.
//!
//! ansible-core's `TaskExecutor._run_loop` lays what each item's result returned in
//! `ansible_facts` over the variables of the next item. Measured on ansible-core 2.19.12:
//! `set_fact: {x: "{{ x + [item] }}"}` over `[1, 2, 3]` ends with `[1, 2, 3]`; the facts of an item
//! that a `when` left out or that failed to render are not carried; a variable built from the fact
//! (`y: "{{ x | length }}"`) renders again with the new value; and the carried fact beats an
//! extra variable and a role parameter for the items of that loop only. The store keeps the old
//! rule: a failed task writes nothing, and the facts land once the task is judged.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::template::Templar;
use crate::vars::HostVars;

use super::Item;
use crate::executor::run::set_fact_names;

/// What the items before the current one set, for one host.
pub(super) struct Carry {
    /// The host's variables before they resolved each other, kept to render again when a variable
    /// is built from a carried name.
    merged: HostVars,
    facts: Map<String, Value>,
    /// The carried names whose value came from a managed host: they stay data.
    data: BTreeSet<String>,
    /// The carried names and the variables built from them, directly or through other
    /// variables: each name is followed once.
    reach: BTreeSet<String>,
    /// A template variable of the host names a carried name.
    derived: bool,
}

impl Carry {
    pub(super) fn new(merged: HostVars) -> Self {
        Self {
            merged,
            facts: Map::new(),
            data: BTreeSet::new(),
            reach: BTreeSet::new(),
            derived: false,
        }
    }

    /// The variables of the next item: `base` when nothing was carried, with the carried facts
    /// laid over it otherwise. When a template variable reads a carried name, the unresolved map
    /// is the start instead, so that the pass that binds the loop variable renders it again.
    pub(super) fn vars(&self, base: &HostVars) -> HostVars {
        if self.facts.is_empty() {
            return base.clone();
        }
        let mut vars = if self.derived {
            self.merged.clone()
        } else {
            base.clone()
        };
        self.lay_over(&mut vars);
        vars
    }

    /// Lays the carried facts over `vars`, each with the trust it was set with.
    pub(super) fn lay_over(&self, vars: &mut HostVars) {
        for (name, value) in &self.facts {
            if self.data.contains(name) {
                vars.insert_untrusted(name.clone(), value.clone());
            } else {
                vars.insert(name.clone(), value.clone());
            }
        }
    }

    /// Adds to `reach` every variable that reads a name already in it, until none is left.
    fn follow(&mut self) {
        loop {
            let found: Vec<String> = self
                .merged
                .map
                .iter()
                .filter(|(key, _)| !self.reach.contains(*key))
                .filter(|(_, value)| self.reach.iter().any(|name| mentions(value, name)))
                .map(|(key, _)| key.clone())
                .collect();
            if found.is_empty() {
                return;
            }
            self.derived = true;
            self.reach.extend(found);
        }
    }

    /// Takes what `item` sets: nothing for an item left out by its `when` or whose keys do not
    /// render. The trust is the one `record_local_facts` writes the same names with.
    pub(super) fn take(&mut self, item: &Item, templar: &Templar) {
        if item.skipped.is_some() {
            return;
        }
        let Ok(names) = set_fact_names(item, templar) else {
            return;
        };
        for (name, value, data) in names {
            if self.reach.insert(name.clone()) {
                self.follow();
            }
            if data {
                self.data.insert(name.clone());
            } else {
                self.data.remove(&name);
            }
            self.facts.insert(name, value.clone());
        }
    }
}

/// Whether a template inside `value` spells `name` as a word. A name inside a longer one does not
/// count; a quoted one does, for `vars['x']`.
fn mentions(value: &Value, name: &str) -> bool {
    match value {
        Value::String(text) => (text.contains("{{") || text.contains("{%")) && spells(text, name),
        Value::Array(items) => items.iter().any(|v| mentions(v, name)),
        Value::Object(map) => map.values().any(|v| mentions(v, name)),
        _ => false,
    }
}

fn spells(text: &str, name: &str) -> bool {
    let word = |c: char| c == '_' || c.is_ascii_alphanumeric();
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::{Carry, mentions};
    use crate::vars::HostVars;

    /// An item starts from the host's resolved variables, and only the variables built from a
    /// carried name go back to their templates: the others are not copied from the unresolved map.
    #[test]
    fn the_variables_an_item_renders_again_are_the_ones_built_from_a_carried_name() {
        let mut merged = HostVars::default();
        merged.map.insert("built".into(), json!("{{ x }}"));
        merged.map.insert("other".into(), json!("{{ 1 + 1 }}"));
        let mut base = HostVars::default();
        base.map.insert("built".into(), json!("old"));
        base.map.insert("other".into(), json!(2));
        let mut carry = Carry::new(merged);
        carry.facts.insert("x".into(), json!(1));
        carry.reach.insert("x".into());
        carry.follow();
        let vars = carry.vars(&base);
        assert_eq!(vars.get("built"), Some(&json!("{{ x }}")));
        assert_eq!(vars.get("other"), Some(&json!(2)));
        assert_eq!(vars.get("x"), Some(&json!(1)));
    }

    /// A variable built from a variable built from a carried name is followed as well.
    #[test]
    fn a_variable_two_hops_from_a_carried_name_is_followed() {
        let mut merged = HostVars::default();
        merged.map.insert("e".into(), json!("{{ [x] }}"));
        merged.map.insert("ee".into(), json!("{{ e | length }}"));
        merged.map.insert("other".into(), json!("{{ y }}"));
        let mut carry = Carry::new(merged);
        carry.reach.insert("x".into());
        carry.follow();
        assert!(carry.derived);
        assert!(carry.reach.contains("ee"));
        assert!(!carry.reach.contains("other"));
    }

    #[rstest]
    #[case::bare("{{ x | length }}", "x", true)]
    #[case::quoted("{{ vars['x'] }}", "x", true)]
    #[case::longer_name("{{ max }}", "x", false)]
    #[case::suffix("{{ x_list }}", "x", false)]
    #[case::attribute("{{ a.x }}", "x", true)]
    #[case::plain_text("x", "x", false)]
    #[case::nested(json!({"k": ["{{ x }}"]}), "x", true)]
    fn a_template_mentions_a_name_only_as_a_word(
        #[case] value: impl Into<serde_json::Value>,
        #[case] name: &str,
        #[case] found: bool,
    ) {
        assert_eq!(mentions(&value.into(), name), found);
    }
}
