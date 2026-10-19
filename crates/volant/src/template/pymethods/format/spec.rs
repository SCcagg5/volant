// SPDX-License-Identifier: GPL-3.0-or-later
//! Python's format specification mini-language: what follows the colon in `{:>8.2f}`.

use minijinja::value::ValueKind;
use minijinja::{Error, Value};

use super::float;
use crate::template::pymethods::args::{raise, type_of};

/// A parsed `[[fill]align][sign][z][#][0][width][grouping][.precision][type]`.
#[derive(Debug, Default)]
pub(crate) struct Spec {
    fill: Option<char>,
    align: Option<char>,
    sign: Option<char>,
    alt: bool,
    zero: bool,
    width: usize,
    group: Option<char>,
    precision: Option<usize>,
    kind: Option<char>,
}

fn number(chars: &[char], at: &mut usize) -> Result<Option<usize>, Error> {
    let from = *at;
    while chars.get(*at).is_some_and(char::is_ascii_digit) {
        *at += 1;
    }
    if from == *at {
        return Ok(None);
    }
    chars[from..*at]
        .iter()
        .collect::<String>()
        .parse()
        .map(Some)
        .map_err(|_| raise("Too many decimal digits in format string"))
}

pub(crate) fn parse(spec: &str, type_name: &str) -> Result<Spec, Error> {
    let chars: Vec<char> = spec.chars().collect();
    let mut out = Spec::default();
    let mut at = 0;
    let is_align = |c: &char| "<>=^".contains(*c);
    if chars.get(1).is_some_and(is_align) {
        out.fill = Some(chars[0]);
        out.align = Some(chars[1]);
        at = 2;
    } else if chars.first().is_some_and(is_align) {
        out.align = Some(chars[0]);
        at = 1;
    }
    if chars.get(at).is_some_and(|c| "+- ".contains(*c)) {
        out.sign = Some(chars[at]);
        at += 1;
    }
    if chars.get(at) == Some(&'z') {
        at += 1;
    }
    if chars.get(at) == Some(&'#') {
        out.alt = true;
        at += 1;
    }
    if chars.get(at) == Some(&'0') {
        out.zero = true;
        at += 1;
    }
    out.width = number(&chars, &mut at)?.unwrap_or(0);
    if chars.get(at).is_some_and(|c| ",_".contains(*c)) {
        out.group = Some(chars[at]);
        at += 1;
    }
    if chars.get(at) == Some(&'.') {
        at += 1;
        out.precision = Some(
            number(&chars, &mut at)?.ok_or_else(|| raise("Format specifier missing precision"))?,
        );
    }
    match chars.len() - at {
        0 => {}
        1 => out.kind = Some(chars[at]),
        _ => {
            return Err(raise(format!(
                "Invalid format specifier '{spec}' for object of type '{type_name}'"
            )));
        }
    }
    Ok(out)
}

/// `format(value, spec)`. An empty spec is `str(value)`.
pub(crate) fn format_value(value: &Value, spec: &str) -> Result<String, Error> {
    if spec.is_empty() {
        return Ok(super::to_str(value));
    }
    match value.kind() {
        ValueKind::String => text(value.as_str().unwrap_or_default(), parse(spec, "str")?),
        ValueKind::Bool => integer(i64::from(value.is_true()), parse(spec, "int")?),
        ValueKind::Number if value.is_integer() => match value.as_i64() {
            Some(n) => integer(n, parse(spec, "int")?),
            None => Err(raise("Python int too large to convert to C ssize_t")),
        },
        ValueKind::Number => {
            let f = f64::try_from(value.clone()).unwrap_or(f64::NAN);
            decimal(f, parse(spec, "float")?)
        }
        _ => Err(raise(format!(
            "unsupported format string passed to {}.__format__",
            type_of(value)
        ))),
    }
}

fn unknown(kind: char, type_name: &str) -> Error {
    raise(format!(
        "Unknown format code '{kind}' for object of type '{type_name}'"
    ))
}

fn text(s: &str, spec: Spec) -> Result<String, Error> {
    if spec.kind.is_some_and(|k| k != 's') {
        return Err(unknown(spec.kind.unwrap_or('s'), "str"));
    }
    if spec.sign.is_some() {
        return Err(raise("Sign not allowed in string format specifier"));
    }
    if spec.alt {
        return Err(raise(
            "Alternate form (#) not allowed in string format specifier",
        ));
    }
    if spec.align == Some('=') {
        return Err(raise(
            "'=' alignment not allowed in string format specifier",
        ));
    }
    if let Some(g) = spec.group {
        return Err(raise(format!("Cannot specify '{g}' with 's'.")));
    }
    let body: String = match spec.precision {
        Some(p) => s.chars().take(p).collect(),
        None => s.to_string(),
    };
    let fill = spec.fill.unwrap_or(if spec.zero { '0' } else { ' ' });
    Ok(pad(&body, "", spec.width, fill, spec.align.unwrap_or('<')))
}

/// `body` in a field `width` wide, `prefix` (sign and base) kept in front of the padding for `=`.
fn pad(body: &str, prefix: &str, width: usize, fill: char, align: char) -> String {
    let len = prefix.chars().count() + body.chars().count();
    let extra = width.saturating_sub(len);
    let fill_of = |n: usize| std::iter::repeat_n(fill, n).collect::<String>();
    match align {
        '>' => format!("{}{prefix}{body}", fill_of(extra)),
        '^' => format!(
            "{}{prefix}{body}{}",
            fill_of(extra / 2),
            fill_of(extra - extra / 2)
        ),
        '=' => format!("{prefix}{}{body}", fill_of(extra)),
        _ => format!("{prefix}{body}{}", fill_of(extra)),
    }
}

