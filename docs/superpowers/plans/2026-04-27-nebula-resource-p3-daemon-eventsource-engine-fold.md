# nebula-resource П3 — Daemon / EventSource Engine Fold Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `Daemon` and `EventSource` topology infrastructure from `nebula-resource` into `nebula-engine` per ADR-0037 + Tech Spec §12. Land `DaemonRegistry` primitive (Tech Spec §12.2) and `EventSourceAdapter<E>: TriggerAction` (Tech Spec §12.3 — corrected for the real `TriggerAction` shape). Shrink `TopologyRuntime<R>` 7 → 5 variants. Restore canon §3.5 ("Resource = pool/SDK client") truth in `nebula-resource`. Closes 🔴 R-010, 🔴 R-011, 🟠 R-012.

**Architecture:** Mechanical move of 4 source files (~686 LOC) from `crates/resource/src/{topology,runtime}/{daemon,event_source}.rs` into a new `crates/engine/src/daemon/` module. Two new engine primitives: `DaemonRegistry` (typed-handle `DashMap` keyed by `ResourceKey`, parallel `start_all`/`stop_all`, parent `CancellationToken` for shutdown propagation) and `EventSourceAdapter<E>` (closure-based `Fn(&E::Event) -> serde_json::Value` payload converter; `start()` runs subscribe+recv loop with biased `tokio::select!` on `ctx.cancellation()`, emits via `ctx.emitter()`). Zero `nebula-resource` references to `Daemon` / `EventSource` post-merge.

**Tech Stack:** Rust 1.95, tokio, tokio-util `CancellationToken`, dashmap, futures `join_all`, tracing. New engine module mirrors the credential precedent at `crates/engine/src/credential/` (registry / dispatchers structure).

**Source documents:**

- [docs/adr/0037-daemon-eventsource-engine-fold.md](../../adr/0037-daemon-eventsource-engine-fold.md) — accepted ADR; engine-fold over sibling crate
- [docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md](../specs/2026-04-24-nebula-resource-tech-spec.md) §12 — landing site (engine module path, `DaemonRegistry` shape sketch, EventSource adapter sketch, per-consumer migration matrix, `TopologyRuntime` shrink mechanics)
- [docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md](../specs/2026-04-24-nebula-resource-redesign-strategy.md) §4.4 (rationale), §4.8 (atomic 5-consumer wave), §5.1 (revisit triggers)
- [docs/tracking/nebula-resource-concerns-register.md](../../tracking/nebula-resource-concerns-register.md) — closes R-010, R-011, R-012
- [crates/engine/src/credential/](../../../crates/engine/src/credential/) — module-shape precedent (1263 LOC, 6 files; ADR-0030 + ADR-0033)
- [crates/action/src/trigger.rs](../../../crates/action/src/trigger.rs) — actual `TriggerAction`/`TriggerHandler`/`Action` shapes (Tech Spec §12.3 sketch was hypothetical)
- [crates/action/src/poll.rs:1051-1456](../../../crates/action/src/poll.rs) — `PollTriggerAdapter` precedent for "run-until-cancelled `start()` with `ctx.cancellation()` + `ctx.emitter().emit()`" pattern

**Closes (concerns register):**

- 🔴 R-010 — Daemon topology has no public start path (`pub(crate)` barrier dissolved by extraction; engine surface is fully public)
- 🔴 R-011 — EventSource same orphan-surface pattern (resolved by fold + adapter)
- 🟠 R-012 — Daemon + EventSource out-of-canon §3.5 ("resource = pool/SDK client")

**Non-goals (explicitly deferred):**

- `nebula-scheduler` sibling crate spinout — Strategy §5.1 trigger ("daemon engine code grows >500 LOC OR ≥2 non-trigger long-running workers materialize"). П3 lands daemon code ~700 LOC in engine; if Strategy §5.1 trigger fires post-merge, opens new cascade. Document trigger evaluation in commit message.
- Doc rewrite (`Architecture.md`, `events.md`, remaining `api-reference.md` sections) → П4
- Per-daemon health-check protocol — Daemon trait stays at `run/restart` shape; orthogonal future work
- DaemonRegistry persistence / crash recovery — engine-internal, in-memory only at P3 boundary
- EventSource → workflow input schema validation — adapter emits `serde_json::Value` opaquely; downstream workflow validates per its own schema

**Cross-consumer impact (Tech Spec §12.4 verified at audit time):**

```
$ rg "nebula_resource::(Daemon|RestartPolicy|DaemonConfig|DaemonRuntime|EventSource|EventSourceConfig|EventSourceRuntime)" crates/{action,sdk,plugin,sandbox}/
(no matches)

$ rg "use nebula_resource::(Daemon|EventSource)" crates/
(no matches)
```

Migration impact for `nebula-action`, `nebula-sdk`, `nebula-plugin`, `nebula-sandbox`: **no-op**. Only `nebula-engine` (the new home) and `nebula-resource` (the source) change.

---

## File Structure

### New files (engine)

| File | Purpose | Approx LOC |
|---|---|---|
| `crates/engine/src/daemon/mod.rs` | Module root: `Daemon` trait, `RestartPolicy`, `Config` (alias `DaemonConfig`), submodule decls, re-exports | ~90 |
| `crates/engine/src/daemon/runtime.rs` | `DaemonRuntime<D>` migrated from `crates/resource/src/runtime/daemon.rs` (493 LOC + 3 unit tests) | ~500 |
| `crates/engine/src/daemon/registry.rs` | NEW — `DaemonRegistry`, `AnyDaemonHandle` trait object, `TypedDaemonHandle<D>`, `DaemonError`, registry tests | ~220 |
| `crates/engine/src/daemon/event_source.rs` | `EventSource` trait + `EventSourceConfig` + `EventSourceRuntime<E>` migrated from `crates/resource/src/{topology,runtime}/event_source.rs` (125 LOC), plus NEW `EventSourceAdapter<E>: TriggerAction` (~110 LOC + tests) | ~280 |

### Modified files (engine)

| File | Change |
|---|---|
| `crates/engine/src/lib.rs` | Add `pub mod daemon;` + re-export block (`Daemon`, `DaemonConfig`, `RestartPolicy`, `DaemonRuntime`, `DaemonRegistry`, `DaemonError`, `EventSource`, `EventSourceConfig`, `EventSourceRuntime`, `EventSourceAdapter`) |
| `crates/engine/Cargo.toml` | Add `serde_json = { workspace = true }` if not transitively present (verify in Task 5); ensure `futures = { workspace = true }` available for `join_all` in `DaemonRegistry::start_all` (likely already via nebula-action) |

### Deleted files (resource)

| File | Reason |
|---|---|
| `crates/resource/src/topology/daemon.rs` | Daemon trait + RestartPolicy + Config moved to engine |
| `crates/resource/src/topology/event_source.rs` | EventSource trait + Config moved to engine |
| `crates/resource/src/runtime/daemon.rs` | DaemonRuntime + tests moved to engine |
| `crates/resource/src/runtime/event_source.rs` | EventSourceRuntime moved to engine |

### Modified files (resource)

| File | Change |
|---|---|
| `crates/resource/src/topology/mod.rs` | Drop `pub mod daemon`, `pub mod event_source`, the two `pub use` lines for `Daemon`/`RestartPolicy`/`EventSource`; update module-level table comment (7 → 5 topologies) |
| `crates/resource/src/runtime/mod.rs` | Drop `pub mod daemon`, `pub mod event_source`, two `Daemon(...)`/`EventSource(...)` enum variants in `TopologyRuntime`, two arms in `tag()`, two intra-doc link refs in module docs |
| `crates/resource/src/topology_tag.rs` | Drop `Daemon` and `EventSource` enum variants, two `as_str()` arms, two doc lines |
| `crates/resource/src/lib.rs` | Drop 5 `pub use` lines (`runtime::daemon::DaemonRuntime`, `runtime::event_source::EventSourceRuntime`, `topology::daemon::config::Config as DaemonConfig`, `topology::daemon::{Daemon, RestartPolicy}`, `topology::event_source::{EventSource, config::Config as EventSourceConfig}`); drop `Cell` from key types table comment if needed (verify); update §"Key types" or topology summary if it lists 7 |
| `crates/resource/src/manager/mod.rs:1373` | Drop the `TopologyRuntime::Daemon(_) => ReloadOutcome::Restarting,` match arm (single line) |
| `crates/resource/README.md` | Drop daemon/event_source rows from topology summary table if present (search before edit) |

### Documentation

| File | Change |
|---|---|
| `docs/tracking/nebula-resource-concerns-register.md` | Mark R-010, R-011, R-012 status `landed П3` with commit SHA + `crates/engine/src/daemon/` link |
| `docs/MATURITY.md` | No change in P3 (maturity bump to `core` is post-cascade per Strategy §6.4) |

### Verification commands

