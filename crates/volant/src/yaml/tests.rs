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

/// A document that nests collections `depth` levels deep, in one of the shapes the reference was
/// measured with. The per-line shapes grow with the square of `depth`, so a table keeps their
/// depth small.
fn nested(shape: &str, depth: usize) -> String {
    let per_line = |item: &dyn Fn(usize) -> String| -> String {
        let mut text = String::new();
        for level in 0..depth {
            text.push_str(&" ".repeat(level));
            text.push_str(&item(level));
            text.push('\n');
        }
        text.push_str(&" ".repeat(depth));
        text.push_str("x\n");
        text
    };
    match shape {
        "block sequence" => format!("{}x\n", "- ".repeat(depth)),
        "block sequence per line" => per_line(&|_| "-".into()),
        "block mapping" => per_line(&|_| "k:".into()),
        "mixed block" => per_line(&|level| if level % 2 == 0 { "-" } else { "k:" }.into()),
        "flow sequence" => format!("{}x{}\n", "[".repeat(depth), "]".repeat(depth)),
        "flow mapping" => format!("{}x{}\n", "{a: ".repeat(depth), "}".repeat(depth)),
        "block then flow" => {
            let flow = depth - depth / 2;
            format!(
                "{}{}x{}\n",
                "- ".repeat(depth / 2),
                "[".repeat(flow),
                "]".repeat(flow)
            )
        }
        other => panic!("no shape {other}"),
    }
}

const SHAPES: [&str; 7] = [
    "block sequence",
    "block sequence per line",
    "block mapping",
    "mixed block",
    "flow sequence",
    "flow mapping",
    "block then flow",
];

/// How many collections deep `value` goes, counted without recursion.
fn json_depth(value: &Value) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(value, 0)];
    while let Some((value, depth)) = stack.pop() {
        match value {
            Value::Array(items) => items.iter().for_each(|v| stack.push((v, depth + 1))),
            Value::Object(map) => map.values().for_each(|v| stack.push((v, depth + 1))),
            _ => deepest = deepest.max(depth),
        }
    }
    deepest
}

/// A document may nest 489 levels: ansible-core 2.19.12 loads a file down to that level
/// (`-e @file`, `vars_files` and the playbook itself, every shape below) and stops it at 490 with
/// `maximum recursion depth exceeded`. The scanner stops a flow collection at 255 levels, a limit
/// of its own that the reference does not have, so a deeper flow collection is refused here.
#[test]
fn nesting_deeper_than_the_reference_loads_is_refused_not_a_stack_overflow() {
    for shape in SHAPES {
        // A block document written one level to a line grows with the square of its depth.
        let per_line =
            shape.contains("per line") || matches!(shape, "block mapping" | "mixed block");
        for depth in [200, 255, 256, 313, 314, 315, 488, 489, 490, 1000, 100_000] {
            if per_line && depth > 1000 {
                continue;
            }
            let loads = depth <= 489 && !(shape.starts_with("flow") && depth > 255);
            let text = nested(shape, depth);
            let loaded = load(&text, "deep.yml");
            if loads {
                let docs = loaded.unwrap_or_else(|e| panic!("{shape} {depth}: {e:#}"));
                assert_eq!(
                    json_depth(&to_json(&docs[0]).unwrap()),
                    depth,
                    "{shape} {depth}"
                );
            } else {
                let err = loaded
                    .err()
                    .unwrap_or_else(|| panic!("{shape} {depth} loaded"));
                let err = format!("{err:#}");
                assert!(
                    err.contains("deep.yml") && err.contains("recursion"),
                    "{shape} {depth}: {err}"
                );
            }
        }
    }
}

/// An alias chain nests as deep as the nodes it names, though the text stays short.
#[test]
fn an_alias_chain_counts_toward_the_nesting_limit() {
    let chain = |levels: usize| {
        let mut text = String::from("a0: &a0 [x]\n");
        for i in 1..levels {
            let _ = writeln!(text, "a{i}: &a{i} [*a{}]", i - 1);
        }
        text
    };
    // The mapping that holds the chain is the first level.
    for (levels, loads) in [(100, true), (488, true), (489, false), (3000, false)] {
        let text = chain(levels);
        let loaded = load(&text, "chain.yml");
        assert_eq!(loaded.is_ok(), loads, "{levels}: {:?}", loaded.err());
        if let Some(err) = loaded.err() {
            assert!(
                format!("{err:#}").contains("maximum recursion depth exceeded"),
                "{err:#}"
            );
        }
    }
}

