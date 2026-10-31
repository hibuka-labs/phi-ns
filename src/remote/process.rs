//! [`ProcessTransport`] — the one production [`Transport`]: a correlated
//! JSON-lines subprocess (sdk-assembly §3.5).
//!
//! Wraps `phi-wire-client`'s `process.rs` — spawn, correlation, cancellation-never-
//! replays, stderr tails — because that code earned its edge cases and a
//! rewrite would re-learn them. What this layer adds is exactly two things:
//!
//! * **deadlines** — every request carries `timeout_ms` and is locally
//!   bounded, so a wedged stub can never hang the engine;
//! * **classification** — the wire's two-way failure split
//!   (`rejected/not_applied` vs `ACTION_OUTCOME_UNKNOWN`, inventory §3.3)
//!   lands in [`NsError::Device`], channel death in [`NsError::Transport`].
//!
//! The wire itself is `phi-wire-protocol`: `{"op": …, "_request_id": …, …params}` in,
//! `{"_request_id": …, "result"|"error": …}` out. Both the mu broker and the
//! cua stub speak it — one implementation, two device families.

use super::transport::Transport;
use crate::error::{DeviceOutcome, NsError};
use async_trait::async_trait;
use phi_wire_client::process::ProcessClient;
use phi_wire_protocol::{Error as WireError, Reply, Request};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// Local grace on top of the wire deadline before the transport gives up.
const DEADLINE_GRACE: Duration = Duration::from_millis(2_000);

/// Default per-request wire deadline when the caller passes none (ms).
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// One correlated channel to one stub process.
pub struct ProcessTransport {
    client: Arc<ProcessClient>,
    timeout_ms: u64,
}

impl ProcessTransport {
    /// Spawn the stub and wire it up. `program` is the stub sidecar (or the
    /// mu broker); `args`/`env` are its configuration. Kills the child on drop.
    pub fn spawn(
        program: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<Arc<Self>, NsError> {
        Self::spawn_with_timeout(program, args, env, DEFAULT_TIMEOUT_MS)
    }

    /// [`ProcessTransport::spawn`] with an explicit wire deadline for callers
    /// that pass no `timeout_ms` of their own.
    pub fn spawn_with_timeout(
        program: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        timeout_ms: u64,
    ) -> Result<Arc<Self>, NsError> {
        let client = ProcessClient::spawn(program, args, env).map_err(map_wire_error)?;
        Ok(Arc::new(Self { client, timeout_ms }))
    }

    /// Stop the stub cleanly. Session teardown paths call this; dropping the
    /// transport also works (kill-on-drop), this just records a real exit.
    pub async fn shutdown(&self) -> Result<(), NsError> {
        self.client.shutdown().await.map_err(map_wire_error)
    }

    /// Why the stub died, if it did — for the observation stream.
    pub fn exit_reason(&self) -> Option<String> {
        self.client.exit_reason()
    }

    fn assemble(&self, method: &str, args: Value) -> Result<Request, NsError> {
        let mut parameters: BTreeMap<String, Value> = match args {
            Value::Null => BTreeMap::new(),
            Value::Object(map) => map.into_iter().collect(),
            other => {
                return Err(NsError::Contract(format!(
                    "request {method} args must be a JSON object, got {other}"
                )));
            }
        };
        parameters
            .entry("timeout_ms".to_string())
            .or_insert(Value::from(self.timeout_ms));
        Ok(Request {
            op: method.to_string(),
            request_id: None,
            parameters,
        })
    }
}

#[async_trait]
impl Transport for ProcessTransport {
    async fn request(&self, method: &str, args: Value) -> Result<Value, NsError> {
        let request = self.assemble(method, args)?;
        let mutating = request.is_mutation();
        let wire_deadline = Duration::from_millis(
            request
                .parameters
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(self.timeout_ms),
        );
        let reply = tokio::time::timeout(wire_deadline + DEADLINE_GRACE, self.client.send(request))
            .await
            .map_err(|_| deadline_failure(method, mutating))?
            .map_err(map_wire_error)?;

        match reply {
            Reply {
                result: Some(value),
                error: None,
                ..
            } => Ok(value),
            Reply {
                result: None,
                error: Some(error),
                ..
            } => Err(classify_reply_error(method, error)),
            // phi-wire-protocol's own contract: exactly one of result/error.
            other => Err(NsError::Transport(format!(
                "protocol mismatch: reply for {method} carries neither result nor error ({other:?})"
            ))),
        }
    }
}

/// A deadline passed on a call the channel never answered.
///
/// For a mutating request the frame is already written (the wire contract
/// never replays), so the effect is unconfirmed; for a read nothing can have
/// been applied, so different arguments — or the same ones — may retry.
fn deadline_failure(method: &str, mutating: bool) -> NsError {
    let outcome = if mutating {
        DeviceOutcome::OutcomeUnknown
    } else {
        DeviceOutcome::Rejected
    };
    NsError::device(
        method,
        outcome,
        "deadline passed with no reply (check m.status for mutating calls)",
    )
}

/// The stub answered with a structured error: classify on the wire's own
/// vocabulary (inventory §3.3).
fn classify_reply_error(method: &str, error: WireError) -> NsError {
    match error.code.as_str() {
        // The device's own "did that land?" unknown — never conflate with a
        // refusal, even for reads (the stub is telling us its ledger has a
        // pending entry).
        "ACTION_OUTCOME_UNKNOWN" => NsError::device(
            method,
            DeviceOutcome::OutcomeUnknown,
            format!("{}: {}", error.code, error.message),
        ),
        // Channel-level verdicts — the stub is gone or was never viable.
        "DEVICE_OFFLINE" | "SETUP_REQUIRED" | "PROTOCOL_MISMATCH" => map_wire_error(error),
        // Everything else is a refusal: STALE_TARGET, INVALID_ARGUMENT,
        // UNSUPPORTED_CAPABILITY, TARGET_NOT_FOREGROUND, TARGET_OCCLUDED,
        // ACTION_REJECTED, SESSION_CLOSED, … — nothing was applied.
        _ => NsError::device(
            method,
            DeviceOutcome::Rejected,
            format!("{}: {}", error.code, error.message),
        ),
    }
}

/// Errors that never had a request attached: spawn failures and channel death.
fn map_wire_error(error: WireError) -> NsError {
    NsError::Transport(format!("{}: {}", error.code, error.message))
}
