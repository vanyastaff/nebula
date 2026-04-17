# Rename credential types to match canon §3.5 naming

**Date:** 2026-04-17
**Authority:** Subordinate to `docs/PRODUCT_CANON.md` §3.5 (integration model).
**Status:** draft — awaiting approval before implementation.

---

## Motivation

Three `*Metadata` types in the Nebula workspace mean different things:

| Struct | Role | What it holds |
|---|---|---|
| `nebula_action::ActionMetadata` | Integration catalog | key, name, version, inputs/outputs, parameters |
| `nebula_resource::ResourceMetadata` | Integration catalog | key, name, description, tags |
| `nebula_credential::CredentialMetadata` | **Runtime operational state** | created_at, last_accessed, version counter, expires_at, rotation_policy |

Canon §3.5 explicitly names the integration-catalog pattern as "**`*Metadata` + `ParameterCollection`**". Action and Resource follow this. Credential breaks it:

- Catalog type is actually called `CredentialDescription` (separate file, `description.rs`)
- The `CredentialMetadata` name is taken by a runtime-state struct with unrelated fields

This is **information leakage through naming**: a reader encountering `CredentialMetadata` by analogy with `ActionMetadata` will expect catalog data and get runtime state instead. Worse, the canon docs say `*Metadata = catalog` while the credential crate has two types with the suffix, only one of which matches canon.

The user has carried this rename on the mental backlog for a while ("я как бы долго собирался это сделать"). This spec executes it.

## Decision

Two renames in `nebula-credential`:

1. **`CredentialMetadata` → `CredentialRecord`** (runtime state — DDIA-standard "record" terminology for a data row about an entity at this moment in time; no name collision with the existing `CredentialState` trait)
2. **`CredentialDescription` → `CredentialMetadata`** (integration catalog — matches canon §3.5 and siblings)

No shim, no re-export alias. Per user standing memory ("never propose adapters/bridges/shims"), the old names go away entirely.

## Why `CredentialRecord` and not `CredentialState` / `CredentialEntry` / `CredentialSnapshot`

- **`CredentialState`** — collides with the `CredentialState` trait already defined in the credential crate (`type State: CredentialState` on the `Credential` trait).
- **`CredentialEntry`** — viable; evokes HashMap entry pattern but slightly vague for a mutable runtime row.
- **`CredentialSnapshot`** — implies point-in-time immutable copy, but the struct is mutated as rotations advance / access timestamps update. Misleading.
- **`CredentialRecord`** — DDIA ch.2 terminology for a persisted entity row. Unambiguous, no collision, matches storage patterns in the workspace (`CredentialRow`, `CredentialAuditRow`).

If the user prefers `CredentialEntry` or another name, swap before execution.

## Scope — files touched

### `crates/credential`
- `src/metadata.rs` → **rename file to** `src/record.rs` (mirrors new struct name); struct + impls + tests renamed.
- `src/description.rs` → **rename file to** `src/metadata.rs`; struct + impls + builder renamed (`CredentialDescriptionBuilder` → `CredentialMetadataBuilder`).
- `src/lib.rs` — module declarations + `pub use` lines updated.
- Internal consumers: `src/accessor.rs`, `src/any.rs`, `src/credential.rs`, `src/credentials/{api_key,basic_auth,oauth2}.rs`, `src/resolver.rs`, `src/rotation/{mod,validation}.rs`, `src/snapshot.rs` — mechanical rename.
- `macros/src/credential.rs` — derive macro emits `CredentialDescription::builder()` calls; update to `CredentialMetadata::builder()`.
- `examples/credential_description.rs` → **rename file to** `examples/credential_metadata.rs`; example code updated.
- `tests/units/pending_lifecycle_tests.rs`, `thundering_herd_tests.rs`, `resolve_snapshot_tests.rs` — rename.

### Workspace consumers
- `crates/action/src/context.rs`, `crates/action/src/testing.rs` — uses `CredentialMetadata` (runtime state) → rename to `CredentialRecord`.
- `crates/engine/src/engine.rs` — uses `CredentialMetadata` (runtime state) → rename to `CredentialRecord`.
- `crates/sdk/src/prelude.rs` — re-exports; update both lines.
- `apps/desktop/src-tauri/src/commands/credentials.rs`, `src/types.rs` — uses `CredentialMetadata` (runtime state) → rename to `CredentialRecord`. Tauri bindings need rebuilding; frontend TypeScript bindings (if generated) will need update. **Note:** if frontend is out of sync, that's a desktop team handoff.

