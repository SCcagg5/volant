// SPDX-License-Identifier: GPL-3.0-or-later
//! `str` methods that cut, trim, pad and change the case of a text.

use minijinja::{Error, Value};

use super::args::{Call, index, is_space, raise, type_of};

/// `split` and `rsplit`: `(sep=None, maxsplit=-1)`.
pub(super) fn split(
    s: &str,
    name: &str,
    call: &Call<'_>,
    from_right: bool,
) -> Result<Value, Error> {
    let slots = call.named(name, &["sep", "maxsplit"])?;
    let sep = match slots[0] {
        None => None,
        Some(v) if v.is_none() => None,
        Some(v) => Some(
            v.as_str()
                .ok_or_else(|| raise(format!("must be str or None, not {}", type_of(v))))?,
        ),
    };
    let max = slots[1].map_or(Ok(-1), index)?;
    let parts: Vec<String> = match sep {
        Some("") => return Err(raise("empty separator")),
        Some(sep) => split_on(s, sep, max, from_right),
        None => split_white(s, max, from_right),
    };
    Ok(Value::from(
        parts.into_iter().map(Value::from).collect::<Vec<_>>(),
    ))
}

fn split_on(s: &str, sep: &str, max: i64, from_right: bool) -> Vec<String> {
    let Ok(limit) = usize::try_from(max) else {
        return s.split(sep).map(str::to_string).collect();
    };
    if from_right {
        let mut parts: Vec<String> = s.rsplitn(limit + 1, sep).map(str::to_string).collect();
        parts.reverse();
        parts
    } else {
        s.splitn(limit + 1, sep).map(str::to_string).collect()
    }
}

/// Splitting on runs of white space: the ends are trimmed away, and what a limit leaves
/// unsplit keeps its own white space on the far side.
fn split_white(s: &str, max: i64, from_right: bool) -> Vec<String> {
    let mut left = max;
    let mut out = Vec::new();
    if from_right {
        let mut rest = s.trim_end_matches(is_space);
        while !rest.is_empty() {
            if left == 0 {
                out.push(rest.to_string());
                rest = "";
                break;
            }
            let cut = rest.rfind(is_space).map_or(0, |at| {
                at + rest[at..].chars().next().map_or(1, char::len_utf8)
            });
            out.push(rest[cut..].to_string());
            rest = rest[..cut].trim_end_matches(is_space);
            left -= 1;
        }
        let _ = rest;
        out.reverse();
    } else {
        let mut rest = s.trim_start_matches(is_space);
        while !rest.is_empty() {
            if left == 0 {
                out.push(rest.to_string());
                break;
            }
            let cut = rest.find(is_space).unwrap_or(rest.len());
            out.push(rest[..cut].to_string());
            rest = rest[cut..].trim_start_matches(is_space);
            left -= 1;
        }
    }
    out
}

/// `strip`, `lstrip` and `rstrip`: `(chars=None)`.
pub(super) fn strip(s: &str, name: &str, call: &Call<'_>) -> Result<Value, Error> {
    let args = call.between(name, 0, 1)?;
    let (left, right) = match name {
        "strip" => (true, true),
        "lstrip" => (true, false),
        _ => (false, true),
    };
    let chars = match args.first() {
        None => None,
        Some(v) if v.is_none() => None,
        Some(v) => Some(
            v.as_str()
                .ok_or_else(|| raise(format!("{name} arg must be None or str")))?,
        ),
    };
    let keep = |c: char| chars.map_or_else(|| !is_space(c), |set| !set.contains(c));
    let mut out = s;
    if left {
        out = out.trim_start_matches(|c| !keep(c));
    }
    if right {
        out = out.trim_end_matches(|c| !keep(c));
    }
    Ok(Value::from(out))
}

/// `center`, `ljust` and `rjust`: `(width, fillchar=' ')`.
pub(super) fn pad(s: &str, name: &str, call: &Call<'_>) -> Result<Value, Error> {
    let args = call.between(name, 1, 2)?;
    let width = index(&args[0])?;
    let fill = match args.get(1) {
        None => ' ',
        Some(v) => {
            let t = v.as_str().ok_or_else(|| {
                raise(format!(
                    "The fill character must be a unicode character, not {}",
                    type_of(v)
                ))
            })?;
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => c,
                _ => {
                    return Err(raise(
                        "The fill character must be exactly one character long",
                    ));
                }
            }
        }
    };
    let len = i64::try_from(s.chars().count()).unwrap_or(i64::MAX);
    if width <= len {
        return Ok(Value::from(s));
    }
    let margin = width - len;
    let left = match name {
        "ljust" => 0,
        "rjust" => margin,
        _ => margin / 2 + (margin & width & 1),
    };
    let fill_of =
        |n: i64| std::iter::repeat_n(fill, usize::try_from(n).unwrap_or(0)).collect::<String>();
    Ok(Value::from(format!(
        "{}{s}{}",
        fill_of(left),
        fill_of(margin - left)
    )))
}

/// `zfill(width)`: zeros after a leading sign.
pub(super) fn zfill(s: &str, call: &Call<'_>) -> Result<Value, Error> {
    let width = index(call.one("zfill")?)?;
    let len = i64::try_from(s.chars().count()).unwrap_or(i64::MAX);
    if width <= len {
        return Ok(Value::from(s));
    }
    let zeros = "0".repeat(usize::try_from(width - len).unwrap_or(0));
    Ok(Value::from(match s.chars().next() {
        Some(sign @ ('+' | '-')) => format!("{sign}{zeros}{}", &s[1..]),
        _ => format!("{zeros}{s}"),
    }))
}

