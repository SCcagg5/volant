// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use serde_json::json;

/// A key written twice keeps its last value instead of being refused.
#[test]
fn a_key_written_twice_keeps_its_last_value() {
    let docs = load(
        "a: 1
b: 2
a: 3
",
        "t.yml",
    )
    .unwrap();
    assert_eq!(to_json(&docs[0]).unwrap(), json!({"a": 3, "b": 2}));
}

#[test]
fn scalars_sequences_and_mappings_convert_to_json() {
    let docs = load("a: 1\nb: [x, 2.5, true, ~]\nc:\n  d: text\n", "t.yml").unwrap();
    let v = to_json(&docs[0]).unwrap();
    assert_eq!(
        v,
        json!({"a": 1, "b": ["x", 2.5, true, null], "c": {"d": "text"}})
    );
}

/// A document of only markers and comments is null, while an empty block scalar and an empty
/// `!!str` stay empty strings, as in PyYAML.
#[test]
fn an_empty_document_is_null_and_an_empty_block_scalar_is_not() {
    for text in ["---\n# only a comment\n", "---\n", "--- # c\n...\n"] {
        let docs = load(text, "t.yml").unwrap();
        assert_eq!(to_json(&docs[0]).unwrap(), Value::Null, "{text:?}");
    }
    for (text, want) in [
        ("a: |\nb: 1\n", json!({"a": "", "b": 1})),
        ("a: |-\n\nb: 2\n", json!({"a": "", "b": 2})),
        ("- |\n- x\n", json!(["", "x"])),
        ("a: !!str\n", json!({"a": ""})),
        ("b: ''\nc: \"\"\n", json!({"b": "", "c": ""})),
    ] {
        let docs = load(text, "t.yml").unwrap();
        assert_eq!(to_json(&docs[0]).unwrap(), want, "{text:?}");
    }
}

#[test]
fn pyyaml_booleans_are_recognised_on_demand_only() {
    let docs = load("a: yes\nb: no\nc: maybe\n", "t.yml").unwrap();
    assert_eq!(as_bool(field(&docs[0], "a").unwrap()), Some(true));
    assert_eq!(as_bool(field(&docs[0], "b").unwrap()), Some(false));
    assert_eq!(as_bool(field(&docs[0], "c").unwrap()), None);
    // Converted as data too, matching PyYAML: unquoted `yes` reads as a variable's value
    // is a boolean, the same as Ansible's own `ignore_errors: yes`.
    assert_eq!(to_json(field(&docs[0], "a").unwrap()).unwrap(), json!(true));
}

#[test]
fn a_quoted_pyyaml_boolean_spelling_stays_a_string() {
    let docs = load("flag: \"yes\"\n", "t.yml").unwrap();
    let node = field(&docs[0], "flag").unwrap();
    assert_eq!(
        as_bool(node),
        Some(true),
        "quoting doesn't stop a keyword field"
    );
    assert_eq!(
        to_json(node).unwrap(),
        json!("yes"),
        "quoting keeps a generic data value a string"
    );
}

/// Every boolean spelling is read the same by the resolver, `as_bool` and `bool_from_str`,
/// which share one list.
#[test]
fn one_list_of_boolean_spellings_serves_every_reader() {
    for (spellings, want) in [(PYYAML_TRUE, true), (PYYAML_FALSE, false)] {
        for spelling in spellings {
            assert_eq!(bool_from_str(spelling), Some(want), "{spelling}");
            let text = format!("flag: {spelling}\n");
            let docs = load(&text, "t.yml").unwrap();
            let node = field(&docs[0], "flag").unwrap();
            assert_eq!(as_bool(node), Some(want), "{spelling} as a keyword");
            assert_eq!(to_json(node).unwrap(), json!(want), "{spelling} as data");
        }
    }
    assert_eq!(bool_from_str("maybe"), None);
}

