// SPDX-License-Identifier: GPL-3.0-or-later
//! Lists and mappings that a template changes in place.
//!
//! minijinja values never change. A template that calls `append` or `update` needs a value that
//! does, so a render whose template calls such a method reads every plain list and mapping of
//! the variables through [`MutList`] and [`MutDict`]. Two names for one value (`b = a`) then
//! name one object, as in Python. The change lives as long as the render of one string.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use indexmap::IndexMap;
use minijinja::value::{Enumerator, Object, ObjectRepr, ValueKind};
use minijinja::{Error, ErrorKind, State, Value};

use super::{dicts, lists};

pub(super) type Dict = IndexMap<Value, Value>;

/// The methods that change the value they are called on. A template that calls one of them is
/// rendered with mutable values.
pub(crate) const MUTATORS: [&str; 11] = [
    "append",
    "extend",
    "insert",
    "remove",
    "pop",
    "sort",
    "reverse",
    "clear",
    "update",
    "setdefault",
    "popitem",
];

/// A list a template can change.
#[derive(Debug, Default)]
pub(super) struct MutList(Mutex<Vec<Value>>);

/// A mapping a template can change.
#[derive(Debug, Default)]
pub(super) struct MutDict(Mutex<Dict>);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MutList {
    /// A list to run one method on, which nothing else reads afterwards.
    pub(super) fn temp(items: Vec<Value>) -> MutList {
        MutList(Mutex::new(items))
    }

    pub(super) fn from_items(items: Vec<Value>) -> Value {
        Value::from_object(MutList(Mutex::new(items)))
    }

    /// Runs `f` on the items. Nothing under `f` may read another mutable value: the lock is held.
    pub(super) fn with<R>(&self, f: impl FnOnce(&mut Vec<Value>) -> R) -> R {
        f(&mut lock(&self.0))
    }

    pub(super) fn items(&self) -> Vec<Value> {
        lock(&self.0).clone()
    }
}

impl MutDict {
    /// A mapping to run one method on, which nothing else reads afterwards.
    pub(super) fn temp(entries: Dict) -> MutDict {
        MutDict(Mutex::new(entries))
    }

    pub(super) fn from_entries(entries: Dict) -> Value {
        Value::from_object(MutDict(Mutex::new(entries)))
    }

    pub(super) fn with<R>(&self, f: impl FnOnce(&mut Dict) -> R) -> R {
        f(&mut lock(&self.0))
    }

    pub(super) fn entries(&self) -> Dict {
        lock(&self.0).clone()
    }
}

impl Object for MutList {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Seq
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        let at = key.as_usize()?;
        let mut items = lock(&self.0);
        let item = items.get(at)?.clone();
        // An item that is a plain list or mapping becomes mutable the first time it is read,
        // and stays so: the next read gives the same object.
        match mutable(&item) {
            Some(made) => {
                items[at] = made.clone();
                Some(made)
            }
            None => Some(item),
        }
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Seq(lock(&self.0).len())
    }

    fn call_method(
        self: &Arc<Self>,
        _state: &State<'_, '_>,
        method: &str,
        args: &[Value],
    ) -> Result<Value, Error> {
        lists::call(self, method, args)
            .unwrap_or_else(|| Err(Error::from(ErrorKind::UnknownMethod)))
    }
}

impl Object for MutDict {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Map
    }

    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        child(&mut lock(&self.0), key)
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        Enumerator::Values(lock(&self.0).keys().cloned().collect())
    }

    fn call_method(
        self: &Arc<Self>,
        _state: &State<'_, '_>,
        method: &str,
        args: &[Value],
    ) -> Result<Value, Error> {
        dicts::call(self, method, args)
            .unwrap_or_else(|| Err(Error::from(ErrorKind::UnknownMethod)))
    }
}