```bash
cargo check -p nebula-resource
cargo check -p nebula-engine
cargo check --workspace
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
cargo nextest run -p nebula-engine --profile ci --no-tests=pass
cargo nextest run --workspace --profile ci --no-tests=pass
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

---

## Task 1 — Engine daemon module skeleton + Daemon trait + config

**Files:**
- Create: `crates/engine/src/daemon/mod.rs`

**Why:** Foundation. Tasks 2-5 depend on `Daemon`/`RestartPolicy`/`DaemonConfig` being in `nebula_engine::daemon`. Mirror the `crates/resource/src/topology/daemon.rs` shape verbatim — type names preserved per Tech Spec §12.2 ("Type-name preservation: `Daemon` trait stays `Daemon`; `RestartPolicy` stays `RestartPolicy`; `DaemonConfig` stays `DaemonConfig`"). The trait still extends `Resource`; engine deps on `nebula-resource` per `Cargo.toml` so the supertrait is reachable.

- [ ] **Step 1: Create the directory and `mod.rs`**

```bash
mkdir -p crates/engine/src/daemon
```

Write `crates/engine/src/daemon/mod.rs`:

```rust
//! Engine daemon module — long-running worker primitives (per ADR-0037).
//!
//! Hosts the `Daemon` trait + `DaemonRuntime` (per-daemon background task with
//! restart policy) + `DaemonRegistry` (engine-side dispatcher across all
//! registered daemons). EventSource adapter onto the `TriggerAction` substrate
//! lives in [`event_source`].
//!
//! Migrated from `nebula-resource` per ADR-0037 ("Daemon / EventSource engine
//! fold") to honor canon §3.5 ("Resource = pool/SDK client").
//!
//! # Cancellation
//!
//! `DaemonRegistry` owns a parent [`tokio_util::sync::CancellationToken`]. Each
//! `DaemonRuntime` registered through it inherits the parent token; calling
//! [`DaemonRegistry::shutdown`] cascades to every daemon loop. Per-run lifecycle
//! is managed by `DaemonRuntime` (see its module docs).
//!
//! # Module layout
//!
//! - [`mod@self`] — `Daemon` trait, `RestartPolicy`, `DaemonConfig`
//! - [`runtime`] — `DaemonRuntime<D>` per-daemon background task
//! - [`registry`] — `DaemonRegistry` engine-side dispatcher
//! - [`event_source`] — `EventSource` trait + `EventSourceAdapter<E>` (TriggerAction adapter)

pub mod event_source;
pub mod registry;
pub mod runtime;

use std::{future::Future, time::Duration};

use nebula_resource::{Resource, ResourceContext};
use tokio_util::sync::CancellationToken;

/// Policy for restarting a daemon after it exits.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Never restart — if the daemon exits, it stays down.
    Never,
    /// Restart only if the daemon exited with an error.
    #[default]
    OnFailure,
    /// Always restart, regardless of exit reason.
    Always,
}

/// Daemon — long-running background worker.
///
/// A long-running worker that runs until cancelled or until it returns.
/// Implementations select on `cancel` for cooperative shutdown; `DaemonRuntime`
/// drives the restart loop per the configured [`RestartPolicy`].
///
/// # Errors
///
/// Returns `Self::Error` if the daemon encounters a fatal error.
pub trait Daemon: Resource {
    /// Runs the daemon loop.
    ///
    /// The implementation should select on `cancel` for cooperative shutdown.
    /// When the token is cancelled, the daemon should clean up and return `Ok(())`.
    fn run(
        &self,
        runtime: &Self::Runtime,
        ctx: &ResourceContext,
        cancel: CancellationToken,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Configuration types for the daemon module.
pub mod config {
    use super::{Duration, RestartPolicy};

    /// Daemon configuration.
    #[derive(Debug, Clone)]
    pub struct Config {
        /// When to restart the daemon after it exits.
        pub restart_policy: RestartPolicy,
        /// Maximum number of restarts before giving up.
        pub max_restarts: u32,
        /// Backoff duration between restarts.
        pub restart_backoff: Duration,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                restart_policy: RestartPolicy::default(),
                max_restarts: 5,
                restart_backoff: Duration::from_secs(1),
            }
        }
    }
}

pub use config::Config as DaemonConfig;
pub use event_source::{
    EventSource, EventSourceAdapter, EventSourceConfig, EventSourceRuntime,
};
pub use registry::{AnyDaemonHandle, DaemonError, DaemonRegistry};
pub use runtime::DaemonRuntime;
```

- [ ] **Step 2: Stub the submodules so `mod.rs` compiles in isolation**

Create the three submodule files as empty stubs so Step 1's `pub mod` declarations don't error out. Tasks 2-5 fill them in.

`crates/engine/src/daemon/runtime.rs`:

```rust
//! Daemon runtime — background run loop with restart policy. (See [`DaemonRuntime`].)
//!
//! Filled in by Task 2.

use crate::daemon::{Daemon, DaemonConfig};

#[allow(dead_code, reason = "filled by Task 2")]
pub struct DaemonRuntime<D: Daemon> {
    _placeholder: std::marker::PhantomData<D>,
    _config: DaemonConfig,
}
```

`crates/engine/src/daemon/registry.rs`:

```rust
//! `DaemonRegistry` — engine-side dispatcher across registered daemons.
//!
//! Filled in by Task 4.

use std::sync::Arc;

#[allow(dead_code, reason = "filled by Task 4")]
pub trait AnyDaemonHandle: Send + Sync {}