/// Each spelling and its type as `ansible-core 2.19.12`'s own PyYAML reads it, measured
/// directly.
#[test]
fn integer_and_float_spellings_are_measured_against_the_reference() {
    let cases: &[(&str, Value)] = &[
        ("010", json!(8)),
        ("\"010\"", json!("010")),
        ("0_755", json!(493)),
        ("08", json!("08")),
        ("0o10", json!("0o10")),
        ("0b1010", json!(10)),
        ("0b_1010", json!(10)),
        ("0x1F", json!(31)),
        ("0x_1F", json!(31)),
        ("1_000", json!(1000)),
        ("1000", json!(1000)),
        ("+12", json!(12)),
        ("-07", json!(-7)),
        ("+0b11", json!(3)),
        ("0", json!(0)),
        ("00", json!(0)),
        ("0_0", json!(0)),
        ("1e3", json!("1e3")),
        ("1e+3", json!("1e+3")),
        ("1.0e3", json!("1.0e3")),
        ("1.0e+3", json!(1000.0)),
        ("2.5", json!(2.5)),
        (".5", json!(0.5)),
        ("1_0.5", json!(10.5)),
        ("1e", json!("1e")),
        ("0xg", json!("0xg")),
    ];
    for (raw, want) in cases {
        let text = format!("x: {raw}\n");
        let docs = load(&text, "t.yml").unwrap();
        let got = to_json(field(&docs[0], "x").unwrap()).unwrap();
        assert_eq!(&got, want, "{raw}");
    }
}

/// Loads each text and compares it with the value ansible-core 2.19.12 gave it (`DataLoader().load`),
/// as JSON.
fn assert_loads_as(cases: &[(&str, Value)]) {
    for (text, want) in cases {
        let docs = load(text, "t.yml").unwrap_or_else(|e| panic!("{text:?}: {e:#}"));
        assert_eq!(&to_json(&docs[0]).unwrap(), want, "{text:?}");
    }
}

/// An alias is the node its anchor names, whatever the node is.
#[test]
fn an_alias_has_the_value_of_its_anchor() {
    assert_loads_as(&[
        ("a: &n 5\nb: *n\n", json!({"a": 5, "b": 5})),
        ("a: &n 1.5\nb: *n\n", json!({"a": 1.5, "b": 1.5})),
        ("a: &n yes\nb: *n\n", json!({"a": true, "b": true})),
        ("a: &n no\nb: *n\n", json!({"a": false, "b": false})),
        ("a: &n ~\nb: *n\n", json!({"a": null, "b": null})),
        ("a: &n abc\nb: *n\n", json!({"a": "abc", "b": "abc"})),
        ("a: &n \"x y\"\nb: *n\n", json!({"a": "x y", "b": "x y"})),
        ("a: &n \"5\"\nb: *n\n", json!({"a": "5", "b": "5"})),
        (
            "a: &n |\n  line1\n  line2\nb: *n\n",
            json!({"a": "line1\nline2\n", "b": "line1\nline2\n"}),
        ),
        (
            "a: &n >\n  a\n  b\nb: *n\n",
            json!({"a": "a b\n", "b": "a b\n"}),
        ),
        ("a: &n 010\nb: *n\n", json!({"a": 8, "b": 8})),
        ("a: &n 0x1F\nb: *n\n", json!({"a": 31, "b": 31})),
        (
            "a: &k key\nm:\n  *k : 1\n",
            json!({"a": "key", "m": {"key": 1}}),
        ),
        (
            "a: &n 5\nb: [*n, *n, x]\n",
            json!({"a": 5, "b": [5, 5, "x"]}),
        ),
        (
            "a: &n yes\nb: [*n, 1]\n",
            json!({"a": true, "b": [true, 1]}),
        ),
        ("a: &s [1, 2]\nb: *s\n", json!({"a": [1, 2], "b": [1, 2]})),
        (
            "a: &m {x: 1}\nb: *m\n",
            json!({"a": {"x": 1}, "b": {"x": 1}}),
        ),
        ("- &n 5\n- *n\n", json!([5, 5])),
        ("a: &n !!str 5\nb: *n\n", json!({"a": "5", "b": "5"})),
        ("a: &n !!int '5'\nb: *n\n", json!({"a": 5, "b": 5})),
    ]);
}

/// `!unsafe` resolves the text as a plain scalar whatever its quotes are, and the alias of such a
/// scalar is the same.
#[test]
fn an_unsafe_scalar_is_resolved_as_a_plain_one() {
    assert_loads_as(&[
        ("a: &n !unsafe 5\nb: *n\n", json!({"a": 5, "b": 5})),
        ("a: !unsafe 5\n", json!({"a": 5})),
        ("a: !unsafe 1.5\n", json!({"a": 1.5})),
        ("a: !unsafe yes\n", json!({"a": true})),
        ("a: !unsafe ~\n", json!({"a": null})),
        ("a: !unsafe abc\n", json!({"a": "abc"})),
        ("a: !unsafe \"5\"\n", json!({"a": 5})),
        ("a: !unsafe 010\n", json!({"a": 8})),
        ("a: !unsafe 0x1F\n", json!({"a": 31})),
        (
            "a: !unsafe |\n  line1\n  line2\n",
            json!({"a": "line1\nline2\n"}),
        ),
        ("!unsafe 5\n", json!(5)),
        (
            "a: &s !unsafe [yes, 010]\nb: *s\n",
            json!({"a": [true, 8], "b": [true, 8]}),
        ),
        ("a: !unsafe ''\n", json!({"a": null})),
        ("a: !unsafe 'yes'\n", json!({"a": true})),
        ("a: !unsafe .5\n", json!({"a": 0.5})),
        ("a: !unsafe 1:30\n", json!({"a": 90})),
        ("a: !unsafe |\n  5\n", json!({"a": 5})),
    ]);
}

