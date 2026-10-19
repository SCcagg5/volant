// SPDX-License-Identifier: GPL-3.0-or-later
//! The methods of Python's `list`.

use std::cmp::Ordering;

use minijinja::value::ValueKind;
use minijinja::{Error, Value};

use super::args::{Call, bound, clamp, index, iterate, raise, type_of};
use super::format::repr;
use super::limits::check_items;
use super::mutable::{MutList, snapshot};

fn none() -> Result<Value, Error> {
    Ok(Value::from(()))
}

/// `Ok` when `me` can take `extra` more items.
fn room(me: &MutList, extra: usize) -> Result<(), Error> {
    check_items(me.read(Vec::len) as u128 + extra as u128)
}

/// Runs a method of `list` on `list`. `None` when `name` is no method of a list.
pub(super) fn call(me: &MutList, name: &str, args: &[Value]) -> Option<Result<Value, Error>> {
    let call = Call::of("list", args);
    Some(match name {
        "append" => call.one(name).and_then(|v| {
            room(me, 1)?;
            let item = snapshot(v);
            me.with(|items| items.push(item));
            none()
        }),
        "extend" => call.one(name).and_then(|v| {
            let more = iterate(v)?;
            room(me, more.len())?;
            let more: Vec<Value> = more.iter().map(snapshot).collect();
            me.with(|items| items.extend(more));
            none()
        }),
        "insert" => call.between(name, 2, 2).and_then(|a| {
            let at = index(&a[0])?;
            room(me, 1)?;
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
            // The item is copied first: comparing it with a list that is itself locked waits for ever.
            let v = snapshot(v);
            match me.read(|items| items.iter().position(|x| *x == v)) {
                Some(at) => {
                    me.with(|items| items.remove(at));
                    none()
                }
                None => Err(raise("list.remove(x): x not in list")),
            }
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
        "copy" => call.none(name).map(|()| me.copy_of()),
        "index" => call.between(name, 1, 3).and_then(|a| {
            let (start, end) = (bound(a.get(1))?, bound(a.get(2))?);
            let item = snapshot(&a[0]);
            me.read(|items| {
                let (start, end) = clamp(items.len(), start, end);
                items
                    .iter()
                    .enumerate()
                    .take(end)
                    .skip(start)
                    .find(|(_, x)| **x == item)
                    .map(|(at, _)| Value::from(at))
                    .ok_or_else(|| raise(format!("{} is not in list", repr(&item))))
            })
        }),
        "count" => call.one(name).map(|v| {
            let item = snapshot(v);
            Value::from(me.read(|items| items.iter().filter(|x| **x == item).count()))
        }),
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
    // Python's reverse sort keeps equal items in their first order, so the comparison is swapped
    // and the sort stays stable.
    let items = merge_sort(me.items().to_vec(), &|a, b| {
        if reverse {
            compare(b, a)
        } else {
            compare(a, b)
        }
    })?;
    me.with(|current| *current = items);
    none()
}

/// A stable sort by a comparison that can fail. `slice::sort_by` may panic when the comparison
/// is not a total order, and a list of mixed types is not one.
fn merge_sort(
    items: Vec<Value>,
    cmp: &dyn Fn(&Value, &Value) -> Result<Ordering, Error>,
) -> Result<Vec<Value>, Error> {
    if items.len() < 2 {
        return Ok(items);
    }
    let mut right = items;
    let left = right.drain(..right.len() / 2).collect::<Vec<_>>();
    let (left, right) = (merge_sort(left, cmp)?, merge_sort(right, cmp)?);
    let mut out = Vec::with_capacity(left.len() + right.len());
    let (mut l, mut r) = (left.into_iter().peekable(), right.into_iter().peekable());
    while let (Some(a), Some(b)) = (l.peek(), r.peek()) {
        // The right item goes first only when it is strictly smaller, which keeps equal items in
        // their order.
        let next = if cmp(b, a)? == Ordering::Less {
            &mut r
        } else {
            &mut l
        };
        out.extend(next.next());
    }
    out.extend(l);
    out.extend(r);
    Ok(out)
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