#[allow(dead_code, reason = "filled by Task 4")]
pub struct DaemonRegistry {
    _placeholder: Vec<Arc<dyn AnyDaemonHandle>>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DaemonError {
    #[error("daemon registry placeholder")]
    Placeholder,
}
```

`crates/engine/src/daemon/event_source.rs`:

```rust
//! EventSource topology + `EventSourceAdapter<E>: TriggerAction`.
//!
//! Filled in by Task 5.

use nebula_resource::Resource;

#[allow(dead_code, reason = "filled by Task 5")]
pub trait EventSource: Resource {}

#[allow(dead_code, reason = "filled by Task 5")]
pub struct EventSourceConfig {
    pub buffer_size: usize,
}

#[allow(dead_code, reason = "filled by Task 5")]
pub struct EventSourceRuntime<E: EventSource> {
    _placeholder: std::marker::PhantomData<E>,
}

#[allow(dead_code, reason = "filled by Task 5")]
pub struct EventSourceAdapter<E: EventSource> {
    _placeholder: std::marker::PhantomData<E>,
}
```

- [ ] **Step 3: Verify the skeleton compiles in engine alone (Cargo.toml deps already present)**

Run: `cargo check -p nebula-engine`

Expected: PASS. Compiler may warn about unused `Resource` import in stubs — acceptable for placeholders; Tasks 2-5 use them.

If `nebula-resource::Resource` import errors out: `nebula-engine` already deps on `nebula-resource` per `crates/engine/Cargo.toml`. Confirm via grep before debugging further.

- [ ] **Step 4: Add module declaration to `crates/engine/src/lib.rs`**

Add after the existing `pub mod credential;` line (alphabetical order):

```rust
pub mod credential;
pub mod credential_accessor;
pub mod daemon;  // NEW
pub mod engine;
```

Re-exports follow in Task 8 — for now, just the `pub mod`.

- [ ] **Step 5: Verify lib.rs change**

Run: `cargo check -p nebula-engine`

Expected: PASS.

- [ ] **Step 6: Commit Task 1**

```bash
git add crates/engine/src/daemon/ crates/engine/src/lib.rs
git commit -m "feat(engine): scaffold daemon module per ADR-0037

Add crates/engine/src/daemon/ skeleton with Daemon trait, RestartPolicy,
DaemonConfig in mod.rs and stubs for runtime.rs / registry.rs /
event_source.rs. Tasks 2-5 fill in the implementations.

Refs: ADR-0037, Tech Spec §12.1-§12.2, R-010, R-011, R-012"
```

---

## Task 2 — Migrate `DaemonRuntime<R>` from resource to engine

**Files:**
- Modify: `crates/engine/src/daemon/runtime.rs` (replace stub with migrated content + tests)
- Read source: `crates/resource/src/runtime/daemon.rs` (493 LOC reference)

**Why:** Verbatim move per Tech Spec §12.2 ("[crates/resource/src/runtime/daemon.rs] (493 LOC) ... move to crates/engine/src/daemon/"). Type-name preservation: `DaemonRuntime` stays `DaemonRuntime`. Cancel-token model + restart loop logic unchanged. Only adapt imports.

The 3 existing unit tests (#318/#323 cancellation regression coverage) move alongside in `#[cfg(test)] mod tests`.

- [ ] **Step 1: Replace `crates/engine/src/daemon/runtime.rs` with the migrated module**

Source the body verbatim from `crates/resource/src/runtime/daemon.rs` (493 LOC). Two import changes are needed; everything else is identical.

**Old imports (from resource crate):**

```rust
use crate::{
    context::ResourceContext,
    error::Error,
    resource::Resource,
    topology::daemon::{Daemon, RestartPolicy, config::Config},
};
```

**New imports (from engine crate):**

```rust
use nebula_resource::{Error, Resource, ResourceContext};

use crate::daemon::{Daemon, DaemonConfig as Config, RestartPolicy};
```

**Test mod imports (lines 264-277 in source):**

```rust
// Old:
use crate::{
    context::ResourceContext,
    error::Error as ResourceError,
    resource::{Resource, ResourceConfig, ResourceMetadata},
    topology::daemon::{Daemon, RestartPolicy, config::Config as DaemonCfg},
};

// New:
use nebula_resource::{
    context::ResourceContext,
    error::Error as ResourceError,
    resource::{Resource, ResourceConfig, ResourceMetadata},
};

use crate::daemon::{Daemon, DaemonConfig as DaemonCfg, RestartPolicy};
```

Everything else (struct fields, methods, `daemon_loop` function, all 3 test fns including their `FlakyDaemon`/`OneShotDaemon` impls) is verbatim from the source file. Module-level `//!` doc comment also copies verbatim (the cancellation-model docs are still accurate).

- [ ] **Step 2: Verify resource exports remain reachable**

The migration relies on `nebula_resource` re-exporting `Resource`, `ResourceContext`, `Error`, `ResourceConfig`, `ResourceMetadata`. Confirm via:

```bash
grep -E "^pub use (resource::|context::|error::)" crates/resource/src/lib.rs
```

Expected output includes:
```
pub use context::ResourceContext;
pub use error::{Error, ErrorKind, ErrorScope, RefreshOutcome, RevokeOutcome, RotationOutcome};
pub use resource::{AnyResource, MetadataCompatibilityError, Resource, ResourceConfig, ResourceMetadata,};
```

If any are missing, add the missing `pub use` to `crates/resource/src/lib.rs` before continuing — they need to be reachable for the engine-side test mod to compile. (Empirically all 5 are already re-exported — a no-op verification.)

- [ ] **Step 3: Compile and run the migrated tests**

```bash
cargo check -p nebula-engine
cargo nextest run -p nebula-engine --profile ci --no-tests=pass daemon::runtime
```

Expected:
- `cargo check`: PASS (engine compiles).
- `cargo nextest`: 3 tests pass — `stop_during_restart_backoff_returns_promptly`, `start_stop_start_lifecycle`, `start_natural_exit_start_lifecycle`.

Note: `cargo check -p nebula-resource` will FAIL at this point because `runtime/daemon.rs` is still importing the now-duplicate `topology::daemon` types — Task 6/7 fix that. This is expected mid-stream.

- [ ] **Step 4: Stage but DO NOT commit yet — Task 4 amends this commit with the registry**

Just `git add crates/engine/src/daemon/runtime.rs`. Tasks 2 + 3 (= Task 2 + the optional inline test) + 4 share the same commit per the commit-grouping below.

---

## Task 3 — Build `DaemonRegistry` primitive

**Files:**
- Modify: `crates/engine/src/daemon/registry.rs` (replace stub with full implementation)

**Why:** Tech Spec §12.2 specifies `DaemonRegistry` as the engine-side equivalent of `nebula-resource::Manager`, scoped to long-running worker lifecycles. Engine bootstrap (or applications) construct it; action/resource code does not touch it directly.

Sketch from Tech Spec §12.2:

```rust
pub struct DaemonRegistry {
    daemons: dashmap::DashMap<DaemonKey, Arc<dyn AnyDaemonHandle>>,
    cancel: CancellationToken,
    event_tx: broadcast::Sender<DaemonEvent>,  // optional — defer to follow-up if no consumer yet
}

impl DaemonRegistry {
    pub fn new() -> Self;
    pub fn register<D: Daemon>(&self, daemon: D, config: D::Config, restart_policy: RestartPolicy) -> Result<(), DaemonError>;
    pub async fn start_all(&self) -> Result<(), DaemonError>;
    pub async fn stop_all(&self) -> Result<(), DaemonError>;
}
```

Implementation note (deviates from sketch where the sketch is loose): `D::Config` is `R::Config` from `Resource`, not `DaemonConfig`; the Tech Spec sketch conflated the two. The actual `register` signature takes `daemon: D`, `runtime: Arc<D::Runtime>`, `config: DaemonConfig`, `ctx: ResourceContext` — matching the existing `DaemonRuntime::start()` signature so registration is the natural composition. `event_tx` channel is **deferred** to a follow-up — no consumer wired in P3 — and called out in commit message + Strategy §5.1 trigger evaluation.

`DaemonKey` = `nebula_core::ResourceKey` (each Daemon has `Resource::key()`). Reuses existing primitives; no new key type.

- [ ] **Step 1: Replace `crates/engine/src/daemon/registry.rs` with the full implementation**

```rust
//! `DaemonRegistry` — engine-side dispatcher across registered daemons.
//!
//! Engine bootstrap or application code constructs a `DaemonRegistry`,
//! registers `Daemon` impls via [`DaemonRegistry::register`], and drives the
//! lifecycle via [`DaemonRegistry::start_all`] / [`DaemonRegistry::stop_all`] /
//! [`DaemonRegistry::shutdown`]. Action and resource code does not touch the
//! registry directly.
//!
//! # Cancellation propagation
//!
//! The registry owns a parent [`CancellationToken`]. Each registered
//! `DaemonRuntime` derives its own per-run child token from the parent (see
//! [`crate::daemon::DaemonRuntime`] cancellation model). [`Self::shutdown`]
//! cancels the parent, which cascades to every running daemon loop; live
//! `DaemonRuntime::stop` calls observe the cancellation via biased `select!`
//! and return promptly.
//!
//! # Fail-closed registration
//!
//! Mirrors the `crate::credential::StateProjectionRegistry` policy from
//! ADR-0030 / Tech Spec §15.6 N7 mitigation: duplicate
//! `D::key()` registration returns
//! [`DaemonError::DuplicateKey`] rather than overwriting. Operators resolve
//! the collision by renaming the daemon's `Resource::key`.

use std::{future::Future, pin::Pin, sync::Arc};

use dashmap::DashMap;
use futures::future::join_all;
use nebula_core::ResourceKey;
use nebula_resource::ResourceContext;
use tokio_util::sync::CancellationToken;

use crate::daemon::{Daemon, DaemonConfig, DaemonRuntime};

/// Object-safe handle that erases `D: Daemon` so different daemon types can
/// share a single `DashMap<ResourceKey, Arc<dyn AnyDaemonHandle>>`.
pub trait AnyDaemonHandle: Send + Sync {
    /// Start this daemon's background loop. See [`DaemonRuntime::start`].
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + '_>>;
    /// Stop this daemon's background loop. See [`DaemonRuntime::stop`].
    fn stop(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
    /// Whether this daemon's loop is currently running.
    fn is_running(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
    /// The daemon's identifying key.
    fn key(&self) -> &ResourceKey;
}

/// Type-preserving handle wrapping a single registered daemon.
struct TypedDaemonHandle<D>
where
    D: Daemon + Clone + Send + Sync + 'static,
    D::Runtime: Send + Sync + 'static,
{
    daemon: D,
    runtime: Arc<D::Runtime>,
    runtime_state: Arc<DaemonRuntime<D>>,
    ctx: ResourceContext,
    key: ResourceKey,
}

impl<D> AnyDaemonHandle for TypedDaemonHandle<D>
where
    D: Daemon + Clone + Send + Sync + 'static,
    D::Runtime: Send + Sync + 'static,
{
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + '_>> {
        Box::pin(async move {
            self.runtime_state
                .start(self.daemon.clone(), Arc::clone(&self.runtime), &self.ctx)
                .await
                .map_err(|e| DaemonError::StartFailed {
                    key: self.key.clone(),
                    reason: e.to_string(),
                })
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move { self.runtime_state.stop().await })
    }

    fn is_running(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { self.runtime_state.is_running().await })
    }

    fn key(&self) -> &ResourceKey {
        &self.key
    }
}

/// Engine-side registry of `Daemon` impls.
///
/// See module docs for the cancellation, fail-closed, and lifecycle model.
pub struct DaemonRegistry {
    daemons: DashMap<ResourceKey, Arc<dyn AnyDaemonHandle>>,
    parent_cancel: CancellationToken,
}

impl DaemonRegistry {
    /// Build an empty registry with a fresh parent [`CancellationToken`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            daemons: DashMap::new(),
            parent_cancel: CancellationToken::new(),
        }
    }

    /// Build a registry whose parent token is the supplied one — useful when
    /// the engine wants daemon shutdown cascaded through a higher-level
    /// `CancellationToken` (e.g. process-wide shutdown).
    #[must_use]
    pub fn with_parent_cancel(parent_cancel: CancellationToken) -> Self {
        Self {
            daemons: DashMap::new(),
            parent_cancel,
        }
    }

    /// Returns the parent cancellation token.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.parent_cancel
    }

    /// Number of registered daemons.
    #[must_use]
    pub fn len(&self) -> usize {
        self.daemons.len()
    }

    /// Whether the registry has any registered daemons.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.daemons.is_empty()
    }

    /// Register a `Daemon` impl. Returns `DaemonError::DuplicateKey` if a
    /// daemon with the same `D::key()` is already registered.
    ///
    /// # Errors
    ///
    /// Returns `DaemonError::DuplicateKey` on collision (fail-closed per
    /// ADR-0030 / Tech Spec §15.6 N7).
    pub fn register<D>(
        &self,
        daemon: D,
        runtime: Arc<D::Runtime>,
        config: DaemonConfig,
        ctx: ResourceContext,
    ) -> Result<(), DaemonError>
    where
        D: Daemon + Clone + Send + Sync + 'static,
        D::Runtime: Send + Sync + 'static,
    {
        let key = D::key();
        if self.daemons.contains_key(&key) {
            return Err(DaemonError::DuplicateKey { key });
        }
        let runtime_state = Arc::new(DaemonRuntime::<D>::new(config, self.parent_cancel.clone()));
        let handle: Arc<dyn AnyDaemonHandle> = Arc::new(TypedDaemonHandle {
            daemon,
            runtime,
            runtime_state,
            ctx,
            key: key.clone(),
        });
        tracing::info!(daemon.key = %key, "daemon registered");
        self.daemons.insert(key, handle);
        Ok(())
    }

    /// Start every registered daemon in parallel.
    ///
    /// Failures are aggregated — a single daemon's failure does not abort
    /// sibling startups. Returns the first error if any daemon failed.
    ///
    /// # Errors
    ///
    /// Returns `DaemonError::StartFailed` for the first daemon that failed
    /// (others may have succeeded; check via [`Self::is_running`]).
    pub async fn start_all(&self) -> Result<(), DaemonError> {
        let handles: Vec<Arc<dyn AnyDaemonHandle>> = self
            .daemons
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect();
        let results = join_all(handles.iter().map(|h| h.start())).await;
        for r in results {
            r?;
        }
        Ok(())
    }

    /// Stop every registered daemon in parallel.
    ///
    /// Per-daemon cancellation flows through the per-run child token; parent
    /// stays live so the registry remains usable for subsequent `start_all`.
    pub async fn stop_all(&self) {
        let handles: Vec<Arc<dyn AnyDaemonHandle>> = self
            .daemons
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect();
        join_all(handles.iter().map(|h| h.stop())).await;
    }

    /// Whether the daemon under `key` is currently running.
    pub async fn is_running(&self, key: &ResourceKey) -> bool {
        match self.daemons.get(key) {
            Some(handle) => handle.is_running().await,
            None => false,
        }
    }

    /// Cancel the parent token and stop every daemon.
    ///
    /// After `shutdown`, the registry's parent token is cancelled and cannot
    /// be reused. Construct a new registry to start over.
    pub async fn shutdown(&self) {
        self.parent_cancel.cancel();
        self.stop_all().await;
    }
}

impl Default for DaemonRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DaemonRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<ResourceKey> = self
            .daemons
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        f.debug_struct("DaemonRegistry")
            .field("daemon_keys", &keys)
            .field("parent_cancelled", &self.parent_cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// Errors produced by [`DaemonRegistry`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DaemonError {
    /// A daemon with the same `Resource::key` is already registered.
    #[error("daemon already registered: {key}")]
    DuplicateKey {
        /// The colliding `Resource::key`.
        key: ResourceKey,
    },
    /// `DaemonRuntime::start` returned an error.
    #[error("daemon start failed for {key}: {reason}")]
    StartFailed {
        /// The daemon whose start failed.
        key: ResourceKey,
        /// The error message from `Resource::Error` propagation.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, atomic::{AtomicU32, Ordering}},
        time::Duration,
    };

    use nebula_core::{ExecutionId, ResourceKey};
    use nebula_resource::{
        context::ResourceContext,
        error::Error as ResourceError,
        resource::{Resource, ResourceConfig, ResourceMetadata},
    };
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::daemon::{Daemon, DaemonConfig, RestartPolicy};

    #[derive(Clone, Debug, Default)]
    struct EmptyCfg;

    nebula_schema::impl_empty_has_schema!(EmptyCfg);

    impl ResourceConfig for EmptyCfg {
        fn fingerprint(&self) -> u64 {
            0
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("registry-test: {0}")]
    struct TestError(&'static str);

    impl From<TestError> for ResourceError {
        fn from(e: TestError) -> Self {
            ResourceError::transient(e.to_string())
        }
    }

    #[derive(Clone)]
    struct CountedDaemon {
        attempts: Arc<AtomicU32>,
    }

    impl Resource for CountedDaemon {
        type Config = EmptyCfg;
        type Runtime = ();
        type Lease = ();
        type Error = TestError;
        type Credential = nebula_credential::NoCredential;

        fn key() -> ResourceKey {
            ResourceKey::new("registry-counted").unwrap()
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _scheme: &<Self::Credential as nebula_credential::Credential>::Scheme,
            _ctx: &ResourceContext,
        ) -> Result<(), TestError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Daemon for CountedDaemon {
        async fn run(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &ResourceContext,
            cancel: CancellationToken,
        ) -> Result<(), TestError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            cancel.cancelled().await;
            Ok(())
        }
    }

    fn make_ctx() -> ResourceContext {
        ResourceContext::minimal(
            nebula_core::scope::Scope {
                execution_id: Some(ExecutionId::new()),
                ..Default::default()
            },
            CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn empty_registry_starts_and_stops() {
        let reg = DaemonRegistry::new();
        assert!(reg.is_empty());
        reg.start_all().await.expect("empty start_all is ok");
        reg.stop_all().await;
    }

    #[tokio::test]
    async fn register_starts_daemon_and_shutdown_cancels() {
        let reg = DaemonRegistry::new();
        let attempts = Arc::new(AtomicU32::new(0));
        let daemon = CountedDaemon {
            attempts: Arc::clone(&attempts),
        };

        reg.register(
            daemon,
            Arc::new(()),
            DaemonConfig {
                restart_policy: RestartPolicy::Never,
                max_restarts: 0,
                restart_backoff: Duration::from_millis(10),
            },
            make_ctx(),
        )
        .expect("register ok");
        assert_eq!(reg.len(), 1);

        reg.start_all().await.expect("start_all ok");
        // Give the daemon time to enter the cancel-await.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert!(reg.is_running(&CountedDaemon::key()).await);

        reg.shutdown().await;
        assert!(!reg.is_running(&CountedDaemon::key()).await);
    }

    #[tokio::test]
    async fn duplicate_register_fails_closed() {
        let reg = DaemonRegistry::new();
        let attempts = Arc::new(AtomicU32::new(0));
        let daemon_a = CountedDaemon {
            attempts: Arc::clone(&attempts),
        };
        let daemon_b = CountedDaemon {
            attempts: Arc::clone(&attempts),
        };

        reg.register(
            daemon_a,
            Arc::new(()),
            DaemonConfig::default(),
            make_ctx(),
        )
        .expect("first register ok");

        let err = reg
            .register(daemon_b, Arc::new(()), DaemonConfig::default(), make_ctx())
            .expect_err("second register must fail-closed");
        assert!(matches!(err, DaemonError::DuplicateKey { .. }));
    }
}
```

- [ ] **Step 2: Verify Cargo deps for the registry are present**

```bash
grep -E "^(dashmap|futures|tokio-util) " crates/engine/Cargo.toml
```

Expected output includes `dashmap = { workspace = true }` and `tokio-util = { workspace = true }`. `futures` may need a workspace addition — verify:

```bash
grep "^futures" crates/engine/Cargo.toml
```

If `futures` is missing, add it:

```toml
futures = { workspace = true }
```

- [ ] **Step 3: Compile and run the registry tests**

```bash
cargo check -p nebula-engine
cargo nextest run -p nebula-engine --profile ci --no-tests=pass daemon::registry
```

Expected: 3 tests pass (`empty_registry_starts_and_stops`, `register_starts_daemon_and_shutdown_cancels`, `duplicate_register_fails_closed`).

- [ ] **Step 4: Commit Tasks 1-3 together**

```bash
git add crates/engine/src/daemon/ crates/engine/src/lib.rs crates/engine/Cargo.toml
git commit -m "feat(engine): add Daemon module — trait, runtime, registry

Migrate Daemon trait + DaemonRuntime<D> + RestartPolicy + DaemonConfig
from nebula-resource to nebula-engine per ADR-0037 / Tech Spec §12.1-§12.2.
Add DaemonRegistry primitive (DashMap<ResourceKey, dyn AnyDaemonHandle>,
parent CancellationToken, parallel start_all/stop_all, fail-closed
duplicate registration per ADR-0030 / N7 mitigation).

Type-name preservation: Daemon, DaemonRuntime, RestartPolicy, DaemonConfig
all keep names (Tech Spec §12.2).

Tests: 3 migrated DaemonRuntime regression tests (#318/#323) + 3 new
registry tests (empty/register-start-shutdown/fail-closed-duplicate).

Refs: ADR-0037, Tech Spec §12.1-§12.2, R-010, R-011, R-012"
```

---

## Task 4 — Migrate `EventSource` trait + runtime + adapter to engine

**Files:**
- Modify: `crates/engine/src/daemon/event_source.rs` (replace stub with full content)
- Read source: `crates/resource/src/topology/event_source.rs` (50 LOC) + `crates/resource/src/runtime/event_source.rs` (75 LOC)

**Why:** Tech Spec §12.3 commits engine-fold for EventSource as a `TriggerAction` adapter. The §12.3 sketch uses hypothetical `type TriggerEvent` and `EventStream<>` types that do not exist in the actual `crates/action/src/trigger.rs:61` `TriggerAction` definition (which has `start`/`stop` methods returning `Result<(), ActionError>`, no associated types, no event-stream). The PollTriggerAdapter at `crates/action/src/poll.rs:1051+` is the canonical "run-until-cancelled `start()` with `ctx.emitter().emit()` + cancel-safe `tokio::select!`" precedent — `EventSourceAdapter` mirrors it.

**Adapter design (canonical, ratifies Tech Spec §12.3 implementation):**

- `EventSourceAdapter<E: EventSource>` stores: typed `source`, `Arc<E::Runtime>`, `ActionMetadata` (caller supplies — EventSource has no inherent action metadata), `EventSourceConfig`, and a closure `Arc<dyn Fn(&E::Event) -> serde_json::Value + Send + Sync>` for payload conversion.
- The closure approach avoids tightening `EventSource::Event: Serialize` at the trait level (current bound is `Send + Clone + 'static`). Callers control serialization shape, including redaction.
- `start()` calls `subscribe()`, then loops `recv()` → call `event_to_payload(&event)` → `ctx.emitter().emit(payload).await`. Biased `tokio::select!` against `ctx.cancellation()` ensures cancel-safe termination.
- `stop()` is a no-op — cancellation flows through `ctx.cancellation()` per the PollTriggerAdapter convention (`crates/action/src/poll.rs:1455`).

- [ ] **Step 1: Replace `crates/engine/src/daemon/event_source.rs` with the full module**

```rust
//! EventSource topology + `EventSourceAdapter<E>: TriggerAction`.
//!
//! Migrated from `crates/resource/src/topology/event_source.rs` and
//! `crates/resource/src/runtime/event_source.rs` per ADR-0037 / Tech Spec
//! §12.3. EventSource lands as a thin adapter onto engine's existing
//! `TriggerAction` substrate.
//!
//! # Why an adapter, not a TriggerAction extension
//!
//! `EventSource: Resource` (needs `R::Runtime`, `R::Error`, `ResourceContext`)
//! and `TriggerAction: Action` (needs `ActionMetadata`, `TriggerContext`,
//! `ActionError`) sit on different bases. Rather than refactor either trait,
//! `EventSourceAdapter<E>` bridges them at construction time:
//! caller supplies `Arc<E::Runtime>`, `ActionMetadata`, and an `event_to_payload`
//! closure; the adapter implements `TriggerAction::start` as a "run-until-cancelled"
//! loop that `subscribe`s + `recv`s + emits via `ctx.emitter()`.
//!
//! This mirrors `crates/action/src/poll.rs::PollTriggerAdapter` (which runs
//! `poll()` in an inline loop driven by `ctx.cancellation()` + `ctx.emitter()`).

use std::{future::Future, sync::Arc};

use nebula_action::{
    Action, ActionError, ActionMetadata, DeclaresDependencies, TriggerAction, TriggerContext,
};
use nebula_resource::{Resource, ResourceContext};

/// EventSource — pull-based event subscription.
///
/// A long-lived event producer where consumers create subscriptions via
/// [`Self::subscribe`] and drain events via [`Self::recv`].
pub trait EventSource: Resource {
    /// The event type produced by this source.
    type Event: Send + Clone + 'static;
    /// An opaque subscription handle for receiving events.
    type Subscription: Send + 'static;

    /// Creates a new subscription to this event source.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the subscription cannot be created.
    fn subscribe(
        &self,
        runtime: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Subscription, Self::Error>> + Send;

    /// Receives the next event from a subscription.
    ///
    /// This method blocks asynchronously until an event is available.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the subscription is broken or the source
    /// has been shut down.
    fn recv(
        &self,
        subscription: &mut Self::Subscription,
    ) -> impl Future<Output = Result<Self::Event, Self::Error>> + Send;
}

/// EventSource configuration.
#[derive(Debug, Clone, Default)]
pub struct EventSourceConfig {
    /// Buffer size for the event channel (transport-dependent semantics).
    pub buffer_size: usize,
}

/// Runtime state for an EventSource — preserves the original
/// `EventSourceRuntime<R>` shape from `nebula-resource` for callers that want
/// the explicit subscribe/recv API outside the `TriggerAction` adapter path.
///
/// Most consumers should use [`EventSourceAdapter`] instead — it folds
/// EventSource into the engine's `TriggerAction` substrate. This struct stays
/// for the rare case where direct subscription management is needed
/// (e.g. testing, ad-hoc engine tooling).
pub struct EventSourceRuntime<E: EventSource> {
    config: EventSourceConfig,
    _phantom: std::marker::PhantomData<E>,
}

impl<E: EventSource> EventSourceRuntime<E> {
    /// Creates a new event source runtime with the given configuration.
    #[must_use]
    pub fn new(config: EventSourceConfig) -> Self {
        Self {
            config,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &EventSourceConfig {
        &self.config
    }
}

impl<E> EventSourceRuntime<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Runtime: Send + Sync + 'static,
{
    /// Creates a new subscription to the event source.
    ///
    /// `E::Error: Into<nebula_resource::Error>` is implied by the `Resource`
    /// supertrait declaration (`type Error: ... + Into<crate::Error>`).
    ///
    /// # Errors
    ///
    /// Propagates errors from [`EventSource::subscribe`].
    pub async fn subscribe(
        &self,
        resource: &E,
        runtime: &E::Runtime,
        ctx: &ResourceContext,
    ) -> Result<E::Subscription, nebula_resource::Error> {
        resource.subscribe(runtime, ctx).await.map_err(Into::into)
    }

    /// Receives the next event from a subscription.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`EventSource::recv`].
    pub async fn recv(
        &self,
        resource: &E,
        subscription: &mut E::Subscription,
    ) -> Result<E::Event, nebula_resource::Error> {
        resource.recv(subscription).await.map_err(Into::into)
    }
}

// ── EventSourceAdapter — bridges EventSource onto TriggerAction ─────────────

/// Adapts an `EventSource` impl as a [`TriggerAction`] so the engine can drive
/// it through the existing trigger lifecycle (`start`/`stop` + emit-via-context).
///
/// # Construction
///
/// Callers supply:
/// - the typed `source: E`,
/// - an `Arc<E::Runtime>` (caller is responsible for building `E::Runtime`
///   — typically via `Resource::create()` outside the adapter),
/// - `ActionMetadata` (EventSource has no inherent action metadata),
/// - `EventSourceConfig` for buffer / flow-control hints,
/// - an `event_to_payload` closure converting `&E::Event` to
///   `serde_json::Value` (caller controls serialization + redaction).
///
/// # Cancellation
///
/// `start()` runs a "run-until-cancelled" loop using a biased `tokio::select!`
/// against `ctx.cancellation()`. Drop-safety: each `recv().await` is the
/// subscription's responsibility; the adapter does not retain in-flight events.
pub struct EventSourceAdapter<E: EventSource> {
    source: E,
    runtime: Arc<E::Runtime>,
    metadata: ActionMetadata,
    #[allow(dead_code, reason = "buffer_size hint for downstream observability")]
    config: EventSourceConfig,
    event_to_payload: Arc<dyn Fn(&E::Event) -> serde_json::Value + Send + Sync>,
}

impl<E> EventSourceAdapter<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Runtime: Send + Sync + 'static,
{
    /// Wrap an EventSource impl as a `TriggerAction`.
    pub fn new<F>(
        source: E,
        runtime: Arc<E::Runtime>,
        metadata: ActionMetadata,
        config: EventSourceConfig,
        event_to_payload: F,
    ) -> Self
    where
        F: Fn(&E::Event) -> serde_json::Value + Send + Sync + 'static,
    {
        Self {
            source,
            runtime,
            metadata,
            config,
            event_to_payload: Arc::new(event_to_payload),
        }
    }
}

impl<E> DeclaresDependencies for EventSourceAdapter<E> where E: EventSource {}

impl<E> Action for EventSourceAdapter<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Runtime: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl<E> TriggerAction for EventSourceAdapter<E>
where
    E: EventSource + Send + Sync + 'static,
    E::Runtime: Send + Sync + 'static,
{
    async fn start(&self, ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        let resource_ctx = ResourceContext::minimal(ctx.scope().clone(), ctx.cancellation().clone());
        let mut subscription = self
            .source
            .subscribe(&self.runtime, &resource_ctx)
            .await
            .map_err(|e| ActionError::transient(e.to_string()))?;

        loop {
            tokio::select! {
                biased;
                () = ctx.cancellation().cancelled() => return Ok(()),
                recv = self.source.recv(&mut subscription) => {
                    match recv {
                        Ok(event) => {
                            let payload = (self.event_to_payload)(&event);
                            if let Err(e) = ctx.emitter().emit(payload).await {
                                tracing::warn!(error = %e, "event_source: emit failed");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "event_source: recv failed; exiting loop");
                            return Err(ActionError::transient(e.to_string()));
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        // Cancellation flows through ctx.cancellation() per PollTriggerAdapter
        // convention; the live start() loop observes it and returns Ok(()).
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

    use nebula_action::{
        ActionMetadata,
        testing::{TestContextBuilder, TestTriggerContext},
    };
    use nebula_core::{ExecutionId, ResourceKey, action_key};
    use nebula_resource::{
        context::ResourceContext,
        error::Error as ResourceError,
        resource::{Resource, ResourceConfig, ResourceMetadata},
    };

    use super::*;

    #[derive(Clone, Debug, Default)]
    struct EmptyCfg;

    nebula_schema::impl_empty_has_schema!(EmptyCfg);

    impl ResourceConfig for EmptyCfg {
        fn fingerprint(&self) -> u64 {
            0
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("event-test: {0}")]
    struct TestError(&'static str);

    impl From<TestError> for ResourceError {
        fn from(e: TestError) -> Self {
            ResourceError::transient(e.to_string())
        }
    }

    /// Test EventSource that emits 3 fixed events then blocks.
    #[derive(Clone)]
    struct ThreeEventSource {
        emitted: Arc<AtomicU32>,
    }

    impl Resource for ThreeEventSource {
        type Config = EmptyCfg;
        type Runtime = ();
        type Lease = ();
        type Error = TestError;
        type Credential = nebula_credential::NoCredential;

        fn key() -> ResourceKey {
            ResourceKey::new("event-three").unwrap()
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _scheme: &<Self::Credential as nebula_credential::Credential>::Scheme,
            _ctx: &ResourceContext,
        ) -> Result<(), TestError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl EventSource for ThreeEventSource {
        type Event = u32;
        type Subscription = ();

        async fn subscribe(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &ResourceContext,
        ) -> Result<Self::Subscription, TestError> {
            Ok(())
        }

        async fn recv(
            &self,
            _subscription: &mut Self::Subscription,
        ) -> Result<Self::Event, TestError> {
            let n = self.emitted.fetch_add(1, Ordering::SeqCst);
            if n < 3 {
                Ok(n)
            } else {
                // Block forever — caller should observe cancellation.
                std::future::pending().await
            }
        }
    }

    fn make_metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.event_source_adapter"),
            "EventSourceAdapterTest",
            "Adapter integration test",
        )
    }

    #[tokio::test]
    async fn adapter_emits_events_until_cancelled() {
        let emitted = Arc::new(AtomicU32::new(0));
        let source = ThreeEventSource {
            emitted: Arc::clone(&emitted),
        };
        let adapter = EventSourceAdapter::new(
            source,
            Arc::new(()),
            make_metadata(),
            EventSourceConfig::default(),
            |e: &u32| serde_json::json!({ "n": *e }),
        );

        let ctx: TestTriggerContext = TestContextBuilder::new().build_trigger().0;
        let cancel = ctx.cancellation().clone();

        // Run start() in background; cancel after a short delay.
        let join = tokio::spawn(async move { adapter.start(&ctx).await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();
        let result = join.await.expect("join ok");
        assert!(result.is_ok(), "start should return Ok on cancellation: {result:?}");

        // 3 events succeeded; 4th call hit pending() then was cancelled.
        assert!(emitted.load(Ordering::SeqCst) >= 3);
    }

    #[tokio::test]
    async fn adapter_stop_is_noop() {
        let source = ThreeEventSource {
            emitted: Arc::new(AtomicU32::new(0)),
        };
        let adapter = EventSourceAdapter::new(
            source,
            Arc::new(()),
            make_metadata(),
            EventSourceConfig::default(),
            |e: &u32| serde_json::json!({ "n": *e }),
        );

        let ctx: TestTriggerContext = TestContextBuilder::new().build_trigger().0;
        // stop() is a no-op — should always succeed.
        adapter.stop(&ctx).await.expect("stop is infallible");
    }
}
```

- [ ] **Step 2: Verify Cargo deps**

```bash
grep -E "^(serde_json|nebula-action) " crates/engine/Cargo.toml
```

Expected: `nebula-action = { path = "../action" }` already present. `serde_json`: verify and add if missing:

```toml
serde_json = { workspace = true }
```

- [ ] **Step 3: Verify TestContextBuilder is exposed by nebula-action**

```bash
grep -n "pub mod testing\|pub use testing" crates/action/src/lib.rs
grep -n "TestTriggerContext\|TestContextBuilder" crates/action/src/testing.rs 2>/dev/null | head
```

Expected: `pub mod testing;` plus `TestContextBuilder` and `TestTriggerContext` accessible. If not exposed at the right path, adjust the test imports — paths may be `nebula_action::testing::*` instead of through the umbrella re-export.

- [ ] **Step 4: Compile and run the adapter tests**

```bash
cargo check -p nebula-engine
cargo nextest run -p nebula-engine --profile ci --no-tests=pass daemon::event_source
```

Expected: 2 tests pass (`adapter_emits_events_until_cancelled`, `adapter_stop_is_noop`).

If `nebula_resource::Error: From<E::Error>` constraint on `EventSourceRuntime` causes friction with tests not declaring the impl: that constraint is on the runtime methods only, not the adapter; the test path uses the adapter and doesn't touch `EventSourceRuntime`. If `EventSourceRuntime` causes orphan-rule issues, simplify it to return `E::Error` directly without the conversion (matches the original behaviour — `crates/resource/src/runtime/event_source.rs` did the conversion via `Into::into` to `crate::Error`, which won't compile cross-crate without a blanket `From` — fall back to returning `E::Error` directly if so).

