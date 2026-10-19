// SPDX-License-Identifier: GPL-3.0-or-later
//! The merge key `<<`, which PyYAML resolves and ansible-core keeps. saphyr's loader has no
//! notion of it: it stores `<<` as an ordinary key.
//!
//! A text that holds `<<` is read in two steps. The parser's events are first built into a tree.
//! Each mapping that has a merge key is flattened the way PyYAML's `flatten_mapping` does it, as
//! its closing event arrives. The tree is then played back into saphyr's loader, which builds the
//! same nodes, positions and scalars it builds for any other text.

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use saphyr::{MarkedYaml, Scalar, YamlLoader};
use saphyr_parser::{Event, ScalarStyle, Span, SpannedEventReceiver, Tag};

/// Feeds `loader` the events of `text`. A text that cannot hold a merge key (it needs the two
/// characters `<<`) goes straight to the parser and skips this module's cost.
pub(super) fn parse<'a>(
    text: &'a str,
    source: &str,
    loader: &mut YamlLoader<'a, MarkedYaml<'a>>,
) -> anyhow::Result<()> {
    if text.contains("<<") {
        load(text, source, loader)
    } else {
        super::limits::feed(text, loader).map_err(Into::into)
    }
}

/// Feeds `loader` the events of `text`, every merge key resolved. A merge key that names
/// something other than a mapping or a list of mappings is an error with PyYAML's message.
pub(super) fn load<'a>(
    text: &'a str,
    source: &str,
    loader: &mut YamlLoader<'a, MarkedYaml<'a>>,
) -> anyhow::Result<()> {
    let mut tree = Tree {
        text,
        source,
        open: Vec::new(),
        top: Vec::new(),
        anchors: HashMap::new(),
        error: None,
    };
    super::limits::feed(text, &mut tree)?;
    if let Some(message) = tree.error {
        anyhow::bail!(message);
    }
    for item in tree.top {
        play(item, loader);
    }
    Ok(())
}

/// What a node starts with: its anchor, its tag and where it begins.
#[derive(Clone)]
struct Head<'a> {
    anchor: usize,
    tag: Option<Cow<'a, Tag>>,
    span: Span,
}

/// A node of the document as the parser gave it. Marks of a stream and a document, scalars and
/// aliases are leaves; a mapping keeps its entries in written order, a key written twice included.
#[derive(Clone)]
enum Item<'a> {
    Leaf(Event<'a>, Span),
    Seq(Head<'a>, Vec<Item<'a>>, Span),
    Map(Head<'a>, Vec<(Item<'a>, Item<'a>)>, Span),
}

impl Item<'_> {
    fn span(&self) -> Span {
        match self {
            Item::Leaf(_, span) => *span,
            Item::Seq(head, ..) | Item::Map(head, ..) => head.span,
        }
    }

    /// PyYAML's name for the kind of node.
    fn kind(&self) -> &'static str {
        match self {
            Item::Leaf(..) => "scalar",
            Item::Seq(..) => "sequence",
            Item::Map(..) => "mapping",
        }
    }
}

/// A container being read, with the key of the entry in progress for a mapping.
enum Open<'a> {
    Seq(Head<'a>, Vec<Item<'a>>),
    Map(Head<'a>, Vec<(Item<'a>, Item<'a>)>, Option<Item<'a>>),
}

/// Builds the tree from the parser's events.
struct Tree<'s, 'a> {
    text: &'a str,
    source: &'s str,
    open: Vec<Open<'a>>,
    top: Vec<Item<'a>>,
    /// The nodes an alias can name, by the parser's anchor number. A node is stored once it is
    /// closed, its merge keys already resolved, as PyYAML's alias names the flattened node.
    anchors: HashMap<usize, Item<'a>>,
    error: Option<String>,
}

impl<'a> SpannedEventReceiver<'a> for Tree<'_, 'a> {
    fn on_event(&mut self, ev: Event<'a>, span: Span) {
        if self.error.is_some() {
            return;
        }
        match ev {
            Event::SequenceStart(anchor, tag) => {
                let head = Head { anchor, tag, span };
                self.open.push(Open::Seq(head, Vec::new()));
            }
            Event::MappingStart(anchor, tag) => {
                let head = Head { anchor, tag, span };
                self.open.push(Open::Map(head, Vec::new(), None));
            }
            Event::SequenceEnd => {
                if let Some(Open::Seq(head, items)) = self.open.pop() {
                    self.close(Item::Seq(head, items, span));
                }
            }
            Event::MappingEnd => {
                if let Some(Open::Map(head, pairs, _)) = self.open.pop() {
                    match self.flatten(&head, pairs) {
                        Ok(pairs) => self.close(Item::Map(head, pairs, span)),
                        Err(message) => self.error = Some(message),
                    }
                }
            }
            Event::Scalar(..) | Event::Alias(_) => self.close(Item::Leaf(ev, span)),
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart(_)
            | Event::DocumentEnd => self.top.push(Item::Leaf(ev, span)),
        }
    }
}

