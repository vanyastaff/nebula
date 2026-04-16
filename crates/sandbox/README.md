# nebula-sandbox

Plugin isolation and sandboxing for the Nebula workflow engine.

**Layer:** Exec
**Canon:** §4.5 (operational honesty), §7.1 (plugin model), §12.6 (**isolation honesty — this is the load-bearing section**)

## Status

**Overall:** `implemented` for the two real execution modes below. **Not a security boundary against malicious native code** — canon §12.6 is the normative statement; this README repeats it because the crate name invites misinterpretation.

**Works today:**

- `InProcessSandbox` — trusted in-process execution for built-in actions. **No isolation** — pure dispatch with capability checks (`SandboxedContext::check_cancelled`). Correctness and cooperative cancellation, not attacker-grade separation.
- `ProcessSandbox` — child-process execution over a JSON envelope protocol (the `nebula-plugin-sdk` duplex broker). **Sequential dispatch** over a line-framed transport. This is the real trust model for community plugins today.
- `ProcessSandboxHandler` — bridges `ProcessSandbox` into `ActionRegistry` so runtime sees a unified `ActionExecutor`.
- `capabilities` module — iOS-style per-plugin capability model (`PluginCapabilities`, network/filesystem/env allowlists).
- `discovery` module — scans directories for plugin binaries via `plugin.toml` markers.
- `os_sandbox` module — OS-level hardening primitives (where supported).
- 3 unit test markers; **0 integration tests**.

**Known gaps / not a security boundary:**

- ⚠️ **`discovery.rs:117` hardcodes `PluginCapabilities::none()`** — TODO: load from config. Today, a discovered plugin has **no capabilities applied**; the capability model is defined but not wired end-to-end from `plugin.toml` through discovery. Until this lands, **the capability allowlist is a `false capability`** per canon §4.5.
- ⚠️ **Plugin IPC today is sequential JSON envelope dispatch** — canon §12.6 is explicit: *"that is the trust model; do not describe it as sandboxed execution of untrusted native code."* Parallelism within a `ProcessSandbox` is a throughput win that is **planned**, not shipping.
- ⚠️ **`os_sandbox` module is present but per-platform backends are partial** — seccomp-bpf (Linux), `sandbox_init` (macOS), `AppContainer` / job objects (Windows), landlock (modern Linux) are the intended backends. Check `src/os_sandbox.rs` for current coverage before claiming any platform-specific hardening.
- ⚠️ **No integration tests.** The cancel path through `SandboxedContext`, the `ProcessSandbox` wire protocol, and the `capabilities` enforcement path are all covered only by unit tests today.
- **3 panic sites** — review each for whether it should be a typed `SandboxError`.

**WASM / WASI is an explicit non-goal.** Canon §12.6 is the normative statement. Rationale: the Rust plugin ecosystem integration authors actually reach for (`redis`, `sqlx` with native drivers, `rdkafka`, `tonic` with native TLS, `*-sys` crates) does not compile to `wasm32-wasip2`, and where parts compile, the feature surface forces authors into host-polyfill folklore that breaks the §3.5 DX promise. Offering WASM as "the future sandbox" would be a §4.5 false capability and a §4.4 DX regression at the same time — so it is **not** on the roadmap and must not appear as "planned" in any crate-level `lib.rs` or README.

## Roadmap (real isolation path)

In priority order — these are the actual work items the crate tracks, replacing any historical "WASM someday" language:

1. **Capability wiring** — close `discovery.rs:117` so `PluginCapabilities` is loaded from `plugin.toml` and enforced at `ProcessSandbox` boundaries. Until this lands, the capability model is advertised but unenforced (canon §4.5 false capability).
2. **`plugin.toml` signing verification** — canon §7.1 describes the signed manifest payload; tooling (`cargo-nebula` or equivalent) needs to verify signatures before the sandbox host trusts a plugin's `plugin.toml`. Host-side verification lives here; publishing-side signing is tooling.
3. **`os_sandbox` per-platform backends** — seccomp-bpf + landlock on Linux, `sandbox_init` on macOS, `AppContainer` / job objects on Windows. Each backend is independent; ship per-platform as they stabilise.
4. **`ProcessSandbox` parallelism** — sequential dispatch is the current §12.6 reality. Bounded parallel dispatch per plugin (with a fair scheduler across plugins) is the §4.1 throughput win that actually matters for real workloads.
5. **Integration tests for the cancel path and protocol envelope** — canon §13 knife scenario step 5 must be green end-to-end against `ProcessSandbox`, not only against `InProcessSandbox`.

## Architecture notes

**Smells tracked as open debt:**

- **`runtime::sandbox.rs` is a dead re-export shim of this crate.** `nebula-runtime/src/sandbox.rs` exists only for "backward compatibility" per its own doc comment. It should be deleted and call sites moved to `nebula_sandbox::*` directly. See `nebula-runtime/README.md` Architecture notes.
- **Naming overlap with `InProcessSandbox` in other crates.** The re-exports `ActionExecutor` / `SandboxRunner` / `InProcessSandbox` appear in both `nebula-sandbox` (this crate, the owner) and `nebula-runtime` (re-exporter). After the runtime shim is removed, this crate is the single source.
- **`capabilities.rs` and `discovery.rs` have a TODO that makes the module's primary promise unenforced.** Consider whether the capability allowlist should be surfaced as `experimental` in `PluginCapabilities`' doc comments until the discovery-path TODO is closed.

**Not smells — intentional:**

- Dependency on `nebula-plugin-sdk` (wire protocol types) is correct: this crate is the **host** of the duplex broker, the SDK is the **plugin** side. The protocol lives in the SDK because plugin authors need to link against it; the sandbox imports it back to (de)serialize messages.

## Scope

- `InProcessSandbox` — trusted execution for first-party built-in actions.
- `ProcessSandbox` — child-process execution for community plugins over a JSON envelope wire protocol.
- `capabilities` — per-plugin capability declarations consumed by the host.
- `discovery` — file-system scan for plugin binaries.
- `os_sandbox` — OS-level primitives (best-effort).

## What this crate provides

| Type / module | Role |
| --- | --- |
| `InProcessSandbox` | Trusted in-process execution. No isolation. |
| `ProcessSandbox` | Child-process execution over JSON envelope. |
| `ProcessSandboxHandler` | Bridge to `ActionRegistry`. |
| `SandboxRunner`, `ActionExecutor`, `ActionExecutorFuture`, `SandboxedContext` | Core sandbox runner abstraction. |
| `capabilities::PluginCapabilities` | iOS-style capability declarations. |
| `discovery` | Scan directories for plugin binaries via `plugin.toml`. |
| `os_sandbox` | OS-level hardening primitives. |
| `SandboxError` | Typed error. |

## Where the contract lives

- Source: `src/lib.rs`, `src/in_process.rs`, `src/process.rs`, `src/capabilities.rs`, `src/discovery.rs`, `src/runner.rs`
- Canon: `docs/PRODUCT_CANON.md` §4.5, §7.1, §12.6
- Glossary: `docs/GLOSSARY.md` §4 (resource/sandbox)

## See also

- `nebula-plugin` — plugin trait + registry (not loading)
- `nebula-plugin-sdk` — plugin-side wire protocol (this crate's counterpart)
- `nebula-runtime` — dispatches actions through sandbox runners
