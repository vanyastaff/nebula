# nebula-storage-port — Agent orientation
> Agent quick-map for `crates/storage-port/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Pure storage contract — object-safe `#[async_trait]` repository traits, port-local DTO rows, plain-data `Scope`, `StorageError`, and the `TransitionBatch` atomic unit-of-work. No backend code.
**Layer:** Core — depends only downward (root AGENTS.md -> Layered Dependency Map). Deps: `nebula-core`, async-trait, serde, chrono, uuid. **No sqlx.**

## Commands
- `cargo check -p nebula-storage-port`
- `cargo nextest run -p nebula-storage-port`  ·  doctests: none (`doctest = false` in Cargo.toml `[lib]`)

## Key files
- `src/lib.rs` — crate root; re-exports `Scope`, `StorageError`, `FencingToken`, `TransitionBatch{,Builder,Outcome}`
- `src/batch.rs` — `TransitionBatch`: private fields, builder-only construction; `commit` writes state+outbox+journal in one CAS+fencing-gated transaction
- `src/store/mod.rs` — ISP-segregated object-safe role traits, including `CredentialPersistence`
- `src/dto/` — private-field lifecycle DTOs, typed `CredentialSelector`, bounded `CredentialVersion`, and structural live/tombstoned records
- `src/scope.rs` — plain-data `Scope { workspace_id, org_id }`; `src/ids.rs` — re-exported core ULIDs + lease `FencingToken`

## Conventions & never-do
- This crate declares *what* storage does; **never implement a backend here** (adapters live in `nebula-storage`). `nebula-tenancy` enforces policy for the general Scope-taking stores; credential persistence is owner-bound directly and intentionally has no tenancy decorator.
- DTOs never depend on higher-tier domain types. Opaque payloads use `serde_json::Value`/bytes, and credential rows/selectors remain port-local so this crate never imports `nebula-credential`.
- `Scope` is a value type with **no policy**; resolving it from a principal and general cross-tenant denial belong to `nebula-tenancy`, not here. `CredentialOwner`/`CredentialSelector` are also data, not actor authority; their public technical constructors must not be exposed through HTTP or `nebula-sdk`.
- Credential persistence exposes only explicit `create`, version-fenced `replace`, and version-fenced `tombstone`; never restore generic overwrite or physical delete.
- Every repository trait stays `#[async_trait]` + `dyn`-compatible (consumed as `Arc<dyn …>`); keep `TransitionBatch` fields private and builder-only so a transition can't skip scope/CAS/fencing.
- Library code uses typed `thiserror`/`StorageError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · ADR-0072 (port/adapter/tenancy contract) · ADR-0041 (RefreshClaimStore shape)
