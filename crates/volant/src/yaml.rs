// SPDX-License-Identifier: GPL-3.0-or-later
//! YAML loading shared by playbooks, variable files and inventories, with the PyYAML habits
//! Ansible content relies on (`yes`/`no` booleans, leading-zero and underscore-grouped
//! integers) and the tags Ansible adds (`!vault`, `!unsafe`).

use std::fmt::Write;

use anyhow::Context;
use saphyr::{Mapping, MarkedYaml, Scalar, Yaml, YamlLoader};
use serde_json::Value;

mod limits;
mod walk;

use walk::{lower, mapping_addresses, mapping_starts};

/// The deepest nesting a YAML text may have, for the code that must read a value back.
pub(crate) const MAX_DEPTH: usize = limits::MAX_DEPTH;

/// Loads every document of `text`. Errors name `source`.
///
/// A key written twice in one mapping keeps its last value, as PyYAML does for ansible-core
/// (which only warns). saphyr refuses it unless told otherwise.
pub fn load<'a>(text: &'a str, source: &str) -> anyhow::Result<Vec<Yaml<'a>>> {
    let docs = marked(text, source)?;
    if holds_nothing(text) {
        return Ok(docs.iter().map(|_| Yaml::Value(Scalar::Null)).collect());
    }
    Ok(docs.into_iter().map(|doc| lower(doc, text)).collect())
}

/// Every document of `text` with saphyr's positions, a key written twice keeping its last value.
fn marked<'a>(text: &'a str, source: &str) -> anyhow::Result<Vec<MarkedYaml<'a>>> {
    let mut loader = YamlLoader::<MarkedYaml>::default();
    loader.allow_duplicate_keys(true);
    limits::feed(text, &mut loader)
        .and_then(|()| loader.error().cloned().map_or(Ok(()), Err))
        .with_context(|| format!("{source}: invalid YAML"))?;
    Ok(loader.into_documents())
}

/// Where each mapping of a loaded document starts, as `(line, column)`, both counted from 1:
/// the position ansible-core's `Origin` gives a task. Keyed by the mapping's address in the
/// documents [`load_with_origins`] returned, which stay where they are while nothing changes them.
///
/// The values of every mapping entry are kept the same way, keyed by the value node's address:
/// the reference points a keyword's error at the value (`until: x` at the `x`).
#[derive(Debug, Default)]
pub struct Origins {
    maps: std::collections::HashMap<usize, (usize, usize)>,
    values: std::collections::HashMap<usize, (usize, usize)>,
    /// Where the key of each entry starts, by the address of its value.
    keys: std::collections::HashMap<usize, (usize, usize)>,
}

impl Origins {
    /// Where `map` starts, when it is a mapping of the documents these origins came with.
    pub fn of(&self, map: &Mapping<'_>) -> Option<(usize, usize)> {
        self.maps.get(&std::ptr::from_ref(map).addr()).copied()
    }

    /// Where `value` starts, when it is the value of a mapping entry of those documents.
    pub fn of_value(&self, value: &Yaml<'_>) -> Option<(usize, usize)> {
        self.values.get(&std::ptr::from_ref(value).addr()).copied()
    }

    /// Where the key of `value`'s entry starts, when it is the value of a mapping entry of
    /// those documents: the reference points a task's action at its key (`debug:`).
    pub fn of_key(&self, value: &Yaml<'_>) -> Option<(usize, usize)> {
        self.keys.get(&std::ptr::from_ref(value).addr()).copied()
    }
}

/// [`load`], and where each mapping of the documents starts.
pub fn load_with_origins<'a>(
    text: &'a str,
    source: &str,
) -> anyhow::Result<(Vec<Yaml<'a>>, Origins)> {
    let marked = marked(text, source)?;
    if holds_nothing(text) {
        return Ok((
            marked.iter().map(|_| Yaml::Value(Scalar::Null)).collect(),
            Origins::default(),
        ));
    }
    // The mappings in the order a walk meets them, on both trees: the marked one gives the
    // positions, the lowered one the addresses.
    let mut starts = (Vec::new(), Vec::new(), Vec::new());
    for doc in &marked {
        mapping_starts(doc, &mut starts);
    }
    let docs: Vec<Yaml<'a>> = marked.into_iter().map(|doc| lower(doc, text)).collect();
    let mut addresses = (Vec::new(), Vec::new());
    for doc in &docs {
        mapping_addresses(doc, &mut addresses);
    }
    // The two walks meet the same nodes in the same order. Should they ever disagree, no
    // position is better than a wrong one: an error block then shows no `Origin:` line.
    if starts.0.len() != addresses.0.len() || starts.1.len() != addresses.1.len() {
        return Ok((docs, Origins::default()));
    }
    Ok((
        docs,
        Origins {
            maps: addresses.0.into_iter().zip(starts.0).collect(),
            keys: addresses.1.iter().copied().zip(starts.2).collect(),
            values: addresses.1.into_iter().zip(starts.1).collect(),
        },
    ))
}

