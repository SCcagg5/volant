// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn parsed(text: &str) -> Result<Option<(String, FileRender)>, String> {
    split(text, &FileRender::default()).map(|o| o.map(|(body, o)| (body.to_string(), o)))
}

/// A text that does not begin with the marker, byte for byte, has no overrides.
#[test]
fn only_a_first_line_that_begins_with_the_marker_counts() {
    for text in [
        "x\n#jinja2: trim_blocks: False\n",
        " #jinja2: trim_blocks: False\n",
        "#Jinja2: trim_blocks: False\n",
        "",
    ] {
        assert_eq!(parsed(text), Ok(None), "{text:?}");
    }
}

/// The options a line sets, as the reference reads them: pairs split on commas, then on the first
/// colon, values read as Python literals, the last of a repeated key kept.
#[test]
fn a_header_sets_the_options_it_names() {
    let (body, o) = parsed(
        "#jinja2: variable_start_string:'[%', variable_end_string:\"%]\", trim_blocks: False,\
         lstrip_blocks :  True , keep_trailing_newline: True, newline_sequence: '\\r\\n'\nrest\n",
    )
    .unwrap()
    .unwrap();
    assert_eq!(body, "rest\n");
    assert_eq!(
        (
            o.syntax.variable_start.as_str(),
            o.syntax.variable_end.as_str()
        ),
        ("[%", "%]")
    );
    assert!(!o.trim_blocks && o.lstrip_blocks && o.keep_trailing_newline);
    assert_eq!(o.newline_sequence, "\r\n");

    let (_, o) = parsed("#jinja2: trim_blocks: False, trim_blocks: True\n")
        .unwrap()
        .unwrap();
    assert!(o.trim_blocks);
    let (_, o) = parsed("#jinja2:lstrip_blocks:True\r\n").unwrap().unwrap();
    assert!(o.lstrip_blocks);
}

/// The values a Python literal can spell a string with.
#[test]
fn a_string_is_read_as_python_reads_it() {
    for (value, want) in [
        ("'[['", "[["),
        ("\"<<\"", "<<"),
        ("('[[')", "[["),
        ("'[' '['", "[["),
        ("'\\x5b\\x5b'", "[["),
        ("'\\u00ab'", "\u{ab}"),
        ("'\\t['", "\t["),
        ("'\\101'", "A"),
        ("r'\\['", "\\["),
        ("\"\"\"[[\"\"\"", "[["),
        ("'a\\qb'", "a\\qb"),
    ] {
        let (_, o) = parsed(&format!("#jinja2: variable_start_string: {value}\n"))
            .unwrap()
            .unwrap();
        assert_eq!(o.syntax.variable_start, want, "{value}");
    }
}

/// Each row is the text `ansible-playbook` printed after `Syntax error in template: ` for the
/// header; the ones that carry a Python object's address in the reference carry none here.
#[test]
fn a_header_the_reference_refuses_is_refused_in_its_words() {
    for (text, err) in [
        (
            "#jinja2: bogus: True\n",
            "Invalid '#jinja2:' override key 'bogus'.",
        ),
        (
            "#jinja2: trim_blocks\n",
            "Missing key-value separator `:` in '#jinja2:' override pair ' trim_blocks'.",
        ),
        (
            "#jinja2: trim_blocks: False,\n",
            "Empty '#jinja2:' override pair not allowed.",
        ),
        ("#jinja2:\n", "Empty '#jinja2:' override pair not allowed."),
        (
            "#jinja2: trim_blocks: False",
            "Missing newline after '#jinja2:' override.",
        ),
        (
            "#jinja2: trim_blocks: nope\n",
            "malformed node or string on line 1",
        ),
        (
            "#jinja2: trim_blocks: true\n",
            "malformed node or string on line 1",
        ),
        (
            "#jinja2: trim_blocks: 'maybe'\n",
            "TemplateOverrides.trim_blocks must be <class 'bool'> instead of <class 'str'>",
        ),
        (
            "#jinja2: trim_blocks: 1\n",
            "TemplateOverrides.trim_blocks must be <class 'bool'> instead of <class 'int'>",
        ),
        (
            "#jinja2: variable_start_string: 5\n",
            "TemplateOverrides.variable_start_string must be <class 'str'> instead of <class 'int'>",
        ),
        (
            "#jinja2: variable_start_string: -1\n",
            "TemplateOverrides.variable_start_string must be <class 'str'> instead of <class 'int'>",
        ),
        (
            "#jinja2: variable_start_string: None\n",
            "TemplateOverrides.variable_start_string must be <class 'str'> instead of <class 'NoneType'>",
        ),
        (
            "#jinja2: line_statement_prefix: 5\n",
            "TemplateOverrides.line_statement_prefix must be <class 'str'> or <class 'NoneType'> instead of <class 'int'>",
        ),
        (
            "#jinja2: line_statement_prefix: ['a']\n",
            "TemplateOverrides.line_statement_prefix must be <class 'str'> or <class 'NoneType'> instead of <class 'list'>",
        ),
        (
            "#jinja2: newline_sequence: 'x'\n",
            "TemplateOverrides.newline_sequence must be one of ('\\n', '\\r\\n', '\\r') instead of 'x'",
        ),
        (
            "#jinja2: variable_start_string:'<,', variable_end_string:',>'\n",
            "unterminated string literal (detected at line 1) (<unknown>, line 1)",
        ),
        (
            "#jinja2: variable_start_string:'{%'\n",
            "Block, variable and comment start strings must be different.",
        ),
    ] {
        assert_eq!(
            parsed(text),
            Err(format!("Syntax error in template: {err}")),
            "{text:?}"
        );
    }
}
