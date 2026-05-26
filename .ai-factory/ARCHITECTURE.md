# Architecture: Layered Modular Workspace (Cargo wrapper-enforced)

> Agent-actionable subset. The **canonical layer map lives in
> [`CLAUDE.md`](../CLAUDE.md) § "Layered Dependency Map"** and is mechanically
> enforced by `cargo deny check` against the `wrappers` allowlists on the
> `[bans].deny` entries in `deny.toml`. The README at the repo root is the
> product-facing crate map. If those disagree with anything below, **CLAUDE.md
> / README / `deny.toml` win** — fix this file, never the canon.

## Overview

Nebula is a **layered modular monolith** built as a Cargo workspace. Every
crate lives in exactly one layer, and inter-layer dependencies are enforced
**mechanically** by `cargo deny check` against the `wrappers` allowlists on
the `[bans].deny` entries in `deny.toml` — a missing entry fails CI before
review. The result is the
simple operational profile of a monolith with the modular discipline of
microservices: any individual crate can be embedded independently, but the
team ships one repo, one toolchain, one CI, one release cadence.

This is not generic "Clean Architecture" or "DDD". It is a Rust-native
pattern: **crates as modules, wrapper rules as compile-time architecture
tests, typed events for cross-crate seams.**

## Decision Rationale

- **Project type:** embedded Rust workflow-automation library (DAG engine,
  credentials, plugins, sandbox, HTTP/webhook surface).
- **Tech stack:** Rust 1.95+ (edition 2024, resolver 3), Tokio,
  `thiserror` + `tracing`.
- **Workspace size:** 29 first-party crates under `crates/` plus
  `apps/server` (see `Cargo.toml [workspace.members]`).
- **Team / scaling profile:** small core team, embeddable by external
  teams → modular discipline matters more than independent deploy.
