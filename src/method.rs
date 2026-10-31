//! [`MethodSpec`] — **the single declaration source** (sdk-assembly §3.1).
//!
//! A pack declares each method once, here. The three views the rest of the
//! system needs are all *derived* from this one source and never maintained
//! separately:
//!
//! | view | derivation | used by |
//! |---|---|---|
//! | `api_doc()` | `name — doc` per method | system prompt |
//! | `help_doc()` | seed line + policy/capture role | `help(name)` on demand |
//! | `capabilities()` | union of `caps` | human review / registry pinning |
//! | host-fn decision key | `class_of_all(caps)` | `Policy` |
//!
//! Consistency is not enforced by tests (that is the lazy fix) — there is no
//! second declaration point to drift from.

use crate::capture::CaptureUse;
use phi_cap::{Capability, CapabilitySpec, class_of_all};

/// One injected method's declaration.
#[derive(Debug, Clone)]
pub struct MethodSpec {
    /// Injection name with its namespace root: `"b.click"` / `"sh.run"`.
    pub name: &'static str,
    /// What this method touches, scoped (declaration vocabulary = `phi_cap`).
    /// Empty means pure computation.
    pub caps: Vec<CapabilitySpec>,
    /// One line of docs; the seed-list line derived from it.
    pub doc: &'static str,
    /// Whether the return may carry multimodal payloads (derives the
    /// `CellEvent::Image` contract — see `image`).
    pub multimodal: bool,
    /// How this method relates to captures (inventory §3.2): a look mints,
    /// a target-acting call consumes — enforced at the chokepoint.
    pub capture: CaptureUse,
}

impl MethodSpec {
    /// Build a declaration (see the table in `provider` docs for examples).
    pub const fn new(
        name: &'static str,
        caps: Vec<CapabilitySpec>,
        doc: &'static str,
        multimodal: bool,
    ) -> Self {
        Self {
            name,
            caps,
            doc,
            multimodal,
            capture: CaptureUse::None,
        }
    }

    /// Set the capture role — chained on the declaration rows that have one.
    pub const fn capture(mut self, capture: CaptureUse) -> Self {
        self.capture = capture;
        self
    }

    /// The policy key for this method: highest-risk class of its caps (O7).
    pub fn capability(&self) -> Capability {
        class_of_all(&self.caps)
    }

    /// The derived seed-list line (`api_doc` view of this one method).
    pub fn api_line(&self) -> String {
        format!("{} — {}", self.name, self.doc)
    }
}

/// Shorthand constructor for [`MethodSpec`].
pub fn m(
    name: &'static str,
    caps: &[CapabilitySpec],
    doc: &'static str,
    multimodal: bool,
) -> MethodSpec {
    MethodSpec::new(name, caps.to_vec(), doc, multimodal)
}
