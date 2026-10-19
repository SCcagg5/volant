// SPDX-License-Identifier: GPL-3.0-or-later
//! The Jinja globals that build a value: `dict`.
//!
//! minijinja's `dict` takes a mapping and keywords only. Python's `dict`, which Jinja hands to
//! templates unchanged, also takes any iterable of pairs, and Ansible roles rely on it
//! (`dict(items)`, `dict(keys | zip(values))`). Its failures are worded as CPython words them.

use std::collections::HashMap;

use minijinja::value::{Rest, ValueKind};
use minijinja::{Error, ErrorKind, Value};

use super::type_name;

/// What makes two keys the same key in a Python `dict`: `1`, `1.0` and `true` are one key.
#[derive(Hash, PartialEq, Eq)]
enum Key {
    None,
    Int(i64),
    Float(u64),
    Text(String),
}

/// A `dict` under construction: the keys in the order they were first set, the value last set.
#[derive(Default)]
struct Entries {
    items: Vec<(Value, Value)>,
    index: HashMap<Key, usize>,
}

impl Entries {
    fn set(&mut self, key: Value, value: Value) -> Result<(), Error> {
        let canonical = match key.kind() {
            ValueKind::None => Key::None,
            ValueKind::Bool | ValueKind::Number => match key.as_i64() {
                Some(whole) => Key::Int(whole),
                None => Key::Float(f64::try_from(key.clone()).map_or(0, f64::to_bits)),
            },
            ValueKind::String => Key::Text(key.to_string()),
            ValueKind::Undefined => return Err(Error::from(ErrorKind::UndefinedError)),
            _ => {
                return Err(invalid(format!("unhashable type: '{}'", python_type(&key))));
            }
        };
        if let Some(&at) = self.index.get(&canonical) {
            self.items[at].1 = value;
        } else {
            self.index.insert(canonical, self.items.len());
            self.items.push((key, value));
        }
        Ok(())
    }
}

fn invalid(message: String) -> Error {
    Error::new(ErrorKind::InvalidOperation, message)
}

/// The name Python gives the type of a value, as the messages of `dict` quote it.
fn python_type(value: &Value) -> &'static str {
    match value.kind() {
        ValueKind::None => "NoneType",
        ValueKind::Bool => "bool",
        ValueKind::Number => serde_json::to_value(value).map_or("int", |json| type_name(&json)),
        ValueKind::String => "str",
        ValueKind::Bytes => "bytes",
        ValueKind::Seq => "list",
        ValueKind::Map => "dict",
        ValueKind::Iterable => "generator",
        _ => "object",
    }
}

/// The items of a sequence, a generator or a string, in order; for a mapping, its keys.
fn items_of(value: &Value) -> Result<Vec<Value>, Error> {
    if value.kind() == ValueKind::String {
        return Ok(value
            .to_string()
            .chars()
            .map(|c| Value::from(c.to_string()))
            .collect());
    }
    Ok(value.try_iter()?.collect())
}

/// `dict(...)`: nothing, a mapping, or any iterable of two-item elements, then keywords.
///
/// Measured on ansible-core 2.19.12: a later pair wins over an earlier one with the same key,
/// a keyword wins over the positional argument, `dict(none)` and `dict(5)` fail as non-iterable
/// and a string is read character by character.
pub(super) fn dict(args: Rest<Value>) -> Result<Value, Error> {
    let mut args = args.0;
    let keywords = match args.last() {
        Some(last) if last.is_kwargs() => args.pop(),
        _ => None,
    };
    if args.len() > 1 {
        return Err(invalid(format!(
            "dict expected at most 1 argument, got {}",
            args.len()
        )));
    }
    let mut entries = Entries::default();
    if let Some(source) = args.first() {
        match source.kind() {
            ValueKind::Undefined => return Err(Error::from(ErrorKind::UndefinedError)),
            ValueKind::Map => {
                for key in source.try_iter()? {
                    let value = source.get_item(&key)?;
                    entries.set(key, value)?;
                }
            }
            ValueKind::Seq | ValueKind::Iterable | ValueKind::String => {
                for (at, element) in items_of(source)?.iter().enumerate() {
                    let (key, value) = pair(at, element)?;
                    entries.set(key, value)?;
                }
            }
            _ => {
                return Err(invalid(format!(
                    "'{}' object is not iterable",
                    python_type(source)
                )));
            }
        }
    }
    if let Some(keywords) = keywords {
        for key in keywords.try_iter()? {
            let value = keywords.get_item(&key)?;
            entries.set(key, value)?;
        }
    }
    Ok(entries.items.into_iter().collect())
}

/// The two items of element number `at`.
fn pair(at: usize, element: &Value) -> Result<(Value, Value), Error> {
    if element.is_undefined() {
        return Err(Error::from(ErrorKind::UndefinedError));
    }
    let items = match element.kind() {
        ValueKind::Seq | ValueKind::Iterable | ValueKind::String | ValueKind::Map => {
            items_of(element)?
        }
        _ => {
            return Err(invalid(format!(
                "cannot convert dictionary update sequence element #{at} to a sequence"
            )));
        }
    };
    let [key, value] = <[Value; 2]>::try_from(items).map_err(|items| {
        invalid(format!(
            "dictionary update sequence element #{at} has length {}; 2 is required",
            items.len()
        ))
    })?;
    Ok((key, value))
}