impl<'a> Tree<'_, 'a> {
    /// Stores a finished node under its anchor, if it has one, and puts it in its parent.
    fn close(&mut self, item: Item<'a>) {
        let anchor = match &item {
            Item::Leaf(Event::Scalar(_, _, anchor, _), _) => *anchor,
            Item::Seq(head, ..) | Item::Map(head, ..) => head.anchor,
            Item::Leaf(..) => 0,
        };
        if anchor > 0 {
            self.anchors.insert(anchor, item.clone());
        }
        match self.open.last_mut() {
            None => self.top.push(item),
            Some(Open::Seq(_, items)) => items.push(item),
            Some(Open::Map(_, pairs, key)) => match key.take() {
                None => *key = Some(item),
                Some(key) => pairs.push((key, item)),
            },
        }
    }

    /// The node an alias names, or `item` itself.
    fn resolve<'t>(&'t self, item: &'t Item<'a>) -> &'t Item<'a> {
        match item {
            Item::Leaf(Event::Alias(id), _) => self.anchors.get(id).unwrap_or(item),
            _ => item,
        }
    }

    /// PyYAML's `flatten_mapping` followed by the construction of the dict. Every merge key goes.
    /// The pairs it names come first, then the explicit ones. A key that comes twice keeps its
    /// first place and its last value. The pairs of the later of two merge keys come later, and
    /// so win, and of a list the earlier mapping wins, since the list is merged back to front.
    fn flatten(
        &self,
        head: &Head<'a>,
        pairs: Vec<(Item<'a>, Item<'a>)>,
    ) -> Result<Vec<(Item<'a>, Item<'a>)>, String> {
        if !pairs.iter().any(|(key, _)| is_merge_key(key)) {
            return Ok(pairs);
        }
        let mut merged = Vec::new();
        let mut explicit = Vec::new();
        for (key, value) in pairs {
            if !is_merge_key(&key) {
                explicit.push((key, value));
                continue;
            }
            match self.resolve(&value) {
                source @ Item::Map(..) => self.take_pairs(source, &mut merged),
                Item::Seq(_, sources, _) => {
                    let mut maps = Vec::new();
                    for source in sources {
                        let source = self.resolve(source);
                        if !matches!(source, Item::Map(..)) {
                            return Err(self.refusal(head, "expected a mapping", source));
                        }
                        maps.push(source);
                    }
                    for source in maps.into_iter().rev() {
                        self.take_pairs(source, &mut merged);
                    }
                }
                other => {
                    return Err(self.refusal(
                        head,
                        "expected a mapping or list of mappings",
                        other,
                    ));
                }
            }
        }
        merged.extend(explicit);
        Ok(unique(merged))
    }

    /// Copies the entries of the mapping `source` onto `out`. The values of a mapping marked
    /// `!unsafe` stay data: ansible-core builds the shared nodes once, as data, so the mapping
    /// that merges them holds data too.
    fn take_pairs(&self, source: &Item<'a>, out: &mut Vec<(Item<'a>, Item<'a>)>) {
        let Item::Map(head, pairs, _) = source else {
            return;
        };
        let unsafe_source = head.tag.as_deref().is_some_and(is_unsafe);
        out.extend(pairs.iter().map(|(key, value)| {
            let value = if unsafe_source {
                self.as_data(value.clone())
            } else {
                value.clone()
            };
            (key.clone(), value)
        }));
    }

    /// `item` marked `!unsafe`, where it can hold a template. An alias is replaced by the node
    /// it names, and a scalar that holds no template syntax is left as it is.
    fn as_data(&self, item: Item<'a>) -> Item<'a> {
        let unsafe_tag = || {
            Some(Cow::Owned(Tag {
                handle: "!".into(),
                suffix: "unsafe".into(),
            }))
        };
        match item {
            Item::Leaf(Event::Alias(id), span) => match self.anchors.get(&id) {
                Some(named) => self.as_data(named.clone()),
                None => Item::Leaf(Event::Alias(id), span),
            },
            Item::Leaf(Event::Scalar(text, style, anchor, None), span)
                if text.contains("{{") || text.contains("{%") =>
            {
                Item::Leaf(Event::Scalar(text, style, anchor, unsafe_tag()), span)
            }
            Item::Seq(mut head, items, end) if head.tag.is_none() => {
                head.tag = unsafe_tag();
                Item::Seq(head, items, end)
            }
            Item::Map(mut head, pairs, end) if head.tag.is_none() => {
                head.tag = unsafe_tag();
                Item::Map(head, pairs, end)
            }
            other => other,
        }
    }

    /// PyYAML's constructor error for a merge key in the mapping that starts at `head`, whose
    /// value `found` is not what a merge key takes.
    fn refusal(&self, head: &Head<'_>, problem: &str, found: &Item<'_>) -> String {
        let (context, culprit) = (self.mark(head.span), self.mark(found.span()));
        let place = |(line, column): (usize, usize)| {
            format!("  in \"{}\", line {line}, column {column}", self.source)
        };
        let mut lines = vec!["while constructing a mapping".to_string()];
        if context != culprit {
            lines.push(place(context));
        }
        lines.push(format!("{problem} for merging, but found {}", found.kind()));
        lines.push(place(culprit));
        lines.join("\n")
    }

    /// Where PyYAML puts the start of the node at `span`, as `(line, column)` counted from 1:
    /// before the anchor and the tag the node is written with, which the parser's span leaves out.
    fn mark(&self, span: Span) -> (usize, usize) {
        let text = self.text;
        let at = text
            .char_indices()
            .nth(span.start.index())
            .map_or(text.len(), |(at, _)| at);
        let mut start = at;
        loop {
            let before = text[..start].trim_end();
            let token = before.rsplit(char::is_whitespace).next().unwrap_or("");
            if !(token.starts_with('&') || token.starts_with('!')) {
                break;
            }
            start = before.len() - token.len();
        }
        if start == at {
            return (span.start.line(), span.start.col() + 1);
        }
        let head = &text[..start];
        let line = head.matches('\n').count() + 1;
        let column = head
            .rsplit('\n')
            .next()
            .map_or(0, |row| row.chars().count())
            + 1;
        (line, column)
    }
}

