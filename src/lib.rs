//! # phi-ns
//!
//! The capability-pack contract for the phi family: one trait, one
//! declaration source, one audit chokepoint.
//!
//! * [`NamespaceProvider`] — what a capability pack *is* (sdk-assembly §3.2)
//! * [`MethodSpec`] — the single declaration source; `api_doc` /
//!   `capabilities` / policy keys all derive from it (§3.1)
//! * [`CaptureLedger`] — the one-use capture ledger: one look authorizes one
//!   action attempt (inventory §3.2)
//! * [`WorkerSink`] — the seam that keeps this crate free of `codexec`
//! * [`ImageRef`] — the multimodal return channel (§4.2)
//! * [`remote::Gate`] — policy + journal welded onto remote calls; audit
//!   un-bypassable by type structure (O16)
//! * [`builtin::PhiNsShell`] — the built-in `sh.*` pack and the reference for
//!   pack authors
//!
//! Dependency direction: `codexec → phi-ns → phi-cap`, and
//! `phi-ns-* ↛ codexec` (sdk-assembly §9.3).

mod doc;
mod error;
mod image;
mod method;
mod provider;
mod sink;

#[cfg(feature = "builtin-shell")]
pub mod builtin;
pub mod capture;
#[cfg(feature = "remote")]
pub mod remote;

pub use capture::{
    CAPTURE_ID_KEY, CAPTURE_KEY, CaptureId, CaptureLedger, CaptureRefusal, CaptureUse,
    ConsumedCapture, face_of,
};
pub use doc::{api_doc, capabilities, help_doc, host_fn_caps};
pub use error::{DeviceOutcome, NsError};
pub use image::{ImageRef, ImageSource};
pub use method::{MethodSpec, m};
pub use provider::{NamespaceProvider, check_provider_contract, compose_snapshot, generic_summary};
pub use sink::{HostCall, NsObject, WorkerSink};
