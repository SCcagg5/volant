// SPDX-License-Identifier: GPL-3.0-or-later
//! What a YAML text may ask of the loader: how deep its collections nest, how many documents it
//! holds, and how much its anchors and aliases copy.
//!
//! The parser's own `load` calls itself once for each level, and so do the conversions that
//! follow it, so a text with a few thousand levels overflows the stack of the thread that reads
//! it. The loader also copies a node at each anchor and at each alias, so a text of a few hundred
//! bytes can stand for gigabytes. [`feed`] reads the parser's events one at a time and refuses a
//! text before the loader builds what it asks for.

use std::collections::HashMap;

use saphyr_parser::{Event, Marker, Parser, ScanError, SpannedEventReceiver};

/// The deepest nesting of collections a document may have. The scanner already stops a flow
/// collection at 255 levels, and a block collection gets the same limit.
///
/// ansible-core 2.19.12 loads a file down to 489 levels and stops it at 490 with `maximum
/// recursion depth exceeded`; its `from_yaml` reads a text far deeper than that. Volant stops
/// earlier on purpose. Converting a document takes about 3.6 KB of stack for each level in a
/// debug build, so 489 levels leave a 2 MiB thread with almost nothing, and no real document
/// comes close to 255.
pub(super) const MAX_DEPTH: usize = 255;

/// The most nodes anchors and aliases may copy in one text. The loader keeps a copy of every
/// anchored node and makes another at every alias. A text that holds no alias, or that keeps to
/// a few shared mappings, is far below this.
pub(super) const MAX_COPIED_NODES: usize = 1_000_000;

/// The most bytes of scalar text anchors and aliases may copy in one text. A scalar is one node
/// whatever its length, and each copy of it is made again by the conversions that follow, so the
/// text is counted too: 64 MiB of copies take about 200 MB of memory, as much as the node limit
/// does. A certificate of 2 KB shared by 1000 hosts is 2 MB.
pub(super) const MAX_COPIED_BYTES: usize = 64 * 1024 * 1024;

/// How much a node holds: the nodes in it, itself included, the bytes of its scalars, and how many
/// collections deep its longest path goes (`0` for a scalar).
#[derive(Clone, Copy)]
struct Size {
    nodes: usize,
    bytes: usize,
    height: usize,
}

/// A collection that is still open: the node it will be, once it closes.
struct Open {
    /// Its anchor, `0` when it has none.
    anchor: usize,
    size: Size,
}

/// What the events seen so far say about the size of the text.
#[derive(Default)]
struct Guard {
    documents: usize,
    open: Vec<Open>,
    /// The size of each anchored node, or `None` while it is open.
    anchored: HashMap<usize, Option<Size>>,
    copied_nodes: usize,
    copied_bytes: usize,
}

impl Guard {
    /// Takes the next event into account, or says why the text is refused.
    fn admit(&mut self, event: &Event<'_>, at: Marker) -> Result<(), ScanError> {
        match event {
            Event::DocumentStart(_) => {
                // PyYAML, and so every loader of ansible-core, stops at the second document.
                // Refusing it here keeps a stream from adding up to more than one text may.
                self.documents += 1;
                if self.documents > 1 {
                    return Err(ScanError::new_str(
                        at,
                        "expected a single document in the stream but found another document",
                    ));
                }
            }
            Event::SequenceStart(anchor, _) | Event::MappingStart(anchor, _) => {
                if self.open.len() >= MAX_DEPTH {
                    return Err(too_deep(at));
                }
                if *anchor > 0 {
                    self.anchored.insert(*anchor, None);
                }
                self.open.push(Open {
                    anchor: *anchor,
                    size: Size {
                        nodes: 1,
                        bytes: 0,
                        height: 1,
                    },
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                if let Some(done) = self.open.pop() {
                    if done.anchor > 0 {
                        self.anchored.insert(done.anchor, Some(done.size));
                        self.copy(done.size, at)?;
                    }
                    self.child(done.size);
                }
            }
            Event::Scalar(text, _, anchor, _) => {
                let size = Size {
                    nodes: 1,
                    bytes: text.len(),
                    height: 0,
                };
                if *anchor > 0 {
                    self.anchored.insert(*anchor, Some(size));
                    self.copy(size, at)?;
                }
                self.child(size);
            }
            Event::Alias(anchor) => {
                // An alias of a collection that is still open names a node that does not exist
                // yet: the loader gives it a placeholder, which counts as one node.
                let size = match self.anchored.get(anchor) {
                    Some(Some(size)) => *size,
                    Some(None) => Size {
                        nodes: 1,
                        bytes: 0,
                        height: 0,
                    },
                    None => {
                        return Err(ScanError::new_str(
                            at,
                            "while parsing node, found unknown anchor",
                        ));
                    }
                };
                if self.open.len() + size.height > MAX_DEPTH {
                    return Err(too_deep(at));
                }
                self.copy(size, at)?;
                self.child(size);
            }
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {}
        }
        Ok(())
    }

    /// Adds a finished node to the collection that holds it.
    fn child(&mut self, node: Size) {
        if let Some(parent) = self.open.last_mut() {
            parent.size.nodes = parent.size.nodes.saturating_add(node.nodes);
            parent.size.bytes = parent.size.bytes.saturating_add(node.bytes);
            parent.size.height = parent.size.height.max(node.height + 1);
        }
    }

    fn copy(&mut self, node: Size, at: Marker) -> Result<(), ScanError> {
        self.copied_nodes = self.copied_nodes.saturating_add(node.nodes);
        self.copied_bytes = self.copied_bytes.saturating_add(node.bytes);
        if self.copied_nodes > MAX_COPIED_NODES {
            return Err(ScanError::new(
                at,
                format!(
                    "the anchors and aliases of the text copy more than {MAX_COPIED_NODES} nodes"
                ),
            ));
        }
        if self.copied_bytes > MAX_COPIED_BYTES {
            return Err(ScanError::new(
                at,
                format!(
                    "the anchors and aliases of the text copy more than {MAX_COPIED_BYTES} bytes"
                ),
            ));
        }
        Ok(())
    }
}

fn too_deep(at: Marker) -> ScanError {
    ScanError::new(
        at,
        format!("maximum recursion depth exceeded: collections nest more than {MAX_DEPTH} levels"),
    )
}

/// Parses `text` into `receiver`, as `Parser::load` does for a stream of documents, and refuses
/// a text that nests too deep, holds a second document, or copies too much before the receiver
/// sees the event that goes over the limit.
pub(super) fn feed<'a, R: SpannedEventReceiver<'a>>(
    text: &'a str,
    receiver: &mut R,
) -> Result<(), ScanError> {
    let mut guard = Guard::default();
    for event in Parser::new_from_str(text) {
        let (event, span) = event?;
        guard.admit(&event, span.start)?;
        receiver.on_event(event, span);
    }
    Ok(())
}
