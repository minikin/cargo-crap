//! The triage client against the recording `TypeSafe` stub.
//!
//! No test here touches the network or needs a key. Built only with the
//! `triage` feature, which is what compiles the client in.

#![cfg(feature = "triage")]

mod support;

use std::path::Path;
use std::time::Duration;

use cargo_crap::duplicates::compare::DuplicatePair;
use cargo_crap::duplicates::extract::Location;
use cargo_crap::duplicates::triage::verdict::{Kind, Verdict};
use cargo_crap::duplicates::triage::{Settings, TriageError, run};
use support::typesafe_stub::{Reply, TypesafeStub};
use tempfile::TempDir;

/// A full `/v1/systemone` answer naming `kind`.
fn answer(kind: &str) -> String {
    format!(
        r#"{{"model":"jev-1.13.0","answers":{{
            "duplication_kind":{{"type":"choice","choice":"{kind}","confidence":0.9}},
            "worth_extracting":{{"type":"score","score":2.0,"confidence":0.8}},
            "divergence_risk":{{"type":"noul","noul":0.6}}}},"usage":{{}}}}"#
    )
}

/// `n` pairs, each a file holding two functions `fn_<i>_a` and `fn_<i>_b`.
fn pairs(
    dir: &Path,
    n: usize,
) -> Vec<DuplicatePair> {
    (0..n)
        .map(|i| {
            let file = dir.join(format!("f{i}.rs"));
            std::fs::write(
                &file,
                format!("fn fn_{i}_a() -> i32 {{ {i} }}\nfn fn_{i}_b() -> i32 {{ {i} }}\n"),
            )
            .expect("write source");
            let at = |line: usize, side: &str| Location {
                file: file.clone(),
                start_line: line,
                end_line: line,
                name: format!("fn_{i}_{side}"),
            };
            DuplicatePair {
                first: at(1, "a"),
                second: at(2, "b"),
                score: 0.9,
            }
        })
        .collect()
}

fn settings(base_url: String) -> Settings {
    Settings {
        model: "jev-latest".to_owned(),
        base_url,
        api_key: Some("test-key".to_owned()),
        timeout: Duration::from_secs(5),
        connect_timeout: Duration::from_secs(3),
        attempts: 3,
        backoff: Duration::from_millis(1),
        // No cache: every test here counts the requests it causes.
        cache_dir: None,
    }
}

#[test]
fn a_successful_answer_decodes_into_one_verdict_per_pair() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::json(&answer("same_logic"))]);
    let verdicts = run_within(pairs(dir.path(), 2), settings(stub.base_url())).expect("triaged");
    assert_eq!(verdicts.len(), 2);
    assert!(verdicts.iter().all(|v| v.kind == Kind::SameLogic));

    let requests = stub.requests();
    assert_eq!(requests.len(), 2, "one request per pair");
    for request in &requests {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/systemone");
        assert_eq!(request.header("authorization"), Some("Bearer test-key"));
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(request.json()["model"], "jev-latest");
    }
}

#[test]
fn verdicts_come_back_in_the_pairs_order() {
    // Requests run in parallel; answers are chosen by content, so a result
    // in the wrong slot would show as the wrong kind.
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::respond_with(|request| {
        let kind = if request.body.contains("fn_1_a") {
            "shared_shape_only"
        } else {
            "same_logic"
        };
        Reply::json(&answer(kind))
    });
    let verdicts = run_within(pairs(dir.path(), 3), settings(stub.base_url())).expect("triaged");
    let kinds: Vec<Kind> = verdicts.iter().map(|v| v.kind).collect();
    assert_eq!(
        kinds,
        [Kind::SameLogic, Kind::SharedShapeOnly, Kind::SameLogic]
    );
}

#[test]
fn a_server_error_then_success_is_retried() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::status(503), Reply::json(&answer("same_logic"))]);
    run_within(pairs(dir.path(), 1), settings(stub.base_url())).expect("the retry succeeds");
    assert_eq!(stub.request_count(), 2);
}

#[test]
fn a_rate_limit_then_success_is_retried() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::status(429), Reply::json(&answer("same_logic"))]);
    run_within(pairs(dir.path(), 1), settings(stub.base_url())).expect("the retry succeeds");
    assert_eq!(stub.request_count(), 2);
}

#[test]
fn a_dropped_connection_then_success_is_retried() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::Drop, Reply::json(&answer("same_logic"))]);
    run_within(pairs(dir.path(), 1), settings(stub.base_url())).expect("the retry succeeds");
    assert_eq!(stub.request_count(), 2);
}

/// `run`, failing the test if it has not returned within ten seconds: a
/// retry loop that never gives up must fail fast, not hang the suite. Every
/// test here goes through it, since any of them can meet a failure that is
/// retried.
fn run_within(
    pairs: Vec<DuplicatePair>,
    settings: Settings,
) -> Result<Vec<Verdict>, TriageError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(run(&pairs, &settings));
    });
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("run returned in time: its retries are bounded")
}

