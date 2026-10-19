// SPDX-License-Identifier: GPL-3.0-or-later
//! One render with no second pass, and whether it read data.

use std::sync::atomic::Ordering;

use serde_json::Value;

use super::{Templar, TemplateError, Vars, context_of};

impl Templar {
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