/// The value under `key`, made mutable in place when it is a plain list or mapping.
pub(super) fn child(entries: &mut Dict, key: &Value) -> Option<Value> {
    let item = entries.get(key)?.clone();
    match mutable(&item) {
        Some(made) => {
            entries.insert(key.clone(), made.clone());
            Some(made)
        }
        None => Some(item),
    }
}

/// The list inside a plain value: one minijinja built from a literal, a filter or a variable.
pub(super) fn plain_list(value: &Value) -> Option<&Vec<Value>> {
    value.downcast_object_ref::<Vec<Value>>()
}

/// The mapping inside a plain value.
pub(super) fn plain_map(value: &Value) -> Option<&Dict> {
    value.downcast_object_ref::<Dict>()
}

/// A mutable copy of a plain list or mapping; `None` for any other value, which is left as it is.
pub(crate) fn mutable(value: &Value) -> Option<Value> {
    if let Some(items) = plain_list(value) {
        return Some(MutList::from_items(items.clone()));
    }
    plain_map(value).map(|entries| MutDict::from_entries(entries.clone()))
}

/// Whether a value is one of the mutable objects of this module.
pub(super) fn is_mutable(value: &Value) -> bool {
    value.downcast_object_ref::<MutList>().is_some()
        || value.downcast_object_ref::<MutDict>().is_some()
}

/// An address that names a plain list or mapping for as long as it lives, so that two reads of
/// one variable find one mutable object.
pub(crate) fn identity(value: &Value) -> Option<usize> {
    plain_list(value)
        .map(|list| std::ptr::from_ref(list) as usize)
        .or_else(|| plain_map(value).map(|map| std::ptr::from_ref(map) as usize))
}

/// Whether a value is a list or mapping of plain values whose mapping keys are all texts, so
/// that its JSON form reads back as the same value.
pub(crate) fn is_literal(value: &Value) -> bool {
    if let Some(items) = plain_list(value) {
        return items.iter().all(is_literal);
    }
    if let Some(entries) = plain_map(value) {
        return entries
            .iter()
            .all(|(k, v)| k.as_str().is_some() && is_literal(v));
    }
    matches!(
        value.kind(),
        ValueKind::None | ValueKind::Bool | ValueKind::Number | ValueKind::String
    )
}

fn holds_mutable(value: &Value) -> bool {
    if is_mutable(value) {
        return true;
    }
    if let Some(items) = plain_list(value) {
        return items.iter().any(holds_mutable);
    }
    plain_map(value).is_some_and(|entries| entries.values().any(holds_mutable))
}

/// A copy of a value that no later change reaches: every mutable list and mapping inside it is
/// replaced by a plain one. ansible-core copies what it hands to a call and what a call hands back,
/// and so does every method here that stores or returns a value.
pub(crate) fn snapshot(value: &Value) -> Value {
    if let Some(list) = value.downcast_object_ref::<MutList>() {
        return Value::from(list.items().iter().map(snapshot).collect::<Vec<_>>());
    }
    if let Some(dict) = value.downcast_object_ref::<MutDict>() {
        return dict
            .entries()
            .iter()
            .map(|(k, v)| (k.clone(), snapshot(v)))
            .collect();
    }
    if !holds_mutable(value) {
        return value.clone();
    }
    if let Some(items) = plain_list(value) {
        return Value::from(items.iter().map(snapshot).collect::<Vec<_>>());
    }
    match plain_map(value) {
        Some(entries) => entries
            .iter()
            .map(|(k, v)| (k.clone(), snapshot(v)))
            .collect(),
        None => value.clone(),
    }
}

/// Whether a value is a list the methods of this module can read: a mutable list or a plain one.
pub(super) fn is_list(value: &Value) -> bool {
    value.kind() == ValueKind::Seq
        && (value.downcast_object_ref::<MutList>().is_some() || plain_list(value).is_some())
}

/// Whether a value is a mapping the methods of this module can read.
pub(super) fn is_dict(value: &Value) -> bool {
    value.kind() == ValueKind::Map
        && (value.downcast_object_ref::<MutDict>().is_some() || plain_map(value).is_some())
}
