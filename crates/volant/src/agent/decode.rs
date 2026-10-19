// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading a message of the agent from its frame, inside the bounds a host's message keeps.
//!
//! A frame may be 64 MiB, which a `slurp` of a large file needs. What a frame costs once parsed
//! depends on its values, not on its bytes, so a result is also bounded by [`shape`]: one over
//! the bound becomes a failed task of its own host, and the run goes on. A message that is not
//! a result (a log line, a blob answer) has no use for a large frame and is cut short.

use std::io;

use volant_protocol::shape::{self, Head};
use volant_protocol::{FromAgent, TaskResult};

/// A frame of this many bytes or fewer holds fewer values than the bound, so it is parsed at once.
/// Nearly every frame is this small: the hot path reads nothing twice.
const UNMEASURED: usize = 64 * 1024;

/// The longest frame of a message that is not a task result.
const MAX_MESSAGE_LEN: usize = 1024 * 1024;

pub(super) fn decode(payload: &[u8]) -> io::Result<FromAgent> {
    if payload.len() <= UNMEASURED {
        return Ok(serde_json::from_slice(payload)?);
    }
    let over = shape::check(payload).err();
    if over.is_none() && payload.len() <= MAX_MESSAGE_LEN {
        return Ok(serde_json::from_slice(payload)?);
    }
    let header = Head::of(payload);
    let is_result = header.as_ref().is_some_and(|h| h.kind == "task_result");
    let len = payload.len();
    match (over, header) {
        (None, Some(h)) if !is_result => Err(refusal(format!(
            "the agent sent a {} message of {len} bytes, over the {MAX_MESSAGE_LEN} byte limit \
             of a message that is not a task result",
            h.kind
        ))),
        (None, _) => Ok(serde_json::from_slice(payload)?),
        (Some(excess), Some(h)) if is_result => Ok(FromAgent::TaskResult {
            batch: h.batch,
            index: h.index,
            result: TaskResult::failed_with(format!(
                "the result of the host is refused: it has {excess} ({len} bytes in its frame)"
            )),
            ran: None,
        }),
        (Some(excess), _) => Err(refusal(format!(
            "the agent sent a message of {len} bytes with {excess}"
        ))),
    }
}

fn refusal(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

#[cfg(test)]
mod tests;