type Starts = (
    Vec<(usize, usize)>,
    Vec<(usize, usize)>,
    Vec<(usize, usize)>,
);

/// Whether every line of `text` is blank, a comment or a document marker. Such a stream holds
/// only empty documents, which PyYAML reads as null; saphyr hands each back as an empty string.
/// Measured on ansible-core 2.19.12, a vars file holding only `---` and a comment loads as empty.
///
/// Decided from the text rather than from the node: saphyr gives an empty plain scalar, an empty
/// block scalar (`a: |`) and an empty `!!str` the same empty string over a zero-width span, and
/// PyYAML reads the last two as `''`. A stream with no line of content holds neither.
fn holds_nothing(text: &str) -> bool {
    text.lines().all(|line| {
        let line = line.trim();
        let line = line
            .strip_prefix("---")
            .or_else(|| line.strip_prefix("..."))
            .map_or(line, str::trim_start);
        line.is_empty() || line.starts_with('#')
    })
}

/// `text`'s byte offset for the `chars`-th character in it: saphyr's markers count characters,
/// while `str` slicing needs bytes.
fn char_to_byte(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

/// Every spelling PyYAML reads as `true`. The single source for both the scalar resolver
/// below and [`bool_from_str`]; the `bool` filter's set is a different one and lives with the
/// filter (see [`crate::template`]).
pub const PYYAML_TRUE: &[&str] = &[
    "true", "True", "TRUE", "yes", "Yes", "YES", "on", "On", "ON",
];

/// Every spelling PyYAML reads as `false`, the counterpart of [`PYYAML_TRUE`].
pub const PYYAML_FALSE: &[&str] = &[
    "false", "False", "FALSE", "no", "No", "NO", "off", "Off", "OFF",
];

/// Re-resolves a scalar the way ansible-core's PyYAML (YAML 1.1) would. `raw` is the node's
/// exact source text; quoting it, or writing it as a block scalar, makes `raw` start with
/// something other than the bare word or digits, so none of the spellings below match and
/// `scalar` is returned untouched — matching PyYAML, which also only resolves *plain* scalars,
/// so a deliberately quoted `"yes"` stays the string a playbook wrote.
///
/// The last arm is the general form of what used to be a list of one-off exceptions: whenever
/// PyYAML's own resolvers refuse a plain scalar that saphyr's YAML 1.2 core schema typed as a
/// number, PyYAML leaves a string, so we do too. Measured against ansible-core 2.19.12, that
/// covers `08` (no valid octal digit), `0o10` (the `0o` prefix is YAML 1.2 only) and `1e3` or
/// `1e+3` (PyYAML's float needs a `.` in the mantissa).
fn pyyaml_scalar<'a>(scalar: Scalar<'a>, raw: &'a str) -> Scalar<'a> {
    if PYYAML_TRUE.contains(&raw) {
        return Scalar::Boolean(true);
    }
    if PYYAML_FALSE.contains(&raw) {
        return Scalar::Boolean(false);
    }
    if let Some(i) = pyyaml_int(raw) {
        return Scalar::Integer(i);
    }
    if let Some(f) = pyyaml_float(raw) {
        return Scalar::FloatingPoint(f.into());
    }
    if matches!(scalar, Scalar::Integer(_) | Scalar::FloatingPoint(_)) {
        return Scalar::String(raw.into());
    }
    scalar
}

/// PyYAML's YAML 1.1 integer spellings: an optional sign, then `0b` binary, `0x` hexadecimal, a
/// bare leading `0` for octal, or a decimal starting `1`-`9` (or the single digit `0`). Every
/// digit run may carry `_` separators. `0o10` is deliberately absent: that prefix arrived with
/// YAML 1.2 and PyYAML leaves it a string.
fn pyyaml_int(raw: &str) -> Option<i64> {
    let (negative, rest) = split_sign(raw);
    let value = if let Some(digits) = rest.strip_prefix("0b") {
        digit_run(digits, 2, |b| matches!(b, b'0' | b'1'))?
    } else if let Some(digits) = rest.strip_prefix("0x") {
        digit_run(digits, 16, |b| b.is_ascii_hexdigit())?
    } else if rest == "0" {
        0
    } else if let Some(digits) = rest.strip_prefix('0') {
        digit_run(digits, 8, |b| matches!(b, b'0'..=b'7'))?
    } else if matches!(rest.as_bytes().first(), Some(b'1'..=b'9')) {
        digit_run(rest, 10, |b| b.is_ascii_digit())?
    } else {
        return None;
    };
    Some(if negative { -value } else { value })
}

/// One run of `radix` digits, `_` separators allowed anywhere in it, as an `i64`. A run that is
/// empty, holds a digit `ok` refuses, or overflows is no match at all: the caller then falls
/// back to what saphyr resolved.
fn digit_run(digits: &str, radix: u32, ok: impl Fn(u8) -> bool) -> Option<i64> {
    if digits.is_empty() || !digits.bytes().all(|b| ok(b) || b == b'_') {
        return None;
    }
    i64::from_str_radix(&digits.replace('_', ""), radix).ok()
}

/// PyYAML's YAML 1.1 float spellings: `.inf`, `.nan`, or a mantissa that must contain a `.`,
/// optionally followed by an exponent whose sign PyYAML **requires**. Measured against
/// ansible-core 2.19.12: `1.0e+3` is the float 1000.0 while `1.0e3`, `1e3` and `1e+3` are all
/// strings there.
fn pyyaml_float(raw: &str) -> Option<f64> {
    let (negative, rest) = split_sign(raw);
    if matches!(rest, ".nan" | ".NaN" | ".NAN") {
        // PyYAML's not-a-number spelling carries no sign.
        return (!negative).then_some(f64::NAN);
    }
    let value = if matches!(rest, ".inf" | ".Inf" | ".INF") {
        f64::INFINITY
    } else {
        let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
            Some((mantissa, exponent)) => (mantissa, Some(exponent)),
            None => (rest, None),
        };
        let (whole, fraction) = mantissa.split_once('.')?;
        let digits = |run: &str| run.bytes().all(|b| b.is_ascii_digit() || b == b'_');
        match whole.as_bytes().first() {
            // `.5`: nothing before the dot means at least one digit after it.
            None if fraction.is_empty() || !digits(fraction) => return None,
            Some(b'0'..=b'9') if !digits(whole) || !digits(fraction) => return None,
            None | Some(b'0'..=b'9') => {}
            _ => return None,
        }
        if let Some(exponent) = exponent {
            let signed = exponent.strip_prefix('+').or(exponent.strip_prefix('-'))?;
            if signed.is_empty() || !signed.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
        }
        rest.replace('_', "").parse().ok()?
    };
    Some(if negative { -value } else { value })
}

