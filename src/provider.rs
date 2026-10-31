//! [`NamespaceProvider`] — the single contract a capability pack implements
//! (sdk-assembly §3.2). Plus the snapshot composition that consults packs.

use crate::capture::{CaptureUse, face_of};
use crate::doc;
use crate::error::NsError;
use crate::method::MethodSpec;
use crate::sink::{NsObject, WorkerSink};
use async_trait::async_trait;
use phi_cap::CapabilitySpec;

/// A capability pack: one namespace family (`sh.*`, `b.*`, `desk.*` + `m.*`).
///
/// Implementing this trait is the whole job of a pack crate. Everything else
/// — prompt docs, approval keys, image contract, snapshots — derives from
/// [`NamespaceProvider::methods`].
#[async_trait]
pub trait NamespaceProvider: Send + Sync {
    /// Root names this pack injects: `["b"]` / `["desk", "m"]` / `["sh"]`.
    /// Each must be a valid worker-language identifier (Python:
    /// `[A-Za-z_][A-Za-z0-9_]*`) and must not clash with an already-registered
    /// root (registration is fail-closed).
    fn namespaces(&self) -> &'static [&'static str];

    /// ★ The single declaration source. Method names carry their root prefix.
    fn methods(&self) -> Vec<MethodSpec>;

    /// Inject capabilities into the worker. The pack decides its injection
    /// mode (in-process shim vs. IPC proxy — sdk-assembly §3.5), but any pack
    /// whose calls could dodge policy/journal MUST go through an IPC proxy.
    async fn install(&self, sink: &mut dyn WorkerSink) -> Result<(), NsError>;

    /// Give a state summary **only for objects this pack owns**; `None` for
    /// everything else. The full snapshot is assembled by the engine from the
    /// worker's `_NS` (sdk-assembly §8).
    fn summarize(&self, _name: &str, _obj: &NsObject) -> Option<String> {
        None
    }

    /// Lifecycle of resident resources. Stateless packs leave these alone.
    async fn on_session_start(&self) -> Result<(), NsError> {
        Ok(())
    }

    /// See [`NamespaceProvider::on_session_start`].
    async fn on_session_end(&self) -> Result<(), NsError> {
        Ok(())
    }

    // ---- derived views (default methods; packs never override) ----

    /// Prompt face — derived from [`NamespaceProvider::methods`].
    fn api_doc(&self) -> String {
        doc::api_doc(&self.methods())
    }

    /// Review face — derived from [`NamespaceProvider::methods`].
    fn capabilities(&self) -> Vec<CapabilitySpec> {
        doc::capabilities(&self.methods())
    }
}

/// Generic fallback summary for model-built objects (sdk-assembly §8.1).
///
/// Packs get first refusal via [`NamespaceProvider::summarize`]; whatever no
/// pack claims is summarized here — typically the worker-side `_phi_summary_`
/// hint or the truncated `repr`.
pub fn generic_summary(obj: &NsObject) -> String {
    match &obj.detail {
        Some(d) => d.clone(),
        None => obj.repr.clone(),
    }
}

/// `namespace: x(int, 41), out(Output, 12 lines), sh(ShellSdk)` — the one
/// line appended to every cell's observation so the model never has to guess
/// what is still alive (sdk-assembly §8).
///
/// Two layers, one line: pack-owned objects answer through their `summarize`;
/// model-built names fall back to [`generic_summary`].
pub fn compose_snapshot(entries: &[NsObject], packs: &[&dyn NamespaceProvider]) -> String {
    let parts: Vec<String> = entries
        .iter()
        .map(|obj| {
            let summary = packs
                .iter()
                .find_map(|p| p.summarize(&obj.name, obj))
                .unwrap_or_else(|| generic_summary(obj));
            if summary.is_empty() {
                format!("{}({})", obj.name, obj.kind)
            } else {
                format!("{}({}, {})", obj.name, obj.kind, summary)
            }
        })
        .collect();
    format!("namespace: {}", parts.join(", "))
}