/// `splitlines(keepends=False)`: lines end at the breaks of `str.splitlines`.
pub(super) fn splitlines(s: &str, call: &Call<'_>) -> Result<Value, Error> {
    let slots = call.named("splitlines", &["keepends"])?;
    let keep = slots[0].is_some_and(Value::is_true);
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if !matches!(
            c,
            '\n' | '\r'
                | '\u{b}'
                | '\u{c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            continue;
        }
        let mut end = at + c.len_utf8();
        if c == '\r' && chars.peek().is_some_and(|&(_, next)| next == '\n') {
            chars.next();
            end += 1;
        }
        lines.push(&s[start..if keep { end } else { at }]);
        start = end;
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    Ok(Value::from(
        lines.into_iter().map(Value::from).collect::<Vec<_>>(),
    ))
}

/// `expandtabs(tabsize=8)`.
pub(super) fn expandtabs(s: &str, call: &Call<'_>) -> Result<Value, Error> {
    let slots = call.named("expandtabs", &["tabsize"])?;
    let size = slots[0].map_or(Ok(8), index)?;
    let size = usize::try_from(size).unwrap_or(0);
    let mut out = String::new();
    let mut column = 0usize;
    for c in s.chars() {
        match c {
            '\t' if size > 0 => {
                let fill = size - column % size;
                out.extend(std::iter::repeat_n(' ', fill));
                column += fill;
            }
            '\t' => {}
            '\n' | '\r' => {
                out.push(c);
                column = 0;
            }
            c => {
                out.push(c);
                column += 1;
            }
        }
    }
    Ok(Value::from(out))
}

/// The title-case form of a character: its upper case, but `ß` gives `Ss`.
fn titlecase(c: char) -> impl Iterator<Item = char> {
    let text: Vec<char> = match c {
        'ß' => vec!['S', 's'],
        'Ǆ' | 'ǆ' => vec!['ǅ'],
        'Ǉ' | 'ǉ' => vec!['ǈ'],
        'Ǌ' | 'ǌ' => vec!['ǋ'],
        'Ǳ' | 'ǳ' => vec!['ǲ'],
        _ => c.to_uppercase().collect(),
    };
    text.into_iter()
}

/// The methods of `str` that take no argument and answer a new text.
pub(super) fn case(s: &str, name: &str) -> Option<String> {
    Some(match name {
        "upper" => s.to_uppercase(),
        "lower" => s.to_lowercase(),
        "casefold" => s.to_lowercase().replace('ß', "ss"),
        "swapcase" => s
            .chars()
            .flat_map(|c| {
                if c.is_uppercase() {
                    c.to_lowercase().collect::<Vec<_>>()
                } else {
                    c.to_uppercase().collect()
                }
            })
            .collect(),
        "capitalize" => {
            let mut chars = s.chars();
            chars.next().map_or_else(String::new, |first| {
                titlecase(first)
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect()
            })
        }
        "title" => {
            let mut after_cased = false;
            let mut out = String::with_capacity(s.len());
            for c in s.chars() {
                if after_cased {
                    out.extend(c.to_lowercase());
                } else {
                    out.extend(titlecase(c));
                }
                after_cased = c.is_lowercase() || c.is_uppercase();
            }
            out
        }
        _ => return None,
    })
}

/// The methods of `str` that take no argument and answer a bool.
pub(super) fn predicate(s: &str, name: &str) -> Option<bool> {
    let all = |f: fn(char) -> bool| !s.is_empty() && s.chars().all(f);
    let cased = |c: char| c.is_lowercase() || c.is_uppercase();
    Some(match name {
        "isdigit" => all(|c| c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric())),
        "isdecimal" => all(|c| c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric())),
        "isnumeric" => all(char::is_numeric),
        "isalpha" => all(char::is_alphabetic),
        "isalnum" => all(char::is_alphanumeric),
        "isspace" => all(is_space),
        "isascii" => s.is_ascii(),
        "islower" => s.chars().any(cased) && !s.chars().any(char::is_uppercase),
        "isupper" => s.chars().any(cased) && !s.chars().any(char::is_lowercase),
        "istitle" => is_title(s),
        "isidentifier" => {
            let mut chars = s.chars();
            chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
                && chars.all(|c| c == '_' || c.is_alphanumeric())
        }
        "isprintable" => s
            .chars()
            .all(|c| !c.is_control() && !is_format_char(c) && (c == ' ' || !c.is_whitespace())),
        _ => return None,
    })
}

/// The common characters of the Unicode category `Cf`, which Python does not call printable.
fn is_format_char(c: char) -> bool {
    matches!(
        u32::from(c),
        0xad | 0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x180e | 0x200b..=0x200f
            | 0x202a..=0x202e | 0x2060..=0x2064 | 0x2066..=0x206f | 0xfeff | 0xfff9..=0xfffb
    )
}

fn is_title(s: &str) -> bool {
    let mut seen = false;
    let mut after_cased = false;
    for c in s.chars() {
        if c.is_uppercase() {
            if after_cased {
                return false;
            }
            after_cased = true;
            seen = true;
        } else if c.is_lowercase() {
            if !after_cased {
                return false;
            }
            seen = true;
        } else {
            after_cased = false;
        }
    }
    seen
}

/// A string argument for `removeprefix` and `removesuffix`.
pub(super) fn affix<'v>(name: &str, value: &'v Value) -> Result<&'v str, Error> {
    let shown = if value.is_none() {
        "None"
    } else {
        type_of(value)
    };
    value
        .as_str()
        .ok_or_else(|| raise(format!("{name}() argument must be str, not {shown}")))
}
