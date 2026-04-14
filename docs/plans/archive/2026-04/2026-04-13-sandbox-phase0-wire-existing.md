# Phase 0 — Wire existing ProcessSandbox into ActionRuntime

**Parent roadmap:** [2026-04-13-sandbox-roadmap.md](2026-04-13-sandbox-roadmap.md)
**Status:** spec
**Estimated effort:** ~1 week
**Blocks:** Phase 1, 2, 3, 4

## Goal

Stop fail-closing on `IsolationLevel != None`. Dispatch non-`None` isolation levels through the existing `SandboxRunner` trait and make `ProcessSandbox` actually reachable from `ActionRuntime`. No protocol changes, no new OS enforcement — this is purely plumbing so we can iterate on anything else.

Phase 0 deliberately **does not fix** the fundamental problems (one-shot protocol, no credential pipeline, no real enforcement). Those are Phase 1. The purpose here is to get a working end-to-end path and un-ignore the test at `runtime.rs:648-679`.

## Non-goals

- Duplex broker protocol (Phase 1).
- Plugin credential access (Phase 1).
- seccomp / cgroups / namespaces (Phase 2).
- Process pooling (Phase 2).
- Cross-platform enforcement (Phase 3).

## Preconditions

- Current main is green (3246 tests).
- `ProcessSandbox` compiles and its unit tests pass.

## Target state

1. `ActionRuntime::new` keeps accepting `Arc<dyn SandboxRunner>`. No signature change.
2. `execute_stateless` (`runtime.rs:238-252`) — for `IsolationLevel::CapabilityGated` or `IsolationLevel::Isolated`, calls `self.sandbox.execute(SandboxedContext::new(ctx), metadata, input)` instead of returning `Fatal`. For `IsolationLevel::None` the direct-call path is unchanged.
3. `execute_stateful` (`runtime.rs:259-312`) — **stays fail-closed** for non-`None` isolation in Phase 0. Stateful sandboxing is a Phase 1 concern (needs the broker loop to keep state across iterations). Document this explicitly in the crate doc.
4. A new `DispatchingSandbox` wrapper that owns a registry of plugin binaries (`HashMap<ActionKey, Arc<ProcessSandbox>>`) and implements `SandboxRunner::execute` by routing to the right `ProcessSandbox` based on `metadata.key`. This is what actually gets handed to `ActionRuntime`.
5. `InProcessSandbox` keeps its current behaviour — for `IsolationLevel::None` actions, `ActionRuntime` still goes direct. `InProcessSandbox` exists for callers that want uniform plumbing but is not on the hot path.
6. Plugin discovery (`discovery.rs`) gains a minimal manifest: a TOML file next to the plugin binary listing declared capabilities. Manifest format and signing are **out of scope** — Phase 0 accepts `PluginCapabilities::trusted()` via config, not unsigned wild-west discovery.
7. The ignored test `execute_uses_sandbox_for_capability_gated` (`runtime.rs:648-679`) is un-ignored and passes.
8. `ActionRuntime` emits `sandbox_plugin_spawn_total{plugin, outcome}` and `sandbox_plugin_timeout_total{plugin}` metrics.

## Work breakdown

### Step 0.1 — Introduce `DispatchingSandbox`
- **Where:** new file `crates/sandbox/src/dispatching.rs`, re-exported from `lib.rs`.
- **What:** struct holding `HashMap<ActionKey, Arc<ProcessSandbox>>` and `Arc<dyn SandboxRunner>` fallback (defaults to `InProcessSandbox`). Implements `SandboxRunner`: look up by `metadata.key`, route to the matched `ProcessSandbox`; if missing, return `ActionError::fatal("no sandbox runner for action {key}")` — **do not** silently fall through to the fallback, that would be a silent capability bypass.
- **Tests:** unit test routing + unit test "missing key returns Fatal".

### Step 0.2 — Route non-`None` isolation through sandbox in `execute_stateless`
- **Where:** `crates/runtime/src/runtime.rs:238-252`.
- **What:** replace the `ActionError::fatal` branch for `IsolationLevel::CapabilityGated | IsolationLevel::Isolated` with a call to `self.sandbox.execute(...)`. Keep the direct-call branch for `None`.
- **Test:** un-ignore `execute_uses_sandbox_for_capability_gated` at `runtime.rs:648-679` and make it pass with a stub `SandboxRunner` impl.

