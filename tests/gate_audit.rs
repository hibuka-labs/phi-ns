//! ★ Gate audit tests (sdk-assembly §3.5 / O16): "audit un-bypassable by type
//! structure, not by code review."
//!
//! The gate owns its transport; `call` is the only public request path. These
//! tests pin the observable consequences: held calls never reach the wire,
//! every attempt is journaled write-ahead, and a failed journal blocks the
//! effect (fail-closed).

#![cfg(feature = "remote")]

use async_trait::async_trait;
use phi_cap::Capability;
use phi_ns::NsError;
use phi_ns::remote::{
    Gate, GateCtx, GateDecision, GateEntry, GateJournal, GateOutcome, GatePolicy, Transport,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Shared record of what actually got sent.
type CallLog = Arc<Mutex<Vec<(String, Value)>>>;

/// Shares its call log so tests can assert what actually got sent.
struct MockTransport {
    calls: CallLog,
    reply: Value,
}

impl MockTransport {
    fn new(reply: Value) -> (Self, CallLog) {
        let calls: CallLog = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: Arc::clone(&calls),
                reply,
            },
            calls,
        )
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn request(&self, method: &str, args: Value) -> Result<Value, NsError> {
        self.calls.lock().unwrap().push((method.to_string(), args));
        Ok(self.reply.clone())
    }
}

struct FixedPolicy(GateDecision);

impl GatePolicy for FixedPolicy {
    fn decide(&self, _ctx: &GateCtx<'_>) -> GateDecision {
        self.0
    }
}

struct RecordingJournal {
    rows: Mutex<Vec<GateEntry>>,
    fail: bool,
}

impl GateJournal for RecordingJournal {
    fn record(&self, entry: GateEntry) -> Result<(), NsError> {
        if self.fail {
            return Err(NsError::Journal("disk full".into()));
        }
        self.rows.lock().unwrap().push(entry);
        Ok(())
    }
}

fn journal() -> Arc<RecordingJournal> {
    Arc::new(RecordingJournal {
        rows: Mutex::new(Vec::new()),
        fail: false,
    })
}

#[tokio::test]
async fn deny_never_reaches_the_transport_but_is_journaled() {
    let (transport, calls) = MockTransport::new(json!({"ok": true}));
    let j = journal();
    let gate = Gate::new(
        transport,
        Arc::new(FixedPolicy(GateDecision::Deny)),
        j.clone(),
    );

    let err = gate
        .call("desk.click", Capability::Device, json!(["#ok"]))
        .await
        .unwrap_err();
    assert!(matches!(err, NsError::Denied { .. }), "{err}");
    assert!(err.is_policy_hold());

    // The effect never ran; exactly one audit row, blocked.
    assert!(calls.lock().unwrap().is_empty(), "transport was touched");
    let rows = j.rows.lock().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].method, "desk.click");
    assert_eq!(rows[0].cap, Capability::Device);
    assert_eq!(rows[0].decision, GateDecision::Deny);
    assert_eq!(rows[0].outcome, GateOutcome::Blocked);
}

#[tokio::test]
async fn ask_suspends_and_is_journaled() {
    let (transport, calls) = MockTransport::new(json!({}));
    let j = journal();
    let gate = Gate::new(
        transport,
        Arc::new(FixedPolicy(GateDecision::Ask)),
        j.clone(),
    );

    let err = gate
        .call("desk.click", Capability::Device, json!(["#ok"]))
        .await
        .unwrap_err();
    assert!(matches!(err, NsError::Suspended { .. }), "{err}");
    assert!(calls.lock().unwrap().is_empty(), "transport was touched");
    let rows = j.rows.lock().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, GateDecision::Ask);
    assert_eq!(rows[0].outcome, GateOutcome::Blocked);
}

#[tokio::test]
async fn allow_dispatches_through_the_gate_and_journals() {
    let (transport, calls) = MockTransport::new(json!({"clicked": true}));
    let j = journal();
    let gate = Gate::new(
        transport,
        Arc::new(FixedPolicy(GateDecision::Allow)),
        j.clone(),
    );

    let v = gate
        .call("desk.click", Capability::Device, json!(["#ok"]))
        .await
        .unwrap();
    assert_eq!(v, json!({"clicked": true}));
    assert_eq!(calls.lock().unwrap().len(), 1);

    let rows = j.rows.lock().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decision, GateDecision::Allow);
    assert_eq!(rows[0].outcome, GateOutcome::Dispatched);
    assert_eq!(rows[0].args_digest.len(), 16);
}

#[tokio::test]
async fn journal_failure_aborts_before_the_effect() {
    // Fail-closed: no journal row → no wire traffic.
    let (transport, calls) = MockTransport::new(json!({"clicked": true}));
    let j = Arc::new(RecordingJournal {
        rows: Mutex::new(Vec::new()),
        fail: true,
    });
    let gate = Gate::new(
        transport,
        Arc::new(FixedPolicy(GateDecision::Allow)),
        j.clone(),
    );

    let err = gate
        .call("desk.click", Capability::Device, json!(["#ok"]))
        .await
        .unwrap_err();
    assert!(matches!(err, NsError::Journal(_)), "{err}");
    assert!(calls.lock().unwrap().is_empty(), "transport was touched");
    assert!(j.rows.lock().unwrap().is_empty());
}

#[test]
fn args_digest_is_stable_and_payload_free() {
    let a = phi_ns::remote::args_digest(&json!({"cmd": "ls", "n": 1}));
    let b = phi_ns::remote::args_digest(&json!({"cmd": "ls", "n": 1}));
    let c = phi_ns::remote::args_digest(&json!({"cmd": "ls", "n": 2}));
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.len(), 16);
}
