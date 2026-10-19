// SPDX-License-Identifier: GPL-3.0-or-later
//! The `argv` argument read the way the reference reads it: `type='list', elements='str'`.

use std::fmt::Write as _;

use serde_json::{Map, Value, json};
use volant_protocol::TaskResult;

/// The words of `argv`, or `None` when the argument is not given. A list keeps its items, a string
/// is split on commas, a number or a boolean is a list of one item, and every item is turned into
/// text. A mapping cannot become a list: the task fails with the reference's sentence.
pub(super) fn argv_arg(args: &Map<String, Value>) -> Result<Option<Vec<String>>, TaskResult> {
    let words = match args.get("argv") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(items)) => items.iter().map(text).collect(),
        Some(Value::String(line)) => line.split(',').map(str::to_string).collect(),
        Some(scalar @ (Value::Bool(_) | Value::Number(_))) => vec![text(scalar)],
        Some(Value::Object(_)) => {
            let mut result = TaskResult::failed_with(
                "argument 'argv' is of type dict and we were unable to convert to list: \
                 <class 'dict'> cannot be converted to a list",
            );
            result.0.insert("changed".into(), json!(false));
            return Err(result);
        }
    };
    Ok(Some(words))
}

/// One item as the reference's `check_type_str` makes it: a string stays, `None` is empty, and
/// anything else is what Python's `str` prints.
fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => repr(other),
    }
}

/// Python's `repr` of a value read from JSON.
fn repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => float(f),
            _ => n.to_string(),
        },
        Value::String(s) => quoted(s),
        Value::Array(items) => {
            format!(
                "[{}]",
                items.iter().map(repr).collect::<Vec<_>>().join(", ")
            )
        }
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(key, item)| format!("{}: {}", quoted(key), repr(item)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Python's `repr` of a finite float: the shortest digits that read back the same, in the
/// exponent form below `1e-4` and from `1e16` up.
fn float(f: f64) -> String {
    let sci = format!("{f:e}");
    let (digits, exponent) = sci.split_once('e').expect("`{:e}` writes an exponent");
    let exponent: i32 = exponent.parse().expect("the exponent is a number");
    if (-4..16).contains(&exponent) {
        let plain = f.to_string();
        if plain.contains('.') {
            plain
        } else {
            format!("{plain}.0")
        }
    } else {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{digits}e{sign}{:02}", exponent.abs())
    }
}

/// Python's `repr` of a string: single quotes unless the text holds one and no double quote, the
/// quote and the backslash escaped, and what Python does not print as `\n`, `\xNN`, `\uNNNN` or
/// `\UNNNNNNNN`.
fn quoted(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
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
            c if unprintable(c) => {
                let code = u32::from(c);
                let _ = match code {
                    0..=0xff => write!(out, "\\x{code:02x}"),
                    0x100..=0xffff => write!(out, "\\u{code:04x}"),
                    _ => write!(out, "\\U{code:08x}"),
                };
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// The characters Python's `str.isprintable` refuses that text commonly holds: controls, the
/// spaces other than ` `, and the format characters. The rarer unassigned code points are not
/// listed.
fn unprintable(c: char) -> bool {
    c.is_control()
        || matches!(
            u32::from(c),
            0xa0 | 0xad | 0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x1680 | 0x180e
                | 0x2000..=0x200f | 0x2028..=0x202f | 0x205f..=0x206f
                | 0x3000 | 0xe000..=0xf8ff | 0xfeff | 0xfff9..=0xfffb | 0xf0000..=0x10ffff
        )
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    fn words(argv: Value) -> Result<Option<Vec<String>>, TaskResult> {
        let Value::Object(args) = json!({ "argv": argv }) else {
            unreachable!()
        };
        argv_arg(&args)
    }

    /// Each item becomes the text Python's `str` gives it, as `check_type_str` of 2.19.12 does.
    #[rstest]
    #[case::int(json!(17), "17")]
    #[case::negative(json!(-3), "-3")]
    #[case::largest_unsigned(json!(18_446_744_073_709_551_615_u64), "18446744073709551615")]
    #[case::float(json!(1.5), "1.5")]
    #[case::whole_float(json!(1.0), "1.0")]
    #[case::negative_zero(json!(-0.0), "-0.0")]
    #[case::small_float(json!(0.0001), "0.0001")]
    #[case::tiny_float(json!(0.00001), "1e-05")]
    #[case::tiny_with_digits(json!(2.5e-7), "2.5e-07")]
    #[case::last_plain_float(json!(1e15), "1000000000000000.0")]
    #[case::first_exponent_float(json!(1e16), "1e+16")]
    #[case::large_float(json!(1.5e300), "1.5e+300")]
    #[case::bool_true(json!(true), "True")]
    #[case::bool_false(json!(false), "False")]
    #[case::null(json!(null), "")]
    #[case::string(json!("a b"), "a b")]
    #[case::list(json!([1, null, true, "a"]), "[1, None, True, 'a']")]
    #[case::empty_list(json!([]), "[]")]
    #[case::mapping(json!({"k": [1, {"z": null}]}), "{'k': [1, {'z': None}]}")]
    #[case::float_in_list(json!([1.0, 2.5e-5]), "[1.0, 2.5e-05]")]
    #[case::single_quote(json!(["a'b"]), r#"["a'b"]"#)]
    #[case::double_quote(json!(["a\"b"]), r#"['a"b']"#)]
    #[case::both_quotes(json!(["it's \"x\""]), r#"['it\'s "x"']"#)]
    #[case::escapes(json!(["é\n\t\\"]), r"['é\n\t\\']")]
    #[case::unprintable(
        json!(["\u{a0}\u{200b}\u{1}\u{7f}\u{ff}\u{e000}\u{1f600}"]),
        "['\\xa0\\u200b\\x01\\x7fÿ\\ue000😀']"
    )]
    fn an_item_is_turned_into_text(#[case] item: Value, #[case] want: &str) {
        let got = words(json!(["a", item])).unwrap().unwrap();
        assert_eq!(got, ["a", want]);
    }

    /// A string splits on commas and a number or a boolean is one word, as `check_type_list` does.
    #[rstest]
    #[case::string(json!("echo,hi"), &["echo", "hi"])]
    #[case::empty_pieces(json!("echo,,hi,"), &["echo", "", "hi", ""])]
    #[case::no_comma(json!("echo hi"), &["echo hi"])]
    #[case::empty_string(json!(""), &[""])]
    #[case::int(json!(5), &["5"])]
    #[case::float(json!(1.5), &["1.5"])]
    #[case::bool(json!(true), &["True"])]
    fn a_scalar_becomes_a_list(#[case] given: Value, #[case] want: &[&str]) {
        assert_eq!(words(given).unwrap().unwrap(), want);
    }

    /// An absent or null `argv` is no list, and a mapping fails with the reference's sentence.
    #[test]
    fn a_missing_argument_is_none_and_a_mapping_fails() {
        assert!(words(json!(null)).unwrap().is_none());
        assert!(argv_arg(&Map::new()).unwrap().is_none());
        let failure = words(json!({"a": "b"})).unwrap_err();
        assert_eq!(
            failure.0["msg"],
            "argument 'argv' is of type dict and we were unable to convert to list: \
             <class 'dict'> cannot be converted to a list"
        );
        assert_eq!(failure.0["failed"], true);
    }
}