/// Splits an optional leading `+`/`-` off `raw`, reporting whether it was `-`.
fn split_sign(raw: &str) -> (bool, &str) {
    match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw.strip_prefix('+').unwrap_or(raw)),
    }
}

/// The value under `key` when `node` is a mapping, `None` otherwise or when absent.
pub fn field<'a>(node: &'a Yaml<'a>, key: &str) -> Option<&'a Yaml<'a>> {
    node.as_mapping_get(key)
}

/// PyYAML's boolean spelling, which Ansible content uses for keywords like `ignore_errors: yes`.
/// `load` already resolves an unquoted `yes`/`no`/`on`/`off` to `Scalar::Boolean` (matching
/// PyYAML's own reading of a variable's value), so the string arm below only ever fires for a
/// *quoted* spelling: a keyword field like `gather_facts: "yes"` is still a boolean to Ansible,
/// even though the same quoting keeps a generic data value a string in `to_json`.
///
/// `0` and `1` count too. Measured on ansible-core 2.19.12: `gather_facts: 1` gathers facts and
/// `gather_facts: 0` does not, while `gather_facts: 2` refuses the load with `The value 2 could
/// not be converted to 'bool'.` Any other integer is not a boolean here either.
pub fn as_bool(node: &Yaml) -> Option<bool> {
    match node {
        Yaml::Value(Scalar::Boolean(b)) => Some(*b),
        Yaml::Value(Scalar::String(s)) => bool_from_str(s.as_ref()),
        Yaml::Value(Scalar::Integer(0)) => Some(false),
        Yaml::Value(Scalar::Integer(1)) => Some(true),
        _ => None,
    }
}

