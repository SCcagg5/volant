// SPDX-License-Identifier: GPL-3.0-or-later
//! The size of a JSON document in values and in nesting, counted from its bytes before anything
//! is built from it.
//!
//! A frame is bounded in bytes, but a byte is not what a document costs once it is parsed: `0,`
//! is two bytes on the wire and about 300 bytes of memory as an element of an array. The count
//! here is what a frame from a host has to stay under before the controller parses it.

use std::fmt;

use serde::Deserialize;

/// The most values (strings, numbers, literals, arrays, objects and object keys) one message
/// from a host may hold. A `setup`, `package_facts` or `service_facts` result is tens of
/// thousands, a `find` of a large tree a few hundred thousand. A result at the bound costs about 150 MB.
pub const MAX_VALUES: usize = 500_000;

/// The deepest nesting one message from a host may have. Real results nest ten levels or fewer;
/// `serde_json` itself stops at 128.
pub const MAX_DEPTH: usize = 64;

/// The fields a message of either side starts from: its type and, for a result, which task it
/// answers. Read while the rest of the document is skipped without being built.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Head {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub batch: u64,
    #[serde(default)]
    pub index: usize,
}

impl Head {
    /// `None` when the document is not a JSON object with a string `type`, or nests deeper than
    /// `serde_json` reads.
    pub fn of(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// What [`measure`] counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub values: usize,
    pub depth: usize,
}

/// Why a document is over a bound, with the figure that was counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Excess {
    Values(usize),
    Depth(usize),
}

impl fmt::Display for Excess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Values(seen) => write!(f, "{seen} values, over the limit of {MAX_VALUES}"),
            Self::Depth(seen) => {
                write!(f, "{seen} levels of nesting, over the limit of {MAX_DEPTH}")
            }
        }
    }
}

/// Counts the values and the deepest nesting of a JSON document without building it. Every
/// string, number, literal, array, object and object key is one value. The count of a document
/// that is not valid JSON is whatever its prefix holds: the parser refuses it afterwards.
pub fn measure(bytes: &[u8]) -> Shape {
    let (mut values, mut depth, mut deepest) = (0usize, 0usize, 0usize);
    let mut in_scalar = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                values += 1;
                in_scalar = false;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => break,
                        _ => i += 1,
                    }
                }
            }
            b'{' | b'[' => {
                values += 1;
                depth += 1;
                deepest = deepest.max(depth);
                in_scalar = false;
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                in_scalar = false;
            }
            b',' | b':' | b' ' | b'\t' | b'\r' | b'\n' => in_scalar = false,
            _ if !in_scalar => {
                values += 1;
                in_scalar = true;
            }
            _ => {}
        }
        i += 1;
    }
    Shape {
        values,
        depth: deepest,
    }
}

/// `Ok` with the shape when the document is inside both bounds, else what is over.
pub fn check(bytes: &[u8]) -> Result<Shape, Excess> {
    let shape = measure(bytes);
    if shape.values > MAX_VALUES {
        return Err(Excess::Values(shape.values));
    }
    if shape.depth > MAX_DEPTH {
        return Err(Excess::Depth(shape.depth));
    }
    Ok(shape)
}

#[cfg(test)]
mod tests;
