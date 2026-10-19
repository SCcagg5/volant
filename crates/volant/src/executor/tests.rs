// SPDX-License-Identifier: GPL-3.0-or-later
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Map, Value};
use tokio::sync::watch;

use super::Abort;
use super::play_checks::{max_fail_percentage, play_flags};

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

/// `gather_facts` and `force_handlers` written as templates are rendered once, when the play starts,
/// against the play's variables; a value that is no boolean stops the run in the reference's words,
/// at exit 4 (measured on ansible-core 2.19.12).
#[rstest]
#[case::gather_true("gather_facts", "{{ on }}", Ok(Some(true)))]
#[case::gather_false("gather_facts", "{{ off }}", Ok(Some(false)))]
#[case::gather_word("gather_facts", "{{ 'no' }}", Ok(Some(false)))]
#[case::force_true("force_handlers", "{{ on }}", Ok(Some(true)))]
#[case::force_false("force_handlers", "{{ off }}", Ok(Some(false)))]
#[case::gather_invalid(
    "gather_facts",
    "{{ 'maybe' }}",
    Err(
        "Error processing keyword 'gather_facts': The value 'maybe' could not be converted to 'bool'."
    )
)]
#[case::force_invalid(
    "force_handlers",
    "{{ 'maybe' }}",
    Err(
        "Error processing keyword 'force_handlers': The value 'maybe' could not be converted to 'bool'."
    )
)]
#[case::gather_number(
    "gather_facts",
    "{{ 2 }}",
    Err("Error processing keyword 'gather_facts': The value 2 could not be converted to 'bool'.")
)]
fn play_flags_are_rendered_when_the_play_starts(
    #[case] key: &str,
    #[case] template: &str,
    #[case] want: Result<Option<bool>, &'static str>,
) {
    let play = crate::playbook::parse(
        &format!("- hosts: all\n  {key}: \"{template}\"\n  tasks: []\n"),
        "x.yml",
    )
    .expect("the play loads")
    .plays
    .remove(0);
    let templar = Templar::new(PathBuf::from("."));
    let vars = Map::from_iter([
        ("on".to_string(), Value::Bool(true)),
        ("off".to_string(), Value::Bool(false)),
    ]);
    let got = play_flags(&play, &templar, &vars);
    match want {
        Ok(flag) => {
            let flags = got.expect("the keyword reads");
            let value = if key == "gather_facts" {
                flags.gather_facts
            } else {
                flags.force_handlers
            };
            assert_eq!(value, flag);
        }
        Err(text) => assert_eq!(format!("{:#}", got.err().expect("refused")), text),
    }
}

/// A play that writes neither as a template renders nothing.
#[test]
fn a_play_without_templated_flags_renders_none() {
    let play = crate::playbook::parse(
        "- hosts: all\n  gather_facts: false\n  force_handlers: true\n  tasks: []\n",
        "x.yml",
    )
    .expect("the play loads")
    .plays
    .remove(0);
    let flags = play_flags(&play, &Templar::new(PathBuf::from(".")), &Map::new()).unwrap();
    assert!(flags.gather_facts.is_none() && flags.force_handlers.is_none());
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
