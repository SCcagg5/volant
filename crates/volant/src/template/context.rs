// SPDX-License-Identifier: GPL-3.0-or-later
//! What a render reads a name from: the variables, which of them came from a host, and
//! the sink that records a read of one.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use minijinja::value::{Enumerator, Object};
use serde_json::{Map, Value};

use super::env::chained_attr;
use super::templar::Templar;
use super::vm::holds_template;
use super::{
    ATTR_KEY, CONCAT_KEY, CONTEXT_KEY, HOSTVARS_KEYS, ITEM_KEY, QUERY_KEY, RECURSION, TAINT_KEY,
    Tainted, TemplateError,
};
use crate::inventory::is_localhost;

/// Names of the map that hold the inventory's own names: the host's name, its short name and its
/// groups. Together with the shared layer they are what a name added to the inventory during the
/// run, from text a host sent, reaches a template through. `hostvars` is not among them: a
/// variable of another host may hold a template of its own, which a read renders.
const INVENTORY_NAMES: [&str; 3] = [
    "inventory_hostname",
    "inventory_hostname_short",
    "group_names",
];

/// Whether `key` is bound by the engine for its own use and never written by a playbook: the
/// internal bindings of a render. A read of such a name gives the value as bound.
fn engine_owned(key: &str) -> bool {
    key.starts_with("__volant_") || key.contains("::")
}

/// Names being resolved at once before a read is called a loop. The reference stops at Python's
/// recursion limit, and a chain this long is never an intended one.
const MAX_RESOLVING: usize = 64;

/// The name, in the variables an extended loop shares between its items, under which the list of
/// all the items is kept once. It is no variable: a render never finds it by name, it reads it
/// through `ansible_loop.allitems`, and `HostVars::own_variables` puts it back into `ansible_loop`.
pub const ALLITEMS_KEY: &str = "volant::allitems";

/// The variable an extended loop binds.
const LOOP_KEY: &str = "ansible_loop";

/// The fields of `ansible_loop` that sit before `allitems`, as the reference lists them.
const BEFORE_ALLITEMS: &str = "revindex0";

/// `fields` with the list of all the items in its place.
pub fn with_allitems(fields: &Map<String, Value>, all: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in fields {
        out.insert(key.clone(), value.clone());
        if key == BEFORE_ALLITEMS {
            out.insert("allitems".into(), all.clone());
        }
    }
    out.entry("allitems").or_insert_with(|| all.clone());
    out
}

/// `ansible_loop` of an extended loop: its own fields, and `allitems` read from the one list the
/// whole loop shares, converted only when a template asks for it.
#[derive(Debug)]
struct LoopView {
    fields: Map<String, Value>,
    base: Arc<Map<String, Value>>,
}

impl Object for LoopView {
    fn get_value(self: &Arc<Self>, key: &minijinja::Value) -> Option<minijinja::Value> {
        let key = key.as_str()?;
        let found = if key == "allitems" {
            self.base.get(ALLITEMS_KEY)
        } else {
            self.fields.get(key)
        };
        found.map(minijinja::Value::from_serialize)
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        let mut keys = Vec::new();
        for key in self.fields.keys() {
            keys.push(minijinja::Value::from(key.as_str()));
            if key == BEFORE_ALLITEMS {
                keys.push(minijinja::Value::from("allitems"));
            }
        }
        Enumerator::Values(keys)
    }
}

#[derive(Debug)]
pub(crate) struct TaintSink(pub(crate) Tainted);

