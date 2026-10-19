// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Map, Value};
use tokio::sync::watch;

use super::Abort;
use super::play_checks::max_fail_percentage;

use crate::template::Templar;

/// A number, a number written as text and a template that renders to one all read; anything
/// else is refused by the keyword's name.
#[rstest]
#[case::integer("25", Ok(Some(25.0)))]
#[case::float("12.5", Ok(Some(12.5)))]
#[case::text("\"50\"", Ok(Some(50.0)))]
#[case::template("\"{{ pct }}\"", Ok(Some(30.0)))]
#[case::none("null", Ok(None))]
#[case::words("most", Err("is not a number"))]
#[case::list("[1]", Err("is not a number"))]
#[case::undefined("\"{{ nope }}\"", Err("max_fail_percentage"))]
fn max_fail_percentage_reads_a_number(
    #[case] written: &str,
    #[case] want: Result<Option<f64>, &'static str>,
) {
    let play = crate::playbook::parse(
        &format!(
            "- hosts: all
  max_fail_percentage: {written}
  tasks: []
"
        ),
        "x.yml",
    )
    .expect("the play loads")
    .plays
    .remove(0);
    let templar = Templar::new(PathBuf::from("."));
    let vars = Map::from_iter([("pct".to_string(), Value::from(30))]);
    let got = max_fail_percentage(&play, &templar, &vars);
    match want {
        Ok(limit) => assert_eq!(got.expect("the keyword reads"), limit),
        Err(needle) => {
            let err = got.expect_err("the keyword is refused");
            assert!(format!("{err:#}").contains(needle), "{err:#}");
        }
    }
}

/// An interruption that arrives first keeps the run an interrupted one: a reason raised after
/// it is dropped, so the run exits 99 and not 1.
#[test]
fn an_interruption_that_came_first_is_not_turned_into_a_reason() {
    let (tx, rx) = watch::channel(false);
    let abort = Abort::new(tx);
    abort.interrupt();
    abort.raise("late".into());
    assert_eq!(abort.reason(), None);
    assert!(*rx.borrow());

    let (tx, rx) = watch::channel(false);
    let abort = Abort::new(tx);
    abort.raise("first".into());
    abort.raise("second".into());
    assert_eq!(abort.reason().as_deref(), Some("first"));
    assert!(*rx.borrow());
}

/// What `render_hosts` makes of one `hosts:` over a three-host inventory, as the reference's
/// post-validation of the play does: the patterns and the name of the play, or the refusal.
#[rstest]
#[case::first_of_a_group("\"{{ groups['web'] | first }}\"", &[], false, Ok((&["w1"][..], "w1")))]
#[case::string_is_one_pattern("\"{{ groups['web'] | join(',') }}\"", &[], false, Ok((&["w1,w2"][..], "w1,w2")))]
#[case::list_is_many("\"{{ groups['web'] }}\"", &[], false, Ok((&["w1", "w2"][..], "['w1', 'w2']")))]
#[case::empty_list("\"{{ [] }}\"", &[], false, Ok((&[][..], "[]")))]
#[case::none("\"{{ none }}\"", &[], false, Ok((&["None"][..], "None")))]
#[case::extra_variable("\"{{ target }}\"", &[("target", "w2")], false, Ok((&["w2"][..], "w2")))]
#[case::play_variable("\"{{ pick }}\"\n  vars: {pick: w1}", &[], false, Ok((&["w1"][..], "w1")))]
#[case::extra_beats_play("\"{{ pick }}\"\n  vars: {pick: w1}", &[("pick", "w2")], false, Ok((&["w2"][..], "w2")))]
#[case::list_of_templates("[\"{{ 'w1' }}\", w2]", &[], false, Ok((&["w1", "w2"][..], "{{ 'w1' }},w2")))]
#[case::all_hosts("\"{{ ansible_play_hosts_all | last }}\"", &[], false, Ok((&["w2"][..], "w2")))]
#[case::hostvars_not_yet("\"{{ hostvars['w1'].x }}\"", &[], false, Err("name"))]
#[case::hostvars_after_a_play("\"{{ hostvars['w1'].x }}\"", &[], true, Err("name"))]
#[case::undefined("\"{{ nosuch }}\"", &[], false, Err("name"))]
#[case::a_ring_of_variables("\"{{ a }}\"\n  vars: {a: '{{ b }}', b: '{{ a }}'}", &[], false, Err("Recursive loop"))]
#[case::empty_string("\"{{ '' }}\"", &[], false, Err("cannot have empty values"))]
#[case::integer("\"{{ 3 }}\"", &[], false, Err("not 'int'"))]
#[case::mapping("\"{{ {'a': 1} }}\"", &[], false, Err("not 'dict'"))]
#[case::list_in_a_list("[\"{{ groups['web'] }}\"]", &[], false, Err("not 'list'"))]
fn a_templated_hosts_renders_to_patterns(
    #[case] hosts: &str,
    #[case] extra: &[(&str, &str)],
    #[case] with_hostvars: bool,
    #[case] want: Result<(&[&str], &str), &str>,
) {
    use crate::executor::{HostsContext, render_hosts};
    use crate::vars::VarStore;

    let tmp = crate::testing::scratch();
    let inventory = crate::testing::write(tmp.path(), "hosts.ini", "[web]\nw1\nw2\n");
    let inventory = crate::inventory::Inventory::load(&inventory).expect("the inventory loads");
    let extra: Map<String, Value> = extra
        .iter()
        .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
        .collect();
    let store = std::sync::Mutex::new(
        VarStore::new(&inventory, None, tmp.path(), extra).expect("the store builds"),
    );
    let text = format!("- hosts: {hosts}\n  tasks: []\n");
    let compiled = crate::testing::compiled(&text);
    let play = crate::testing::play(&text).plays.remove(0);
    let all = vec!["w1".to_string(), "w2".to_string()];
    let got = render_hosts(
        &play,
        &HostsContext {
            templar: &Templar::new(PathBuf::from(".")),
            vars: &store,
            compiled: &compiled,
            all_hosts: &all,
            with_hostvars,
        },
    );
    match want {
        Ok((patterns, name)) => {
            let got = got.expect("the hosts render");
            assert_eq!(got.host_patterns, patterns);
            assert_eq!(got.hosts, patterns.join(","));
            assert_eq!(got.name, name);
            // The reference prints a play that `none` named with no name at all.
            assert_eq!(got.nameless, hosts.contains("none"), "{hosts}");
        }
        Err(needle) => {
            let err = format!("{:#}", got.expect_err("the hosts are refused"));
            assert!(err.contains(needle), "{err}");
        }
    }
}
