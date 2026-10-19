// SPDX-License-Identifier: GPL-3.0-or-later
//! The `#jinja2:` first line of a template: `key: value` pairs that replace the options the task
//! gave. Read off `TemplateOverrides._extract_template_overrides` of ansible-core 2.19.12, which
//! splits the line on every comma, splits each pair on its first colon and reads the value with
//! Python's `ast.literal_eval`.

use std::fmt::Write as _;

use super::FileRender;

const HEADER: &str = "#jinja2:";

/// A value `ast.literal_eval` returns, for the kinds an override can use or be refused for.
#[derive(Debug, PartialEq)]
enum Lit {
    Str(String),
    Bool(bool),
    None,
    Int,
    Float,
    /// A list, a tuple, a mapping or a set: only its class is kept.
    Container(&'static str),
}

impl Lit {
    /// Python's name for the value's class, as the reference's type errors give it.
    fn class(&self) -> &'static str {
        match self {
            Self::Str(_) => "str",
            Self::Bool(_) => "bool",
            Self::None => "NoneType",
            Self::Int => "int",
            Self::Float => "float",
            Self::Container(class) => class,
        }
    }
}

/// The text without its `#jinja2:` line and the options that line gives, or `None` when the
/// text does not begin with one. The error is the reference's, with the same lead.
pub(super) fn split<'t>(
    text: &'t str,
    base: &FileRender,
) -> Result<Option<(&'t str, FileRender)>, String> {
    let Some(rest) = text.strip_prefix(HEADER) else {
        return Ok(None);
    };
    let fail = |msg: String| format!("Syntax error in template: {msg}");
    let Some(eol) = rest.find('\n') else {
        return Err(fail(format!("Missing newline after '{HEADER}' override.")));
    };
    let (line, body) = (&rest[..eol], &rest[eol + 1..]);
    let mut given: Vec<(&str, Lit)> = Vec::new();
    for pair in line.split(',') {
        if pair.trim().is_empty() {
            return Err(fail(format!("Empty '{HEADER}' override pair not allowed.")));
        }
        let Some((key, value)) = pair.split_once(':') else {
            return Err(fail(format!(
                "Missing key-value separator `:` in '{HEADER}' override pair {}.",
                py_repr(pair)
            )));
        };
        let key = key.trim();
        if !FIELDS.contains(&key) {
            return Err(fail(format!(
                "Invalid '{HEADER}' override key {}.",
                py_repr(key)
            )));
        }
        given.push((key, literal_eval(value).map_err(fail)?));
    }
    let mut out = base.clone();
    // The reference checks the fields of the result in declaration order, after it has read
    // every pair, and a key given twice keeps its last value.
    for field in FIELDS {
        if let Some((_, value)) = given.iter().rev().find(|(k, _)| *k == field) {
            apply(&mut out, field, value).map_err(fail)?;
        }
    }
    out.syntax.check_starts().map_err(|msg| fail(msg.into()))?;
    Ok(Some((body, out)))
}

/// The fields of `TemplateOverrides`, in the order the reference declares and checks them.
const FIELDS: [&str; 12] = [
    "block_start_string",
    "block_end_string",
    "variable_start_string",
    "variable_end_string",
    "comment_start_string",
    "comment_end_string",
    "line_statement_prefix",
    "line_comment_prefix",
    "trim_blocks",
    "lstrip_blocks",
    "newline_sequence",
    "keep_trailing_newline",
];

fn apply(out: &mut FileRender, field: &str, value: &Lit) -> Result<(), String> {
    let wrong = |expected: &str| {
        format!(
            "TemplateOverrides.{field} must be <class '{expected}'> instead of <class '{}'>",
            value.class()
        )
    };
    let text = || match value {
        Lit::Str(s) => Ok(s.clone()),
        _ => Err(wrong("str")),
    };
    let flag = || match value {
        Lit::Bool(b) => Ok(*b),
        _ => Err(wrong("bool")),
    };
    let prefix = || match value {
        Lit::Str(s) => Ok(Some(s.clone())),
        Lit::None => Ok(None),
        _ => Err(format!(
            "TemplateOverrides.{field} must be <class 'str'> or <class 'NoneType'> instead of <class '{}'>",
            value.class()
        )),
    };
    let s = &mut out.syntax;
    match field {
        "block_start_string" => s.block_start = text()?,
        "block_end_string" => s.block_end = text()?,
        "variable_start_string" => s.variable_start = text()?,
        "variable_end_string" => s.variable_end = text()?,
        "comment_start_string" => s.comment_start = text()?,
        "comment_end_string" => s.comment_end = text()?,
        "line_statement_prefix" => s.line_statement_prefix = prefix()?,
        "line_comment_prefix" => s.line_comment_prefix = prefix()?,
        "trim_blocks" => out.trim_blocks = flag()?,
        "lstrip_blocks" => out.lstrip_blocks = flag()?,
        "keep_trailing_newline" => out.keep_trailing_newline = flag()?,
        _ => {
            out.newline_sequence = match &value {
                Lit::Str(s) if matches!(s.as_str(), "\n" | "\r\n" | "\r") => s.clone(),
                Lit::Str(s) => {
                    return Err(format!(
                        "TemplateOverrides.newline_sequence must be one of ('\\n', '\\r\\n', '\\r') instead of {}",
                        py_repr(s)
                    ));
                }
                _ => return Err(wrong("str")),
            };
        }
    }
    Ok(())
}