impl TaintSink {
    pub(crate) fn taint(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Object for TaintSink {}

/// What a template renders against: one host's own variables, and — on the path where a host
/// runs a task — the whole inventory's view behind `hostvars`.
///
/// The view is shared rather than merged into the map, because merging it copied every host's
/// variables into every host's context for every task, which is a cost quadratic in the size of
/// the inventory for something most tasks never read. Anything holding only a map converts with
/// `From`, so a caller with no inventory in hand passes `&Map` and gets no `hostvars`.
#[derive(Clone, Copy)]
pub struct Vars<'a> {
    pub map: &'a Map<String, Value>,
    pub hostvars: Option<&'a Arc<Map<String, Value>>>,
    /// The values the whole inventory shares — `groups` and the play's host lists — read out of
    /// here rather than copied into every host's map for every task. Consulted *before* `map`,
    /// because these used to be written into it last and last is what wins. A name written into
    /// the map later still wins, because `HostVars::insert` takes it out of here first.
    pub shared: Option<&'a Arc<Map<String, Value>>>,
    /// The host's gathered facts, a layer shared rather than copied into `map` for every task.
    /// Consulted after `shared` and before `map`: whoever builds it leaves out every fact a layer
    /// of `map` outranks, so the facts left answer ahead of the layers below them.
    pub facts: Option<&'a Arc<Map<String, Value>>>,
    /// Names in `map` whose value came from a managed host. Reading one of them during a render
    /// makes the result data: see `render_in`.
    pub untrusted: Option<&'a BTreeSet<String>>,
    /// Hosts that have at least one such name, for the `hostvars[other]` path. Coarser than the
    /// reference, which tags each string: reading any name of such a host taints the render.
    pub untrusted_hosts: Option<&'a BTreeSet<String>>,
    /// Variables an item shares with the others of its loop, read after `map`: a name in `map`
    /// answers first. A caller that owns its whole map leaves it empty.
    pub base: Option<&'a Arc<Map<String, Value>>>,
}

impl<'a> From<&'a Map<String, Value>> for Vars<'a> {
    fn from(map: &'a Map<String, Value>) -> Self {
        Vars {
            map,
            hostvars: None,
            shared: None,
            facts: None,
            untrusted: None,
            untrusted_hosts: None,
            base: None,
        }
    }
}

/// Converted values, kept for as long as the object holding them. Converting a name when it is
/// asked for rather than converting the whole map up front only pays off if asking twice costs
/// once: a template reading `groups` inside a loop asks for it on every iteration, and each of
/// those conversions is the size of the inventory. Nothing behind a memo changes while it lives -
/// the map is cloned into the context and the shared view is replaced whole, never mutated - so
/// there is nothing to invalidate.
#[derive(Debug, Default)]
struct Memo(Mutex<HashMap<String, minijinja::Value>>);

impl Memo {
    pub(super) fn lookup(&self, key: &str) -> Option<minijinja::Value> {
        self.0.lock().expect("template memo").get(key).cloned()
    }

    /// Not held across the conversion: resolving a variable renders a template that reads other
    /// variables, and those reads come back here.
    pub(super) fn store(&self, key: &str, value: &minijinja::Value) {
        self.0
            .lock()
            .expect("template memo")
            .insert(key.to_string(), value.clone());
    }

    pub(super) fn get(
        &self,
        key: &str,
        convert: impl FnOnce() -> Option<minijinja::Value>,
    ) -> Option<minijinja::Value> {
        if let Some(value) = self.lookup(key) {
            return Some(value);
        }
        let value = convert()?;
        self.store(key, &value);
        Some(value)
    }
}

