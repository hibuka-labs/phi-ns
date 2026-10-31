//! The one error type of the namespace layer.

/// Errors raised by the namespace layer: registration, install, host calls,
/// the gate, and the worker seam.
#[derive(Debug, thiserror::Error)]
pub enum NsError {
    /// Namespace root invalid, duplicated, or clashing with an existing one.
    /// Registration is fail-closed: nothing installs until every name checks out.
    #[error("namespace name invalid or already registered: {0}")]
    NamespaceConflict(String),

    /// A method name that no pack declared — fail-closed, never dispatched.
    #[error("method {0} is not declared by any pack")]
    UndeclaredMethod(String),

    /// A provider's declaration violates the contract (see
    /// [`crate::check_provider_contract`]).
    #[error("provider contract violated: {0}")]
    Contract(String),

    /// The worker seam (register / eval / read `_NS`) failed.
    #[error("worker sink error: {0}")]
    Sink(String),

    /// Transport failure under [`crate::remote::RemoteNs`] — the channel
    /// itself is gone (spawn failure, EOF, write failure). Recovery is
    /// session-level; nothing about the last call is knowable here.
    #[error("transport error: {0}")]
    Transport(String),

    /// A device-side call failed in a way the wire can classify
    /// (sdk-assembly §3.5, inventory §3.3): the remote answered with a
    /// structured verdict, and the two verdicts must not be conflated —
    /// one is "nothing happened", the other is "never replay".
    #[error("device {outcome} for {method}: {reason}")]
    Device {
        /// The declared method that failed (for the observation stream).
        method: String,
        /// Whether nothing happened or the effect is unknowable.
        outcome: DeviceOutcome,
        /// Remote's own diagnostics, kept verbatim.
        reason: String,
    },

    /// Policy refused the call — the effect did not run.
    #[error("denied {method} ({cap}): {reason}")]
    Denied {
        /// Method that was refused.
        method: String,
        /// Coarse capability class it was decided under.
        cap: String,
        /// Why, for the observation stream.
        reason: String,
    },

    /// The capture gate refused (inventory §3.2): the call acts on a capture
    /// and none authorized it — nothing reached the device. Same retry shape
    /// as [`DeviceOutcome::Rejected`] (look again, then act), but the verdict
    /// is the host ledger's, not the wire's.
    #[error("capture gate refused {method}: {reason}")]
    Capture {
        /// Method that was refused.
        method: String,
        /// Why (`CaptureRefusal`'s reason: consumed / expired / never taken).
        reason: String,
    },

    /// Policy asked for a decision — the engine suspends until a human or an
    /// upstream approver decides (codexec-design §8.1).
    #[error("suspended {method} ({cap}): awaiting decision")]
    Suspended {
        /// Method held for a decision.
        method: String,
        /// Coarse capability class it was decided under.
        cap: String,
    },

    /// Journal write failed — the action is aborted (fail-closed).
    #[error("journal error: {0}")]
    Journal(String),

    /// Capability-pack install failed.
    #[error("install failed: {0}")]
    Install(String),
}

impl NsError {
    /// True when a policy held the call back (`Deny` or `Ask`). The engine
    /// maps both to "this step is halted" (codexec-design D4).
    pub const fn is_policy_hold(&self) -> bool {
        matches!(self, Self::Denied { .. } | Self::Suspended { .. })
    }

    /// Build a classified device failure without spelling the struct out.
    pub fn device(
        method: impl Into<String>,
        outcome: DeviceOutcome,
        reason: impl Into<String>,
    ) -> Self {
        Self::Device {
            method: method.into(),
            outcome,
            reason: reason.into(),
        }
    }
}

/// What a device-side failure means for retry (inventory §3.3 — the wire's
/// own two-way split: `rejected/not_applied` vs `ACTION_OUTCOME_UNKNOWN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceOutcome {
    /// The remote refused before applying anything — different arguments may
    /// succeed, and re-running the same call cannot double-fire.
    #[error("rejected (nothing applied)")]
    Rejected,
    /// Input reached the device but its effect is unconfirmed (mid-dispatch
    /// failure, or a deadline passed on a mutating call). **Never replay** —
    /// ask the device's outcome ledger (`m.status`) instead.
    #[error("outcome unknown (do not replay)")]
    OutcomeUnknown,
}