/// An explicit tag of the core schema decides the type of a scalar, quoted or not.
#[test]
fn an_explicit_core_tag_picks_the_type() {
    assert_loads_as(&[
        ("a: !!str 5\n", json!({"a": "5"})),
        ("a: !!str yes\n", json!({"a": "yes"})),
        ("a: !!str ~\n", json!({"a": "~"})),
        ("a: !!int \"5\"\n", json!({"a": 5})),
        ("a: !!int 0x10\n", json!({"a": 16})),
        ("a: !!int 010\n", json!({"a": 8})),
        ("a: !!int 1_000\n", json!({"a": 1000})),
        ("a: !!int 0o10\n", json!({"a": 8})),
        ("a: !!float 5\n", json!({"a": 5.0})),
        ("a: !!float \"1.5\"\n", json!({"a": 1.5})),
        ("a: !!bool yes\n", json!({"a": true})),
        ("a: !!bool \"yes\"\n", json!({"a": true})),
        ("a: !!bool YES\n", json!({"a": true})),
        ("a: !!null x\n", json!({"a": null})),
        ("a: !!null 5\n", json!({"a": null})),
        ("a: !!python/unicode 5\n", json!({"a": "5"})),
        ("a: !!int ' 5 '\n", json!({"a": 5})),
        ("a: !!int --5\n", json!({"a": 5})),
        ("a: !!float --5\n", json!({"a": 5.0})),
    ]);
}

/// The rows of the plain scalars the resolver types, with the near misses it leaves as strings.
/// Integers beyond `i64` are not in the table, and neither are the non-finite floats, which the
/// test after it holds.
#[test]
fn plain_scalars_are_resolved_as_pyyaml_resolves_them() {
    assert_loads_as(&[
        ("a: 1:30\n", json!({"a": 90})),
        ("a: 1:30:00\n", json!({"a": 5400})),
        ("a: 190:20:30\n", json!({"a": 685230})),
        ("a: 1_0:30\n", json!({"a": 630})),
        ("a: 0:30\n", json!({"a": "0:30"})),
        ("a: 1:30.5\n", json!({"a": 90.5})),
        ("a: 190:20:30.15\n", json!({"a": 685230.15})),
        ("a: -190:20:30\n", json!({"a": -685230})),
        ("a: 1.5:30\n", json!({"a": "1.5:30"})),
        ("a: 9:60:60\n", json!({"a": "9:60:60"})),
        ("a: +.5\n", json!({"a": "+.5"})),
        ("a: -.5\n", json!({"a": "-.5"})),
        ("a: .5\n", json!({"a": 0.5})),
        ("a: 5.\n", json!({"a": 5.0})),
        ("a: 1.e+3\n", json!({"a": 1000.0})),
        ("a: 1._5\n", json!({"a": 1.5})),
        ("a: 010\n", json!({"a": 8})),
        ("a: 0o10\n", json!({"a": "0o10"})),
        ("a: 0x1F\n", json!({"a": 31})),
        ("a: 09\n", json!({"a": "09"})),
        ("a: 1e3\n", json!({"a": "1e3"})),
        ("a: 1.0e+3\n", json!({"a": 1000.0})),
        ("a: ~\n", json!({"a": null})),
        ("a: null\n", json!({"a": null})),
        ("a: Yes\n", json!({"a": true})),
        ("a: yEs\n", json!({"a": "yEs"})),
        ("a: 0_8\n", json!({"a": "0_8"})),
        ("a: 00_7\n", json!({"a": 7})),
        ("a: 0B1\n", json!({"a": "0B1"})),
    ]);
}