/// The root of a render: the host's variables, each converted when it is asked for rather than
/// all of them up front, and `hostvars` as an object that hands out one host's view on demand.
#[derive(Debug)]
pub(super) struct Context {
    pub(super) vars: Map<String, Value>,
    pub(super) base: Option<Arc<Map<String, Value>>>,
    pub(super) hostvars: Option<Arc<Map<String, Value>>>,
    pub(super) shared: Option<Arc<Map<String, Value>>>,
    pub(super) facts: Option<Arc<Map<String, Value>>>,
    pub(super) untrusted: BTreeSet<String>,
    pub(super) untrusted_hosts: BTreeSet<String>,
    tainted: Tainted,
    /// Set when a pass left text that must stay as it is: see `Templar::render_in`. Not data:
    /// nothing is refused because of it.
    literal: AtomicBool,
    /// How many names the render has read, to tell a pass that read none.
    reads: AtomicUsize,
    memo: Memo,
    templar: Templar,
    /// The names whose value is being rendered now, in the order the reads nested.
    resolving: Mutex<Vec<String>>,
    /// Names whose rendered value was built from something a managed host wrote. A read of one is
    /// as much a read of host data as a read of the name it was built from.
    soiled: Mutex<BTreeSet<String>>,
    /// Whether the render read the inventory's own names since the flag was last cleared.
    inventory_read: AtomicBool,
    /// Whether `soiled` holds a name, so that a render that resolved nothing never takes its lock.
    any_soiled: AtomicBool,
    /// Set when `failed` is: the first error a nested render raised that the read cannot carry.
    failure: Mutex<Option<TemplateError>>,
    failed: AtomicBool,
}

impl Context {
    fn fail(&self, error: TemplateError) {
        self.failure
            .lock()
            .expect("template failure")
            .get_or_insert(error);
        self.failed.store(true, Ordering::Relaxed);
    }

    /// What a variable holding a template reads as: the value the template renders to, with the
    /// context the reading template runs in, as the reference does when it reads a variable. The
    /// name was not a managed host's when it got here, so its text is the playbook's.
    ///
    /// Not stored when the render did not finish, so a later read tries again.
    fn resolve(self: &Arc<Self>, key: &str, raw: &Value) -> (minijinja::Value, bool) {
        {
            let mut stack = self.resolving.lock().expect("template resolving");
            if stack.len() >= MAX_RESOLVING || stack.iter().any(|name| name == key) {
                drop(stack);
                self.fail(TemplateError(RECURSION.into()));
                return (minijinja::Value::UNDEFINED, false);
            }
            stack.push(key.to_string());
        }
        let ctx = minijinja::Value::from_dyn_object(Arc::clone(self));
        // A failure the render under way already met stays its own: the nested render reports
        // the failures of its variable, and takes only those.
        let earlier = self.failure.lock().expect("template failure").take();
        let earlier_failed = self.failed.swap(false, Ordering::Relaxed);
        // The render under way keeps what it has read so far; the nested one starts clean so that
        // it can tell whether this variable's own text came from a host.
        let before = self.tainted.swap(false, Ordering::Relaxed);
        let mut from_host = false;
        let rendered = self
            .templar
            .render_read(raw, &ctx, &self.tainted, &mut from_host);
        from_host |= self.tainted.load(Ordering::Relaxed);
        self.tainted.store(before || from_host, Ordering::Relaxed);
        self.resolving.lock().expect("template resolving").pop();
        if earlier_failed {
            *self.failure.lock().expect("template failure") = earlier;
            self.failed.store(true, Ordering::Relaxed);
        }
        if from_host {
            self.soiled
                .lock()
                .expect("template soiled")
                .insert(key.to_string());
            self.any_soiled.store(true, Ordering::Relaxed);
        }
        match rendered {
            Ok(value) => (minijinja::Value::from_serialize(&value), true),
            // A variable that reads a name with no value has none either, which is what
            // `default` and `is defined` ask.
            Err(error) if error.is_undefined() => (minijinja::Value::UNDEFINED, false),
            Err(error) => {
                self.fail(error);
                (minijinja::Value::UNDEFINED, false)
            }
        }
    }

