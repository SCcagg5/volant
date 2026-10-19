// SPDX-License-Identifier: GPL-3.0-or-later
use crate::testing::{scratch, templar};
use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::*;

/// A managed host's value lands in the file as text and is never rendered again.
#[test]
fn a_host_value_in_a_template_is_written_as_text() {
    let marker_guard = scratch();
    let marker = marker_guard.path().join("marker");
    let payload = format!("{{{{ lookup('pipe', 'touch {}') }}}}", marker.display());
    let mut map = Map::new();
    map.insert("motd".into(), Value::String(payload.clone()));
    let untrusted: BTreeSet<String> = ["motd".to_string()].into();
    let vars = Vars {
        map: &map,
        hostvars: None,
        shared: None,
        facts: None,
        untrusted: Some(&untrusted),
        untrusted_hosts: None,
    };
    let out = templar()
        .render_file("banner: {{ motd }}\n", vars, &FileRender::default())
        .unwrap();
    assert_eq!(out, format!("banner: {payload}\n"));
    assert!(
        !marker.exists(),
        "the host's template ran on the controller"
    );
}

/// Even an author value is not rendered twice: `{% raw %}` writes its braces, measured on
/// ansible-core 2.19.12 (`{{ literal }}` in the rendered file).
#[test]
fn a_rendered_file_is_never_rendered_again() {
    let out = templar()
        .render_file(
            "{% raw %}{{ literal }}{% endraw %}\n",
            &Map::new(),
            &FileRender::default(),
        )
        .unwrap();
    assert_eq!(out, "{{ literal }}\n");
}

const OPTS: &str = "top\n  {% if true %}\n  in\n  {% endif %}\nend\n";

/// The four shapes measured on the reference with this very template.
#[test]
fn the_template_module_s_options_are_the_reference_s() {
    let r = |o: FileRender| templar().render_file(OPTS, &Map::new(), &o).unwrap();
    assert_eq!(r(FileRender::default()), "top\n    in\n  end\n");
    assert_eq!(
        r(FileRender {
            lstrip_blocks: true,
            ..Default::default()
        }),
        "top\n  in\nend\n"
    );
    assert_eq!(
        r(FileRender {
            trim_blocks: false,
            ..Default::default()
        }),
        "top\n  \n  in\n  \nend\n"
    );
    assert_eq!(
        r(FileRender {
            newline_sequence: "\r\n".into(),
            ..Default::default()
        }),
        "top\r\n    in\r\n  end\r\n"
    );
}

/// The final newline of the file survives, and a template that renders to nothing but it writes
/// it: measured, a role's `sshd_config_snippet.j2` with its `if` false is one byte, `\n`.
#[test]
fn the_final_newline_is_kept() {
    let r = |t: &str| {
        templar()
            .render_file(t, &Map::new(), &FileRender::default())
            .unwrap()
    };
    assert_eq!(r("{% if false %}x{% endif %}\n"), "\n");
    assert_eq!(r("a"), "a");
    assert_eq!(r("a\n\n"), "a\n\n");
    assert_eq!(r("\n\n"), "\n\n");
    assert_eq!(r("{% if true %}x{% endif %}\n\n"), "x\n\n");
    assert_eq!(r("{% raw %}a\n{% endraw %}\n\n"), "a\n\n");
}

/// Jinja2's lexer rewrites every `\r\n` and lone `\r` in the source to `\n` before it ever sees
/// the template; minijinja does not.
#[test]
fn the_source_s_own_crlf_is_normalised_like_the_reference() {
    assert_eq!(
        templar()
            .render_file("{{ 1 }} x\r\ny\r\n", &Map::new(), &FileRender::default())
            .unwrap(),
        "1 x\ny\n"
    );
    assert_eq!(
        templar()
            .render_file(
                "{{ 1 }} x\r\ny\r\n",
                &Map::new(),
                &FileRender {
                    newline_sequence: "\r\n".into(),
                    ..Default::default()
                }
            )
            .unwrap(),
        "1 x\r\ny\r\n"
    );
    // The reference counts trailing newlines on the raw text, not the normalised copy: a
    // trailing `\r\n\r\n` is one `\n` run broken by a `\r`, not two.
    assert_eq!(
        templar()
            .render_file("{{ 1 }}x\r\n\r\n", &Map::new(), &FileRender::default())
            .unwrap(),
        "1x\n"
    );
}

