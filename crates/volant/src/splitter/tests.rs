// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

/// With `check_raw`, only the command options leave the line: `script`'s other words,
/// `key=value` or not, are its arguments.
#[test]
fn check_raw_keeps_every_other_word_in_the_line() {
    let (options, raw) = parse_kv_raw("run.sh a=1 'b c' creates=/x chdir=/tmp").unwrap();
    assert_eq!(options, pairs(&[("creates", "/x"), ("chdir", "/tmp")]));
    assert_eq!(raw.as_deref(), Some("run.sh a=1 'b c'"));
}

/// The command line of `command` and `shell`, row by row as `parse_kv(.., check_raw=True)` of
/// ansible-core 2.19.12 returns it.
#[rstest::rstest]
#[case::creates("echo hi creates=/etc/passwd", &[("creates", "/etc/passwd")], Some("echo hi"))]
#[case::chdir_first("chdir=/etc pwd", &[("chdir", "/etc")], Some("pwd"))]
#[case::double_quoted("echo hi creates=\"/tmp/a b\"", &[("creates", "/tmp/a b")], Some("echo hi"))]
#[case::single_quoted("pwd chdir='/tmp/a b'", &[("chdir", "/tmp/a b")], Some("pwd"))]
#[case::template("echo hi creates={{ p ~ ' x' }}", &[("creates", "{{ p ~ ' x' }}")], Some("echo hi"))]
#[case::block("echo hi {% if a %}x{% endif %} removes=/y", &[("removes", "/y")], Some("echo hi {% if a %}x{% endif %}"))]
#[case::other_key("echo foo=bar cmd=x", &[], Some("echo foo=bar cmd=x"))]
#[case::quoted_other_key("echo 'a=b c' creates=/x", &[("creates", "/x")], Some("echo 'a=b c'"))]
#[case::escaped_equals("echo a\\=b creates=/x", &[("creates", "/x")], Some("echo a=b"))]
#[case::spaces_kept("echo  a   b creates=/x", &[("creates", "/x")], Some("echo  a   b"))]
#[case::newline_kept("echo a\necho b creates=/x", &[("creates", "/x")], Some("echo a\necho b"))]
#[case::all_options(
    "x creates=/a removes=/b chdir=/c executable=/bin/bash stdin=in strip_empty_ends=false",
    &[("creates", "/a"), ("removes", "/b"), ("chdir", "/c"), ("executable", "/bin/bash"), ("stdin", "in"), ("strip_empty_ends", "false")],
    Some("x")
)]
#[case::only_options("creates=/a", &[("creates", "/a")], None)]
#[case::empty("", &[], None)]
fn a_command_line_gives_up_only_the_command_options(
    #[case] line: &str,
    #[case] options: &[(&str, &str)],
    #[case] raw: Option<&str>,
) {
    let (got, rest) = parse_kv_raw(line).unwrap();
    assert_eq!(got, pairs(options), "{line}");
    assert_eq!(rest.as_deref(), raw, "{line}");
}

fn kv(args: &str) -> Vec<(String, String)> {
    parse_kv(args).unwrap().0
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// In a `set_fact:` string a Jinja block with spaces stays one value, and quotes and escapes
/// are handled as ansible-core 2.19.12 does.
#[test]
fn a_jinja_block_with_spaces_stays_one_value() {
    assert_eq!(
        kv(r#"a={{ base ~ '/x' }} b="p q" c='r s' d={% if true %}yes{% endif %} e=x\ty f=k\=v"#),
        pairs(&[
            ("a", "{{ base ~ '/x' }}"),
            ("b", "p q"),
            ("c", "r s"),
            ("d", "{% if true %}yes{% endif %}"),
            ("e", "x\ty"),
            ("f", r"k\=v"),
        ])
    );
    assert_eq!(
        kv(r#"g={{ dict(k=1) }} h=' t ' i=u"v w""#),
        pairs(&[("g", "{{ dict(k=1) }}"), ("h", " t "), ("i", r#"u"v w""#)])
    );
}

#[test]
fn words_without_a_key_are_joined_back_as_raw_params() {
    assert_eq!(
        parse_kv("echo {{ a b }} x=1 'c d'").unwrap(),
        (
            pairs(&[("x", "1")]),
            Some("echo {{ a b }} 'c d'".to_string())
        )
    );
    assert_eq!(parse_kv("").unwrap(), (Vec::new(), None));
}

#[test]
fn an_unclosed_block_or_quote_fails_the_split() {
    for bad in ["a={{ b", "a='b c", "a={% if x"] {
        let err = parse_kv(bad).unwrap_err().to_string();
        assert!(
            err.starts_with("failed at splitting arguments"),
            "{bad}: {err}"
        );
    }
}
