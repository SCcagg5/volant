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
        "{# c #}",
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

/// The lookup this module used before it had a table: it walks the text from the start.
fn char_to_byte_by_walking(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

/// The offset table gives the offset a walk from the start gives, at every character of a text
/// and past both ends of it.
#[test]
fn the_offset_table_matches_a_walk_from_the_start() {
    let long_ascii = "key: value\n".repeat(500);
    let long_mixed = "clé: «valeur» 日本語 😀\r\n".repeat(200);
    for text in [
        "",
        "a",
        "\n",
        "\r\n",
        "a: 1\r\nb: 2\r\n",
        "plain ascii: yes\n",
        "é",
        "é: 1\n",
        "x: é\n",
        "日本語: 日本語\n",
        "emoji: 😀😀 end\n",
        "😀",
        "a\u{0}b\u{7f}c",
        "tab\there \u{a0} nbsp \u{feff}bom",
        long_ascii.as_str(),
        long_mixed.as_str(),
    ] {
        let offsets = Offsets::new(text);
        let count = text.chars().count();
        for chars in (0..=count + 3).chain([usize::MAX]) {
            assert_eq!(
                offsets.byte(chars),
                char_to_byte_by_walking(text, chars),
                "{text:?} at character {chars}"
            );
        }
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