/// The spellings above, on their own, for the places that read a boolean out of plain text
/// rather than out of a YAML node: `ansible.cfg` keys and the environment variables that
/// override them, which Ansible reads with the same set of words.
pub fn bool_from_str(text: &str) -> Option<bool> {
    if PYYAML_TRUE.contains(&text) {
        return Some(true);
    }
    PYYAML_FALSE.contains(&text).then_some(false)
}

/// A string under `!unsafe` is data, never a template. Where it holds template syntax it is
/// stored as a one-expression template that renders back to its own text through
/// `volant_unsafe`, which also stops the render from going over the result again. Every string
/// leaf of a list or mapping is treated the same way; mapping keys are never rendered.
pub(crate) fn unsafe_data(mut value: Value) -> Value {
    let mut open = vec![&mut value];
    while let Some(node) = open.pop() {
        match node {
            Value::String(s) if s.contains("{{") || s.contains("{%") => {
                *s = wrap(s, "volant_unsafe");
            }
            Value::Array(items) => open.extend(items.iter_mut()),
            Value::Object(map) => open.extend(map.values_mut()),
            _ => {}
        }
    }
    value
}

/// `text` as data that is never rendered, whatever it holds.
pub(crate) fn unsafe_string(text: &str) -> String {
    wrap(text, "volant_unsafe")
}

/// `text` as the one-expression template `{{ "<text>" | <filter> }}`.
fn wrap(text: &str, filter: &str) -> String {
    let mut lit = String::from("{{ \"");
    for c in text.chars() {
        match c {
            '\\' => lit.push_str(r"\\"),
            '"' => lit.push_str(r#"\""#),
            '\n' => lit.push_str(r"\n"),
            '\r' => lit.push_str(r"\r"),
            '\t' => lit.push_str(r"\t"),
            c if c.is_control() => _ = write!(lit, "\\u{:04x}", c as u32),
            c => lit.push(c),
        }
    }
    lit.push_str("\" | ");
    lit.push_str(filter);
    lit.push_str(" }}");
    lit
}

/// The text `unsafe_data` wrapped, when `s` is exactly such a wrapper and nothing else. Read
/// back by hand, never through the templater: `s` may be any string of a variable map.
pub(crate) fn unsafe_text(s: &str) -> Option<String> {
    unwrapped(s, "volant_unsafe")
}

/// What a `!vault` value loads as: its ciphertext in a wrapper that decrypts when the value is
/// rendered, so a value nothing reads is never decrypted. The result is data, as `!unsafe` is:
/// measured on ansible-core 2.19.12, `{{ x }}` inside a decrypted value is not rendered.
pub(crate) fn vault_data(ciphertext: &str) -> String {
    wrap(ciphertext, "volant_vault")
}

/// The ciphertext `vault_data` wrapped, read back by hand like [`unsafe_text`].
pub(crate) fn vault_text(s: &str) -> Option<String> {
    unwrapped(s, "volant_vault")
}

fn unwrapped(s: &str, filter: &str) -> Option<String> {
    let body = s
        .strip_prefix("{{ \"")?
        .strip_suffix(format!("\" | {filter} }}}}").as_str())?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return None,
            '\\' => match chars.next()? {
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                _ => return None,
            },
            c => out.push(c),
        }
    }
    Some(out)
}

/// Converts a node to JSON. `!vault` and `!unsafe` load as data, see `vault_data` and
/// `unsafe_data`.
pub fn to_json(node: &Yaml) -> anyhow::Result<Value> {
    walk::to_json(node)
}

#[cfg(test)]
mod tests;