### Not touched
- `crates/storage/src/rows/credential.rs` — already has own naming (`CredentialRow`, `PendingCredentialRow`, `CredentialAuditRow`), distinct from credential crate's domain types. No collision, no change.

## JSON / wire format impact

`CredentialMetadata` (old runtime-state struct) serialized as its fields (created_at, version counter, etc.). After rename to `CredentialRecord`, the struct's **field shape is unchanged** — only the Rust type name changes. If it was serialized with `#[serde(tag = "...")]` or similar name-sensitive tagging, check. Spot-check `serde` attributes in `metadata.rs` before the rename.

`CredentialDescription` becomes `CredentialMetadata` — same story: field shape unchanged. Any serialized description data (example: plugin descriptor JSON) will deserialize into the renamed type unchanged.

**Breaking change surface**: Rust API consumers only. No wire format break expected. If serde uses the struct name somewhere (e.g. `#[serde(rename = "...")]` that mirrored the type name explicitly), fix locally.

## Verification

- `cargo check --workspace` — clean
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo nextest run --workspace` — all tests pass (expect ~3420)
- `cargo test --workspace --doc` — clean
- `cargo +nightly fmt --all` — applied
- `rg "CredentialDescription" crates/ apps/ --glob "*.rs"` — zero production hits (spec and docs allowed)
- `rg "CredentialMetadata" crates/credential/src/description.rs` → file no longer exists
- `rg "CredentialMetadata" crates/credential/src/record.rs` → file no longer exists (was metadata.rs, renamed to record.rs)
- Knife `knife_scenario_end_to_end` — still passes
- Credential tests — still pass (units and integration)

## Canon alignment

- **§3.5** — credential now matches the `*Metadata + ParameterCollection` pattern explicitly.
- **§14** — no shim, no re-export alias (per user memory).
- **§17 DoD** — tests green, README/docs drift check in credential crate.

## Risks

| Risk | Mitigation |
|---|---|
| Tauri frontend generates TS bindings from Rust types with name-sensitive codegen | Check `apps/desktop/src-tauri/` for `ts-rs` or similar binding generators; regenerate if needed. If out-of-scope (desktop team owns), note in commit body as handoff |
| `CredentialDescriptionBuilder` is heavily used by credential macro codegen | Mechanical rename; macro emits the new name after the rename. Test derive expansion after: `cargo expand -p nebula-credential --test derive_tests` if available, or just run credential tests |
| Serde tags may reference old names | Grep `#[serde(rename` and `#[serde(tag` in credential crate before rename |
| Storage migration already has `CredentialRow` naming — confuses reviewers who think our `CredentialRecord` is the storage row | Add a short paragraph in `crates/credential/src/record.rs` crate docs clarifying the distinction: `CredentialRecord` is domain runtime state; `nebula_storage::rows::CredentialRow` is the persisted row. Different concerns, intentionally distinct types |

## Out of scope

- Refactoring the 12 error enums in `nebula-credential` (Ousterhout classitis candidate) — separate Q3 discussion item.
- Introducing a shared `IntegrationMetadata` trait (variant C from brainstorming) — deferred; spec 21 schema migration may supersede.
- Updating canon §3.5 docs to explicitly disambiguate "catalog metadata" vs "runtime record" — maybe later; current canon language is consistent with this rename.
- **Evaluating whether `CredentialRecord` should live in `nebula-credential` at all** — this rename is a first step. Per DDIA concern separation, runtime operational state (created_at, last_accessed, version counter, expires_at, rotation_policy) is arguably a **storage concern**, not an integration concern. `nebula-storage` already has `CredentialRow`, `PendingCredentialRow`, `CredentialAuditRow`. A future refactor may:
  - (a) delete `CredentialRecord` entirely and route through `storage::CredentialRow` directly, OR
  - (b) move `CredentialRecord` into `nebula-storage` as domain row adjacent to `CredentialRow`, OR
  - (c) keep `CredentialRecord` in credential crate but demote it from public API to `pub(crate)` since consumers go through storage anyway.

  **Follow-up task added to backlog:** "Evaluate `CredentialRecord` placement — storage concern or credential concern?" Decision deferred until more consumers are mapped (specifically: does the Tauri desktop app read `CredentialRecord` directly, or only through storage?).

## Implementation approach

Single atomic PR on main. Per CLAUDE.md "delete over deprecate" — no shim.

Agent instructions:
1. Read this spec fully.
2. Execute file renames first (git mv for history preservation).
3. Rename structs and builders next, then update all consumers via find-and-replace.
4. Run verification cycle.
5. Commit with clear message citing spec + canon §3.5.

Branch: `main`. Single commit.
