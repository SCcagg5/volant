// SPDX-License-Identifier: GPL-3.0-or-later
use minijinja::{Environment, Value};
use serde_json::json;

use super::mutable::{identity, is_literal, mutable, plain_list, plain_map, snapshot};

/// The mutable objects stand in for the plain lists and mappings minijinja builds, found by
/// their Rust type. An upgrade that changes those types fails here, not in a template.
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

/// Runs `f` on its own thread and fails when it has not finished in `secs`, so that a hang shows
/// as a failure and not as a run that never ends.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(secs))
        .expect("the template did not finish")
}

fn render(template: &'static str) -> Result<serde_json::Value, String> {
    within(20, move || {
        crate::testing::templar()
            .render(template, &serde_json::Map::new())
            .map_err(|e| e.0)
    })
}

/// A list or mapping that is handed to its own method (as the item to remove, or as a key) is
/// compared and hashed while the method holds it. That used to wait on itself for ever.
#[test]
fn a_container_given_to_its_own_method_does_not_wait_on_itself() {
    let err = render("{% set l = [[1]] %}{{ l.remove(l) }}").unwrap_err();
    assert!(err.contains("not in list"), "{err}");
    for template in [
        "{% set d = {} %}{{ d.setdefault(d, 1) }}",
        "{% set d = {} %}{{ d.update([[d, 1]]) }}",
        "{% set d = {} %}{{ d.get(d) }}",
    ] {
        let err = render(template).unwrap_err();
        assert!(err.contains("unhashable type: 'dict'"), "{template}: {err}");
    }
    assert_eq!(
        render(">{% set l = [[1]] %}{% set _ = l.extend(l) %}{% set _ = l[0].append(2) %}{{ l }}"),
        Ok(json!(">[[1, 2], [1]]"))
    );
}

/// A copy of a list that holds copies of it shares them as a graph: twenty-eight rounds used to
/// take nine seconds, because each copy walked the structure as a tree.
#[test]
fn copying_a_list_that_holds_copies_of_itself_is_linear() {
    let out = render(
        "{% set l = [1] %}{% for i in range(60) %}{% set _ = l.append(l.copy()) %}{% endfor %}{{ l | length }}",
    );
    assert_eq!(out, Ok(json!(61)));
}

/// A list of mixed types is no total order: sorting it is an error and never a panic, for any
/// length and order of the items.
#[test]
fn sorting_mixed_types_fails_without_a_panic() {
    for template in [
        "{% set l = [3, 'a', 2, 'b', 1, 'c', 0, 'd', 9, 'e', 8, 'f', 7, 'g', 6, 'h', 5] %}{{ l.sort() }}",
        "{% set l = ['b', 1, 'a'] %}{{ l.sort() }}",
        "{% set l = [[1, 'a'], [1, 2], [0, 'b'], [1, 3]] %}{{ l.sort() }}",
    ] {
        let out = render(template);
        assert!(out.is_err(), "{template}: {out:?}");
    }
}

#[test]
fn a_reverse_sort_is_stable_and_orders_the_items() {
    assert_eq!(
        render(
            "{% set l = [[1, 'a'], [2, 'b'], [1, 'c']] %}{% set _ = l.sort(reverse=True) %}{{ l }}"
        ),
        Ok(json!([[2, "b"], [1, "c"], [1, "a"]]))
    );
}

/// `l.extend(l)` in a loop doubles a list; it stops at the limit and does not run out of memory.
#[test]
fn a_list_stops_growing_at_the_limit() {
    let out =
        render("{% set l = [1] %}{% for i in range(60) %}{% set _ = l.extend(l) %}{% endfor %}");
    assert!(out.is_err(), "{out:?}");
}
