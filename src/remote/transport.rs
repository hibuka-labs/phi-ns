//! [`Transport`] — how one request line reaches a remote capability pack
//! (sdk-assembly §3.5).
//!
//! Thin on purpose: the pack author implements "how to send a JSON line and
//! how to read one back", nothing else. Policy/journal never live here — they
//! are welded on by [`super::Gate`] before any line leaves.
//!
//! The production implementation wraps `phi-wire-client`'s
//! `process.rs` (correlated JSON-lines, cancellation never replays) — do not
//! rewrite it.

use crate::error::NsError;
use async_trait::async_trait;

/// One correlated request/response channel to a remote capability pack.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send one request and await its correlated response.
    ///
    /// Contract: a cancelled request is **never replayed** (side effects must
    /// not double-fire), and the error must keep the remote's tail diagnostics
    /// so the observation stream can show why.
    async fn request(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, NsError>;
}
