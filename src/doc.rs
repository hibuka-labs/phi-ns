//! Derived views of a pack's [`MethodSpec`] declarations (sdk-assembly §3.1).
//!
//! These functions are the **only** implementations of the `api_doc()`,
//! `help_doc()` and `capabilities()` views. Packs never override them; there
//! is no second place where the same facts are restated.

use crate::capture::CaptureUse;
use crate::method::MethodSpec;
use phi_cap::CapabilitySpec;

/// The prompt-face view: one `name — doc` line per method.
pub fn api_doc(methods: &[MethodSpec]) -> String {
    methods
        .iter()
        .map(MethodSpec::api_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The on-demand view of one declaration (`help("sh.run")` inside a cell):
/// the seed line plus what policy and the capture chokepoint key on. Same
/// source as [`api_doc`] — the doc text is never re-stated elsewhere, so a
/// `help()` answer cannot drift from the seed list or the engine's behavior.
pub fn help_doc(method: &MethodSpec) -> String {
    let mut out = method.api_line();
    out.push('\n');
    out.push_str(&format!("capability: {}", method.capability()));
    if method.multimodal {
        out.push_str(" (returns images)");
    }
    match method.capture {
        CaptureUse::None => {}
        CaptureUse::Mint => {
            out.push_str("\ncapture: mints a capture id (pass it to a consuming call)")
        }
        CaptureUse::Consume => out.push_str("\ncapture: consumes a capture id"),
    }
    out
}

/// The review-face view: the declaration union, first-seen order, de-duped.
pub fn capabilities(methods: &[MethodSpec]) -> Vec<CapabilitySpec> {
    let mut out: Vec<CapabilitySpec> = Vec::new();
    for m in methods {
        for c in &m.caps {
            if !out.contains(c) {
                out.push(c.clone());
            }
        }
    }
    out
}

/// The decision-face view: the policy key per method (O7 highest-risk class).
pub fn host_fn_caps(methods: &[MethodSpec]) -> Vec<(&'static str, phi_cap::Capability)> {
    methods.iter().map(|m| (m.name, m.capability())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::CaptureUse;
    use crate::method::m;
    use phi_cap::{device_input, net_http};

    #[test]
    fn help_doc_carries_the_declaration_line_and_the_roles_policy_keys_on() {
        let spec = m(
            "b.click",
            &[device_input(&["tab"])],
            "click(sel): click the element a selector points at",
            false,
        )
        .capture(CaptureUse::Consume);
        let doc = help_doc(&spec);
        let lines: Vec<&str> = doc.lines().collect();
        assert_eq!(
            lines[0],
            "b.click — click(sel): click the element a selector points at"
        );
        assert!(doc.contains("capability: device"), "class line: {doc}");
        assert!(doc.contains("consumes a capture"), "capture line: {doc}");
    }

    #[test]
    fn help_doc_starts_with_the_same_line_api_doc_serves_the_prompt() {
        let spec = m("b.goto", &[net_http(&["*"])], "goto(url): navigate", true);
        let seed = api_doc(std::slice::from_ref(&spec));
        assert_eq!(help_doc(&spec).lines().next(), seed.lines().next());
        assert!(help_doc(&spec).contains("returns images"));
    }

    #[test]
    fn pure_methods_get_no_capture_line() {
        let spec = m(
            "sh.env",
            &[],
            "env(name): read an environment variable",
            false,
        );
        let doc = help_doc(&spec);
        assert!(doc.contains("capability: pure"), "{doc}");
        assert!(!doc.contains("capture:"), "{doc}");
    }
}