/// `<<` written plain, or any key tagged `!!merge`: the keys PyYAML resolves to its merge tag.
fn is_merge_key(key: &Item<'_>) -> bool {
    match key {
        Item::Leaf(Event::Scalar(text, style, _, tag), _) => match tag {
            None => *style == ScalarStyle::Plain && text == "<<",
            Some(tag) => tag.is_yaml_core_schema() && tag.suffix == "merge",
        },
        _ => false,
    }
}

fn is_unsafe(tag: &Tag) -> bool {
    tag.handle == "!" && tag.suffix == "unsafe"
}

/// `pairs` with a key that appears again dropped, its first place kept and its last value taken,
/// as a Python dict keeps it. Keys are told apart by what they resolve to; a key that is not a
/// scalar is never the same as another.
fn unique<'a>(pairs: Vec<(Item<'a>, Item<'a>)>) -> Vec<(Item<'a>, Item<'a>)> {
    let mut places: HashMap<Scalar<'a>, usize> = HashMap::with_capacity(pairs.len());
    let mut out: Vec<(Item<'a>, Item<'a>)> = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        let scalar = match &key {
            Item::Leaf(Event::Scalar(text, style, _, tag), _) => {
                Scalar::parse_from_cow_and_metadata(text.clone(), *style, tag.as_ref())
            }
            _ => None,
        };
        match scalar.map(|scalar| places.entry(scalar)) {
            Some(Entry::Occupied(place)) => out[*place.get()].1 = value,
            Some(Entry::Vacant(place)) => {
                place.insert(out.len());
                out.push((key, value));
            }
            None => out.push((key, value)),
        }
    }
    out
}

/// Plays `item` back as the events it came from.
fn play<'a>(item: Item<'a>, to: &mut YamlLoader<'a, MarkedYaml<'a>>) {
    match item {
        Item::Leaf(event, span) => to.on_event(event, span),
        Item::Seq(head, items, end) => {
            to.on_event(Event::SequenceStart(head.anchor, head.tag), head.span);
            for item in items {
                play(item, to);
            }
            to.on_event(Event::SequenceEnd, end);
        }
        Item::Map(head, pairs, end) => {
            to.on_event(Event::MappingStart(head.anchor, head.tag), head.span);
            for (key, value) in pairs {
                play(key, to);
                play(value, to);
            }
            to.on_event(Event::MappingEnd, end);
        }
    }
}
