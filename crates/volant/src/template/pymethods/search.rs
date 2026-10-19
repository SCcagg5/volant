// SPDX-License-Identifier: GPL-3.0-or-later
//! `str` methods that look for a text inside another, and those that join or replace.

use minijinja::value::ValueKind;
use minijinja::{Error, Value};

use super::args::{Call, bound, clamp, index, raise, text, type_of};

/// A text read by character position, as Python reads it. Byte offsets stand in for the
/// positions when the text is ASCII.
struct Chars<'a> {
    s: &'a str,
    len: usize,
}

impl<'a> Chars<'a> {
    fn new(s: &'a str) -> Self {
        let len = if s.is_ascii() {
            s.len()
        } else {
            s.chars().count()
        };
        Self { s, len }
    }

    /// The byte offset of the character at `at`.
    fn byte(&self, at: usize) -> usize {
        if self.s.len() == self.len {
            at
        } else {
            self.s
                .char_indices()
                .nth(at)
                .map_or(self.s.len(), |(b, _)| b)
        }
    }

    /// The character position of the byte offset `at`.
    fn pos(&self, at: usize) -> usize {
        if self.s.len() == self.len {
            at
        } else {
            self.s[..at].chars().count()
        }
    }

    fn slice(&self, start: usize, end: usize) -> &'a str {
        &self.s[self.byte(start)..self.byte(end)]
    }
}

/// The window `[start, end)` of a search call, from its optional `start` and `end` arguments.
fn window<'a>(s: &'a str, call: &[Value], from: usize) -> Result<(Chars<'a>, usize, usize), Error> {
    let chars = Chars::new(s);
    let start = bound(call.get(from))?;
    let end = bound(call.get(from + 1))?;
    let (start, end) = clamp(chars.len, start, end);
    Ok((chars, start, end))
}

/// `find`, `rfind`, `index` and `rindex`: `(sub[, start[, end]])`. `None` when not found.
pub(super) fn locate(s: &str, name: &str, call: &Call<'_>) -> Result<Option<usize>, Error> {
    let args = call.between(name, 1, 3)?;
    let sub = text(&args[0], &format!("{name}() argument 1"))?;
    let (chars, start, end) = window(s, args, 1)?;
    if start > chars.len || end < start || end - start < sub.chars().count() {
        return Ok(None);
    }
    let hay = chars.slice(start, end);
    let found = if name.starts_with('r') {
        hay.rfind(sub)
    } else {
        hay.find(sub)
    };
    Ok(found.map(|at| start + Chars::new(hay).pos(at)))
}

/// `find` and `rfind` answer `-1` where `index` and `rindex` raise.
pub(super) fn find(s: &str, name: &str, call: &Call<'_>) -> Result<Value, Error> {
    let found = locate(s, name, call)?;
    match (found, name.ends_with("index")) {
        (Some(at), _) => Ok(Value::from(at)),
        (None, false) => Ok(Value::from(-1)),
        (None, true) => Err(raise("substring not found")),
    }
}

/// `count(sub[, start[, end]])`: occurrences that do not overlap.
pub(super) fn count(s: &str, call: &Call<'_>) -> Result<Value, Error> {
    let args = call.between("count", 1, 3)?;
    let sub = text(&args[0], "count() argument 1")?;
    let (chars, start, end) = window(s, args, 1)?;
    if start > chars.len || end < start {
        return Ok(Value::from(0));
    }
    let hay = chars.slice(start, end);
    Ok(Value::from(if sub.is_empty() {
        end - start + 1
    } else {
        hay.matches(sub).count()
    }))
}

/// `startswith` and `endswith`: `(prefix[, start[, end]])`, the prefix a text or a tuple.
pub(super) fn affix_match(s: &str, name: &str, call: &Call<'_>) -> Result<Value, Error> {
    let args = call.between(name, 1, 3)?;
    let (chars, start, end) = window(s, args, 1)?;
    let at_start = name == "startswith";
    let test = |sub: &str| {
        let n = sub.chars().count();
        if end < start || end - start < n {
            return false;
        }
        let part = if at_start {
            chars.slice(start, start + n)
        } else {
            chars.slice(end - n, end)
        };
        part == sub
    };
    let first = &args[0];
    if let Some(sub) = first.as_str() {
        return Ok(Value::from(test(sub)));
    }
    if first.kind() == ValueKind::Seq {
        for item in first.try_iter()? {
            let sub = item.as_str().ok_or_else(|| {
                raise(format!(
                    "tuple for {name} must only contain str, not {}",
                    type_of(&item)
                ))
            })?;
            if test(sub) {
                return Ok(Value::from(true));
            }
        }
        return Ok(Value::from(false));
    }
    Err(raise(format!(
        "{name} first arg must be str or a tuple of str, not {}",
        type_of(first)
    )))
}

/// `replace(old, new, count=-1)`.
pub(super) fn replace(s: &str, call: &Call<'_>) -> Result<Value, Error> {
    if call.pos.len() < 2 {
        return Err(raise(format!(
            "replace() takes at least 2 positional arguments ({} given)",
            call.pos.len()
        )));
    }
    let slots = call.named("replace", &["old", "new", "count"])?;
    let old = text(slots[0].unwrap_or(&call.pos[0]), "replace() argument 1")?;
    let new = text(slots[1].unwrap_or(&call.pos[1]), "replace() argument 2")?;
    let count = slots[2].map_or(Ok(-1), index)?;
    Ok(Value::from(match usize::try_from(count) {
        Ok(n) => s.replacen(old, new, n),
        Err(_) => s.replace(old, new),
    }))
}

/// `join(iterable)`: every item must be a text.
pub(super) fn join(s: &str, call: &Call<'_>) -> Result<Value, Error> {
    let items = call.one("join")?;
    let iter = match items.kind() {
        ValueKind::Seq | ValueKind::Map | ValueKind::Iterable | ValueKind::String => {
            items.try_iter()?
        }
        _ => return Err(raise("can only join an iterable")),
    };
    let mut out = String::new();
    for (at, item) in iter.enumerate() {
        let Some(part) = item.as_str() else {
            return Err(raise(format!(
                "sequence item {at}: expected str instance, {} found",
                type_of(&item)
            )));
        };
        if at > 0 {
            out.push_str(s);
        }
        out.push_str(part);
    }
    Ok(Value::from(out))
}

/// `partition` and `rpartition`: the text before, the separator and the text after.
pub(super) fn partition(s: &str, name: &str, call: &Call<'_>) -> Result<Value, Error> {
    let arg = call.one(name)?;
    let sep = arg
        .as_str()
        .ok_or_else(|| raise(format!("must be str, not {}", type_of(arg))))?
        .to_string();
    if sep.is_empty() {
        return Err(raise("empty separator"));
    }
    let found = if name == "partition" {
        s.find(&sep)
    } else {
        s.rfind(&sep)
    };
    let parts = match found {
        Some(at) => [&s[..at], &sep, &s[at + sep.len()..]].map(str::to_string),
        None if name == "partition" => [s.to_string(), String::new(), String::new()],
        None => [String::new(), String::new(), s.to_string()],
    };
    Ok(Value::from(
        parts.into_iter().map(Value::from).collect::<Vec<_>>(),
    ))
}
