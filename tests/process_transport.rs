//! [`ProcessTransport`] against a fake stub (D9: 假 stub 跑 CI).
//!
//! One scripted stub, one failure shape per test — the wire's two-way split
//! (inventory §3.3) and the local deadline split must stay distinguishable,
//! because "nothing happened" and "never replay" are opposite advice.

use phi_ns::remote::{ProcessTransport, Transport};
use phi_ns::{DeviceOutcome, NsError};
use serde_json::json;
use std::path::PathBuf;

fn stub() -> (String, Vec<String>) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("stub_echo.py");
    (
        "python3".into(),
        vec![script.to_string_lossy().into_owned()],
    )
}

fn spawn() -> std::sync::Arc<ProcessTransport> {
    let (program, args) = stub();
    ProcessTransport::spawn(&program, &args, &Default::default()).expect("stub spawns")
}

fn device_outcome(err: &NsError) -> DeviceOutcome {
    match err {
        NsError::Device { outcome, .. } => *outcome,
        other => panic!("expected a classified device failure, got {other:?}"),
    }
}

#[tokio::test]
async fn echo_round_trips_params_through_the_wire() {
    let transport = spawn();
    let value = transport
        .request("echo", json!({"target": "x", "n": 2}))
        .await
        .expect("echo answers");
    assert_eq!(value["op"], "echo");
    // timeout_ms is injected by the transport; the caller's params ride along.
    assert_eq!(value["params"]["target"], "x");
    assert_eq!(value["params"]["n"], 2);
    assert!(value["params"]["timeout_ms"].as_u64().is_some());
}

#[tokio::test]
async fn wire_refusal_is_device_rejected() {
    let transport = spawn();
    let err = transport
        .request("refuse", json!({}))
        .await
        .expect_err("stub refuses");
    assert_eq!(device_outcome(&err), DeviceOutcome::Rejected);
    assert!(err.to_string().contains("UNSUPPORTED_CAPABILITY"), "{err}");
}

#[tokio::test]
async fn wire_outcome_unknown_is_never_conflated_with_refusal() {
    let transport = spawn();
    let err = transport
        .request("unknown", json!({}))
        .await
        .expect_err("stub reports unknown outcome");
    assert_eq!(device_outcome(&err), DeviceOutcome::OutcomeUnknown);
}

#[tokio::test]
async fn wire_offline_is_a_channel_failure_not_a_device_verdict() {
    let transport = spawn();
    let err = transport
        .request("offline", json!({}))
        .await
        .expect_err("stub reports offline");
    assert!(
        matches!(err, NsError::Transport(_)),
        "DEVICE_OFFLINE must map to Transport, got {err:?}"
    );
}

#[tokio::test]
async fn stub_death_mid_request_is_a_channel_failure() {
    let transport = spawn();
    let err = transport
        .request("die", json!({}))
        .await
        .expect_err("stub exits without answering");
    assert!(
        matches!(err, NsError::Transport(_)),
        "EOF must map to Transport, got {err:?}"
    );
}

#[tokio::test]
async fn a_deadline_on_a_mutating_call_means_do_not_replay() {
    let transport = spawn();
    // op=action is a mutation per the wire's own `is_mutation`; the stub
    // sleeps past any deadline, so the effect is unconfirmed.
    let err = transport
        .request("action", json!({"timeout_ms": 1, "sleep_s": 60}))
        .await
        .expect_err("mutating call outruns its deadline");
    assert_eq!(device_outcome(&err), DeviceOutcome::OutcomeUnknown);
}

#[tokio::test]
async fn a_deadline_on_a_read_is_just_a_refusal() {
    let transport = spawn();
    // op=observe is a read: nothing could have been applied, so the call may
    // simply run again.
    let err = transport
        .request("observe", json!({"timeout_ms": 1, "sleep_s": 60}))
        .await
        .expect_err("read outruns its deadline");
    assert_eq!(device_outcome(&err), DeviceOutcome::Rejected);
}

#[tokio::test]
async fn non_object_args_are_a_contract_error_not_a_hang() {
    let transport = spawn();
    let err = transport
        .request("echo", json!(["positional"]))
        .await
        .expect_err("positional args cannot flatten onto the wire");
    assert!(
        matches!(err, NsError::Contract(_)),
        "expected Contract, got {err:?}"
    );
}

#[tokio::test]
async fn shutdown_records_a_real_exit() {
    let transport = spawn();
    transport.shutdown().await.expect("clean stop");
    assert!(transport.exit_reason().is_some());
}