    fn read(self: &Arc<Self>, key: &str, untrusted: bool) -> Option<(minijinja::Value, bool)> {
        // A map that carries a `hostvars` key of its own - a fixture, a recorded scope -
        // keeps being read from the map when no view came with it, so the two sources never
        // disagree about which one answers.
        if key == "hostvars"
            && let Some(hostvars) = &self.hostvars
        {
            let view = minijinja::Value::from_object(Hostvars {
                hosts: Arc::clone(hostvars),
                listed: self.shared.clone(),
                untrusted_hosts: self.untrusted_hosts.clone(),
                keys_untrusted: self.untrusted.contains(HOSTVARS_KEYS),
                tainted: Arc::clone(&self.tainted),
                memo: Memo::default(),
            });
            return Some((view, true));
        }
        // Before the host's own map, never after: these are the inventory-wide values, and
        // they used to be merged into that map last, so they beat a fact of the same name.
        // A loop variable or a registered name that collides with one of them is not here
        // to be found: `HostVars::insert` drops it from the host's shared map as it writes.
        if let Some(shared) = &self.shared
            && let Some(value) = shared.get(key)
        {
            return Some((minijinja::Value::from_serialize(value), true));
        }
        // Between the two: the facts that answer, the ones no layer of the map outranks.
        if let Some(facts) = &self.facts
            && let Some(value) = facts.get(key)
        {
            return Some((minijinja::Value::from_serialize(value), true));
        }
        if key == LOOP_KEY
            && let (Some(Value::Object(fields)), Some(base)) = (self.vars.get(key), &self.base)
            && base.contains_key(ALLITEMS_KEY)
        {
            let view = minijinja::Value::from_object(LoopView {
                fields: fields.clone(),
                base: Arc::clone(base),
            });
            return Some((view, true));
        }
        if let Some(value) = self.vars.get(key) {
            if !untrusted && !engine_owned(key) && holds_template(value) {
                return Some(self.resolve(key, value));
            }
            return Some((minijinja::Value::from_serialize(value), true));
        }
        // The variables the items of a loop share hold no template that a render could change:
        // `HostVars::shared_by_items` keeps in `map` every value that has one, except the text of
        // a managed host, which is data. So a read of this layer has nothing to resolve.
        let value = self.base.as_ref()?.get(key)?;
        Some((minijinja::Value::from_serialize(value), true))
    }
}

/// Runs one render pass and says whether it read the inventory's own names: the shared layer,
/// the host's name and its groups. What it read stays marked for a pass that
/// contains this one.
pub(super) fn tracking_inventory<T>(ctx: &minijinja::Value, pass: impl FnOnce() -> T) -> (T, bool) {
    let Some(context) = ctx.downcast_object_ref::<Context>() else {
        return (pass(), false);
    };
    let earlier = context.inventory_read.swap(false, Ordering::Relaxed);
    let out = pass();
    let read = context.inventory_read.load(Ordering::Relaxed);
    context
        .inventory_read
        .store(earlier || read, Ordering::Relaxed);
    (out, read)
}

/// The error a nested render raised that its reader could not take, if one did, and clears it. A
/// read cannot raise from inside the engine: it answers undefined, and the render that made the
/// read reports the cause in place of whatever that undefined value did to it.
pub(super) fn take_failure(ctx: &minijinja::Value) -> Option<TemplateError> {
    let ctx = ctx.downcast_object_ref::<Context>()?;
    if !ctx.failed.swap(false, Ordering::Relaxed) {
        return None;
    }
    ctx.failure.lock().expect("template failure").take()
}

/// Starts a render over `ctx`: clears the mark of the last one and returns the reads so far.
pub(super) fn begin(ctx: &minijinja::Value) -> usize {
    set_literal(ctx, false);
    reads(ctx)
}

/// Records that the render over `ctx` left text that no further pass may render.
pub(super) fn set_literal(ctx: &minijinja::Value, on: bool) {
    if let Some(root) = ctx.downcast_object_ref::<Context>() {
        root.literal.store(on, Ordering::Relaxed);
    }
}

/// How many names the renders over `ctx` have read so far.
pub(super) fn reads(ctx: &minijinja::Value) -> usize {
    ctx.downcast_object_ref::<Context>()
        .map_or(0, |root| root.reads.load(Ordering::Relaxed))
}

