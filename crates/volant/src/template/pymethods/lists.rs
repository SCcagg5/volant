// SPDX-License-Identifier: GPL-3.0-or-later
//! The methods of Python's `list`.

use std::cmp::Ordering;

use minijinja::value::ValueKind;
use minijinja::{Error, Value};

use super::args::{Call, bound, clamp, index, iterate, raise, type_of};
use super::format::repr;
use super::mutable::{MutList, snapshot};

fn none() -> Result<Value, Error> {
    Ok(Value::from(()))
}

/// Runs a method of `list` on `list`. `None` when `name` is no method of a list.
pub(super) fn call(me: &MutList, name: &str, args: &[Value]) -> Option<Result<Value, Error>> {
    let call = Call::of("list", args);
    Some(match name {
        "append" => call.one(name).and_then(|v| {
            let item = snapshot(v);
            me.with(|items| items.push(item));
            none()
        }),
        "extend" => call.one(name).and_then(|v| {
            let more: Vec<Value> = iterate(v)?.iter().map(snapshot).collect();
            me.with(|items| items.extend(more));
            none()
        }),
        "insert" => call.between(name, 2, 2).and_then(|a| {
            let at = index(&a[0])?;
            let item = snapshot(&a[1]);
            me.with(|items| {
                let len = i64::try_from(items.len()).unwrap_or(i64::MAX);
                let at = if at < 0 {
                    (at + len).max(0)
                } else {
                    at.min(len)
                };
                items.insert(usize::try_from(at).unwrap_or(0), item);
            });
            none()
        }),
        "remove" => call.one(name).and_then(|v| {
            me.with(|items| match items.iter().position(|x| x == v) {
                Some(at) => {
                    items.remove(at);
                    none()
                }
                None => Err(raise("list.remove(x): x not in list")),
            })
        }),
        "pop" => call.between(name, 0, 1).and_then(|a| {
            let at = a.first().map_or(Ok(-1), index)?;
            me.with(|items| {
                if items.is_empty() {
                    return Err(raise("pop from empty list"));
                }
                let len = i64::try_from(items.len()).unwrap_or(i64::MAX);
                let at = if at < 0 { at + len } else { at };
                match usize::try_from(at).ok().filter(|&i| i < items.len()) {
                    Some(i) => Ok(items.remove(i)),
                    None => Err(raise("pop index out of range")),
                }
            })
        }),
        "sort" => sort(me, &call),
        "reverse" => call.none(name).map(|()| {
            me.with(|items| items.reverse());
            Value::from(())
        }),
        "clear" => call.none(name).map(|()| {
            me.with(Vec::clear);
            Value::from(())
        }),
        "copy" => call.none(name).map(|()| snapshot(&Value::from(me.items()))),
        "index" => call.between(name, 1, 3).and_then(|a| {
            let items = me.items();
            let (start, end) = clamp(items.len(), bound(a.get(1))?, bound(a.get(2))?);
            items
                .iter()
                .enumerate()
                .take(end)
                .skip(start)
                .find(|(_, x)| *x == &a[0])
                .map(|(at, _)| Value::from(at))
                .ok_or_else(|| raise(format!("{} is not in list", repr(&a[0]))))
        }),
        "count" => call
            .one(name)
            .map(|v| Value::from(me.items().iter().filter(|x| *x == v).count())),
        _ => return None,
    })
}

/// `sort(*, key=None, reverse=False)`.
fn sort(me: &MutList, call: &Call<'_>) -> Result<Value, Error> {
    if !call.pos.is_empty() {
        return Err(raise("sort() takes no positional arguments"));
    }
    let mut reverse = false;
    for (key, value) in &call.kw {
        match key.as_str() {
            "reverse" => reverse = value.is_true(),
            "key" if value.is_none() => {}
            "key" => {
                return Err(raise(if value.kind() == ValueKind::Plain {
                    "a sort key function is not available yet".to_string()
                } else {
                    format!("'{}' object is not callable", type_of(value))
                }));
            }
            other => {
                return Err(raise(format!(
                    "'{other}' is an invalid keyword argument for sort()"
                )));
            }
        }
    }
    let mut items = me.items();
    let mut failure = None;
    items.sort_by(|a, b| {
        compare(a, b).unwrap_or_else(|e| {
            failure.get_or_insert(e);
            Ordering::Equal
        })
    });
    if let Some(e) = failure {
        return Err(e);
    }
    if reverse {
        // Python's reverse sort keeps equal items in their first order.
        items.sort_by(|a, b| compare(b, a).unwrap_or(Ordering::Equal));
    }
    me.with(|current| *current = items);
    none()
}

/// Python's `<` between two values, for those that can be ordered.
fn compare(a: &Value, b: &Value) -> Result<Ordering, Error> {
    let kinds = (a.kind(), b.kind());
    let number = |v: &Value| match v.kind() {
        ValueKind::Bool => Some(f64::from(u8::from(v.is_true()))),
        ValueKind::Number => f64::try_from(v.clone()).ok(),
        _ => None,
    };
    if let (Some(x), Some(y)) = (number(a), number(b)) {
        return x
            .partial_cmp(&y)
            .ok_or_else(|| raise("cannot order a number that is not a number"));
    }
    match kinds {
        (ValueKind::String, ValueKind::String) => Ok(a.as_str().cmp(&b.as_str())),
        (ValueKind::Seq, ValueKind::Seq) => {
            let (x, y) = (iterate(a)?, iterate(b)?);
            for (p, q) in x.iter().zip(&y) {
                match compare(p, q)? {
                    Ordering::Equal => {}
                    other => return Ok(other),
                }
            }
            Ok(x.len().cmp(&y.len()))
        }
        _ => Err(raise(format!(
            "'<' not supported between instances of '{}' and '{}'",
            type_of(b),
            type_of(a)
        ))),
    }
}
