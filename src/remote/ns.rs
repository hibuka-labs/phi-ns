//! [`RemoteNs`] — a capability pack whose implementation lives in another
//! process (sdk-assembly §3.5).
//!
//! The pack author's job is only [`Transport`]: how to send a JSON line and
//! how to parse one. Everything else — declarations, host-fn proxies, the
//! gate — is here. The proxies forward through [`Gate::call`] and there is no
//! accessor for the raw transport, so skipping policy/journal is not a
//! mistake anyone can make.

use crate::error::NsError;
use crate::method::MethodSpec;
use crate::provider::NamespaceProvider;
use crate::remote::gate::Gate;
use crate::remote::transport::Transport;
use crate::sink::{HostCall, WorkerSink};
use async_trait::async_trait;
use phi_cap::Capability;
use std::sync::Arc;

/// A namespace family served by a remote pack process.
pub struct RemoteNs<S: Transport> {
    gate: Arc<Gate<S>>,
    namespaces: &'static [&'static str],
    methods: Vec<MethodSpec>,
}

impl<S: Transport + 'static> RemoteNs<S> {
    /// Wire a remote pack: its declarations plus the gated transport.
    pub fn new(
        gate: Arc<Gate<S>>,
        namespaces: &'static [&'static str],
        methods: Vec<MethodSpec>,
    ) -> Self {
        Self {
            gate,
            namespaces,
            methods,
        }
    }
}

/// Worker-side proxy for one remote method — the only bridge to the gate.
struct GateProxy<S: Transport> {
    gate: Arc<Gate<S>>,
    method: &'static str,
    cap: Capability,
}

#[async_trait]
impl<S: Transport + 'static> HostCall for GateProxy<S> {
    async fn call(&self, args: Vec<serde_json::Value>) -> Result<serde_json::Value, NsError> {
        self.gate
            .call(self.method, self.cap, serde_json::Value::Array(args))
            .await
    }
}

#[async_trait]
impl<S: Transport + 'static> NamespaceProvider for RemoteNs<S> {
    fn namespaces(&self) -> &'static [&'static str] {
        self.namespaces
    }

    fn methods(&self) -> Vec<MethodSpec> {
        self.methods.clone()
    }

    async fn install(&self, sink: &mut dyn WorkerSink) -> Result<(), NsError> {
        for m in &self.methods {
            let proxy = GateProxy {
                gate: Arc::clone(&self.gate),
                method: m.name,
                cap: m.capability(),
            };
            sink.register_fn(m.name, Box::new(proxy))?;
        }
        Ok(())
    }

    // Remote objects summarize themselves through their namespace entries;
    // nothing to add here until a pack needs live remote state (P3).
}
