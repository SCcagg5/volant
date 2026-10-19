// SPDX-License-Identifier: GPL-3.0-or-later
//! One pass over a string, as the reference's `Templar.template` does it.

use minijinja::machinery::WhitespaceConfig;
use serde_json::Value;

use super::file::trailing_newlines;
use super::vm::{Node, convert_error, render_counted, single_expression};
use super::{Templar, TemplateError};

impl Templar {
    /// Whether the reference would render `text`: it holds `{{`, `{%` or `{#`. A substring test,
    /// because every string a render reads goes through it.
    pub fn is_template(text: &str) -> bool {
        let mut rest = text;
        while let Some(at) = rest.find('{') {
            if matches!(rest.as_bytes().get(at + 1), Some(b'{' | b'%' | b'#')) {
                return true;
            }
            rest = &rest[at + 1..];
        }
        false
    }

    /// Whether `text` holds a statement block or a comment, the two tags that print nothing and so
    /// can leave a result with one node (or none) where the text around them is not a node.
    fn has_statement(text: &str) -> bool {
        text.contains("{%") || text.contains("{#")
    }

    /// One pass over a string, as the reference's `Templar.template` does it: Jinja2 with
    /// `trim_blocks` on and `lstrip_blocks` off, one trailing line end dropped before the lexer
    /// runs and put back on a string result, and the output nodes joined into a string unless
    /// there is one node (its value, with its type) or none (`None`).
    pub(super) fn render_once(
        &self,
        text: &str,
        ctx: &minijinja::Value,
    ) -> Result<Value, TemplateError> {
        if !Self::is_template(text) {
            return Ok(Value::String(text.to_string()));
        }
        // Whitespace around the expression is text: the reference keeps it, so the result is a
        // string. One line end after it is not: the lexer drops it.
        let body = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text);
        if body.trim() == body
            && let Some(expr) = single_expression(body)
        {
            let value = self.evaluate_in(expr, ctx)?;
            return Ok(with_line_ends(value, text));
        }
        // The lexer reads a lone `\r` and `\r\n` as `\n`.
        let source = if text.contains('\r') {
            std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            std::borrow::Cow::Borrowed(text)
        };
        let ws = WhitespaceConfig {
            keep_trailing_newline: false,
            lstrip_blocks: false,
            trim_blocks: true,
        };
        let (out, emitted) =
            render_counted(self, "<string>", &source, ctx, ws).map_err(convert_error)?;
        // Without a statement, a comment or a `none` every `{{ }}` is a node of its own and the
        // text between is one too, so one node means the text is one expression, handled above.
        if Self::has_statement(text) || emitted.printed_none() {
            match emitted.single(&out) {
                Node::None => return Ok(Value::Null),
                Node::Value(value) => {
                    let value = serde_json::to_value(&value).map_err(|e| {
                        TemplateError(format!("cannot convert template result: {e}"))
                    })?;
                    return Ok(with_line_ends(value, text));
                }
                Node::Text => {}
            }
        }
        Ok(with_line_ends(Value::String(out), text))
    }
}

/// A string result gets back the line ends the lexer dropped from the end of `template`: as many
/// as the template had beyond the ones the result has, as the reference counts them.
fn with_line_ends(mut value: Value, template: &str) -> Value {
    if let Value::String(text) = &mut value {
        let wanted = trailing_newlines(template);
        let got = trailing_newlines(text);
        if wanted > got {
            text.extend(std::iter::repeat_n('\n', wanted - got));
        }
    }
    value
}
