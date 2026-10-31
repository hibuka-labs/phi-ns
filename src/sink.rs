//! [`WorkerSink`] — the seam that keeps `phi-ns` free of `codexec`
//! (sdk-assembly §3.2).
//!
//! Capability packs receive a `&mut dyn WorkerSink` at install time, never a
//! concrete worker: register a host function, evaluate install-time code in
//! the worker, read the worker's `_NS` back. `codexec`'s `Worker` implements
//! this trait; the dependency arrow only ever points `codexec → phi-ns`.

use crate::error::NsError;
use async_trait::async_trait;

/// A host-side implementation behind an injected callable.
///
/// Worker-side shims call across the process boundary into this; the engine
/// runs policy + journal at that single choke point before dispatching here.
#[async_trait]
pub trait HostCall: Send + Sync {
    /// Run one call. `args` is the positional-argument JSON array the shim
    /// sent; the return value is marshalled back to the worker language.
    async fn call(&self, args: Vec<serde_json::Value>) -> Result<serde_json::Value, NsError>;
}

/// One entry of the worker's global namespace, as the engine sees it.
///
/// The worker language computes `kind` (type name) and `repr`; a pack's
/// shim may add `detail` via a `_phi_summary_()` method on the injected
/// object (the summary *part* of `name(kind, summary)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsObject {
    /// Binding name in the worker's global scope.
    pub name: String,
    /// Type name as the worker language reports it (`"int"`, `"ShellSdk"`).
    pub kind: String,
    /// Truncated `repr(value)` — the fallback summary for model-built values.
    pub repr: String,
    /// Summary part from the object's own shim (`_phi_summary_()`), if any.
    pub detail: Option<String>,
}

/// The minimal worker seam every capability pack installs through.
#[async_trait]
pub trait WorkerSink: Send {
    /// Register a host function under a dotted name (`"sh.run"`). The
    /// worker-side namespace gets a callable proxy; the implementation runs
    /// on the host behind policy + journal.
    fn register_fn(&mut self, name: &str, call: Box<dyn HostCall>) -> Result<(), NsError>;

    /// Evaluate source in the worker — how a pack installs its shim classes
    /// (`sh`, `Output`, …) into the worker namespace.
    ///
    /// Shims call back with `_phi_call(name, args)` (synchronous; raises into
    /// the cell when policy holds the call) — that name is the eval contract
    /// between pack shims and the worker runtime.
    async fn eval(&mut self, code: &str) -> Result<(), NsError>;

    /// Read the worker's `_NS` back for snapshot assembly (sdk-assembly §8).
    async fn namespace_entries(&mut self) -> Result<Vec<NsObject>, NsError>;
}
