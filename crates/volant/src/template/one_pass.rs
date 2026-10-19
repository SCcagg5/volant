// SPDX-License-Identifier: GPL-3.0-or-later
//! One render with no second pass, and whether it read data.

use std::sync::atomic::Ordering;

use minijinja::machinery::WhitespaceConfig;
use serde_json::Value;

use super::context::take_failure;
use super::vm::{convert_error, render_str, single_expression};
use super::{Templar, TemplateError, Vars, context_of};

impl Templar {
    /// One pass over `text`: a result that is itself a template is left as it is.
    pub(super) fn render_once(
        &self,
        text: &str,
        ctx: &minijinja::Value,
    ) -> Result<Value, TemplateError> {
        if !Self::is_template(text) {
            return Ok(Value::String(text.to_string()));
        }
        // Whitespace around the expression, a final newline included, is text: the reference
        // keeps it, so the result is a string.
        if text.trim() == text
            && let Some(expr) = single_expression(text)
        {
            return self.evaluate_in(expr, ctx);
        }
        let ws = WhitespaceConfig {
            keep_trailing_newline: true,
            ..WhitespaceConfig::default()
        };
        let rendered = render_str(self, "<string>", text, ctx, ws)
            .map(Value::String)
            .map_err(convert_error);
        take_failure(ctx).map_or(rendered, Err)
    }

    /// `render_one_pass`, saying as well whether the pass read a name that is data: a value from
    /// a managed host, or a name the caller marked. The result is what the one pass made and
    /// nothing renders it again, so text an author built out of braces stays text.
    pub(crate) fn render_one_pass_tainted<'a>(
        &self,
        text: &str,
        vars: impl Into<Vars<'a>>,
    ) -> Result<(Value, bool), TemplateError> {
        let (ctx, tainted) = context_of(self, vars.into());
        let value = self.render_once(text, &ctx)?;
        Ok((value, tainted.load(Ordering::Relaxed)))
    }
}