/// A date and a datetime have no counterpart in the value model: they load as the string that
/// ansible-core prints for them.
#[test]
fn a_date_is_the_text_ansible_core_prints_for_it() {
    assert_loads_as(&[
        ("a: 2020-01-02\n", json!({"a": "2020-01-02"})),
        ("a: 2020-1-2\n", json!({"a": "2020-1-2"})),
        (
            "a: 2020-01-02 10:11:12\n",
            json!({"a": "2020-01-02T10:11:12"}),
        ),
        (
            "a: 2020-01-02T10:11:12Z\n",
            json!({"a": "2020-01-02T10:11:12+00:00"}),
        ),
        (
            "a: 2020-01-02t10:11:12.5+01:00\n",
            json!({"a": "2020-01-02T10:11:12.500000+01:00"}),
        ),
        ("a: !!timestamp 2020-1-2\n", json!({"a": "2020-01-02"})),
        (
            "a: !!timestamp '2020-01-02 10:11:12 -5'\n",
            json!({"a": "2020-01-02T10:11:12-05:00"}),
        ),
        (
            "a: !!timestamp '2020-01-02 10:11:12 +05:30'\n",
            json!({"a": "2020-01-02T10:11:12+05:30"}),
        ),
        (
            "a: !!timestamp 2020-01-02t10:11:12.123456789Z\n",
            json!({"a": "2020-01-02T10:11:12.123456+00:00"}),
        ),
    ]);
}

/// The text of each error is ansible-core 2.19.12's, after `YAML parsing failed: `. Some of it
/// is Python's own: the load stops at the first scalar that cannot be built.
const REFUSED: &[(&str, &str)] = &[
    ("a: !vault 5\n", "The '!vault' tag requires a string value."),
    (
        "a: !vault yes\n",
        "The '!vault' tag requires a string value.",
    ),
    ("a: !vault ~\n", "The '!vault' tag requires a string value."),
    (
        "a: !vault 1.5\n",
        "The '!vault' tag requires a string value.",
    ),
    (
        "a: !vault [1]\n",
        "The '!vault' tag requires a string value.",
    ),
    ("a: !vault\n", "The '!vault' tag requires a string value."),
    (
        "a: !!int abc\n",
        "invalid literal for int() with base 10: 'abc'",
    ),
    (
        "a: !!float abc\n",
        "could not convert string to float: 'abc'",
    ),
    ("a: !!bool 5\n", "'5'"),
    ("a: !!bool maybe\n", "'maybe'"),
    (
        "a: !!timestamp abc\n",
        "'NoneType' object has no attribute 'groupdict'",
    ),
    (
        "a: !!seq 5\n",
        "Expected a sequence node, but found scalar.",
    ),
    ("a: !!map 5\n", "Expected a mapping node, but found scalar."),
    (
        "a: !!merge x\n",
        "Could not determine a constructor for the tag 'tag:yaml.org,2002:merge'.",
    ),
    (
        "a: !!value x\n",
        "Could not determine a constructor for the tag 'tag:yaml.org,2002:value'.",
    ),
    (
        "a: !!yaml x\n",
        "Could not determine a constructor for the tag 'tag:yaml.org,2002:yaml'.",
    ),
    (
        "a: !!python/str x\n",
        "Could not determine a constructor for the tag 'tag:yaml.org,2002:python/str'.",
    ),
    (
        "a: !!python/object:os.system x\n",
        "Could not determine a constructor for the tag 'tag:yaml.org,2002:python/object:os.system'.",
    ),
    (
        "a: !!unknown x\n",
        "Could not determine a constructor for the tag 'tag:yaml.org,2002:unknown'.",
    ),
    (
        "a: !custom x\n",
        "Could not determine a constructor for the tag '!custom'.",
    ),
    (
        "a: !custom {a: 1}\n",
        "Could not determine a constructor for the tag '!custom'.",
    ),
    (
        "a: !custom [1]\n",
        "Could not determine a constructor for the tag '!custom'.",
    ),
    (
        "a: !<tag:example.com,2000:x> y\n",
        "Could not determine a constructor for the tag 'tag:example.com,2000:x'.",
    ),
    ("a: !!int\n", "string index out of range"),
    ("a: !!bool\n", "''"),
    ("a: !!set 5\n", "Expected a mapping node, but found scalar."),
    (
        "a: !!omap 5\n",
        "While constructing an ordered map expected a sequence, but found scalar.",
    ),
    (
        "a: !!pairs 5\n",
        "While constructing pairs expected a sequence, but found scalar.",
    ),
    (
        "a: !!python/dict 5\n",
        "Expected a mapping node, but found scalar.",
    ),
    ("a: !!set x\n", "Expected a mapping node, but found scalar."),
    (
        "a: !!seq ''\n",
        "Expected a sequence node, but found scalar.",
    ),
    (
        "a: !!map ''\n",
        "Expected a mapping node, but found scalar.",
    ),
    ("a: !!int ''\n", "string index out of range"),
    ("a: !!float ''\n", "string index out of range"),
    ("a: !!int 0b\n", "invalid literal for int() with base 2: ''"),
    (
        "a: !!int 0x\n",
        "invalid literal for int() with base 16: ''",
    ),
    (
        "a: !!int 08\n",
        "invalid literal for int() with base 8: '08'",
    ),
    (
        "a: !!int 1:x\n",
        "invalid literal for int() with base 10: 'x'",
    ),
    (
        "a: !!float ABC\n",
        "could not convert string to float: 'abc'",
    ),
    ("a: !!float 1:x\n", "could not convert string to float: 'x'"),
    ("a: !!bool Maybe\n", "'maybe'"),
    ("a: !!bool ''\n", "''"),
    ("a: !!timestamp 2020-13-45\n", "month must be in 1..12"),
    (
        "a: !!timestamp 2020-02-30\n",
        "day is out of range for month",
    ),
    (
        "a: !!timestamp '2020-01-02 25:00:00'\n",
        "hour must be in 0..23",
    ),
    (
        "a: !!timestamp '2020-01-02 10:61:00'\n",
        "minute must be in 0..59",
    ),
    (
        "a: !!timestamp 20-1-2\n",
        "'NoneType' object has no attribute 'groupdict'",
    ),
    (
        "a: !!timestamp '2020-01-02 10:11:12 +24:00'\n",
        "offset must be a timedelta strictly between -timedelta(hours=24) and timedelta(hours=24), not datetime.timedelta(days=1).",
    ),
    ("a: !unsafe |\n  yes\n", "'yes\\n'"),
    (
        "a: !vault \"5\"\n",
        "The '!vault' tag requires a string value.",
    ),
    (
        "a: !vault ''\n",
        "The '!vault' tag requires a string value.",
    ),
    (
        "a: !vault |\n  5\n",
        "The '!vault' tag requires a string value.",
    ),
    ("a: !vault |\n  yes\n", "'yes\\n'"),
];

