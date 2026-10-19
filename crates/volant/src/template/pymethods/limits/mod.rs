// SPDX-License-Identifier: GPL-3.0-or-later
//! The size a text built by a method may reach.
//!
//! A width, a precision or a count in a template can come from a host (`'a'.ljust(r.stdout)`). Python
//! tries to build the text and stops with a `MemoryError` when the machine cannot. Volant stops
//! earlier, with a task error that names the limit, before it allocates.

use minijinja::value::Rest;
use minijinja::{Error, State, Value};

use super::args::{Call, raise};

/// The most characters one method may build: 256 MiB of text. Real templates are far below it.
pub(crate) const MAX_CHARS: u128 = 256 * 1024 * 1024;

/// `Ok` when a result of `chars` characters is within the limit.
pub(crate) fn check(chars: u128) -> Result<usize, Error> {
    match usize::try_from(chars) {
        Ok(n) if chars <= MAX_CHARS => Ok(n),
        _ => Err(raise(format!(
            "the result would hold {chars} characters: Volant builds at most {MAX_CHARS} (256 MiB)"
        ))),
    }
}

/// The `indent` filter, which adds `width` blanks to each line: the result is checked first.
pub(crate) fn indent(state: &State, args: Rest<Value>) -> Result<Value, Error> {
    let call = Call::new(args.get(1..).unwrap_or_default());
    let width = call
        .pos
        .first()
        .or_else(|| call.kw.iter().find(|(k, _)| k == "width").map(|(_, v)| v));
    if let (Some(text), Some(width)) = (args.first().and_then(Value::as_str), width) {
        let each = match width.as_str() {
            Some(blanks) => blanks.chars().count() as u128,
            None => i128::try_from(width.clone()).map_or(0, |n| u128::try_from(n).unwrap_or(0)),
        };
        let lines = text.matches('\n').count() as u128 + 1;
        check(text.len() as u128 + lines * each)?;
    }
    Value::from_function(minijinja::filters::indent).call(state, &args)
}

#[cfg(test)]
mod tests;