- **Key factor:** modularity is a hard product constraint (see README "Why
  Nebula") — the `wrappers` allowlists in `deny.toml` make the layer
  boundaries cheap to enforce and impossible to drift quietly past.

## What agents need to know on top of CLAUDE.md / README / deny.toml

- Each layer depends only on layers below it; cross-cutting crates are
  importable at any level. The exact allowlist is each
  `[bans].deny[].wrappers` field in `deny.toml`.
- Cross-crate communication between siblings at the same layer goes through
  `nebula-eventbus`, not direct imports.
- **`nebula-credential` is shared infrastructure**, not a single-tier
  Business crate. Exec / Business / API tiers and the first-party backends
  (`credential-builtin`, `credential-vault`, `credential-runtime`,
  `credential-testutil`) all consume the credential contract directly. The
  exact consumer set is locked in `deny.toml` under the
  `nebula-credential` entry's `wrappers` field.
- **`nebula-storage-port`** (Core) is the object-safe storage seam every
  storage consumer depends on. **`nebula-storage`** (Exec) is the sole
  adapter implementation. **`nebula-tenancy`** (Business) is the
  scope-enforcing decorator that wraps a raw `storage-port` adapter so a
  tenant scope is substituted on every call before it reaches a handler
  (ADR-0072).
- **`nebula-telemetry` is gone** — merged into `nebula-metrics` as the
  single metrics path (ADR-0046). If you see `nebula-telemetry` referenced
  anywhere in the working tree, it is drift, not real.
- **`nebula-system` is gone** — the cross-platform host-probe crate was
  deleted (#668). There is no process-monitoring crate today.

## Folder Structure

Full workspace member list is in **`Cargo.toml [workspace.members]`**. The
canonical map of which crate sits in which layer is in **`CLAUDE.md` §
"Layered Dependency Map"**. Per-crate layout convention:

```
crates/<crate>/
├── Cargo.toml
├── README.md                 # human entry point
├── src/
│   ├── lib.rs                # crate root + re-exports
│   ├── error.rs              # thiserror types if non-trivial
│   └── ...                   # internal modules
├── tests/                    # integration tests
├── benches/                  # criterion benches if relevant
├── docs/                     # design docs (where present)
├── fuzz/                     # fuzz targets (excluded from workspace)
└── macros/                   # proc-macro sub-crate, if needed
```

Macros live in their own sub-crate (`crates/<crate>/macros/`) to keep
proc-macro build cost out of the runtime crate.

## Dependency Rules

### Allowed

- ✅ Any crate may depend on **Cross-cutting** crates (`error`, `log`,
  `eventbus`, `metrics`, `resilience`).
- ✅ **Core** depends on Cross-cutting only.
- ✅ **Business** depends on Core + Cross-cutting.
- ✅ **Exec** depends on Business + Core + Cross-cutting.
- ✅ **API / Public** depends on Exec + Business + Core + Cross-cutting.
- ✅ A crate's `macros/` sub-crate may depend on its parent contract crate
  only as a `dev-dependency` for tests.

### Forbidden (enforced by the `wrappers` allowlists on the `[bans].deny` entries in `deny.toml`)

- ❌ Cross-cutting crates may **not** depend on Core / Business / Exec / API.
- ❌ Core crates may **not** depend on Business / Exec / API.
- ❌ Business crates may **not** depend on Exec / API.
- ❌ Exec crates may **not** depend on API. **Exception (allowlisted):**
  `nebula-engine` may be wrapped by `apps/server` (`nebula-server`) and by
  `nebula-credential-runtime` (acyclic edge per ADR-0081); dev-only by
  `crates/api/tests/knife.rs`. The allowlist also reserves an entry for the
  planned `nebula-cli` binary, which does not exist in the workspace yet.
  See `deny.toml` for the full rationale comments.
- ❌ Sibling crates at the same layer may **not** import each other
  directly — cross-crate communication goes through `nebula-eventbus`
  (typed events) or through a shared lower-layer contract crate.
- ❌ `nebula-resource` is shared infra: only the explicit wrapper allowlist
  (`action`, `engine`, `plugin`, `sandbox`, `sdk`) may depend on it. API
  and Core must not.
- ❌ `nebula-credential-builtin` is a first-party scaffold: plugin authors
  depend on the contract crate `nebula-credential`, **not** on `-builtin`.
- ❌ Adding a new wrapper without a `reason` string in `deny.toml` is a CI
  failure.

A new cross-crate edge requires either an existing wrapper rule or an
explicit `deny.toml` change with a `reason` (and, for security-sensitive
paths, `CODEOWNERS` sign-off).

## Layer / Module Communication

- **Down the stack: direct typed calls.** API → Exec → Business → Core →
  Cross-cutting. Inputs are typed; errors are `thiserror` enums;
  observability spans + metrics are emitted at the call site.
- **Up the stack: `nebula-eventbus`.** Lower layers publish typed events;
  higher layers subscribe. No upward direct calls. The eventbus is stable
  for `CredentialEvent` and used by the engine; `ExecutionEvent` is still
  on raw `mpsc` (migration tracked separately).
- **Cross-cutting concerns (logging, metrics, errors) are imported, not
  wrapped.** Use `tracing` directly; do not invent crate-local façades.
- **Public extension surface = `nebula-sdk` + `nebula-plugin-sdk`.**
  Third-party integrators depend on these two crates only; the `wrappers`
  allowlists in `deny.toml` pin who is allowed to depend on each.
- **Composition roots.** Wiring concrete impls happens in `apps/server`
  (binary) or, for in-process integration tests, in `crates/api/tests/`.
  Library crates do not perform global wiring.

## Key Principles

1. **Crates are modules; layers are enforced at compile time.** A merge
   that widens a layer boundary either updates the relevant
   `[bans].deny[].wrappers` entry in `deny.toml` with a `reason` (and
   review) or fails CI. There is no soft "gentle reminder" path.
2. **Types over tests.** Workflow shape, action I/O, parameter schemas, and
   auth patterns are Rust types. If it compiles, the shape is valid. Tests
   verify behaviour, not type safety.
3. **Explicit over magic.** No global state, no service locators, no
   ambient config. Actions receive everything via `Context`. If a
   dependency is not in the function signature, it does not exist.
4. **Delete over deprecate (internals).** For internal architecture,
   replace the wrong API rather than adapt around it. No shims, no
   bridges, no `legacy_compat` flags. The public `nebula-sdk` and plugin
   contracts are the exception — they get a clear deprecation path because
   they are external contracts.
5. **Security by default.** Secrets are encrypted (AES-256-GCM with AAD
   binding), zeroized on `Drop`, redacted in `Debug`. The safe path is the
   only path.
6. **Observability is part of Definition of Done.** Every new state,
   error, or hot path ships with a typed error variant **and** a tracing
   span / event **and** an invariant check — not as a follow-up.
7. **ADRs are revisable.** Architecture decisions live as ADRs. If
   following one forces workarounds, **supersede** the ADR — do not patch
   around it.

## Code Examples

### Down-the-stack typed call (Action → Resource)

```rust
// crates/action/src/some_action.rs
use nebula_resource::ResourceHandle;
use nebula_error::ActionError;
use tracing::instrument;

#[instrument(skip(ctx, handle))]
pub async fn checkout(
    ctx: &ActionContext,
    handle: ResourceHandle<HttpPool>,
) -> Result<Receipt, ActionError> {
    let conn = handle.acquire().await?;          // typed error from -resource
    conn.post("/checkout", &ctx.payload).await   // bubbles up as ActionError
        .map_err(ActionError::from)
}
```

### Up-the-stack via `nebula-eventbus` (no direct upward import)

```rust
// crates/credential/src/store.rs (Business layer)
use nebula_eventbus::EventBus;

pub async fn rotate(bus: &EventBus, id: CredentialId) -> Result<(), CredentialError> {
    let prev = self.encrypted_record(id).await?;
    let next = rotate_keys(&prev)?;
    self.persist(&next).await?;
    bus.publish(CredentialEvent::Rotated { id, at: now() }).await; // higher layers subscribe
    Ok(())
}

// crates/engine/src/subscriber.rs (Exec layer) — subscribes, never imports back
let mut sub = bus.subscribe::<CredentialEvent>();
while let Some(evt) = sub.recv().await { /* react */ }
```

### Forbidden: direct upward import (would fail `cargo deny`)

```rust
// crates/credential/src/lib.rs ❌
use nebula_engine::ExecutionContext; // NO — Business depending on Exec
```

`cargo deny check bans` flags this and CI fails. Fix: invert via eventbus,
or move the shared type down to Core / Cross-cutting.

### Adding a new layer-crossing edge (the only legitimate path)

```toml
# deny.toml
[[bans.wrappers]]
crate = "nebula-storage"
wrappers = [
  "nebula-engine",
  "nebula-api",          # ← new edge
]
reason = "ADR-NNNN: api/<X> path needs direct storage access for Y"
```

No `reason` → CI rejects the diff.

## Anti-Patterns

- ❌ **Sibling-crate `use` at the same layer.** Even if `cargo build`
  succeeds via a transitive path, route the seam through `nebula-eventbus`
  or a shared lower-layer crate.
- ❌ **`Box<dyn Error>` / `anyhow::Error` in library APIs.** Use typed
  `thiserror` errors. `anyhow` is for binaries only.
- ❌ **`async-trait` on hot paths.** Prefer `#[async_fn_in_trait]` (Rust
  1.75+ stable) — verify against current 1.95+ idioms.
- ❌ **`Arc<Mutex<…>>` as the default for shared state.** Reach for
  `parking_lot::Mutex`, `arc-swap`, `dashmap`, or single-writer designs
  first.
- ❌ **Per-crate `examples/` directories.** Runnable examples live in the
  root-level `examples/` workspace member.
- ❌ **`unwrap()` / `expect()` / `panic!()` in library code.** Allowed in
  tests, `const`, and binaries per `clippy.toml`.
- ❌ **"Just one helper in the wrong crate."** Cross-crate placement is a
  boundary decision — restructure, do not normalise drift.
- ❌ **Wrapping `tracing` / `metrics` / `error` in a crate-local façade.**
  Cross-cutting crates are designed to be imported directly.
- ❌ **Patching around an ADR.** If the ADR forces workarounds, write a
  superseding ADR — do not accumulate compensating code.
- ❌ **`let _ = transition_node(...)` / silently ignoring `Result`.**
  Either handle the typed error or propagate it; engine state machines
  have caused bugs from swallowed transitions.
