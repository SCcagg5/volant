// SPDX-License-Identifier: GPL-3.0-or-later
//! What a YAML document may ask of the loader: how deep its collections nest, and how many nodes
//! its anchors and aliases copy.
//!
//! The parser's own `load` calls itself once for each level, and so do the conversions that
//! follow it, so a document with a few thousand levels overflows the stack of the thread that
//! reads it. The loader also copies a node at each anchor and at each alias, so a document of a
//! few hundred bytes can stand for gigabytes. [`feed`] reads the parser's events one at a time
//! and refuses a document before the loader builds what it asks for.

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

/// The most nodes anchors and aliases may copy in one document. The loader keeps a copy of every
/// anchored node and makes another at every alias. A document that holds no alias, or that keeps
/// to a few shared mappings, is far below this.
pub(super) const MAX_COPIED_NODES: usize = 1_000_000;

/// A collection that is still open: the node it will be, once it closes.
struct Open {
    /// Its anchor, `0` when it has none.
    anchor: usize,
    /// The nodes in it so far, itself included.
    nodes: usize,
    /// How many collections deep the longest path below it goes, itself included.
    height: usize,
}

/// What the events seen so far say about the size of the document.
#[derive(Default)]
struct Guard {
    open: Vec<Open>,
    /// The size of each anchored node: its node count and its height, or `None` while it is open.
    anchored: HashMap<usize, Option<(usize, usize)>>,
    copied: usize,
}

impl Guard {
    /// Takes the next event into account, or says why the document is refused.
    fn admit(&mut self, event: &Event<'_>, at: Marker) -> Result<(), ScanError> {
        match event {
            Event::DocumentStart(_) => {
                self.anchored.clear();
                self.copied = 0;
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
                    nodes: 1,
                    height: 1,
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                if let Some(done) = self.open.pop() {
                    if done.anchor > 0 {
                        self.anchored
                            .insert(done.anchor, Some((done.nodes, done.height)));
                        self.copy(done.nodes, at)?;
                    }
                    self.child(done.nodes, done.height);
                }
            }
            Event::Scalar(_, _, anchor, _) => {
                if *anchor > 0 {
                    self.anchored.insert(*anchor, Some((1, 0)));
                    self.copy(1, at)?;
                }
                self.child(1, 0);
            }
            Event::Alias(anchor) => {
                // An alias of a collection that is still open names a node that does not exist
                // yet: the loader gives it a placeholder, which counts as one node.
                let (nodes, height) = match self.anchored.get(anchor) {
                    Some(Some(size)) => *size,
                    Some(None) => (1, 0),
                    None => {
                        return Err(ScanError::new_str(
                            at,
                            "while parsing node, found unknown anchor",
                        ));
                    }
                };
                if self.open.len() + height > MAX_DEPTH {
                    return Err(too_deep(at));
                }
                self.copy(nodes, at)?;
                self.child(nodes, height);
            }
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {}
        }
        Ok(())
    }

    /// Adds a finished node to the collection that holds it.
    fn child(&mut self, nodes: usize, height: usize) {
        if let Some(parent) = self.open.last_mut() {
            parent.nodes = parent.nodes.saturating_add(nodes);
            parent.height = parent.height.max(height + 1);
        }
    }

    fn copy(&mut self, nodes: usize, at: Marker) -> Result<(), ScanError> {
        self.copied = self.copied.saturating_add(nodes);
        if self.copied > MAX_COPIED_NODES {
            return Err(ScanError::new(
                at,
                format!(
                    "the anchors and aliases of the document copy more than {MAX_COPIED_NODES} nodes"
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
/// a document that nests too deep or copies too many nodes before the receiver sees the event
/// that goes over the limit.
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