fn syntax(parts: [&str; 6]) -> Syntax {
    let [
        variable_start,
        variable_end,
        block_start,
        block_end,
        comment_start,
        comment_end,
    ] = parts.map(String::from);
    Syntax {
        variable_start,
        variable_end,
        block_start,
        block_end,
        comment_start,
        comment_end,
        ..Syntax::default()
    }
}

/// Every row was written by `template` of ansible-core 2.19.12 with `x: val`, `items: [1, 2]` and
/// the delimiters of the row; the other four stay `{{ }}`, `{% %}`, `{# #}`.
#[test]
#[expect(clippy::too_many_lines)]
fn custom_delimiters_read_a_template_as_the_reference_does() {
    const D: [&str; 6] = ["{{", "}}", "{%", "%}", "{#", "#}"];
    let vars = serde_json::json!({"x": "val", "items": [1, 2]});
    let Value::Object(vars) = vars else {
        unreachable!()
    };
    let all = syntax(["[[", "]]", "[%", "%]", "[#", "#]"]);
    let vs = syntax(["[[", "]]", "{%", "%}", "{#", "#}"]);
    for (text, syn, want) in [
        (
            "<! x }} and {{ x }}\n",
            syntax(["<!", D[1], D[2], D[3], D[4], D[5]]),
            "val and {{ x }}\n",
        ),
        (
            "{% if true %>yes{% endif %>\n",
            syntax([D[0], D[1], D[2], "%>", D[4], D[5]]),
            "yes\n",
        ),
        (
            "a<# c #>b{# d #}c\n",
            syntax([D[0], D[1], D[2], D[3], "<#", D[5]]),
            "ac\n",
        ),
        (
            "a{# c #>b\n",
            syntax([D[0], D[1], D[2], D[3], D[4], "#>"]),
            "ab\n",
        ),
        (
            "[[ x ]] [% if true %]y[% endif %] [# c #] {{ x }} {% if 1 %}z{% endif %}\n",
            all.clone(),
            "val y  {{ x }} {% if 1 %}z{% endif %}\n",
        ),
        (
            "<!! x !!> @@ if true @@ y @@ endif @@\n",
            syntax(["<!!", "!!>", "@@", "@@", D[4], D[5]]),
            "val  y \n",
        ),
        ("[[ x ]] [[ [1,2][0] ]]\n", vs.clone(), "val 1\n"),
        (
            "[[ items[0]]] [[ {'a': [1]}['a'][0]]]\n",
            vs.clone(),
            "1 1\n",
        ),
        (
            "$( (1+2)*3 ) $( x|upper ) {{ x }}\n",
            syntax(["$(", ")", D[2], D[3], D[4], D[5]]),
            "9 VAL {{ x }}\n",
        ),
        (
            "<% if true %>y<% endif %> {{ x }}\n",
            syntax([D[0], D[1], "<%", "%>", D[4], D[5]]),
            "y val\n",
        ),
        (
            "a{* c *}b {# not comment #}\n",
            syntax([D[0], D[1], D[2], D[3], "{*", "*}"]),
            "ab {# not comment #}\n",
        ),
        (
            "{# if true #}y{# endif #} {* c *}\n",
            syntax([D[0], D[1], "{#", "#}", "{*", "*}"]),
            "y \n",
        ),
        (
            "v=${ {'a': {'b': 2}}['a']['b'] } ${ x }\n",
            syntax(["${", "}", D[2], D[3], D[4], D[5]]),
            "v=2 val\n",
        ),
        (
            "[[ \"]]\" ]] [[ 'a]]b' ]] [[ '[[' ]] [[ \"{{\" ]]\n",
            vs.clone(),
            "]] a]]b [[ {{\n",
        ),
        ("[% set s = '%]' %][[ s ]]\n", all.clone(), "%]\n"),
        (
            "[% raw %][[ x ]] [% if %] [% endraw %] [[ x ]]\n",
            all.clone(),
            "[[ x ]] [% if %]  val\n",
        ),
        ("{% raw %}[[ x ]]{% endraw %}\n", vs.clone(), "[[ x ]]\n"),
        (
            "{{ x }} [[ x ]] {% if 1 %}q{% endif %} {# c #}\n",
            vs.clone(),
            "{{ x }} val q \n",
        ),
        (
            "a\n  [%- if true -%]\n  b\n  [%- endif -%]\n  c [[- x -]]  d\n",
            all.clone(),
            "abcvald\n",
        ),
        (
            "a\n[% if true %]\nb\n[% endif %]\nc\n",
            all.clone(),
            "a\nb\nc\n",
        ),
        (
            "[% for i in items %][[ i ]],[% endfor %]\n",
            all.clone(),
            "1,2,\n",
        ),
        (
            "[[ nope | default('d') ]] [[ x | upper ]]\n",
            vs.clone(),
            "d VAL\n",
        ),
        ("[[x]][[ x]][[x ]]\n", vs.clone(), "valvalval\n"),
        (
            "<!! x !!> ,\n",
            syntax(["<!!", "!!>", D[2], D[3], D[4], D[5]]),
            "val ,\n",
        ),
        (
            "\\BLOCK{for i in items}\\VAR{i},\\BLOCK{endfor}\\#{c}\n",
            syntax(["\\VAR{", "}", "\\BLOCK{", "}", "\\#{", "}"]),
            "1,2,\n",
        ),
    ] {
        let options = FileRender {
            syntax: syn,
            ..Default::default()
        };
        assert_eq!(
            templar().render_file(text, &vars, &options).unwrap(),
            want,
            "{text:?}"
        );
    }
}