fn sign_of(negative: bool, spec: &Spec) -> &'static str {
    match (negative, spec.sign) {
        (true, _) => "-",
        (false, Some('+')) => "+",
        (false, Some(' ')) => " ",
        _ => "",
    }
}

/// Digits grouped by `sep`, every `size` digits from the right.
fn grouped(digits: &str, sep: char, size: usize) -> String {
    let n = digits.chars().count();
    let mut out = String::new();
    for (at, c) in digits.chars().enumerate() {
        if at > 0 && (n - at).is_multiple_of(size) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// A number's field: the grouped digits and what follows them (`tail`), widened with zeros
/// when the field is zero-padded, placed after `prefix` (the sign and the base).
fn finish(digits: &str, tail: &str, prefix: &str, spec: &Spec, size: usize) -> String {
    let fill = spec.fill.unwrap_or(if spec.zero { '0' } else { ' ' });
    let align = spec.align.unwrap_or(if spec.zero { '=' } else { '>' });
    let Some(sep) = spec.group else {
        return pad(&format!("{digits}{tail}"), prefix, spec.width, fill, align);
    };
    let mut digits = digits.to_string();
    let mut body = grouped(&digits, sep, size);
    if fill == '0' && align == '=' {
        let target = spec
            .width
            .saturating_sub(prefix.chars().count() + tail.chars().count());
        while body.chars().count() < target || body.starts_with(sep) {
            digits.insert(0, '0');
            body = grouped(&digits, sep, size);
        }
    }
    pad(&format!("{body}{tail}"), prefix, spec.width, fill, align)
}

fn integer(n: i64, spec: Spec) -> Result<String, Error> {
    let kind = spec.kind.unwrap_or('d');
    if "eEfFgG%".contains(kind) {
        // An integer given a float type is converted first.
        return decimal(n as f64, spec);
    }
    if !"bcdnoxX".contains(kind) {
        return Err(unknown(kind, "int"));
    }
    if spec.precision.is_some() {
        return Err(raise("Precision not allowed in integer format specifier"));
    }
    if kind == 'c' {
        if spec.sign.is_some() {
            return Err(raise("Sign not allowed with integer format specifier 'c'"));
        }
        if let Some(g) = spec.group {
            return Err(raise(format!("Cannot specify '{g}' with 'c'.")));
        }
        let c = u32::try_from(n)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| raise("%c arg not in range(0x110000)"))?;
        let fill = spec.fill.unwrap_or(' ');
        return Ok(pad(
            &c.to_string(),
            "",
            spec.width,
            fill,
            spec.align.unwrap_or('>'),
        ));
    }
    if spec.group == Some(',') && "boxX".contains(kind) {
        return Err(raise(format!("Cannot specify ',' with '{kind}'.")));
    }
    let magnitude = n.unsigned_abs();
    let digits = match kind {
        'b' => format!("{magnitude:b}"),
        'o' => format!("{magnitude:o}"),
        'x' => format!("{magnitude:x}"),
        'X' => format!("{magnitude:X}"),
        _ => magnitude.to_string(),
    };
    let base = match (spec.alt, kind) {
        (true, 'b') => "0b",
        (true, 'o') => "0o",
        (true, 'x') => "0x",
        (true, 'X') => "0X",
        _ => "",
    };
    let prefix = format!("{}{base}", sign_of(n < 0, &spec));
    let size = if "boxX".contains(kind) { 4 } else { 3 };
    Ok(finish(&digits, "", &prefix, &spec, size))
}

fn decimal(f: f64, spec: Spec) -> Result<String, Error> {
    let kind = spec.kind;
    if kind.is_some_and(|k| !"eEfFgGn%".contains(k)) {
        return Err(unknown(kind.unwrap_or('f'), "float"));
    }
    let negative = f.is_sign_negative() && !f.is_nan();
    let prefix = sign_of(negative, &spec).to_string();
    if !f.is_finite() {
        let upper = kind.is_some_and(|k| k.is_ascii_uppercase());
        let body = float::special(f.abs(), upper);
        let fill = spec.fill.unwrap_or(' ');
        return Ok(pad(
            &body,
            &prefix,
            spec.width,
            fill,
            spec.align.unwrap_or('>'),
        ));
    }
    let a = f.abs();
    let precision = spec.precision;
    let body = match kind {
        Some('e' | 'E') => float::exp_style(a, precision.unwrap_or(6), spec.alt, kind == Some('E')),
        Some('f' | 'F') => float::fixed(a, precision.unwrap_or(6), spec.alt),
        Some('g' | 'G' | 'n') => {
            float::general(a, precision.unwrap_or(6), spec.alt, kind == Some('G'))
        }
        Some('%') => format!(
            "{}%",
            float::fixed(a * 100.0, precision.unwrap_or(6), spec.alt)
        ),
        _ => float::plain(a, precision),
    };
    let (int_part, rest) = body
        .find(|c: char| !c.is_ascii_digit())
        .map_or((body.as_str(), ""), |at| (&body[..at], &body[at..]));
    Ok(finish(int_part, rest, &prefix, &spec, 3))
}