- [ ] **Step 5: Commit Task 4**

```bash
git add crates/engine/src/daemon/event_source.rs crates/engine/Cargo.toml
git commit -m "feat(engine): add EventSource module + TriggerAction adapter

Migrate EventSource trait + EventSourceConfig + EventSourceRuntime from
nebula-resource to nebula-engine per ADR-0037 / Tech Spec §12.3. Add
EventSourceAdapter<E>: TriggerAction with closure-based payload converter
(Fn(&E::Event) -> serde_json::Value), 'run-until-cancelled' start() loop
mirroring PollTriggerAdapter convention (subscribe + recv + emit via
ctx.emitter, biased tokio::select! on ctx.cancellation()).

Tech Spec §12.3 sketch used hypothetical TriggerAction::TriggerEvent /
EventStream<> types not present in actual crates/action/src/trigger.rs.
Implementation reconciles with the real Action+TriggerAction shape;
closure approach avoids tightening EventSource::Event: Serialize at the
trait level.

Tests: 2 new adapter tests (emits-until-cancelled, stop-is-noop).

Refs: ADR-0037, Tech Spec §12.3, R-011, R-012"
```

---

## Task 5 — Shrink `TopologyRuntime<R>` 7 → 5 in nebula-resource

**Files:**
- Modify: `crates/resource/src/runtime/mod.rs` (drop variants + match arms + intra-doc links + submodule decls)
- Modify: `crates/resource/src/topology_tag.rs` (drop `Daemon` and `EventSource` variants + `as_str` arms + doc lines)
- Modify: `crates/resource/src/manager/mod.rs:1373` (drop the daemon match arm in `reload_config`)

