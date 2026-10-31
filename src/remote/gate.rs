//! [`Gate`] — the **single chokepoint** where policy and journal are welded
//! onto every remote call (sdk-assembly §3.5 / O16).
//!
//! Device packs MUST go over IPC; "over IPC" only means audit-unbypassable if
//! nothing can talk to the transport except through here. So the Gate **owns**
//! its transport: the field is private and the only public request path is
//! [`Gate::call`], which journals (write-ahead, fail-closed) and asks policy
//! before a single byte leaves. That is a property of the type structure, not
//! of a code-review promise.
//!
//! Pack authors implement [`Transport`] — send a line, parse a line — and
//! never get an opportunity to skip the Gate.

use crate::error::NsError;
use crate::remote::transport::Transport;
use phi_cap::Capability;
use std::sync::Arc;

/// What the gate's policy answered for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// Run it.
    Allow,
    /// Hold for a human/upstream decision; the engine suspends (codexec
    /// design §8.1).
    Ask,
    /// Refuse.
    Deny,
}

/// What actually happened to the call once decided — the journal's outcome
/// granularity at the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateOutcome {
    /// Dispatched to the transport.
    Dispatched,
    /// Held back; the effect never ran.
    Blocked,
}

/// What a gate policy sees. Coarse class + who is asking + what with.
pub struct GateCtx<'a> {
    /// Method name as declared (`"desk.click"`).
    pub method: &'a str,
    /// Coarse capability class (the policy key).
    pub cap: Capability,
    /// Stable digest of the call arguments (summary, not the payload).
    pub args_digest: &'a str,
}

/// Application-side decision maker. The gate calls it on every request.
pub trait GatePolicy: Send + Sync {
    /// Decide one call.
    fn decide(&self, ctx: &GateCtx<'_>) -> GateDecision;
}

/// One audit row produced at the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateEntry {
    /// Wall-clock time (unix milliseconds).
    pub time_ms: u64,
    /// Method name.
    pub method: String,
    /// Coarse capability class it was decided under.
    pub cap: Capability,
    /// Stable digest of the arguments.
    pub args_digest: String,
    /// What policy answered.
    pub decision: GateDecision,
    /// What happened next.
    pub outcome: GateOutcome,
}

/// Application-side audit sink. `record` failing must fail the call —
/// audit-closed means "no journal, no effect".
pub trait GateJournal: Send + Sync {
    /// Record one side-effecting call. Errors propagate (fail-closed).
    fn record(&self, entry: GateEntry) -> Result<(), NsError>;
}

/// The chokepoint: owns the transport, exposes only [`Gate::call`].
///
/// `codexec` adapts its engine-level `Policy`/`Journal` onto
/// [`GatePolicy`]/[`GateJournal`] (see `codexec::policy`) so there is one
/// audit trail for both in-process and remote packs.
pub struct Gate<S: Transport> {
    transport: S,
    policy: Arc<dyn GatePolicy>,
    journal: Arc<dyn GateJournal>,
}

impl<S: Transport> Gate<S> {
    /// Build the gate. Both hooks are injected — the gate itself makes no
    /// value judgments (engine mechanism, application policy).
    pub fn new(transport: S, policy: Arc<dyn GatePolicy>, journal: Arc<dyn GateJournal>) -> Self {
        Self {
            transport,
            policy,
            journal,
        }
    }

    /// **The only way to reach the transport.**
    ///
    /// Order is fixed: decide → journal (write-ahead) → dispatch. Journal
    /// failure aborts before the effect; `Ask`/`Deny` never reach the wire.
    pub async fn call(
        &self,
        method: &str,
        cap: Capability,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, NsError> {
        let args_digest = args_digest(&args);
        let decision = self.policy.decide(&GateCtx {
            method,
            cap,
            args_digest: &args_digest,
        });
        let outcome = match decision {
            GateDecision::Allow => GateOutcome::Dispatched,
            GateDecision::Ask | GateDecision::Deny => GateOutcome::Blocked,
        };
        // Write-ahead audit: if we cannot journal, we do not act.
        self.journal.record(GateEntry {
            time_ms: now_ms(),
            method: method.to_string(),
            cap,
            args_digest,
            decision,
            outcome,
        })?;
        match decision {
            GateDecision::Allow => self.transport.request(method, args).await,
            GateDecision::Ask => Err(NsError::Suspended {
                method: method.to_string(),
                cap: cap.to_string(),
            }),
            GateDecision::Deny => Err(NsError::Denied {
                method: method.to_string(),
                cap: cap.to_string(),
                reason: "policy denied at the gate".into(),
            }),
        }
    }
}

/// Stable argument digest for audit rows (FNV-1a 64, hex) — correlation
/// without storing payloads.
pub fn args_digest(args: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(args).unwrap_or_default();
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