/// A start string the text does not hold makes it plain text: measured, it is written as it is,
/// line ends and a missing final newline included, and `newline_sequence` does not touch it.
#[test]
fn a_text_without_a_start_string_is_written_as_it_is() {
    let vs = FileRender {
        syntax: syntax(["[[", "]]", "[%", "%]", "[#", "#]"]),
        newline_sequence: "\r".into(),
        ..Default::default()
    };
    let r = |t: &str, o: &FileRender| templar().render_file(t, &Map::new(), o).unwrap();
    assert_eq!(r("a\r\nb\r\n", &FileRender::default()), "a\r\nb\r\n");
    assert_eq!(r("a\rb\r", &FileRender::default()), "a\rb\r");
    assert_eq!(r("{{ x }}\r\n{# c #}", &vs), "{{ x }}\r\n{# c #}");
    assert_eq!(r("plain\n\n\n", &FileRender::default()), "plain\n\n\n");
}

/// The newline sequence of a `#jinja2:` line turns the template's own line ends; the ones the
/// reference appends to keep the final newline are the task's.
#[test]
fn a_header_s_newline_sequence_and_the_task_s_are_two_things() {
    let r = |t: &str, seq: &str| {
        let o = FileRender {
            newline_sequence: seq.into(),
            ..Default::default()
        };
        templar().render_file(t, &Map::new(), &o).unwrap()
    };
    assert_eq!(
        r("#jinja2: newline_sequence:'\\r\\n'\n{{ 1 }}\ny\n", "\n"),
        "1\r\ny\n"
    );
    assert_eq!(
        r("#jinja2: newline_sequence:'\\n'\n{{ 1 }}\ny\n", "\r\n"),
        "1\ny\r\n"
    );
    // The reference counts only `\n` at the end, so a `\r` sequence appends one per newline.
    assert_eq!(
        r("a\n{% if 1 %}\nb\n{% endif %}\nc\n\n", "\r"),
        "a\rb\rc\r\r\r"
    );
}

/// A line prefix is refused: minijinja and Jinja2 disagree about the lines around a line statement
/// and about a line comment, measured on ansible-core 2.19.12.
#[test]
fn line_statements_and_comments_are_refused() {
    for header in ["line_statement_prefix: '#'", "line_comment_prefix: '##'"] {
        let err = templar()
            .render_file(
                &format!("#jinja2: {header}\nx\n"),
                &Map::new(),
                &FileRender::default(),
            )
            .unwrap_err();
        let name = header.split(':').next().unwrap();
        assert_eq!(
            err.0,
            format!("Syntax error in template: {name} is not supported yet")
        );
    }
}

/// What the lexer cannot read is refused by name, never rendered wrong: an empty string, and an
/// end string that begins with whitespace (minijinja skips whitespace before it looks for it).
#[test]
fn delimiters_the_lexer_cannot_use_are_refused_by_name() {
    for (syn, err) in [
        (
            syntax(["{{", " }}", "{%", "%}", "{#", "#}"]),
            "variable_end_string must not begin with whitespace",
        ),
        (
            syntax(["{{", "}}", "{%", " %]", "{#", "#}"]),
            "block_end_string must not begin with whitespace",
        ),
        (
            syntax(["{{", "}}", "{%", "%}", "", "#}"]),
            "comment_start_string must not be an empty string",
        ),
    ] {
        let options = FileRender {
            syntax: syn,
            ..Default::default()
        };
        let got = templar().render_file("{{ 1 }}", &Map::new(), &options);
        assert_eq!(
            got.unwrap_err().0,
            format!("Syntax error in template: {err}")
        );
    }
}
