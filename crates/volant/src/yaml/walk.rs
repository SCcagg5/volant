// SPDX-License-Identifier: GPL-3.0-or-later
//! The walks over a loaded document: dropping the source positions, listing where each mapping
//! starts, and converting to JSON.
//!
//! None of them calls itself. Each keeps its own stack of the nodes it has open, so the stack of
//! the thread holds one small frame however deep the document goes: a recursive walk took about
//! 3.6 KB of stack for each level in a debug build, which let 489 levels fill a 2 MiB thread.

use std::borrow::Cow;

use anyhow::{anyhow, bail};
use saphyr::{Mapping, MarkedYaml, Scalar, Tag, Yaml, YamlData};
use serde_json::{Map, Value};

use super::{Starts, char_to_byte, pyyaml_scalar, unsafe_data, vault_data};

/// A collection of the source tree whose children are being lowered.
#[expect(
    clippy::large_enum_variant,
    reason = "the frames live in a heap vector"
)]
enum Lowering<'a> {
    Sequence {
        items: std::vec::IntoIter<MarkedYaml<'a>>,
        out: Vec<Yaml<'a>>,
    },
    Mapping {
        entries: std::vec::IntoIter<(MarkedYaml<'a>, MarkedYaml<'a>)>,
        /// The value of the entry whose key is being lowered.
        value: Option<MarkedYaml<'a>>,
        /// The key of the entry whose value is being lowered.
        key: Option<Yaml<'a>>,
        out: Mapping<'a>,
    },
    Tagged(Cow<'a, Tag>),
}

/// Drops the source positions `MarkedYaml` tracks once loading is done, applying PyYAML's
/// scalar resolution along the way where it's confirmed to diverge from saphyr's YAML 1.2 core
/// schema (see `pyyaml_scalar`). Every other function in this module keeps working on the
/// plain `Yaml` tree it already expected; only `load` needs to know this happened.
pub(super) fn lower<'a>(root: MarkedYaml<'a>, text: &'a str) -> Yaml<'a> {
    let mut open: Vec<Lowering<'a>> = Vec::new();
    let mut next = Some(root);
    let mut done: Option<Yaml<'a>> = None;
    loop {
        if let Some(node) = next.take() {
            match node.data {
                YamlData::Value(scalar) => {
                    let start = char_to_byte(text, node.span.start.index());
                    let end = char_to_byte(text, node.span.end.index());
                    let raw = text.get(start..end).unwrap_or_default();
                    done = Some(Yaml::Value(pyyaml_scalar(scalar, raw)));
                }
                YamlData::Sequence(items) => open.push(Lowering::Sequence {
                    out: Vec::with_capacity(items.len()),
                    items: items.into_iter(),
                }),
                YamlData::Mapping(map) => open.push(Lowering::Mapping {
                    entries: map.into_iter().collect::<Vec<_>>().into_iter(),
                    value: None,
                    key: None,
                    out: Mapping::new(),
                }),
                YamlData::Tagged(tag, inner) => {
                    open.push(Lowering::Tagged(tag));
                    next = Some(*inner);
                    continue;
                }
                YamlData::Alias(id) => done = Some(Yaml::Alias(id)),
                YamlData::Representation(v, style, tag) => {
                    done = Some(Yaml::Representation(v, style, tag));
                }
                YamlData::BadValue => done = Some(Yaml::BadValue),
            }
        }
        if let Some(child) = done.take() {
            match open.last_mut() {
                None => return child,
                Some(Lowering::Sequence { out, .. }) => out.push(child),
                Some(Lowering::Mapping {
                    value, key, out, ..
                }) => {
                    if value.is_some() {
                        *key = Some(child);
                    } else if let Some(key) = key.take() {
                        out.insert(key, child);
                    }
                }
                Some(Lowering::Tagged(_)) => {
                    if let Some(Lowering::Tagged(tag)) = open.pop() {
                        done = Some(Yaml::Tagged(tag, Box::new(child)));
                    }
                    continue;
                }
            }
        }
        match open.last_mut() {
            Some(Lowering::Sequence { items, out }) => match items.next() {
                Some(item) => next = Some(item),
                None => done = Some(Yaml::Sequence(std::mem::take(out))),
            },
            Some(Lowering::Mapping {
                entries,
                value,
                out,
                ..
            }) => match value.take() {
                Some(value) => next = Some(value),
                None => match entries.next() {
                    Some((key, entry)) => {
                        *value = Some(entry);
                        next = Some(key);
                    }
                    None => done = Some(Yaml::Mapping(std::mem::take(out))),
                },
            },
            _ => {}
        }
        if done.is_some() {
            open.pop();
        }
    }
}

/// Where each mapping of the source tree starts, and each key and value of it, in the order a
/// walk down the tree meets them.
pub(super) fn mapping_starts(root: &MarkedYaml<'_>, out: &mut Starts) {
    enum Work<'t, 'a> {
        Node(&'t MarkedYaml<'a>),
        Entry(&'t MarkedYaml<'a>, &'t MarkedYaml<'a>),
    }
    let at = |n: &MarkedYaml<'_>| (n.span.start.line(), n.span.start.col() + 1);
    let mut work = vec![Work::Node(root)];
    while let Some(item) = work.pop() {
        match item {
            Work::Node(node) => match &node.data {
                YamlData::Sequence(items) => work.extend(items.iter().rev().map(Work::Node)),
                YamlData::Mapping(map) => {
                    out.0.push(at(node));
                    let entries: Vec<_> = map.iter().map(|(k, v)| Work::Entry(k, v)).collect();
                    work.extend(entries.into_iter().rev());
                }
                YamlData::Tagged(_, inner) => work.push(Work::Node(inner)),
                _ => {}
            },
            Work::Entry(key, value) => {
                out.1.push(at(value));
                out.2.push(at(key));
                work.push(Work::Node(value));
                work.push(Work::Node(key));
            }
        }
    }
}

