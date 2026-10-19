// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

use rstest::rstest;

#[rstest]
#[case::empty("", 0, 0)]
#[case::scalar("12", 1, 0)]
#[case::literals("[true,false,null]", 4, 1)]
#[case::key_and_value(r#"{"a":1}"#, 3, 1)]
#[case::spaces_and_newlines("[\n 1 ,\t2 ,\r\n3 ]", 4, 1)]
#[case::numbers_with_signs_and_exponents("[-1.5e+10,0.25]", 3, 1)]
#[case::a_string_with_structure_in_it(r#"["]},[{:,", "x"]"#, 3, 1)]
#[case::an_escaped_quote(r#"["a\"],[b", "c"]"#, 3, 1)]
#[case::an_escaped_backslash_before_the_end(r#"["a\\", "b"]"#, 3, 1)]
#[case::nested(r#"{"a":[{"b":[[]]}]}"#, 7, 5)]
#[case::the_hostile_array(r#"{"x":[0,0,0,0,0,0,0,0,0,0]}"#, 13, 2)]
#[case::an_unterminated_string(r#"["abc"#, 2, 1)]
#[case::closers_without_openers("]]}}1", 1, 0)]
fn counts_values_and_nesting(#[case] json: &str, #[case] values: usize, #[case] depth: usize) {
    assert_eq!(measure(json.as_bytes()), Shape { values, depth });
}

/// What a valid document is made of is what `serde_json` builds from it, one node for each
/// value and one string for each key.
#[rstest]
#[case::flat(r#"{"a":1,"b":"two","c":[true,null],"d":{"e":-3.5}}"#)]
#[case::strings(r#"["a\"b","\\","\u00e9",""]"#)]
#[case::deep("[[[[[[[1]]]]]]]")]
fn agrees_with_the_parser(#[case] json: &str) {
    fn nodes(v: &serde_json::Value) -> usize {
        match v {
            serde_json::Value::Array(a) => 1 + a.iter().map(nodes).sum::<usize>(),
            serde_json::Value::Object(o) => 1 + o.iter().map(|(_, v)| 1 + nodes(v)).sum::<usize>(),
            _ => 1,
        }
    }
    let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(measure(json.as_bytes()).values, nodes(&parsed));
}

fn array_of(n: usize) -> Vec<u8> {
    let mut bytes = b"[".to_vec();
    bytes.extend(std::iter::repeat_n(&b"0,"[..], n).flatten());
    bytes.extend(b"0]");
    bytes
}

fn nested(levels: usize) -> Vec<u8> {
    [vec![b'['; levels], vec![b']'; levels]].concat()
}

#[rstest]
#[case::at_the_value_bound(array_of(MAX_VALUES - 2), None)]
#[case::one_value_over(array_of(MAX_VALUES - 1), Some(Excess::Values(MAX_VALUES + 1)))]
#[case::far_over(array_of(5 * MAX_VALUES), Some(Excess::Values(5 * MAX_VALUES + 2)))]
#[case::at_the_depth_bound(nested(MAX_DEPTH), None)]
#[case::one_level_over(nested(MAX_DEPTH + 1), Some(Excess::Depth(MAX_DEPTH + 1)))]
#[case::a_big_string_is_one_value(
    format!(r#"["{}"]"#, "x".repeat(8 << 20)).into_bytes(),
    None
)]
fn checks_both_bounds(#[case] bytes: Vec<u8>, #[case] expected: Option<Excess>) {
    assert_eq!(check(&bytes).err(), expected);
}

#[test]
fn the_message_names_the_figure_and_the_limit() {
    assert_eq!(
        Excess::Values(750_000).to_string(),
        "750000 values, over the limit of 500000"
    );
    assert_eq!(
        Excess::Depth(70).to_string(),
        "70 levels of nesting, over the limit of 64"
    );
}