// ---- contract check (reused by every pack's tests) ----

/// Run the consistency checks every capability pack must pass
/// (sdk-assembly §9.7): declaration source self-consistency, namespace
/// validity, derived views in agreement, bounded docs.
///
/// Wire this into a pack's tests with [`provider_contract_tests`].
pub fn check_provider_contract(ns: &dyn NamespaceProvider) -> Result<(), String> {
    let roots = ns.namespaces();
    if roots.is_empty() {
        return Err("namespaces() is empty".into());
    }
    let mut seen_roots: Vec<&str> = Vec::new();
    for r in roots {
        if !is_ident(r) {
            return Err(format!("namespace root {r:?} is not a valid identifier"));
        }
        if seen_roots.contains(r) {
            return Err(format!("namespace root {r:?} declared twice"));
        }
        seen_roots.push(r);
    }

    let methods = ns.methods();
    if methods.is_empty() {
        return Err("methods() is empty".into());
    }
    let mut seen_names: Vec<&str> = Vec::new();
    for m in &methods {
        if seen_names.contains(&m.name) {
            return Err(format!("method {:?} declared twice", m.name));
        }
        seen_names.push(m.name);

        let Some((root, tail)) = m.name.split_once('.') else {
            return Err(format!("method {:?} lacks its namespace root", m.name));
        };
        if !roots.contains(&root) {
            return Err(format!(
                "method {:?} is rooted at {root:?}, which namespaces() does not declare",
                m.name
            ));
        }
        if !is_ident(tail) {
            return Err(format!(
                "method {:?} tail is not a valid identifier",
                m.name
            ));
        }
        if m.doc.trim().is_empty() {
            return Err(format!("method {:?} has empty doc", m.name));
        }
        if m.doc.len() > 200 {
            return Err(format!(
                "method {:?} doc is {} chars; keep one line (≤200)",
                m.name,
                m.doc.len()
            ));
        }
    }

    // Derived views must agree with the source — no drift by construction,
    // asserted anyway so a future refactor cannot invent a second source.
    let doc_face = ns.api_doc();
    for m in &methods {
        if !doc_face.contains(m.name) || !doc_face.contains(m.doc) {
            return Err(format!("api_doc() does not reflect method {:?}", m.name));
        }
    }
    let caps_face = ns.capabilities();
    for m in &methods {
        for c in &m.caps {
            if !caps_face.contains(c) {
                return Err(format!(
                    "capabilities() is missing a declaration of {:?}",
                    m.name
                ));
            }
        }
    }
    if caps_face.len() > methods.iter().map(|m| m.caps.len()).sum::<usize>() {
        return Err("capabilities() grew beyond the declaration source".into());
    }

    // Capture roles (inventory §3.2) must close over a face: consuming without
    // any look to mint would fail closed forever — a declaration bug, caught
    // at registration instead of at the first act.
    let mut minting_faces: Vec<&str> = Vec::new();
    for m in &methods {
        if m.capture == CaptureUse::Mint {
            minting_faces.push(face_of(m.name));
        }
    }
    for m in &methods {
        if m.capture == CaptureUse::Consume && !minting_faces.contains(&face_of(m.name)) {
            return Err(format!(
                "method {:?} consumes captures but no method on face {:?} mints one",
                m.name,
                face_of(m.name)
            ));
        }
    }
    Ok(())
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

/// One-line integration for a pack's test suite:
///
/// ```ignore
/// phi_ns::provider_contract_tests!(PhiNsBrowser::new(cfg));
/// ```
#[macro_export]
macro_rules! provider_contract_tests {
    ($ctor:expr) => {
        #[test]
        fn ns_provider_contract() {
            let ns = $ctor;
            $crate::check_provider_contract(&ns).expect("provider contract violated");
        }
    };
}
