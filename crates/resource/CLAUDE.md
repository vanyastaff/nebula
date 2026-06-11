# nebula-resource — Claude Code orientation
> Agent quick-map for `crates/resource/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Engine-owned resource lifecycle (acquire / health-check / hot-reload / scope-bounded release) for pool & SDK-client integrations, handed to actions as a drop-releasing `ResourceGuard`.
**Layer:** Business — depends only downward (root CLAUDE.md → Layered Dependency Map).

## Commands
- `cargo check -p nebula-resource`
- `cargo nextest run -p nebula-resource`  ·  doctests: `cargo test -p nebula-resource --doc`
- `cargo test -p nebula-resource --features test-util` — the rotation integration tests need `nebula-credential/test-util` (dev-only; never widen scope)
- Derive crate: `cargo check -p nebula-resource-macros` (companion in `macros/`); examples in root `nebula-examples` (`--example resource_*`)

## Key files
- `src/lib.rs` — crate facade + re-exports; `cell::Cell` deliberately NOT re-exported (use `SlotCell`)
- `src/resource.rs` — `Resource` trait (4 assoc types, slot-rotation hooks), `ResourceConfig`, `ResourceMetadata`
- `src/slot.rs` / `src/cell.rs` — `SlotCell` (public, generation-stamped) vs internal epoch-blind `cell::Cell`
- `src/registry.rs` — type-erased registry, scope-aware lookup, `(key, scope)` dedup
- `src/manager/` — `Manager::register(RegistrationSpec)` funnel, acquire dispatch, shutdown/drain
- `src/topology/` + `src/runtime/` — `Pooled` / `Resident` traits and their runtimes
- `src/release_queue.rs` — `ReleaseQueue` best-effort async drain (canon §11.4); `src/recovery/` — thundering-herd `RecoveryGate`

## Conventions & never-do
- Credentials are declared as `#[credential(key="…")] field: SlotCell<CredentialGuard<C>>`; read via derive-emitted `self.<field>_slot()` (`Option<Arc<…>>`, handle `None`/unbound) — never off the raw cell. No singular `Resource::Credential`; `NoCredential` is gone.
- This crate is NOT a connection driver, retry pipeline, secret holder, or expression evaluator — it owns the lifecycle wrapper only (see Non-goals).
- Async release is best-effort on crash; never assume "release ran" without an explicit checkpoint (canon §11.4).
- `#![forbid(unsafe_code)]` + `#![warn(missing_docs)]` are active; lifecycle work emits a `ResourceEvent` variant (observability is DoD).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, migration recipe (pre-v4 → v4), topology & shared-resource reference
- `docs/topology-reference.md` — topology selection guidance; canon invariants L2-§11.4 / §13.3