/// The address of each mapping of the lowered tree, and of each value in it, in the order
/// [`mapping_starts`] lists the positions.
pub(super) fn mapping_addresses(root: &Yaml<'_>, out: &mut (Vec<usize>, Vec<usize>)) {
    enum Work<'t, 'a> {
        Node(&'t Yaml<'a>),
        Entry(&'t Yaml<'a>, &'t Yaml<'a>),
    }
    let mut work = vec![Work::Node(root)];
    while let Some(item) = work.pop() {
        match item {
            Work::Node(node) => match node {
                Yaml::Sequence(items) => work.extend(items.iter().rev().map(Work::Node)),
                Yaml::Mapping(map) => {
                    out.0.push(std::ptr::from_ref(map).addr());
                    let entries: Vec<_> = map.iter().map(|(k, v)| Work::Entry(k, v)).collect();
                    work.extend(entries.into_iter().rev());
                }
                Yaml::Tagged(_, inner) => work.push(Work::Node(inner)),
                _ => {}
            },
            Work::Entry(key, value) => {
                out.1.push(std::ptr::from_ref(value).addr());
                work.push(Work::Node(value));
                work.push(Work::Node(key));
            }
        }
    }
}

/// A collection of the lowered tree whose children are being converted.
enum Converting<'t, 'a> {
    Array {
        items: std::slice::Iter<'t, Yaml<'a>>,
        out: Vec<Value>,
    },
    Object {
        entries: std::vec::IntoIter<(&'t Yaml<'a>, &'t Yaml<'a>)>,
        key: Option<String>,
        out: Map<String, Value>,
    },
    /// A `!unsafe`, and whether it is the outermost: it wraps the strings below it, once.
    Unsafe { outermost: bool },
}

/// Converts a node to JSON. `!vault` and `!unsafe` load as data, see `vault_data` and
/// `unsafe_data`. A `!unsafe` inside a `!unsafe` wraps nothing more, as wrapping again at each
/// level doubled every string.
pub(super) fn to_json(root: &Yaml) -> anyhow::Result<Value> {
    let mut open: Vec<Converting<'_, '_>> = Vec::new();
    let mut unsafe_levels = 0usize;
    let mut next = Some(root);
    let mut done: Option<Value> = None;
    loop {
        if let Some(node) = next.take() {
            match node {
                Yaml::Value(Scalar::Null) => done = Some(Value::Null),
                Yaml::Value(Scalar::Boolean(b)) => done = Some(Value::Bool(*b)),
                Yaml::Value(Scalar::Integer(i)) => done = Some(Value::from(*i)),
                Yaml::Value(Scalar::FloatingPoint(f)) => {
                    done = Some(
                        serde_json::Number::from_f64(f.into_inner())
                            .map_or_else(|| Value::String(f.to_string()), Value::Number),
                    );
                }
                Yaml::Value(Scalar::String(s)) => done = Some(Value::String(s.to_string())),
                Yaml::Sequence(items) => open.push(Converting::Array {
                    items: items.iter(),
                    out: Vec::with_capacity(items.len()),
                }),
                Yaml::Mapping(map) => open.push(Converting::Object {
                    entries: map.iter().collect::<Vec<_>>().into_iter(),
                    key: None,
                    out: Map::new(),
                }),
                Yaml::Tagged(tag, inner) if tag.handle == "!" && tag.suffix == "vault" => {
                    match &**inner {
                        Yaml::Value(Scalar::String(s)) => {
                            done = Some(Value::String(vault_data(s)));
                        }
                        _ => bail!("!vault takes a string"),
                    }
                }
                Yaml::Tagged(tag, inner) if tag.handle == "!" && tag.suffix == "unsafe" => {
                    open.push(Converting::Unsafe {
                        outermost: unsafe_levels == 0,
                    });
                    unsafe_levels += 1;
                    next = Some(&**inner);
                    continue;
                }
                Yaml::Tagged(tag, _) => bail!("unsupported YAML tag {}{}", tag.handle, tag.suffix),
                Yaml::Alias(_) => bail!("YAML aliases are not supported yet"),
                Yaml::Representation(..) => bail!("unresolved YAML scalar"),
                Yaml::BadValue => bail!("invalid YAML value"),
            }
        }
        if let Some(child) = done.take() {
            match open.last_mut() {
                None => return Ok(child),
                Some(Converting::Array { out, .. }) => out.push(child),
                Some(Converting::Object { key, out, .. }) => {
                    if let Some(key) = key.take() {
                        out.insert(key, child);
                    }
                }
                Some(Converting::Unsafe { .. }) => {
                    if let Some(Converting::Unsafe { outermost }) = open.pop() {
                        unsafe_levels -= 1;
                        done = Some(if outermost { unsafe_data(child) } else { child });
                    }
                    continue;
                }
            }
        }
        match open.last_mut() {
            Some(Converting::Array { items, out }) => match items.next() {
                Some(item) => next = Some(item),
                None => done = Some(Value::Array(std::mem::take(out))),
            },
            Some(Converting::Object { entries, key, out }) => match entries.next() {
                Some((k, v)) => {
                    *key = Some(
                        k.as_str()
                            .ok_or_else(|| anyhow!("mapping keys must be strings"))?
                            .to_string(),
                    );
                    next = Some(v);
                }
                None => done = Some(Value::Object(std::mem::take(out))),
            },
            _ => {}
        }
        if done.is_some() {
            open.pop();
        }
    }
}
