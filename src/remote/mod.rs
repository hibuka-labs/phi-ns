//! Remote capability packs: the shared [`Gate`] + [`RemoteNs`] + transport
//! seam (sdk-assembly §3.5, O16).
//!
//! Feature `remote` — kept in `phi-ns` rather than a separate crate on
//! purpose: `Gate` and `NamespaceProvider` are both contract-layer concepts,
//! and "look at the trait, know the whole shape" is worth more than a crate
//! boundary. P3 decided the compile-weight question the header anticipated:
//! the production [`ProcessTransport`] wraps `phi-wire-client` here (one wire
//! implementation for both device families). If the mu dependency ever drags,
//! split it (one `Cargo.toml` line + three `use`s).

mod gate;
mod ns;
mod process;
mod transport;

pub use gate::{
    Gate, GateCtx, GateDecision, GateEntry, GateJournal, GateOutcome, GatePolicy, args_digest,
};
pub use ns::RemoteNs;
pub use process::{DEFAULT_TIMEOUT_MS, ProcessTransport};
pub use transport::Transport;