/// How many scalars `value` holds, counted without recursion.
fn leaf_count(value: &Value) -> usize {
    let (mut leaves, mut stack) = (0, vec![value]);
    while let Some(value) = stack.pop() {
        match value {
            Value::Array(items) => stack.extend(items),
            Value::Object(map) => stack.extend(map.values()),
            _ => leaves += 1,
        }
    }
    leaves
}

/// Each level holds nine aliases of the one before: `levels` of them make a document of a few
/// hundred bytes that stands for `9^levels` nodes.
fn alias_bomb(levels: usize) -> String {
    let mut text = String::from("a0: &a0 [x,x,x,x,x,x,x,x,x]\n");
    for i in 1..levels {
        let aliases = vec![format!("*a{}", i - 1); 9].join(",");
        let _ = writeln!(text, "a{i}: &a{i} [{aliases}]");
    }
    text
}

/// Anchors inside anchors, `levels` deep with `width` scalars at each level. The loader keeps a
/// copy of every anchored node, so the copies add up to about `levels^2 * width / 2` nodes.
fn nested_anchors(levels: usize, width: usize) -> String {
    let row = vec!["x"; width].join(",");
    let mut text = String::new();
    for i in 0..levels {
        let _ = write!(text, "[&a{i} [{row},");
    }
    text.push('x');
    text.push_str(&"]]".repeat(levels));
    text.push('\n');
    text
}

/// Aliases and anchors copy nodes, and the copies are bounded: a document of a few hundred bytes
/// must not stand for gigabytes. The limit is 1000000 copied nodes. A document that copies less
/// loads whole.
#[test]
fn aliases_and_anchors_copy_a_bounded_number_of_nodes() {
    // The 341-byte document that used up 3.6 GB at six levels and was killed at seven.
    let refused = [
        ("billion laughs, 7 levels", alias_bomb(7)),
        ("billion laughs, 9 levels", alias_bomb(9)),
        // Two flow levels to each anchor, so 60 anchors stay under the flow limit of 255.
        ("nested anchors", nested_anchors(60, 1000)),
    ];
    for (what, text) in refused {
        let err = load(&text, "bomb.yml")
            .err()
            .unwrap_or_else(|| panic!("{what} loaded"));
        let err = format!("{err:#}");
        assert!(
            err.contains("bomb.yml") && err.contains("1000000"),
            "{what}: {err}"
        );
    }
    for (what, text, leaves) in [
        (
            "billion laughs, 4 levels",
            alias_bomb(4),
            9 + 81 + 729 + 6561,
        ),
        ("nested anchors", nested_anchors(20, 20), 20 * 20 + 1),
    ] {
        let docs = load(&text, "ok.yml").unwrap_or_else(|e| panic!("{what}: {e:#}"));
        assert_eq!(leaf_count(&to_json(&docs[0]).unwrap()), leaves, "{what}");
    }
}

/// A file that uses aliases the way an inventory does keeps loading, with the values the
/// aliases name.
#[test]
fn a_file_that_reuses_a_few_anchors_loads_whole() {
    let mut text =
        String::from("common: &common {ntp: [a, b, c], dns: {x: 1, y: 2}, tags: &tags [t1, t2]}\n");
    for host in 0..300 {
        let _ = writeln!(text, "h{host}: {{vars: *common, tags: *tags, id: {host}}}");
    }
    let docs = load(&text, "inv.yml").unwrap();
    let json = to_json(&docs[0]).unwrap();
    assert_eq!(json["h299"]["vars"]["dns"], json!({"x": 1, "y": 2}));
    assert_eq!(json["h7"]["tags"], json!(["t1", "t2"]));
}

