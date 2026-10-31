//! The consistency suite every capability pack runs (sdk-assembly §9.7).
//!
//! `phi-ns-*` packs reuse it with one line:
//!
//! ```ignore
//! phi_ns::provider_contract_tests!(PhiNsBrowser::new(cfg));
//! ```

use phi_ns::{NamespaceProvider, NsObject, check_provider_contract, compose_snapshot};

#[cfg(feature = "builtin-shell")]
mod shell {
    use super::*;
    use phi_ns::generic_summary;

    phi_ns::provider_contract_tests!(phi_ns::builtin::PhiNsShell::builtin());

    #[test]
    fn shell_pack_declarations_derive_cleanly() {
        let ns = phi_ns::builtin::PhiNsShell::builtin();
        let methods = ns.methods();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "sh.run");
        // Derived views come from the same source — no separate maintenance.
        assert!(ns.api_doc().contains("sh.run — "));
        assert_eq!(ns.capabilities().len(), 1);
        assert_eq!(methods[0].capability(), phi_cap::Capability::Process);
    }

    #[test]
    fn summarize_only_answers_for_own_objects() {
        let ns = phi_ns::builtin::PhiNsShell::builtin();
        let foreign = NsObject {
            name: "rows".into(),
            kind: "list".into(),
            repr: "42".into(),
            detail: Some("42".into()),
        };
        assert_eq!(ns.summarize("rows", &foreign), None);
        assert_eq!(generic_summary(&foreign), "42");
    }
}

#[test]
fn contract_check_catches_drift() {
    struct BadRoot;
    #[async_trait::async_trait]
    impl NamespaceProvider for BadRoot {
        fn namespaces(&self) -> &'static [&'static str] {
            &["b"]
        }
        fn methods(&self) -> Vec<phi_ns::MethodSpec> {
            vec![phi_ns::m("x.go", &[], "not rooted at b", false)]
        }
        async fn install(&self, _sink: &mut dyn phi_ns::WorkerSink) -> Result<(), phi_ns::NsError> {
            Ok(())
        }
    }
    let err = check_provider_contract(&BadRoot).unwrap_err();
    assert!(err.contains("namespaces() does not declare"), "{err}");
}

#[test]
fn snapshot_separates_pack_objects_from_model_names() {
    struct OwnsShell;
    #[async_trait::async_trait]
    impl NamespaceProvider for OwnsShell {
        fn namespaces(&self) -> &'static [&'static str] {
            &["sh"]
        }
        fn methods(&self) -> Vec<phi_ns::MethodSpec> {
            vec![phi_ns::m("sh.run", &[], "run a command", false)]
        }
        async fn install(&self, _sink: &mut dyn phi_ns::WorkerSink) -> Result<(), phi_ns::NsError> {
            Ok(())
        }
        fn summarize(&self, _name: &str, obj: &NsObject) -> Option<String> {
            match obj.kind.as_str() {
                "ShellSdk" => Some(String::new()),
                "Output" => Some(obj.detail.clone().unwrap_or_default()),
                _ => None,
            }
        }
    }
    let ns = OwnsShell;
    let packs: Vec<&dyn NamespaceProvider> = vec![&ns];

    let entries = vec![
        NsObject {
            name: "sh".into(),
            kind: "ShellSdk".into(),
            repr: "ShellSdk()".into(),
            detail: Some(String::new()),
        },
        NsObject {
            name: "x".into(),
            kind: "int".into(),
            repr: "41".into(),
            detail: Some("41".into()),
        },
        NsObject {
            name: "out".into(),
            kind: "Output".into(),
            repr: "12 lines".into(),
            detail: Some("12 lines".into()),
        },
    ];

    let line = compose_snapshot(&entries, &packs);
    // Pack-owned: `sh(ShellSdk)` (mine, nothing to add) and `out(Output, …)`.
    // Model-built: generic `x(int, 41)`.
    assert_eq!(
        line,
        "namespace: sh(ShellSdk), x(int, 41), out(Output, 12 lines)"
    );
}