### Step 0.3 — Keep `execute_stateful` fail-closed (document why)
- **Where:** `crates/runtime/src/runtime.rs:266-270`.
- **What:** leave the `Fatal` branch in place. Update the inline comment to point at `2026-04-13-sandbox-phase1-broker.md` explaining that stateful sandboxing needs the duplex broker. Update `.project/context/crates/runtime.md` trap entry accordingly.

### Step 0.4 — Minimal plugin manifest loader
- **Where:** `crates/sandbox/src/discovery.rs:87` (the `TODO: load from config`).
- **What:** read `<plugin>.toml` next to the binary with `{key, capabilities}`. If missing → refuse to register the plugin (not `trusted()`). Parse `capabilities` as a list of `Capability` enum values via `serde`. Unit-test happy path + missing manifest + malformed manifest.
- **Not:** signing, publishing, registry. That's Phase 4.

### Step 0.5 — Wire `DispatchingSandbox` into engine construction
- **Where:** wherever `ActionRuntime::new` is called in `nebula-engine`. The agent report pointed at `crates/engine/src/engine.rs:1495-1502`; verify and update.
- **What:** when the engine boots, discover plugins via `discovery::discover_directory`, build `DispatchingSandbox`, pass to `ActionRuntime::new`. Configurable directory via existing engine config.
- **Default:** if no plugin directory configured, fall back to `InProcessSandbox` (existing behaviour).

### Step 0.6 — Metrics
- **Where:** `crates/sandbox/src/process.rs` around `call()` at line 63.
- **What:** increment `sandbox_plugin_spawn_total{plugin, outcome=success|failure}` on every `cmd.spawn()` result. Increment `sandbox_plugin_timeout_total{plugin}` in the timeout branch at line 170. Use `nebula-metrics::MetricsRegistry` the same way runtime already does.

### Step 0.7 — Integration test
- **Where:** `crates/sandbox/tests/integration_phase0.rs` (new).
- **What:** build a tiny plugin binary in `examples/sandbox-smoke-plugin/` (root-level `examples/`, per project convention) that uses `nebula-plugin-protocol::run` and echoes input. Run it end-to-end through `DispatchingSandbox` → `ProcessSandbox` → binary → response. Assert success. Assert denied-capability manifest produces a Fatal at registration time.

## Acceptance criteria

- [ ] `cargo nextest run -p nebula-runtime -p nebula-sandbox` green.
- [ ] `runtime.rs:648-679` test un-ignored and passing.
- [ ] New integration test in `crates/sandbox/tests/integration_phase0.rs` green.
- [ ] `cargo clippy --workspace -- -D warnings` clean.
- [ ] `.project/context/crates/runtime.md` and `.project/context/crates/sandbox.md` updated: "Phase 0 landed — CapabilityGated stateless dispatch active; stateful still fail-closed; broker protocol pending (Phase 1)".
- [ ] Metrics appear for spawned plugins on a smoke run.
- [ ] A plugin without a manifest fails to register with a clear error, not with `PluginCapabilities::none()` silently.

## Risks

| Risk | Mitigation |
|------|------------|
| `ProcessSandbox::call` is per-call spawn → slow tests | Acceptable for Phase 0 smoke test; pool in Phase 2 |
| `env_clear()` breaks plugins expecting `PATH` | Document in manifest that `PATH` must be explicitly declared |
| Engine construction becomes infallible → making sandbox wiring fallible leaks errors | Wrap discovery failure as engine-startup error, not runtime error |
| Silently falling back to `InProcessSandbox` for missing plugins → capability bypass | `DispatchingSandbox` returns `Fatal` on missing key, never falls through |

## Context file updates required

- `.project/context/crates/runtime.md`: note that `execute_stateless` now dispatches non-`None` isolation through sandbox; `execute_stateful` still fail-closes; link to roadmap.
- `.project/context/crates/sandbox.md`: note `DispatchingSandbox` as the composition layer; note minimal manifest loader; note enforced plugin manifest requirement.
- `.project/context/active-work.md`: add Phase 0 as in-progress.
- `.project/context/pitfalls.md`: update "InProcessSandbox only" pitfall to reflect Phase 0 landing.
