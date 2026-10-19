// SPDX-License-Identifier: GPL-3.0-or-later
use minijinja::{Environment, Value};
use serde_json::json;

use super::mutable::{identity, is_literal, mutable, plain_list, plain_map, snapshot};

/// The mutable objects stand in for the plain lists and mappings minijinja builds, which this
/// module finds by the Rust type behind them. An upgrade of minijinja that changes those types
/// fails here, not in a template.
#[test]
fn plain_lists_and_mappings_are_told_from_other_objects() {
    assert!(plain_list(&Value::from_serialize(json!([1, 2]))).is_some());
    assert!(plain_map(&Value::from_serialize(json!({"a": 1}))).is_some());
    assert!(plain_list(&Value::from(vec![Value::from(1)])).is_some());
    let namespace = Environment::new()
        .compile_expression("namespace()")
        .and_then(|e| e.eval(()))
        .unwrap();
    assert!(mutable(&namespace).is_none());
    assert!(identity(&namespace).is_none());
    assert!(mutable(&Value::from("text")).is_none());
}

#[test]
fn one_plain_value_has_one_identity_and_a_copy_has_another() {
    let list = Value::from_serialize(json!([1, 2]));
    let same = list.clone();
    let other = Value::from_serialize(json!([1, 2]));
    assert_eq!(identity(&list), identity(&same));
    assert_ne!(identity(&list), identity(&other));
}

#[test]
fn a_snapshot_holds_no_mutable_object() {
    let made = mutable(&Value::from_serialize(json!({"a": [1], "b": {"c": []}}))).unwrap();
    let inner = made.get_attr("a").unwrap();
    assert!(plain_list(&inner).is_none());
    let copy = snapshot(&made);
    assert!(plain_map(&copy).is_some());
    assert!(plain_list(&copy.get_attr("a").unwrap()).is_some());
    assert_eq!(
        serde_json::to_value(&copy).unwrap(),
        json!({"a": [1], "b": {"c": []}})
    );
}

#[test]
fn only_a_literal_that_json_carries_back_is_routed() {
    assert!(is_literal(&Value::from_serialize(
        json!([1, "a", null, [true], {"k": 1.5}])
    )));
    let int_key = Value::from_iter([(Value::from(1), Value::from("a"))]);
    assert!(!is_literal(&int_key));
}