/// Python's `repr` of a string: single quotes, unless the text holds one and no double quote.
pub(crate) fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// The literals an override can hold: strings (adjacent ones join), `True`, `False`, `None` and
/// numbers, in parentheses or not. Anything else is what `ast.literal_eval` calls malformed.
fn literal_eval(src: &str) -> Result<Lit, String> {
    let src = src.trim();
    if let Some(inner) = src.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
        && !inner.trim().is_empty()
        && !inner.contains(',')
    {
        return literal_eval(inner);
    }
    match src {
        "True" => return Ok(Lit::Bool(true)),
        "False" => return Ok(Lit::Bool(false)),
        "None" => return Ok(Lit::None),
        _ => {}
    }
    match (src.chars().next(), src.chars().last()) {
        (Some('['), Some(']')) => return Ok(Lit::Container("list")),
        (Some('{'), Some('}')) if src.contains(':') || src == "{}" => {
            return Ok(Lit::Container("dict"));
        }
        (Some('{'), Some('}')) => return Ok(Lit::Container("set")),
        (Some('('), Some(')')) => return Ok(Lit::Container("tuple")),
        _ => {}
    }
    let digits = src.strip_prefix(['-', '+']).unwrap_or(src);
    if !digits.is_empty()
        && digits.starts_with(|c: char| c.is_ascii_digit())
        && digits
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'))
    {
        let plain = digits.chars().all(|c| c.is_ascii_digit() || c == '_');
        let radix =
            digits.len() > 2 && matches!(&digits[..2], "0x" | "0X" | "0o" | "0O" | "0b" | "0B");
        return if plain || radix {
            Ok(Lit::Int)
        } else if digits.parse::<f64>().is_ok() {
            Ok(Lit::Float)
        } else {
            Err(malformed())
        };
    }
    let mut rest = src;
    let mut joined = String::new();
    let mut any = false;
    while !rest.is_empty() {
        let (piece, tail) = string_literal(rest)?;
        joined.push_str(&piece);
        rest = tail.trim_start();
        any = true;
    }
    if any {
        Ok(Lit::Str(joined))
    } else {
        Err(malformed())
    }
}

fn malformed() -> String {
    "malformed node or string on line 1".into()
}

/// One string literal at the start of `src` and what follows it.
fn string_literal(src: &str) -> Result<(String, &str), String> {
    let mut chars = src.char_indices();
    let mut raw = false;
    let (quote_at, quote) = loop {
        match chars.next() {
            Some((_, 'r' | 'R')) if !raw => raw = true,
            Some((_, 'u' | 'U')) if !raw => {}
            Some((at, q @ ('\'' | '"'))) => break (at, q),
            _ => return Err(malformed()),
        }
    };
    let body = &src[quote_at + 1..];
    let triple = body.starts_with([quote; 2].iter().collect::<String>().as_str());
    let (body, close) = if triple {
        (&body[2..], [quote; 3].iter().collect::<String>())
    } else {
        (body, quote.to_string())
    };
    let unterminated = || "unterminated string literal (detected at line 1) (<unknown>, line 1)";
    let mut out = String::new();
    let mut iter = body.char_indices().peekable();
    while let Some((at, c)) = iter.next() {
        if body[at..].starts_with(&close) {
            return Ok((out, &body[at + close.len()..]));
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some((_, esc)) = iter.next() else {
            return Err(unterminated().into());
        };
        if raw {
            out.push('\\');
            out.push(esc);
            continue;
        }
        let mut hex = |n: usize| -> Result<char, String> {
            let digits: String = (0..n).filter_map(|_| iter.next().map(|(_, c)| c)).collect();
            u32::from_str_radix(&digits, 16)
                .ok()
                .filter(|_| digits.len() == n)
                .and_then(char::from_u32)
                .ok_or_else(|| "(unicode error) escape sequence is not valid".to_string())
        };
        match esc {
            '\n' => {}
            '\\' | '\'' | '"' => out.push(esc),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'a' => out.push('\u{7}'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            '0'..='7' => {
                let mut code = esc.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match iter.peek().and_then(|(_, c)| c.to_digit(8)) {
                        Some(d) => {
                            code = code * 8 + d;
                            iter.next();
                        }
                        None => break,
                    }
                }
                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
            }
            'x' => out.push(hex(2)?),
            'u' => out.push(hex(4)?),
            'U' => out.push(hex(8)?),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    Err(unterminated().into())
}

#[cfg(test)]
mod tests;