**Why:** Tech Spec §12.5 mechanical shrink. After Task 5, `nebula-resource` retains zero references to `Daemon`/`EventSource` types — Task 6 deletes the source files; this task severs the integration points first so the compiler surfaces every remaining dangling reference.

- [ ] **Step 1: Edit `crates/resource/src/runtime/mod.rs`**

Apply these three changes:

1. Drop the two `pub mod` lines (`pub mod daemon;`, `pub mod event_source;`).
2. Drop intra-doc links from the module-level `//!` (lines 13-14).
3. Drop the two enum variants and the two `tag()` match arms.

Replace the file contents with:

```rust
//! Topology runtime implementations.
//!
//! Each topology trait ([`Pooled`], [`Resident`], [`Service`], [`Transport`],
//! [`Exclusive`]) has a corresponding runtime struct that manages instance
//! lifecycle, and a dispatch enum ([`TopologyRuntime`]) that erases the
//! topology at the registration level.
//!
//! [`Pooled`]: crate::topology::pooled::Pooled
//! [`Resident`]: crate::topology::resident::Resident
//! [`Service`]: crate::topology::service::Service
//! [`Transport`]: crate::topology::transport::Transport
//! [`Exclusive`]: crate::topology::exclusive::Exclusive

pub mod exclusive;
pub mod managed;
pub mod pool;
pub mod resident;
pub mod service;
pub mod transport;

use crate::{resource::Resource, topology_tag::TopologyTag};

/// Dispatch enum for all topology runtimes.
///
/// Each variant holds the runtime state for a specific topology. The
/// engine stores one `TopologyRuntime<R>` per registered resource,
/// inside [`ManagedResource`](managed::ManagedResource).
pub enum TopologyRuntime<R: Resource> {
    /// Pool of N interchangeable instances with checkout/recycle.
    Pool(pool::PoolRuntime<R>),
    /// Single shared instance, clone on acquire.
    Resident(resident::ResidentRuntime<R>),
    /// Long-lived runtime with short-lived tokens.
    Service(service::ServiceRuntime<R>),
    /// Shared connection with multiplexed sessions.
    Transport(transport::TransportRuntime<R>),
    /// One caller at a time via semaphore(1).
    Exclusive(exclusive::ExclusiveRuntime<R>),
}

impl<R: Resource> TopologyRuntime<R> {
    /// Returns the topology tag for this runtime variant.
    pub fn tag(&self) -> TopologyTag {
        match self {
            Self::Pool(_) => TopologyTag::Pool,
            Self::Resident(_) => TopologyTag::Resident,
            Self::Service(_) => TopologyTag::Service,
            Self::Transport(_) => TopologyTag::Transport,
            Self::Exclusive(_) => TopologyTag::Exclusive,
        }
    }
}
```