#[test]
fn a_scalar_pyyaml_cannot_build_stops_the_load_with_its_message() {
    for (text, message) in REFUSED {
        let err = load(text, "t.yml").unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            format!("t.yml: invalid YAML: {message}"),
            "{text:?}"
        );
    }
}

/// The spellings of infinity and not-a-number that PyYAML reads as floats, and the near misses
/// it leaves as strings.
#[test]
fn the_non_finite_floats_load_as_the_reference_loads_them() {
    for (text, want) in [
        (".inf", Some(f64::INFINITY)),
        (".Inf", Some(f64::INFINITY)),
        ("+.inf", Some(f64::INFINITY)),
        ("-.INF", Some(f64::NEG_INFINITY)),
        ("!!float -.nan", Some(f64::NAN)),
        (".NaN", Some(f64::NAN)),
        ("-.nan", None),
        ("inf", None),
        ("nan", None),
    ] {
        let source = format!("x: {text}\n");
        let docs = load(&source, "t.yml").unwrap();
        let node = field(&docs[0], "x").unwrap();
        match want {
            Some(want) => {
                let got = node.as_floating_point().expect(text);
                assert!(got == want || (got.is_nan() && want.is_nan()), "{text}");
            }
            None => assert!(node.as_str().is_some(), "{text}"),
        }
    }
}

