// SPDX-License-Identifier: GPL-3.0-or-later
//! The variables a play's `hosts` reads, settled before the pattern renders.
//!
//! The reference resolves a variable when a template reads it, so a chain of variables that name
//! each other settles however long it is, a ring is refused, and the text a template builds is
//! never rendered again. This engine renders a result again when it still holds a template, which
//! cannot tell the two apart. So the variables the pattern reads, and the ones those read, are
//! settled here, each to a fixpoint, and the pattern then renders once.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::template::{Templar, Vars};
use crate::vars::PlayView;

/// The deepest chain of variables that settles. The reference stops a little short of a hundred,
/// where Python's own recursion limit ends it.
const DEPTH: usize = 100;

const RING: &str = "Recursive loop detected in template.";

/// `{{ ... }}` and `{% ... %}`, the parts of a string that read variables.
static SEGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\{\{.*?\}\}|\{%.*?%\}").expect("a valid pattern"));
static QUOTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"'[^']*'|"[^"]*""#).expect("a valid pattern"));
static NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("a valid pattern"));

/// The names `value` reads. `code` holds the ones written as variables; `text` adds the names
/// that only appear inside a string, which `lookup('vars', 'name')` reads and a ring never goes
/// through.
fn reads(value: &Value, code: &mut Vec<String>, text: &mut Vec<String>) {
    match value {
        Value::String(s) if Templar::is_template(s) => {
            for segment in SEGMENT.find_iter(s) {
                let segment = segment.as_str();
                let bare = QUOTED.replace_all(segment, " ");
                for found in NAME.find_iter(&bare) {
                    // An attribute (`a.b`) or a filter (`| b`) is not a variable.
                    let before = bare[..found.start()].trim_end().chars().next_back();
                    if !matches!(before, Some('.' | '|')) {
                        code.push(found.as_str().to_string());
                    }
                }
                for quoted in QUOTED.find_iter(segment) {
                    let inner = &quoted.as_str()[1..quoted.as_str().len() - 1];
                    text.extend(NAME.find_iter(inner).map(|n| n.as_str().to_string()));
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| reads(v, code, text)),
        Value::Object(map) => map.values().for_each(|v| reads(v, code, text)),
        _ => {}
    }
}

fn names(value: &Value) -> (Vec<String>, Vec<String>) {
    let (mut code, mut text) = (Vec::new(), Vec::new());
    reads(value, &mut code, &mut text);
    (code, text)
}

/// Whether following the variables named in `start` comes back to one of them, or goes deeper
/// than [`DEPTH`].
fn rings(start: &[String], vars: &Map<String, Value>) -> bool {
    fn visit(name: &str, vars: &Map<String, Value>, path: &mut Vec<String>) -> bool {
        let Some(value) = vars.get(name) else {
            return false;
        };
        if path.iter().any(|p| p == name) || path.len() >= DEPTH {
            return true;
        }
        path.push(name.to_string());
        let found = names(value).0.iter().any(|dep| visit(dep, vars, path));
        path.pop();
        found
    }
    start.iter().any(|name| visit(name, vars, &mut Vec::new()))
}

/// The variables of `vars` that `start` reads, through each other.
fn closure(start: &[String], vars: &Map<String, Value>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut todo: Vec<String> = start.to_vec();
    while let Some(name) = todo.pop() {
        let Some(value) = vars.get(&name) else {
            continue;
        };
        if seen.insert(name) {
            let (code, text) = names(value);
            todo.extend(code.into_iter().chain(text));
        }
    }
    seen
}

/// `view`'s variables with the ones `patterns` read settled, and the names that are data.
///
/// # Errors
/// The message of a ring, of a chain too deep, or of a variable that has no value.
pub(super) fn settle(
    patterns: &[String],
    templar: &Templar,
    view: &PlayView,
) -> Result<(Map<String, Value>, BTreeSet<String>), String> {
    let (mut code, mut text) = (Vec::new(), Vec::new());
    for pattern in patterns {
        let (c, t) = names(&Value::String(pattern.clone()));
        code.extend(c);
        text.extend(t);
    }
    if rings(&code, &view.vars) {
        return Err(RING.to_string());
    }
    let start: Vec<String> = code.into_iter().chain(text).collect();
    let mut part: Map<String, Value> = closure(&start, &view.vars)
        .into_iter()
        .filter_map(|name| view.vars.get(&name).map(|v| (name, v.clone())))
        .collect();
    let mut data = view.untrusted.clone();
    let over = |part: &Map<String, Value>| {
        let mut all = view.vars.clone();
        all.extend(part.clone());
        all
    };
    let mut settled = false;
    for _ in 0..DEPTH {
        let base = over(&part);
        let (next, now) = templar.resolve_vars_tainted(view.vars_over(&base, &data));
        data = now;
        let next: Map<String, Value> = part
            .keys()
            .filter_map(|k| next.get(k).map(|v| (k.clone(), v.clone())))
            .collect();
        if next == part {
            settled = true;
            break;
        }
        part = next;
    }
    if !settled {
        return Err(RING.to_string());
    }
    let all = over(&part);
    // What is still a template did not render in the passes above, which leave a failure for the
    // place the value is read. This is that place: an undefined variable in a chain is refused.
    for (name, value) in &part {
        if !data.contains(name) {
            check(templar, value, &view.vars_over(&all, &data))?;
        }
    }
    Ok((all, data))
}

fn check(templar: &Templar, value: &Value, vars: &Vars<'_>) -> Result<(), String> {
    match value {
        Value::String(s) if Templar::is_template(s) => templar
            .render_one_pass_tainted(s, *vars)
            .map(|_| ())
            .map_err(|err| err.0),
        Value::Array(items) => items.iter().try_for_each(|v| check(templar, v, vars)),
        Value::Object(map) => map.values().try_for_each(|v| check(templar, v, vars)),
        _ => Ok(()),
    }
}
