// SPDX-License-Identifier: GPL-3.0-or-later
//! Questions about a template's text or an expression's compiled operations, which decide how
//! [`super::vm`] renders it.

use minijinja::machinery::{Instruction, Instructions};

use super::pymethods::mutable::MUTATORS;
use super::{CONST_KEY, MKLIST_KEY};

/// `{{ expr }}` and nothing else: no text around, no second expression, no statement. A `}}`
/// inside is no end when it closes a bracket the expression opened or sits in a string literal:
/// `{'a': {'x': 5}}` closing on `}}}` and `'}}'` are one expression, `1 }}{{ 2` is two. A `-`
/// right after `{{` or right before `}}` is whitespace control, never a minus sign.
pub(super) fn single_expression(text: &str) -> Option<&str> {
    let t = text.trim();
    let inner = t.strip_prefix("{{")?.strip_suffix("}}")?;
    if !closes_only_what_it_opens(inner) {
        return None;
    }
    let inner = inner.strip_prefix('-').unwrap_or(inner);
    let inner = inner.strip_suffix('-').unwrap_or(inner);
    Some(inner.trim())
}

/// Whether every bracket `expr` closes outside a string literal is one it opened, and no
/// statement or comment opens there. A `{%` inside a string literal is text. minijinja's own
/// parser cannot be asked: its lexer panics on a `}` with nothing open.
fn closes_only_what_it_opens(expr: &str) -> bool {
    let mut depth = 0usize;
    let mut quote = None;
    let mut chars = expr.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, '{') if matches!(chars.peek(), Some('%' | '#')) => return false,
            (None, '{' | '[' | '(') => depth += 1,
            (None, '}' | ']' | ')') => match depth.checked_sub(1) {
                Some(d) => depth = d,
                None => return false,
            },
            _ => {}
        }
    }
    true
}

/// Whether an instruction pushes a list written out in the template, `(a, b)` or `['a', 'b']`: what
/// stands on the right of `%` as a tuple does. A list that a variable holds, or a call answers, is
/// one argument, as it is in Python.
pub(super) fn writes_a_tuple(op: Option<&Instruction<'_>>) -> bool {
    match op {
        Some(Instruction::BuildList(Some(_))) => true,
        Some(Instruction::LoadConst(value)) => value.kind() == minijinja::value::ValueKind::Seq,
        // The same two, after the rewrite of a template that changes lists in place.
        Some(Instruction::CallFunction(name, _)) => {
            *name == MKLIST_KEY
                || name
                    .strip_prefix(CONST_KEY)
                    .is_some_and(|json| json.starts_with('['))
        }
        _ => false,
    }
}

/// Whether compiled code calls a method that changes a list or mapping in place.
pub(super) fn mutates(instructions: &Instructions<'_>) -> bool {
    let mut at = 0;
    while let Some(op) = instructions.get(at) {
        if let Instruction::CallMethod(name, _) = op
            && MUTATORS.contains(name)
        {
            return true;
        }
        at += 1;
    }
    false
}

/// Whether `text` holds a `{% raw %}` tag, with or without whitespace control.
pub(super) fn opens_raw(text: &str) -> bool {
    let mut rest = text;
    while let Some(at) = rest.find("{%") {
        rest = &rest[at + 2..];
        let tag = rest.trim_start_matches(['-', '+']).trim_start();
        if let Some(after) = tag.strip_prefix("raw")
            && after.starts_with(|c: char| c.is_whitespace() || matches!(c, '-' | '+' | '%'))
        {
            return true;
        }
    }
    false
}