#[test]
fn vault_and_unsafe_text_load_as_data() {
    let docs = load(
        "secret: !vault |\n  $ANSIBLE_VAULT;1.1;AES256\n  3132\n",
        "t.yml",
    )
    .unwrap();
    let loaded = to_json(&docs[0]).unwrap();
    let wrapped = loaded["secret"].as_str().unwrap();
    assert_eq!(
        vault_text(wrapped).as_deref(),
        Some("$ANSIBLE_VAULT;1.1;AES256\n3132\n")
    );
    assert_eq!(unsafe_text(wrapped), None);
    let docs = load(
        "raw: !unsafe 'a \"{{ x }}\" \\ b'\nl: [!unsafe '{{ y }}', 3, plain]\nn: !unsafe [{k: '{{ z }}'}, 1]\n",
        "t.yml",
    )
    .unwrap();
    assert_eq!(
        to_json(&docs[0]).unwrap(),
        json!({
            "raw": r#"{{ "a \"{{ x }}\" \\ b" | volant_unsafe }}"#,
            "l": [r#"{{ "{{ y }}" | volant_unsafe }}"#, 3, "plain"],
            "n": [{"k": r#"{{ "{{ z }}" | volant_unsafe }}"#}, 1],
        })
    );
}

#[test]
fn unsafe_text_escapes_and_reads_back_whatever_it_holds() {
    for text in [
        "{{ a }} }} \"q\"",
        "{% if x %}",
        "{{ a }}\\",
        "{{ \u{1f600} }}",
        "{{ a\u{85}b }}\n\r\t",
    ] {
        let wrapped = unsafe_data(Value::String(text.into()));
        assert_eq!(
            unsafe_text(wrapped.as_str().unwrap()).as_deref(),
            Some(text)
        );
    }
    for not_a_wrapper in [
        "{{ \"a\" | volant_unsafe }} {{ b }}",
        "{{ \"a\"b\" | volant_unsafe }}",
        "{{ \"a\\qb\" | volant_unsafe }}",
    ] {
        assert_eq!(unsafe_text(not_a_wrapper), None, "{not_a_wrapper:?}");
    }
}

/// Scalars after multi-byte characters keep the typing their source text gives them.
#[test]
fn scalars_after_multibyte_text_are_typed_from_their_own_source() {
    let docs = load(
        "# é 😀\nnom: «é»\nflag: yes\nmode: 010\nbig: 1_000\n",
        "t.yml",
    )
    .unwrap();
    assert_eq!(
        to_json(&docs[0]).unwrap(),
        json!({"nom": "«é»", "flag": true, "mode": 8, "big": 1000})
    );
}

/// A document of 20000 mappings loads in time that grows with its size. The bound is far above
/// what a linear load takes, and far below what a quadratic one takes.
#[test]
fn a_document_of_twenty_thousand_mappings_loads() {
    let mut text = String::from("---\n");
    for i in 0..20_000 {
        let port = 8000 + i % 1000;
        writeln!(
            text,
            "service_{i}:\n  name: web-{i}\n  port: {port}\n  enabled: yes"
        )
        .unwrap();
        writeln!(text, "  tags: [a, \"b {i}\"]").unwrap();
    }
    let started = std::time::Instant::now();
    let docs = load(&text, "t.yml").unwrap();
    let took = started.elapsed();
    let value = to_json(&docs[0]).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 20_000);
    assert_eq!(value["service_19999"]["enabled"], json!(true));
    assert!(took < std::time::Duration::from_secs(60), "took {took:?}");
}

#[test]
fn syntax_errors_name_the_source() {
    let err = load("a: [1, 2\n", "broken.yml").unwrap_err();
    assert!(format!("{err:#}").contains("broken.yml"));
}

#[test]
fn missing_fields_are_none_not_panics() {
    let docs = load("a: 1\n", "t.yml").unwrap();
    assert!(field(&docs[0], "zzz").is_none());
    assert!(field(&docs[0], "a").is_some());
}

/// Each row is a document and the JSON ansible-core 2.19.12 loads it as, keys in its order: the
/// merged pairs first, an explicit key taking the place of the merged key it overrides.
#[test]
fn a_merge_key_merges_as_pyyaml_does() {
    let rows: &[(&str, &str)] = &[
        (
            "base: &b {a: 1, b: 2}\nd: {<<: *b, c: 3}\n",
            r#"{"base":{"a":1,"b":2},"d":{"a":1,"b":2,"c":3}}"#,
        ),
        (
            "base: &b {a: 1, b: 2}\nd: {c: 3, <<: *b}\n",
            r#"{"base":{"a":1,"b":2},"d":{"a":1,"b":2,"c":3}}"#,
        ),
        (
            "base: &b {a: 1, b: 2}\nd: {a: 9, <<: *b}\n",
            r#"{"base":{"a":1,"b":2},"d":{"a":9,"b":2}}"#,
        ),
        (
            "base: &b {a: 1, b: 2}\nd: {<<: *b, a: 9}\n",
            r#"{"base":{"a":1,"b":2},"d":{"a":9,"b":2}}"#,
        ),
        (
            "x: &x {a: 1, k: x}\ny: &y {a: 2, b: 2, k: y}\nd: {<<: [*x, *y], z: 0}\n",
            r#"{"x":{"a":1,"k":"x"},"y":{"a":2,"b":2,"k":"y"},"d":{"a":1,"b":2,"k":"x","z":0}}"#,
        ),
        (
            "x: &x {a: 1, k: x}\ny: &y {a: 2, b: 2, k: y}\nd: {<<: [*y, *x], z: 0}\n",
            r#"{"x":{"a":1,"k":"x"},"y":{"a":2,"b":2,"k":"y"},"d":{"a":2,"k":"y","b":2,"z":0}}"#,
        ),
        (
            "a: &a {p: 1}\nb: &b {<<: *a, q: 2}\nd: {<<: *b, r: 3}\n",
            r#"{"a":{"p":1},"b":{"p":1,"q":2},"d":{"p":1,"q":2,"r":3}}"#,
        ),
        (
            "x: &x {a: 1}\ny: &y {<<: *x, b: 2}\nd: {<<: [*y], c: 1}\n",
            r#"{"x":{"a":1},"y":{"a":1,"b":2},"d":{"a":1,"b":2,"c":1}}"#,
        ),
        // Two merge keys in one mapping: the later one wins.
        (
            "x: &x {a: 1}\ny: &y {b: 2}\nd:\n  <<: *x\n  <<: *y\n  c: 3\n",
            r#"{"x":{"a":1},"y":{"b":2},"d":{"a":1,"b":2,"c":3}}"#,
        ),
        (
            "x: &x {a: 1, b: 2}\nd: {<<: *x, <<: {b: 3, c: 4}}\n",
            r#"{"x":{"a":1,"b":2},"d":{"a":1,"b":3,"c":4}}"#,
        ),
        // A key written twice keeps its last value, also next to a merge key.
        ("d: {a: 1, a: 2}\n", r#"{"d":{"a":2}}"#),
        (
            "x: &x {a: 1}\nd: {a: 5, a: 6, <<: *x}\n",
            r#"{"x":{"a":1},"d":{"a":6}}"#,
        ),
        // The merge source can be written in place, as a block, or as a tagged mapping.
        (
            "d: {<<: {a: 1, b: 2}, c: 3}\n",
            r#"{"d":{"a":1,"b":2,"c":3}}"#,
        ),
        ("d:\n  <<:\n    a: 1\n  c: 3\n", r#"{"d":{"a":1,"c":3}}"#),
        ("d: {<<: !!map {b: 2}}\n", r#"{"d":{"b":2}}"#),
        (
            "x: &x [{a: 1}, {b: 2}]\nd: {<<: *x, c: 3}\n",
            r#"{"x":[{"a":1},{"b":2}],"d":{"b":2,"a":1,"c":3}}"#,
        ),
        // Nothing to merge, and a merge key at the top of the document.
        ("x: &x {}\nd: {<<: *x}\n", r#"{"x":{},"d":{}}"#),
        ("d: {<<: [], c: 1}\n", r#"{"d":{"c":1}}"#),
        (
            "base: &b {a: 1}\n<<: *b\nz: 1\n",
            r#"{"a":1,"base":{"a":1},"z":1}"#,
        ),
        (
            "- &a {x: 1}\n- {<<: *a, y: 2}\n",
            r#"[{"x":1},{"x":1,"y":2}]"#,
        ),
        // An explicit key, and a tagged one.
        (
            "x: &x {a: 1}\nd:\n  ? <<\n  : *x\n",
            r#"{"x":{"a":1},"d":{"a":1}}"#,
        ),
        (
            "x: &x {a: 1}\nd: {!!merge '<<': *x}\n",
            r#"{"x":{"a":1},"d":{"a":1}}"#,
        ),
        // Quoted, the key is text.
        (
            "x: &x {a: 1}\nd: {'<<': *x}\n",
            r#"{"x":{"a":1},"d":{"<<":{"a":1}}}"#,
        ),
        (
            "x: &x {a: 1}\nd: {\"<<\": *x}\n",
            r#"{"x":{"a":1},"d":{"<<":{"a":1}}}"#,
        ),
        // The characters `<<` elsewhere are no merge key.
        (
            "cmd: |\n  cat <<EOF\n  EOF\nb: {k: 1}\n",
            r#"{"cmd":"cat <<EOF\nEOF\n","b":{"k":1}}"#,
        ),
    ];
    for (text, want) in rows {
        let docs = load(text, "t.yml").unwrap();
        let got = serde_json::to_string(&to_json(&docs[0]).unwrap()).unwrap();
        assert_eq!(&got, want, "{text:?}");
    }
}

/// A merge source marked `!unsafe` stays data in the mapping that merges it: ansible-core 2.19.12
/// does not render `{{ q }}` in `d.a`.
#[test]
fn a_merged_unsafe_mapping_stays_data() {
    let text = "x: &x !unsafe {a: '{{ q }}', n: {m: '{{ q }}'}}\nd: {<<: *x, c: '{{ q }}'}\n";
    let docs = load(text, "t.yml").unwrap();
    let d = to_json(&docs[0]).unwrap()["d"].clone();
    assert_eq!(
        unsafe_text(d["a"].as_str().unwrap()).as_deref(),
        Some("{{ q }}")
    );
    assert_eq!(
        unsafe_text(d["n"]["m"].as_str().unwrap()).as_deref(),
        Some("{{ q }}")
    );
    assert_eq!(d["c"], json!("{{ q }}"));
}

/// A merge key whose value is not a mapping or a list of mappings stops the load with the
/// message of PyYAML's constructor, naming the mapping and the value.
#[test]
fn a_bad_merge_value_is_a_constructor_error() {
    let rows: &[(&str, &str)] = &[
        (
            "d: {<<: 1, c: 2}\n",
            "while constructing a mapping\n  in \"t.yml\", line 1, column 4\nexpected a mapping or list of mappings for merging, but found scalar\n  in \"t.yml\", line 1, column 9",
        ),
        (
            "d:\n  <<: [1]\n",
            "while constructing a mapping\n  in \"t.yml\", line 2, column 3\nexpected a mapping for merging, but found scalar\n  in \"t.yml\", line 2, column 8",
        ),
        (
            "x: &x {a: 1}\nd: {<<: [*x, 2]}\n",
            "while constructing a mapping\n  in \"t.yml\", line 2, column 4\nexpected a mapping for merging, but found scalar\n  in \"t.yml\", line 2, column 14",
        ),
        (
            "d: {<<: [[{a: 1}]]}\n",
            "while constructing a mapping\n  in \"t.yml\", line 1, column 4\nexpected a mapping for merging, but found sequence\n  in \"t.yml\", line 1, column 10",
        ),
        // An alias names the node it points at.
        (
            "a: &s foo\nd: {<<: *s}\n",
            "while constructing a mapping\n  in \"t.yml\", line 2, column 4\nexpected a mapping or list of mappings for merging, but found scalar\n  in \"t.yml\", line 1, column 4",
        ),
        (
            "a: &s\n  - 1\nd: {<<: [*s]}\n",
            "while constructing a mapping\n  in \"t.yml\", line 3, column 4\nexpected a mapping for merging, but found sequence\n  in \"t.yml\", line 1, column 4",
        ),
        // Columns count characters, not bytes.
        (
            "é: 1\na: &s foo\nd: {é: 2, <<: *s}\n",
            "while constructing a mapping\n  in \"t.yml\", line 3, column 4\nexpected a mapping or list of mappings for merging, but found scalar\n  in \"t.yml\", line 2, column 4",
        ),
        (
            "é: 1\nd: {<<: [1, é]}\n",
            "while constructing a mapping\n  in \"t.yml\", line 2, column 4\nexpected a mapping for merging, but found scalar\n  in \"t.yml\", line 2, column 10",
        ),
        // A tag counts as part of the node too.
        (
            "d: {<<: !!str foo}\n",
            "while constructing a mapping\n  in \"t.yml\", line 1, column 4\nexpected a mapping or list of mappings for merging, but found scalar\n  in \"t.yml\", line 1, column 9",
        ),
        (
            "a: !!str &s foo\nd: {<<: *s}\n",
            "while constructing a mapping\n  in \"t.yml\", line 2, column 4\nexpected a mapping or list of mappings for merging, but found scalar\n  in \"t.yml\", line 1, column 4",
        ),
        (
            "<<: 1\n",
            "while constructing a mapping\n  in \"t.yml\", line 1, column 1\nexpected a mapping or list of mappings for merging, but found scalar\n  in \"t.yml\", line 1, column 5",
        ),
    ];
    for (text, want) in rows {
        let err = format!("{:#}", load(text, "t.yml").unwrap_err());
        assert_eq!(err, format!("t.yml: invalid YAML: {want}"), "{text:?}");
    }
}

/// The positions of mappings stay available when a document holds a merge key.
#[test]
fn origins_survive_a_merge_key() {
    let text = "- &a {x: 1}\n- {<<: *a, y: 2}\n";
    let (docs, origins) = load_with_origins(text, "t.yml").unwrap();
    let items = docs[0].as_vec().unwrap();
    let Yaml::Mapping(second) = &items[1] else {
        panic!("a mapping");
    };
    assert_eq!(origins.of(second), Some((2, 3)));
    let value = field(&items[1], "y").unwrap();
    assert_eq!(origins.of_value(value), Some((2, 15)));
}
