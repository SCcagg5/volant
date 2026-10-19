// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading a message of the agent from its frame, inside the bounds a host's message keeps.
//!
//! A frame may be 64 MiB, which a `slurp` of a large file needs. What a frame costs once parsed
//! depends on its values, not on its bytes, so every frame is counted by [`shape`] before it is
//! parsed. A result over a bound becomes a failed task of its own host, and the run goes on. A
//! message that is not a result (a log line, a blob answer) has no use for a large frame and is
//! cut short. A frame whose type cannot be read is not a result.

use std::io;

use volant_protocol::shape::{self, Head};
use volant_protocol::{FromAgent, TaskResult};

/// The longest frame of a message that is not a task result.
const MAX_MESSAGE_LEN: usize = 1024 * 1024;

/// Whether the controller waits for the answer of task `index` of batch `batch`.
pub(super) type Pending<'a> = &'a dyn Fn(u64, usize) -> bool;

pub(super) fn decode(payload: &[u8], pending: Pending<'_>) -> io::Result<FromAgent> {
    let len = payload.len();
    match shape::check(payload) {
        Ok(_) if len <= MAX_MESSAGE_LEN => read(payload),
        Ok(_) => match Head::of(payload) {
            Some(head) if head.kind == "task_result" => read(payload),
            head => Err(refusal(format!(
                "the agent sent {} message of {len} bytes, over the {MAX_MESSAGE_LEN} byte \
                 limit of a message that is not a task result",
                head.map_or_else(|| "a".to_string(), |h| format!("a {}", h.kind)),
            ))),
        },
        Err(excess) => refuse(
            payload,
            &format!("it has {excess} ({len} bytes in its frame)"),
            pending,
        ),
    }
}

/// The answer to a frame that is not parsed, for the reason `why`: a failed task when the frame
/// is a result for a task the controller waits for, so that its host fails alone, and an
/// unreadable frame otherwise. A result that names no task, or one nobody waits for, would be
/// dropped as a late answer and leave the run waiting.
fn refuse(payload: &[u8], why: &str, pending: Pending<'_>) -> io::Result<FromAgent> {
    if let Some(head) = Head::of(payload)
        && head.kind == "task_result"
        && let (Some(batch), Some(index)) = (head.batch, head.index)
        && pending(batch, index)
    {
        return Ok(FromAgent::TaskResult {
            batch,
            index,
            result: TaskResult::failed_with(format!("the result of the host is refused: {why}")),
            ran: None,
        });
    }
    Err(refusal(format!(
        "the agent sent a message of {} bytes that no task waits for and that is refused: {why}",
        payload.len()
    )))
}

fn read(payload: &[u8]) -> io::Result<FromAgent> {
    Ok(serde_json::from_slice(payload)?)
}

fn refusal(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

#[cfg(test)]
mod tests;
