# nebula-credential-testutil — Claude Code orientation
> Agent quick-map for `crates/credential-testutil/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** In-memory test doubles for the two `nebula-credential` storage ports (`CredentialStore`, `PendingStateStore`) so downstream tests exercise the contract without Vault/Postgres/an OAuth IdP.
**Layer:** Business (alongside `nebula-credential`) — depends only downward. `publish = false`, internal test-only.

## Commands
- `cargo check -p nebula-credential-testutil`
- `cargo nextest run -p nebula-credential-testutil`  ·  doctests: `cargo test -p nebula-credential-testutil --doc`

## Key files
- `src/lib.rs` — public surface: `InMemoryStore`, `InMemoryPendingStore`, `in_memory_pair()`; re-exports `store_memory` + `pending_store_memory`.
- `src/store_memory.rs` — `InMemoryStore` impl of `nebula_credential::store::CredentialStore` (`Arc<RwLock<HashMap>>`).
- `src/pending_store_memory.rs` — `InMemoryPendingStore` impl of `nebula_credential::pending_store::PendingStateStore`.

## Conventions & never-do
- These are **test shims**, behaviour-identical to the canonical `nebula_storage::credential::{InMemoryStore, InMemoryPendingStore}`. Production composition roots/examples/docs MUST import the `nebula-storage` variants, never this crate.
- Do NOT consume from production code — crate is `publish = false`; consumer set is locked by the `deny.toml` `[bans].deny` `wrappers` allowlist (`nebula-credential-runtime` via its `test-util` feature + dev-deps; `nebula-tenancy` dev-deps).
- Do NOT cargo-cult the `tokio::RwLock<HashMap<...>>` pattern into production — the guard never crosses `.await` here (perf-irrelevant shim); doing so elsewhere is the issue-#587 perf cost.
- Keep scope tight: no snapshot fixtures, redaction asserts, scheme builders, or first-party credential catalog (that's `nebula-credential-builtin`); cloning a store shares one backing map, all data drops with the last clone.
- Library code uses typed `thiserror`/`NebulaError` (`StoreError`/`PendingStoreError`); no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · `docs/MATURITY.md` (M12.2 extraction record, 2026-05-20) · ADR-0081 (M6 resource/credential integration)
