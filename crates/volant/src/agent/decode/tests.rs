// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

use rstest::rstest;
use serde_json::json;
use volant_protocol::LogLevel;

fn result_frame(body: &str) -> Vec<u8> {
    format!(r#"{{"type":"task_result","batch":7,"index":3,"result":{body}}}"#).into_bytes()
}

fn array(n: usize) -> String {
    format!("[{}0]", "0,".repeat(n - 1))
}

fn failure_of(frame: &[u8]) -> String {
    let FromAgent::TaskResult {
        batch,
        index,
        result,
        ran,
    } = decode(frame).unwrap()
    else {
        panic!("a task result");
    };
    assert_eq!((batch, index, ran), (7, 3, None));
    assert!(result.failed(), "the result was read whole");
    result.0["msg"].as_str().unwrap().to_string()
}

#[test]
fn an_ordinary_result_is_read_as_it_always_was() {
    let frame = result_frame(r#"{"changed":true,"rc":0,"stdout":"hi"}"#);
    let FromAgent::TaskResult {
        batch,
        index,
        result,
        ..
    } = decode(&frame).unwrap()
    else {
        panic!("a task result");
    };
    assert_eq!((batch, index), (7, 3));
    assert_eq!(result.0["stdout"], json!("hi"));
}

/// Large and legitimate: file content as one string, a long list of small objects.
#[rstest]
#[case::ten_megabytes_of_text(format!(r#"{{"stdout":"{}"}}"#, "x".repeat(10 << 20)))]
#[case::half_a_million_values(array(shape::MAX_VALUES / 2 - 8))]
#[case::as_many_values_as_may_be(format!(r#"{{"x":{}}}"#, array(shape::MAX_VALUES - 15)))]
fn a_large_result_inside_the_bounds_is_read(#[case] body: String) {
    let body = if body.starts_with('[') {
        format!(r#"{{"x":{body}}}"#)
    } else {
        body
    };
    let frame = result_frame(&body);
    assert!(shape::check(&frame).is_ok());
    let FromAgent::TaskResult { result, .. } = decode(&frame).unwrap() else {
        panic!("a task result");
    };
    assert!(
        !result.failed(),
        "{:?}",
        result.0.keys().collect::<Vec<_>>()
    );
}

#[rstest]
#[case::one_array(
    format!(r#"{{"x":{}}}"#, array(2_000_000)),
    "it has 2000011 values, over the limit of 1000000 (4000059 bytes in its frame)"
)]
#[case::many_small_objects(
    format!("{{\"x\":[{}{{}}]}}", "{},".repeat(1_100_000)),
    "values, over the limit of 1000000"
)]
#[case::object_keys_count(
    format!("{{{}\"z\":0}}", "\"k\":0,".repeat(600_000)),
    "values, over the limit of 1000000"
)]
#[case::nested_too_deep(
    format!(r#"{{"x":{}{}}}"#, "[".repeat(70_000), "]".repeat(70_000)),
    "it has 70002 levels of nesting, over the limit of 64"
)]
fn a_result_over_a_bound_becomes_a_failed_task(#[case] body: String, #[case] says: &str) {
    let message = failure_of(&result_frame(&body));
    assert!(message.contains(says), "{message}");
    assert!(
        message.starts_with("the result of the host is refused"),
        "{message}"
    );
}

/// Only a result may be large. Anything else the agent says is a few hundred bytes.
#[rstest]
#[case::a_log_line_at_the_limit(log_frame(MAX_MESSAGE_LEN - 60), None)]
#[case::a_log_line_past_it(log_frame(MAX_MESSAGE_LEN), Some("1048576 byte limit"))]
#[case::a_batch_end_with_values(
    format!(
        r#"{{"type":"batch_done","batch":1,"outcome":{{"kind":"completed"}},"x":{}}}"#,
        array(2_000_000)
    )
    .into_bytes(),
    Some("it has 2000011 values, over the limit of 1000000")
)]
fn a_message_that_is_not_a_result_is_cut_short(
    #[case] frame: Vec<u8>,
    #[case] refused: Option<&str>,
) {
    match (refused, decode(&frame)) {
        (None, Ok(FromAgent::Log { level, .. })) => assert_eq!(level, LogLevel::Info),
        (Some(says), Err(err)) => {
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert!(err.to_string().contains(says), "{err}");
        }
        (_, other) => panic!("{other:?}"),
    }
}

fn log_frame(len: usize) -> Vec<u8> {
    format!(
        r#"{{"type":"log","level":"info","message":"{}"}}"#,
        "x".repeat(len)
    )
    .into_bytes()
}

#[rstest]
#[case::cut_short(br#"{"type":"task_result","batch":7,"index":3,"result":{"x":["#.iter().copied().chain(std::iter::repeat_n(b'0', 100_000)).collect::<Vec<u8>>())]
#[case::not_an_object(vec![b'['; 100_000])]
#[case::no_type(format!(r#"{{"x":"{}"}}"#, "y".repeat(100_000)).into_bytes())]
fn a_large_frame_that_does_not_read_is_an_error(#[case] frame: Vec<u8>) {
    assert!(decode(&frame).is_err());
}

/// A field that does not read is no way past the 1 MiB rule: a frame whose head does not read is
/// not a result.
#[rstest]
#[case::index_a_string(r#""type":"log","index":"x""#)]
#[case::index_a_float(r#""type":"log","index":1.5"#)]
#[case::index_null(r#""type":"log","index":null"#)]
#[case::index_negative(r#""type":"log","index":-1"#)]
#[case::index_an_object(r#""type":"log","index":{}"#)]
#[case::batch_a_string(r#""type":"log","batch":"x""#)]
#[case::batch_negative(r#""type":"log","batch":-3"#)]
#[case::type_a_number(r#""type":5"#)]
#[case::type_null(r#""type":null"#)]
#[case::no_type(r#""level":"info""#)]
#[case::a_result_with_a_junk_index(r#""type":"task_result","batch":1,"index":"x""#)]
#[case::a_result_with_a_junk_batch(r#""type":"task_result","batch":1.5,"index":0"#)]
fn a_long_frame_whose_head_does_not_read_is_cut_short(#[case] fields: &str) {
    let frame = format!(r#"{{{fields},"message":"{}"}}"#, "x".repeat(2 << 20));
    let err = decode(frame.as_bytes()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("1048576 byte limit"), "{err}");
}

/// The nesting bound holds for a frame of any size, small ones included.
#[rstest]
#[case::at_the_bound(shape::MAX_DEPTH - 1, true)]
#[case::one_over(shape::MAX_DEPTH, false)]
#[case::just_under_what_serde_json_reads(120, false)]
#[case::past_what_serde_json_reads(500, false)]
fn the_nesting_of_a_small_frame_is_bounded(#[case] levels: usize, #[case] read: bool) {
    let body = format!(
        r#"{{"x":{}{}}}"#,
        "[".repeat(levels - 1),
        "]".repeat(levels - 1)
    );
    let frame = result_frame(&body);
    assert!(frame.len() < 4096);
    let FromAgent::TaskResult { result, .. } = decode(&frame).unwrap() else {
        panic!("a task result");
    };
    assert_eq!(result.failed(), !read);
    if !read {
        let msg = result.0["msg"].as_str().unwrap();
        assert!(
            msg.contains("levels of nesting, over the limit of 64"),
            "{msg}"
        );
    }
}
