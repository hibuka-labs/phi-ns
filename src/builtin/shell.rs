//! `sh.*` — the built-in shell pack (sdk-assembly §5.1 / §3.5).
//!
//! Reference implementation of a pack: declaration here, host-side effect
//! behind policy + journal, a thin Python shim in the worker. Injection is
//! the in-process-shim shape (allowed for `sh.*`: the side effect goes
//! through the OS and the process tree is killable), but every `sh.run` still
//! crosses the host seam via `_phi_call` — the engine's single hook point.

use crate::error::NsError;
use crate::method::{MethodSpec, m};
use crate::provider::NamespaceProvider;
use crate::sink::{HostCall, NsObject, WorkerSink};
use async_trait::async_trait;
use phi_cap::process_spawn;

/// The built-in `sh.*` pack.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhiNsShell;

impl PhiNsShell {
    /// `sh.*` with its stock methods (`sh.run`).
    pub fn builtin() -> Self {
        Self
    }
}

/// Worker-side shim installed by [`NamespaceProvider::install`].
///
/// `_phi_call(name, args)` is provided by the worker runtime (the
/// [`WorkerSink`] eval contract): one synchronous host call that raises into
/// the cell when policy holds the call.
const PY_SHIM: &str = r#"
class Output:
    """Result of sh.run: captured stdout/stderr plus the exit code."""
    def __init__(self, d):
        self.stdout = d.get("stdout", "") or ""
        self.stderr = d.get("stderr", "") or ""
        self.code = d.get("code", -1)

    @property
    def lines(self):
        return self.stdout.splitlines()

    def __str__(self):
        if self.stderr:
            return self.stdout + self.stderr
        return self.stdout

    def __repr__(self):
        return "%d lines" % len(self.lines)

    def _phi_summary_(self):
        return "%d lines" % len(self.lines)


class ShellSdk:
    def run(self, cmd):
        """Run a shell command; returns Output(stdout, stderr, code)."""
        return Output(_phi_call("sh.run", [cmd]))


sh = ShellSdk()
"#;

/// Host-side implementation of `sh.run`.
struct ShRun;

#[async_trait]
impl HostCall for ShRun {
    async fn call(&self, args: Vec<serde_json::Value>) -> Result<serde_json::Value, NsError> {
        let cmd = args
            .first()
            .and_then(|v| v.as_str())
            .ok_or_else(|| NsError::Sink("sh.run(cmd) expects one string argument".into()))?;
        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
            .map_err(|e| NsError::Sink(format!("sh.run could not spawn: {e}")))?;
        Ok(serde_json::json!({
            "stdout": String::from_utf8_lossy(&out.stdout),
            "stderr": String::from_utf8_lossy(&out.stderr),
            "code": out.status.code().unwrap_or(-1),
        }))
    }
}

#[async_trait]
impl NamespaceProvider for PhiNsShell {
    fn namespaces(&self) -> &'static [&'static str] {
        &["sh"]
    }

    fn methods(&self) -> Vec<MethodSpec> {
        vec![
            // `process:spawn{}` is the honest scope: a shell runs whatever it
            // is given. Policy decides per call (often `Ask`).
            m(
                "sh.run",
                &[process_spawn()],
                "run a shell command; returns Output(stdout, stderr, code)",
                false,
            ),
        ]
    }

    async fn install(&self, sink: &mut dyn WorkerSink) -> Result<(), NsError> {
        sink.register_fn("sh.run", Box::new(ShRun))?;
        sink.eval(PY_SHIM).await
    }

    fn summarize(&self, _name: &str, obj: &NsObject) -> Option<String> {
        match obj.kind.as_str() {
            // Own objects only. `Some("")` means "mine, nothing to add" and
            // renders as `sh(ShellSdk)`; `None` would fall to the generic path.
            "ShellSdk" => Some(String::new()),
            "Output" => Some(obj.detail.clone().unwrap_or_else(|| obj.repr.clone())),
            _ => None,
        }
    }
}