/// Whether the render over `ctx` left such text.
pub(super) fn is_literal(ctx: &minijinja::Value) -> bool {
    ctx.downcast_object_ref::<Context>()
        .is_some_and(|root| root.literal.load(Ordering::Relaxed))
}

/// Puts the shared list of all the items back into `ansible_loop` and drops the shared entry.
pub fn complete_loop(all: &mut Map<String, Value>) {
    let Some(list) = all.shift_remove(ALLITEMS_KEY) else {
        return;
    };
    if let Some(Value::Object(fields)) = all.get(LOOP_KEY) {
        let whole = with_allitems(fields, &list);
        all.insert(LOOP_KEY.into(), Value::Object(whole));
    }
}

impl Context {
    /// The host's own variables as one map: the base shared by the items of a loop, then the
    /// map over it.
    pub(super) fn flat_vars(&self) -> Map<String, Value> {
        let mut all = self.base.as_deref().cloned().unwrap_or_default();
        all.extend(self.vars.iter().map(|(k, v)| (k.clone(), v.clone())));
        complete_loop(&mut all);
        all
    }
}

impl Object for Context {
    fn get_value(self: &Arc<Self>, key: &minijinja::Value) -> Option<minijinja::Value> {
        let key = key.as_str()?;
        // Not a variable: the VM looks the root up before the globals, so a variable of that
        // name (`-e '{"volant::concat": 1}'`) would otherwise replace every `~`.
        if key == CONCAT_KEY || key == QUERY_KEY || key == ITEM_KEY || key == ALLITEMS_KEY {
            return None;
        }
        if let Some(attr) = key.strip_prefix(ATTR_KEY) {
            return Some(chained_attr(attr.to_string()));
        }
        if key == TAINT_KEY {
            return Some(minijinja::Value::from_object(TaintSink(Arc::clone(
                &self.tainted,
            ))));
        }
        if key == CONTEXT_KEY {
            return Some(minijinja::Value::from_dyn_object(Arc::clone(self)));
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        // Before the memo, never after: a second read of the same name is answered from the
        // cache and would otherwise leave the render looking clean. The same goes for the
        // inventory's own names.
        if INVENTORY_NAMES.contains(&key)
            || self
                .shared
                .as_ref()
                .is_some_and(|shared| shared.contains_key(key))
        {
            self.inventory_read.store(true, Ordering::Relaxed);
        }
        // A `hostvars` the map carries as a plain value lists its hosts on any read, so a host
        // name that came from a managed host taints every read of it.
        let untrusted = self.untrusted.contains(key);
        if untrusted
            || (self.any_soiled.load(Ordering::Relaxed)
                && self.soiled.lock().expect("template soiled").contains(key))
            || (key == "hostvars"
                && self.hostvars.is_none()
                && self.untrusted.contains(HOSTVARS_KEYS))
        {
            self.tainted.store(true, Ordering::Relaxed);
        }
        if let Some(value) = self.memo.lookup(key) {
            return Some(value);
        }
        let (value, keep) = self.read(key, untrusted)?;
        if keep {
            self.memo.store(key, &value);
        }
        Some(value)
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        // The host's own names, `map` over the base a loop's items share: a later layer lists
        // only the names neither of them holds, so a name is never listed twice.
        let own = |key: &str| {
            self.vars.contains_key(key) || self.base.as_ref().is_some_and(|b| b.contains_key(key))
        };
        let mut keys: Vec<minijinja::Value> = self
            .vars
            .keys()
            .map(|k| minijinja::Value::from(k.as_str()))
            .collect();
        for key in self.base.iter().flat_map(|b| b.keys()) {
            if !self.vars.contains_key(key) && key != ALLITEMS_KEY {
                keys.push(minijinja::Value::from(key.as_str()));
            }
        }
        if self.hostvars.is_some() && !own("hostvars") {
            keys.push(minijinja::Value::from("hostvars"));
        }
        for key in self.shared.iter().flat_map(|s| s.keys()) {
            if !own(key) {
                keys.push(minijinja::Value::from(key.as_str()));
            }
        }
        for key in self.facts.iter().flat_map(|f| f.keys()) {
            if !own(key) && !self.shared.as_ref().is_some_and(|s| s.contains_key(key)) {
                keys.push(minijinja::Value::from(key.as_str()));
            }
        }
        Enumerator::Values(keys)
    }
}

/// `hostvars`: a map whose values are converted one host at a time. Iterating it still costs
/// the whole inventory, which is what a `hostvars | dict2items` asks for; reading one host
/// costs one host.
#[derive(Debug)]
pub(super) struct Hostvars {
    pub(super) hosts: Arc<Map<String, Value>>,
    /// The inventory-wide values, whose `groups` say which hosts the inventory lists. The map
    /// answers for the three spellings of the controller whether they are listed or not, and an
    /// iteration shows only the listed ones, as the reference's does.
    listed: Option<Arc<Map<String, Value>>>,
    /// Hosts holding at least one name that came from a managed host. Reading any name of such
    /// a host taints the render, which is coarser than the reference's per-string tag.
    untrusted_hosts: BTreeSet<String>,
    /// Whether one of the host names came from a managed host: listing them taints the render.
    keys_untrusted: bool,
    tainted: Tainted,
    memo: Memo,
}

impl Object for Hostvars {
    fn get_value(self: &Arc<Self>, key: &minijinja::Value) -> Option<minijinja::Value> {
        let key = key.as_str()?;
        // Before the memo, for the same reason as in `Context`.
        if self.untrusted_hosts.contains(key) {
            self.tainted.store(true, Ordering::Relaxed);
        }
        self.memo.get(key, || {
            self.hosts
                .get(key)
                .map(|entry| minijinja::Value::from_serialize(&*crate::vars::settled(entry)))
        })
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        if self.keys_untrusted {
            self.tainted.store(true, Ordering::Relaxed);
        }
        let unlisted: BTreeSet<&str> = match &self.listed {
            Some(shared) if self.hosts.keys().any(|k| is_localhost(k)) => {
                let all = shared
                    .get("groups")
                    .and_then(|groups| groups.get("all"))
                    .and_then(Value::as_array);
                let in_all = |name: &str| all.is_some_and(|a| a.iter().any(|h| h == name));
                self.hosts
                    .keys()
                    .map(String::as_str)
                    .filter(|k| is_localhost(k) && !in_all(k))
                    .collect()
            }
            _ => BTreeSet::new(),
        };
        Enumerator::Values(
            self.hosts
                .keys()
                .filter(|k| !unlisted.contains(k.as_str()))
                .map(|k| minijinja::Value::from(k.as_str()))
                .collect(),
        )
    }
}

pub(super) fn context_of(templar: &Templar, vars: Vars<'_>) -> (minijinja::Value, Tainted) {
    let tainted: Tainted = Arc::new(AtomicBool::new(false));
    let ctx = minijinja::Value::from_object(Context {
        vars: vars.map.clone(),
        base: vars.base.cloned(),
        hostvars: vars.hostvars.cloned(),
        shared: vars.shared.cloned(),
        facts: vars.facts.cloned(),
        untrusted: vars.untrusted.cloned().unwrap_or_default(),
        untrusted_hosts: vars.untrusted_hosts.cloned().unwrap_or_default(),
        tainted: Arc::clone(&tainted),
        literal: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
        memo: Memo::default(),
        templar: templar.clone(),
        resolving: Mutex::new(Vec::new()),
        soiled: Mutex::new(BTreeSet::new()),
        inventory_read: AtomicBool::new(false),
        any_soiled: AtomicBool::new(false),
        failure: Mutex::new(None),
        failed: AtomicBool::new(false),
    });
    (ctx, tainted)
}