- [ ] **Step 2: Edit `crates/resource/src/topology_tag.rs`**

Drop `Daemon` and `EventSource` variants and their `as_str` arms:

```rust
//! Topology identifier tag.

use std::fmt;

/// Identifies which topology a resource handle was acquired from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TopologyTag {
    /// Pool — N interchangeable instances.
    Pool,
    /// Resident — one shared instance, clone on acquire.
    Resident,
    /// Service — long-lived runtime, short-lived tokens.
    Service,
    /// Transport — shared connection, multiplexed sessions.
    Transport,
    /// Exclusive — one caller at a time.
    Exclusive,
}

impl TopologyTag {
    /// Returns the tag as a static string slice.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pool => "pool",
            Self::Resident => "resident",
            Self::Service => "service",
            Self::Transport => "transport",
            Self::Exclusive => "exclusive",
        }
    }
}

impl fmt::Display for TopologyTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
```

- [ ] **Step 3: Edit `crates/resource/src/manager/mod.rs:1373` to drop the daemon arm**

Locate the `reload_config` function and the line:

```rust
TopologyRuntime::Daemon(_) => ReloadOutcome::Restarting,
```

Delete that single line. The `reload_config` function should still compile — `TopologyRuntime` no longer carries `Daemon`, so the arm is unreachable. (`ReloadOutcome::Restarting` enum variant stays per Tech Spec §12.5: "engine-side daemons may still surface it via their own reload path".)

Verify no other lines in `crates/resource/src/manager/` reference `TopologyRuntime::Daemon` or `TopologyRuntime::EventSource`:

```bash
grep -rn "TopologyRuntime::\(Daemon\|EventSource\)" crates/resource/src/
```

Expected: no matches after edits.

- [ ] **Step 4: Verify resource crate fails compilation as expected**

```bash
cargo check -p nebula-resource
```

Expected: FAIL — the deleted enum variants surface dangling references in `topology/daemon.rs`, `topology/event_source.rs`, `runtime/daemon.rs`, `runtime/event_source.rs`, and the `lib.rs` re-exports. This is what Task 6 cleans up. **Do not commit yet** — fold Task 5 + Task 6 into one logical commit.

---

## Task 6 — Delete source files in nebula-resource and prune re-exports

**Files:**
- Delete: `crates/resource/src/topology/daemon.rs`
- Delete: `crates/resource/src/topology/event_source.rs`
- Delete: `crates/resource/src/runtime/daemon.rs`
- Delete: `crates/resource/src/runtime/event_source.rs`
- Modify: `crates/resource/src/topology/mod.rs` (drop `pub mod` + `pub use` lines + table comment rows)
- Modify: `crates/resource/src/lib.rs` (drop 5 `pub use` lines for daemon/event_source surface)

**Why:** Tech Spec §12.5 commits zero references to daemon/event_source in `nebula-resource` post-extraction. After Task 5 severed integration sites, this task removes the leaves.

- [ ] **Step 1: Delete the four source files**

```bash
git rm crates/resource/src/topology/daemon.rs
git rm crates/resource/src/topology/event_source.rs
git rm crates/resource/src/runtime/daemon.rs
git rm crates/resource/src/runtime/event_source.rs
```

- [ ] **Step 2: Edit `crates/resource/src/topology/mod.rs`**

Replace with:

```rust
//! Topology traits for resource management.
//!
//! Each topology describes a different access pattern for resources:
//!
//! | Topology | Pattern |
//! |----------|---------|
//! | [`Pooled`] | N interchangeable instances with checkout/recycle |
//! | [`Resident`] | One shared instance, clone on acquire |
//! | [`Service`] | Long-lived runtime, short-lived tokens |
//! | [`Transport`] | Shared connection, multiplexed sessions |
//! | [`Exclusive`] | One caller at a time via semaphore |
//!
//! `Daemon` (long-running worker) and `EventSource` (pull-based event
//! subscription) live in [`nebula_engine::daemon`] per ADR-0037 — canon §3.5
//! reserves "Resource" for pool/SDK clients.

pub mod exclusive;
pub mod pooled;
pub mod resident;
pub mod service;
pub mod transport;

pub use exclusive::Exclusive;
pub use pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision};
pub use resident::Resident;
pub use service::{Service, TokenMode};
pub use transport::Transport;
```

- [ ] **Step 3: Edit `crates/resource/src/lib.rs` to drop daemon/event_source re-exports**

Locate and remove these blocks (around lines 94-115):

```rust
// REMOVE these two lines from the runtime re-export block:
    daemon::DaemonRuntime,
    event_source::EventSourceRuntime,

// REMOVE this single line (line 106):
pub use topology::daemon::config::Config as DaemonConfig;

// EDIT the topology re-export block — remove daemon and event_source rows:
pub use topology::{
    daemon::{Daemon, RestartPolicy},                                    // REMOVE
    event_source::{EventSource, config::Config as EventSourceConfig},   // REMOVE
    exclusive::{Exclusive, config::Config as ExclusiveConfig},
    pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision, config::Config as PoolConfig},
    resident::{Resident, config::Config as ResidentConfig},
    service::{Service, TokenMode, config::Config as ServiceConfig},
    transport::{Transport, config::Config as TransportConfig},
};
```

After edit, the relevant section of `lib.rs` should read:

```rust
// Runtime types — needed for `Manager::register()`.
pub use runtime::TopologyRuntime;
pub use runtime::{
    exclusive::ExclusiveRuntime,
    managed::ManagedResource,
    pool::{PoolRuntime, PoolStats},
    resident::ResidentRuntime,
    service::ServiceRuntime,
    transport::TransportRuntime,
};
pub use state::{ResourcePhase, ResourceStatus};
// Topology configurations — used at registration time.
pub use topology::{
    exclusive::{Exclusive, config::Config as ExclusiveConfig},
    pooled::{BrokenCheck, InstanceMetrics, Pooled, RecycleDecision, config::Config as PoolConfig},
    resident::{Resident, config::Config as ResidentConfig},
    service::{Service, TokenMode, config::Config as ServiceConfig},
    transport::{Transport, config::Config as TransportConfig},
};
pub use topology_tag::TopologyTag;
```

- [ ] **Step 4: Resource crate compiles**

```bash
cargo check -p nebula-resource
```

Expected: PASS. Any remaining dangling reference (e.g., in `crates/resource/README.md` or some rustdoc link inside `manager/`) surfaces here — fix them before continuing. Common culprits:

- Module docs that say "Seven topology traits" — change to "Five topology traits" in `crates/resource/src/lib.rs:9`.
- README topology summary — verify with `grep -i "daemon\|event.source" crates/resource/README.md`; trim if present.
- Doc comment `//! See ...` references — should be self-fixing once trait is gone.

- [ ] **Step 5: Workspace compiles**

```bash
cargo check --workspace
```

Expected: PASS. Any consumer crate that referenced daemon/event_source surfaces here. Per Tech Spec §12.4 Phase 1 audit + this plan's audit at planning time, no consumer references exist — but re-verify.

- [ ] **Step 6: Commit Tasks 5+6 together (atomic enum shrink + cleanup)**

```bash
git add -A crates/resource/src/topology/ crates/resource/src/runtime/ crates/resource/src/topology_tag.rs crates/resource/src/manager/mod.rs crates/resource/src/lib.rs crates/resource/README.md
git commit -m "feat(resource)!: drop Daemon and EventSource topologies (engine fold)

Per ADR-0037 / Tech Spec §12.5: TopologyRuntime<R> shrinks 7 → 5
variants. Daemon and EventSource trait + runtime + config now live in
nebula-engine::daemon (Tasks 1-4 of П3 implementation plan).

Mechanical changes:
- Delete crates/resource/src/{topology,runtime}/{daemon,event_source}.rs
- Drop Daemon/EventSource variants from TopologyRuntime + tag() arms
- Drop Daemon/EventSource variants from TopologyTag + as_str arms
- Drop daemon special-case from reload_config (manager/mod.rs:1373)
- Drop pub use re-exports from topology/mod.rs and lib.rs
- Update module docs (5 topologies, not 7) + topology table

Cross-consumer impact: zero. nebula-action/sdk/plugin/sandbox have no
Daemon/EventSource references (verified at audit time). Only nebula-engine
(the new home) gains daemon symbols.

Restores canon §3.5 truth in nebula-resource: 'Resource = pool/SDK client'.

Closes R-010, R-011, R-012 (concerns register).
Refs: ADR-0037, Tech Spec §12.5"
```

---

## Task 7 — Engine `lib.rs` re-exports

**Files:**
- Modify: `crates/engine/src/lib.rs` (add daemon re-export block)

**Why:** Public surface for downstream callers — the `pub mod daemon` from Task 1 only exposes `nebula_engine::daemon::Daemon`; convention across the engine crate is to surface key types at the crate root (`nebula_engine::Daemon`). Mirrors the existing credential re-export block at `crates/engine/src/lib.rs:70-73`.

- [ ] **Step 1: Add the re-export block**

Locate the `pub use credential::...;` block around `crates/engine/src/lib.rs:70-73`. Add immediately after it:

```rust
pub use daemon::{
    AnyDaemonHandle, Daemon, DaemonConfig, DaemonError, DaemonRegistry, DaemonRuntime,
    EventSource, EventSourceAdapter, EventSourceConfig, EventSourceRuntime, RestartPolicy,
};
```

- [ ] **Step 2: Verify**

```bash
cargo check -p nebula-engine
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-engine --no-deps
```

Expected: both PASS. Doc gate catches any broken intra-doc link in `crates/engine/src/daemon/` modules.

- [ ] **Step 3: Commit**

```bash
git add crates/engine/src/lib.rs
git commit -m "feat(engine): re-export daemon module surface at crate root

Mirror credential re-export pattern: surface Daemon, DaemonRuntime,
DaemonRegistry, DaemonConfig, RestartPolicy, AnyDaemonHandle, DaemonError,
EventSource, EventSourceConfig, EventSourceRuntime, EventSourceAdapter at
nebula_engine::* so consumers don't need to write nebula_engine::daemon::*.

Refs: ADR-0037, Tech Spec §12.1"
```

---

## Task 8 — Workspace verification gate

**Files:** none (verification only)

**Why:** Final gate before concerns-register update. CLAUDE.md "non-negotiable evidence-before-assertion" — no completion claim without command output proving each gate is green.

- [ ] **Step 1: Format check**

```bash
cargo +nightly fmt --all -- --check
```

Expected: no output (all formatted). If anything emits a diff, run `cargo +nightly fmt --all` and re-stage; the doc edits in Task 6 may shift line wrap.

- [ ] **Step 2: Clippy on the whole workspace**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS. Common new-failure modes from this PR:
- `clippy::missing_docs_in_private_items` if engine has a tighter lint than resource — confirm by checking `crates/engine/src/lib.rs` `#![warn(...)]`.
- `clippy::needless_pass_by_value` on the `event_to_payload: F` constructor parameter — if it fires, accept by value (the field stores `Arc<dyn Fn>` so the move is intentional).

- [ ] **Step 3: Whole-workspace tests**

```bash
cargo nextest run --workspace --profile ci --no-tests=pass
```

Expected: PASS. Test count baseline post-П2: 3645. Expected delta:
- +3 daemon registry tests (`empty_registry_starts_and_stops`, `register_starts_daemon_and_shutdown_cancels`, `duplicate_register_fails_closed`)
- +2 event source adapter tests (`adapter_emits_events_until_cancelled`, `adapter_stop_is_noop`)
- ±0 daemon runtime tests (3 migrated, not added/removed)
- Total: ~3650 ± 2 (account for transitively-affected tests).

If a test fails: do NOT mark Task 8 complete. Diagnose; if the failure is genuine (e.g. timing-flaky test under nextest), surface to the user before proceeding.

- [ ] **Step 4: Doc gate**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Expected: PASS. Common breakage class — intra-doc links to deleted `crate::topology::daemon::*` or `crate::runtime::daemon::*` paths inside resource. Fix by replacing with prose ("Daemon and EventSource live in `nebula_engine::daemon`") since rustdoc can't resolve cross-crate intra-doc links to engine without the crate-prefix syntax.

- [ ] **Step 5: Soak the test count**

Capture the before/after test count for the commit message in Task 9:

```bash
cargo nextest run --workspace --profile ci --no-tests=pass 2>&1 | grep -E "Summary|tests run"
```

Note the actual number for the commit message.

---

## Task 9 — Update concerns register R-010, R-011, R-012

**Files:**
- Modify: `docs/tracking/nebula-resource-concerns-register.md`

**Why:** Cascade Phase 7 ownership — every cascade PR closes the concerns it lands. Mirrors П1 (R-001) and П2 (R-002, R-003, R-004, R-005, R-023) closure pattern.

- [ ] **Step 1: Edit the three rows**

In `docs/tracking/nebula-resource-concerns-register.md`, locate the "Topology surface" table and replace the three status cells:

```md
| R-010 | Daemon topology has no public start path (pub(crate) barrier) | 🔴 | tech-spec-material | Phase 1 §1.2 dx-tester `runtime/managed.rs:35` | **landed П3** (`<short-sha>`); extracted to `nebula-engine::daemon` per ADR-0037 / Tech Spec §12.1-§12.2; `DaemonRegistry` provides public start/stop/shutdown surface | Strategy §4.4 + ADR-0037 |
| R-011 | EventSource same orphan-surface pattern — 0 Manager-level tests | 🔴 | tech-spec-material | Phase 1 §1.6 convergent | **landed П3** (`<short-sha>`); EventSource trait + `EventSourceAdapter<E>: TriggerAction` in `nebula-engine::daemon::event_source` per Tech Spec §12.3 | Strategy §4.4 + ADR-0037 |
| R-012 | Daemon + EventSource out-of-canon §3.5 ("resource = pool/SDK client") | 🟠 | tech-spec-material | Phase 1 §1.6 tech-lead | **landed П3** (`<short-sha>`); `nebula-resource` retains zero refs to Daemon/EventSource; canon §3.5 truth restored | Strategy §4.4 + ADR-0037 |
```

Substitute `<short-sha>` with the actual short-SHA after rebasing/merging the previous tasks.

- [ ] **Step 2: Verify lifecycle invariants**

```bash
grep -E "^\| R-01[012] " docs/tracking/nebula-resource-concerns-register.md
```

Expected: 3 rows, all containing `**landed П3**`.

- [ ] **Step 3: Commit**

```bash
git add docs/tracking/nebula-resource-concerns-register.md
git commit -m "docs(tracking): close R-010/R-011/R-012 (Daemon/EventSource engine fold)

ADR-0037 + Tech Spec §12 fully landed in П3. Daemon and EventSource
topology infrastructure migrated from nebula-resource to
nebula-engine::daemon; TopologyRuntime<R> enum shrunk 7 → 5; canon §3.5
truth restored in nebula-resource.

Status updates:
- R-010 → landed П3 (Daemon public start path via DaemonRegistry)
- R-011 → landed П3 (EventSource via EventSourceAdapter<E>: TriggerAction)
- R-012 → landed П3 (canon §3.5 alignment — zero daemon/event_source refs in nebula-resource)

Refs: ADR-0037, Tech Spec §12.1-§12.5, Strategy §4.4"
```

---

## Task 10 — Strategy §5.1 trigger evaluation + PR open

**Files:** none (PR description content)

**Why:** Strategy §5.1 records the trigger condition for opening a `nebula-scheduler` follow-up cascade: *"Daemon-specific engine code grows beyond ~500 LOC OR non-trigger long-running workers proliferate beyond 2."* P3 lands ~700 LOC in `crates/engine/src/daemon/` — over the 500 LOC threshold. The PR description must record this evaluation honestly so future cascades aren't surprised.

- [ ] **Step 1: Compute the actual engine daemon LOC**

```bash
wc -l crates/engine/src/daemon/*.rs | tail -1
```

Capture the number for the PR body.

- [ ] **Step 2: Open the PR**

```bash
git push -u origin claude/resource-p3-daemon-engine-fold
gh pr create --title "feat(resource,engine)!: П3 — Daemon/EventSource engine fold (ADR-0037)" --body "$(cat <<'EOF'
## Summary

Third PR in the nebula-resource redesign cascade (П1: trait shape; П2: rotation L2; **П3: Daemon/EventSource engine fold**; П4: doc rewrite).

Extract `Daemon` and `EventSource` topology infrastructure from `nebula-resource` into `nebula-engine` per [ADR-0037](docs/adr/0037-daemon-eventsource-engine-fold.md) + [Tech Spec §12](docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md). Restore canon §3.5 ("Resource = pool/SDK client") truth.

## Closes

- 🔴 R-010 — Daemon topology has no public start path (`pub(crate)` barrier dissolved by extraction)
- 🔴 R-011 — EventSource same orphan-surface pattern (resolved by fold + adapter)
- 🟠 R-012 — Daemon + EventSource out-of-canon §3.5

## What changed

### nebula-engine (NEW: `crates/engine/src/daemon/`)

- `mod.rs` — `Daemon` trait, `RestartPolicy`, `DaemonConfig`
- `runtime.rs` — `DaemonRuntime<D>` migrated verbatim (with cancellation-model + #318/#323 regression coverage)
- `registry.rs` — `DaemonRegistry` primitive per Tech Spec §12.2 (typed handle `DashMap` keyed by `ResourceKey`, parallel `start_all`/`stop_all`, parent `CancellationToken` for shutdown propagation, fail-closed duplicate registration)
- `event_source.rs` — `EventSource` trait + `EventSourceConfig` + `EventSourceRuntime<E>` migrated; NEW `EventSourceAdapter<E>: TriggerAction` (closure-based payload converter, `subscribe`+`recv` loop, biased `tokio::select!` cancellation)

### nebula-resource (cleanup)

- `TopologyRuntime<R>`: 7 → 5 variants
- 4 source files deleted (`topology/daemon.rs`, `topology/event_source.rs`, `runtime/daemon.rs`, `runtime/event_source.rs`)
- `TopologyTag::Daemon` and `::EventSource` removed
- `lib.rs` re-exports pruned (5 lines)
- `manager/mod.rs:1373` daemon arm in `reload_config` removed
- Module docs updated (5 topologies, not 7)

### Cross-consumer impact

Zero. `nebula-action`/`sdk`/`plugin`/`sandbox` had no daemon/event_source references (verified at audit time). Only `nebula-engine` (the new home) and `nebula-resource` (the source) change.

### Strategy §5.1 trigger evaluation

> *"Daemon-specific engine code grows beyond ~500 LOC OR non-trigger long-running workers proliferate beyond 2."*

This PR lands `<actual-loc-number>` LOC in `crates/engine/src/daemon/`. The first half of the trigger fires. **No new cascade opens immediately** — single-source extraction is in scope per ADR-0037; the §5.1 trigger is evaluated post-merge alongside future non-trigger worker additions. Tracking deferred to whichever next workspace change adds a non-trigger worker (which would push the trigger to "AND" satisfaction).

## Test plan

- [x] `cargo nextest run --workspace --profile ci --no-tests=pass` — `<test-count>` tests pass (baseline post-П2: 3645; +5 new tests for registry + adapter)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [x] `cargo +nightly fmt --all -- --check` — clean
- [x] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — clean
- [x] No external `nebula_resource::Daemon`/`EventSource` users (`rg "nebula_resource::(Daemon|EventSource)" crates/`)

## Refs

- ADR-0037 — Daemon / EventSource engine fold (accepted, amended in place 2026-04-25)
- Tech Spec §12 — Daemon/EventSource extraction landing site
- Strategy §4.4 (rationale), §4.8 (atomic 5-consumer wave), §5.1 (revisit triggers)
- Concerns register: closes R-010, R-011, R-012

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Substitute `<actual-loc-number>` and `<test-count>` with values captured during verification.

- [ ] **Step 2: Post PR — capture URL + share**

Return the PR URL to the user. Subagent-driven execution returns control to the user; the user reviews CodeRabbit feedback per the П1+П2 process pattern.

---

## Self-Review Checklist

After all 10 tasks complete, the implementer should walk this checklist before declaring done:

1. **Spec coverage:**
   - [x] ADR-0037 §Decision bullet 1 (Daemon trait + DaemonRuntime → engine) — Tasks 1-3
   - [x] ADR-0037 §Decision bullet 2 (EventSource trait + adapter → engine) — Task 4
   - [x] ADR-0037 §Decision bullet 3 (TopologyRuntime 7 → 5) — Task 5
   - [x] ADR-0037 §Decision bullet 4 (zero refs in nebula-resource post-extraction) — Tasks 5-6
   - [x] ADR-0037 §Decision bullet 5 (existing tests migrate alongside) — Task 2 (3 tests preserved)
   - [x] Tech Spec §12.1 (engine module path = `crates/engine/src/daemon/`) — Task 1
   - [x] Tech Spec §12.2 (`DaemonRegistry` primitive shape) — Task 3
   - [x] Tech Spec §12.3 (EventSource → TriggerAction adapter) — Task 4 (with sketch reconciled to real trait shape)
   - [x] Tech Spec §12.5 (TopologyRuntime shrink + match-arm sweep + reload_config daemon arm removed) — Task 5

2. **Placeholder scan:** no `TBD`, `TODO`, `implement later`, "add appropriate error handling", "similar to Task N" anywhere in this plan. ✅

3. **Type consistency:**
   - `DaemonRuntime<D>` vs `DaemonRuntime<R>`: this plan uses `D` (consistent with `D: Daemon`); source uses `R: Resource`. Both work; engine version normalises on `D` per the trait it bounds. ✅
   - `DaemonRegistry::register` signature matches `TypedDaemonHandle::start` invocation. ✅
   - `EventSourceAdapter::new` closure signature `Fn(&E::Event) -> serde_json::Value` matches usage in `start()`. ✅

4. **Re-exports symmetry:**
   - Engine `lib.rs` re-exports (Task 7) include every type the cleanup at resource `lib.rs` (Task 6) drops, plus the new `DaemonRegistry`, `DaemonError`, `AnyDaemonHandle`, `EventSourceAdapter`. ✅

5. **Commit boundaries match brief:**
   - Tasks 1-3 → 1 commit (`feat(engine): add Daemon module`)
   - Task 4 → 1 commit (`feat(engine): add EventSource module + adapter`)
   - Tasks 5-6 → 1 commit (`feat(resource)!: drop Daemon and EventSource topologies`)
   - Task 7 → 1 commit (`feat(engine): re-export daemon module surface`)
   - Task 9 → 1 commit (`docs(tracking): close R-010/R-011/R-012`)
   - 5 commits total — within brief's "4-6 logical commits". ✅

6. **Dropped concerns:**
   - Strategy §5.1 trigger evaluation honestly recorded in PR description (not skipped). ✅
   - PR description names cross-consumer impact verification (`rg` command + result). ✅
