The capability-pack contract for the phi family: one trait, one declaration source, one audit chokepoint.

* `NamespaceProvider` — what a capability pack *is*.
* `MethodSpec` — the single declaration source; `api_doc` / `capabilities` /
  policy keys all derive from it.
* `CaptureLedger` — the one-use capture ledger: one look authorizes one
  action attempt.
* `WorkerSink` — the seam that keeps this crate free of `codexec`.
* `ImageRef` — the multimodal return channel.
* `remote::Gate` — policy + journal welded onto remote calls; audit is
  un-bypassable by type structure.
* `builtin::PhiNsShell` — the built-in `sh.*` pack and the reference for pack
  authors.

Dependency direction: `codexec -> phi-ns -> phi-cap`, and the pack crates
(`phi-ns-browser`, `phi-ns-desktop`, `phi-ns-mobile`) never depend on
`codexec` — assembly happens above them.

```toml
[dependencies]
phi-ns = "0.1.0"
```

The `remote` feature (on by default) adds the process transport for IPC
capability packs; it builds on the `phi-wire` crates, the phi family's
JSON-lines provider wire.

## License

MIT.