/// A stream holds one document, as in ansible-core 2.19.12, whose loaders all stop at the second
/// one with `expected a single document in the stream`. The limits on nesting and on copies are
/// for the whole text, so a stream of documents cannot add up to more than one document may.
#[test]
fn a_stream_holds_one_document() {
    for (what, text) in [
        ("two documents", "---\na: 1\n---\nb: 2\n".to_string()),
        ("a trailing marker", "a: 1\n---\n".to_string()),
        ("empty documents", "---\n---\n".to_string()),
        ("many empty documents", "---\n".repeat(100_000)),
        (
            "three alias bombs",
            format!("---\n{0}---\n{0}---\n{0}", alias_bomb(5)),
        ),
    ] {
        let err = load(&text, "stream.yml")
            .err()
            .unwrap_or_else(|| panic!("{what} loaded"));
        let err = format!("{err:#}");
        assert!(
            err.contains("stream.yml") && err.contains("single document"),
            "{what}: {err}"
        );
    }
    for text in [
        "---\na: 1\n",
        "# c\n---\na: 1\n",
        "a: 1\n...\n",
        "---\n",
        "",
    ] {
        assert!(load(text, "one.yml").is_ok(), "{text:?}");
    }
}

/// A scalar that an alias names is copied with its text, so the bytes count too. Ten aliases of
/// a one-megabyte scalar load, and eighty exceed the limit of 64 MiB, in every scalar style.
#[test]
fn aliases_of_a_big_scalar_are_counted_by_their_bytes() {
    let text = "x".repeat(1_000_000);
    let styles = [
        ("plain", format!("a: &a {text}\n")),
        ("single-quoted", format!("a: &a '{text}'\n")),
        ("double-quoted", format!("a: &a \"{text}\\n\"\n")),
        ("literal", format!("a: &a |\n  {text}\n")),
        (
            "multi-line plain",
            format!(
                "a: &a {}y\n",
                format!("{}\n  ", "x".repeat(99)).repeat(10_000)
            ),
        ),
    ];
    for (style, anchor) in styles {
        for (aliases, loads) in [(10, true), (80, false)] {
            let doc = format!("{anchor}b: [{}]\n", vec!["*a"; aliases].join(","));
            let loaded = load(&doc, "big.yml");
            assert_eq!(
                loaded.is_ok(),
                loads,
                "{style} x {aliases}: {:?}",
                loaded.err()
            );
            if let Some(err) = loaded.err() {
                let err = format!("{err:#}");
                assert!(
                    err.contains("big.yml") && err.contains("67108864 bytes"),
                    "{style}: {err}"
                );
            }
        }
    }
}

/// `!unsafe` inside `!unsafe` wraps a string once. Wrapping again at every level doubled the
/// string each time, so a document of a few hundred bytes took gigabytes.
#[test]
fn nested_unsafe_wraps_a_string_once() {
    let once = unsafe_data(Value::String("{{".into()));
    for levels in [1, 2, 3, 32, 254] {
        let text = format!(
            "{}\"{{{{\"{}\n",
            "{a: !unsafe ".repeat(levels),
            "}".repeat(levels)
        );
        let docs = load(&text, "unsafe.yml").unwrap();
        let mut value = to_json(&docs[0]).unwrap();
        for _ in 0..levels {
            value = value["a"].take();
        }
        assert_eq!(value, once, "{levels} levels");
    }
}

/// The deepest document converts on three quarters of the 2 MiB stack of a worker thread, in a
/// debug build, where a frame is largest. Each shape is one that costs more than a plain level:
/// a tag on every level, and block and flow mixed.
#[test]
fn a_document_at_the_limit_converts_on_a_small_stack() {
    let mut tagged = String::new();
    for level in 0..489 {
        let _ = writeln!(tagged, "{}- !unsafe", " ".repeat(level));
    }
    tagged.push_str(&" ".repeat(489));
    tagged.push_str("x\n");
    let shapes = [
        ("plain", "- ".repeat(489) + "x\n"),
        ("tagged", tagged),
        (
            "mixed",
            "- ".repeat(244) + &"[".repeat(245) + "x" + &"]".repeat(245) + "\n",
        ),
    ];
    for (what, text) in shapes {
        let worker = std::thread::Builder::new()
            .stack_size(1_572_864)
            .spawn(move || {
                let docs = load(&text, "deep.yml").unwrap();
                let json = to_json(&docs[0]).unwrap();
                assert!(json_depth(&json) >= 489, "{what}");
            });
        worker
            .unwrap()
            .join()
            .unwrap_or_else(|_| panic!("{what} failed"));
    }
}