#[test]
fn a_persistent_failure_errs_after_a_bounded_number_of_attempts() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::status(503)]);
    let err =
        run_within(pairs(dir.path(), 1), settings(stub.base_url())).expect_err("never succeeds");
    assert!(err.to_string().contains("503"), "{err}");
    assert_eq!(
        stub.request_count(),
        3,
        "bounded: three attempts, then give up"
    );
}

#[test]
fn a_client_error_is_not_retried() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::status(422)]);
    let err = run_within(pairs(dir.path(), 1), settings(stub.base_url())).expect_err("rejected");
    assert!(err.to_string().contains("422"), "{err}");
    assert_eq!(stub.request_count(), 1, "a 4xx will not change on retry");
}

#[test]
fn one_failing_pair_of_four_fails_the_batch() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::respond_with(|request| {
        if request.body.contains("fn_3_a") {
            Reply::status(500)
        } else {
            Reply::json(&answer("same_logic"))
        }
    });
    let err =
        run_within(pairs(dir.path(), 4), settings(stub.base_url())).expect_err("all or nothing");
    assert!(err.to_string().contains("500"), "{err}");
}

#[test]
fn a_missing_key_errs_naming_it_with_zero_requests_made() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::json(&answer("same_logic"))]);
    let settings = Settings {
        api_key: None,
        ..settings(stub.base_url())
    };
    let err = run_within(pairs(dir.path(), 2), settings).expect_err("no key");
    assert!(err.to_string().contains("TYPESAFE_API_KEY"), "{err}");
    assert_eq!(stub.request_count(), 0);
}

#[test]
fn no_pairs_means_no_requests() {
    let stub = TypesafeStub::scripted(vec![Reply::json(&answer("same_logic"))]);
    let verdicts = run_within(Vec::new(), settings(stub.base_url())).expect("nothing to do");
    assert_eq!(verdicts, []);
    assert_eq!(stub.request_count(), 0);
}

#[test]
fn an_unreachable_api_errs_naming_where_it_looked() {
    let dir = TempDir::new().expect("temp dir");
    // A port that was free a moment ago and has nothing listening now.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port();
    let base_url = format!("http://127.0.0.1:{port}");
    let err =
        run_within(pairs(dir.path(), 1), settings(base_url.clone())).expect_err("unreachable");
    assert!(err.to_string().contains(&base_url), "{err}");
}

#[test]
fn an_answer_that_does_not_decode_errs() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::json(r#"{"answers":{}}"#)]);
    let err = run_within(pairs(dir.path(), 1), settings(stub.base_url())).expect_err("undecodable");
    assert!(err.to_string().contains("duplication_kind"), "{err}");
    assert_eq!(stub.request_count(), 1, "a malformed answer is not retried");
}

#[test]
fn a_source_that_changed_since_the_scan_errs_before_its_request() {
    let dir = TempDir::new().expect("temp dir");
    let mut pairs = pairs(dir.path(), 1);
    pairs[0].second.end_line = 99;
    let stub = TypesafeStub::scripted(vec![Reply::json(&answer("same_logic"))]);
    let err = run_within(pairs, settings(stub.base_url())).expect_err("stale span");
    assert!(err.to_string().contains("f0.rs"), "{err}");
    assert_eq!(stub.request_count(), 0);
}

#[test]
fn an_error_whose_body_cannot_be_read_is_classified_by_its_status() {
    // A 401 cut off mid-body is still a 401: final, and named as such, not
    // retried as if the API were unreachable.
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![Reply::Raw(
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 99\r\nConnection: close\r\n\r\n{\"err"
            .to_owned(),
    )]);
    let err = run_within(pairs(dir.path(), 1), settings(stub.base_url())).expect_err("bad key");
    assert!(err.to_string().contains("401"), "{err}");
    assert_eq!(
        stub.request_count(),
        1,
        "a 401 is final even with its body lost"
    );
}

#[test]
fn a_rate_limit_waits_as_long_as_the_api_asks() {
    let dir = TempDir::new().expect("temp dir");
    let stub = TypesafeStub::scripted(vec![
        Reply::status(429).with_header("Retry-After", "1"),
        Reply::json(&answer("same_logic")),
    ]);
    let started = std::time::Instant::now();
    run_within(pairs(dir.path(), 1), settings(stub.base_url())).expect("the retry succeeds");
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "waited {:?}, not the second the API asked for",
        started.elapsed()
    );
}

#[test]
fn pairs_are_requested_concurrently_whatever_pool_calls_run() {
    // Network waits must not be sized to the caller's pool, or starve it.
    let dir = TempDir::new().expect("temp dir");
    // Delayed outside the stub's lock, so the stub itself never serialises
    // the requests; only the client could.
    let stub = TypesafeStub::scripted(vec![Reply::delayed(
        Duration::from_millis(300),
        Reply::json(&answer("same_logic")),
    )]);
    let pairs = pairs(dir.path(), 8);
    let settings = settings(stub.base_url());
    let one_thread = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("a pool");
    let started = std::time::Instant::now();
    one_thread
        .install(|| run(&pairs, &settings))
        .expect("triaged");
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "eight 300 ms requests took {:?}: they ran one at a time",
        started.elapsed()
    );
}
