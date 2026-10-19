// SPDX-License-Identifier: GPL-3.0-or-later
//! Questions about compiled code that decide how [`super::vm::route_calls`] rewrites it.

use minijinja::machinery::{Instruction, Instructions};

use super::pymethods::mutable::MUTATORS;
use super::{CONST_KEY, MKLIST_KEY};

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
