# Replace custom `InterfaceVersion` with `semver::Version`

**Date:** 2026-04-17
**Authority:** Subordinate to `docs/PRODUCT_CANON.md`. Closes canon §12.1 layer violation (`workflow → action`) as a side-effect.
**Status:** draft — awaiting user approval before implementation

---

## Motivation

`nebula-action::InterfaceVersion` is a 30-line custom struct that pairs a `major` + `minor` u32 with a hand-rolled `is_compatible_with` check. It's used by 6 crates (action, workflow, engine, runtime, sandbox, plugin) to version action interfaces and support versioned dispatch (`runtime.execute_action_versioned`).

Three problems with the current shape:

1. **Reinvents a standard.** The Rust ecosystem has `semver` (maintained by rust-lang, used by Cargo itself) that covers major.minor.patch + pre-release + build metadata, parsing, and robust compat semantics (`^1.0`, `~1.2`, ranges). Our type has only the first 40% of that and hand-rolled compat rules.
2. **Layer violation.** `crates/workflow` currently depends on `crates/action` solely to import `InterfaceVersion`, which breaks canon §12.1 one-way layering (Core → Business). Moving the type to `nebula-core` would fix this but still leaves a bespoke type.
3. **No upgrade path.** If someone wants `^1.0` range pinning later, we'd have to rebuild what `semver::VersionReq` already does.

Replacing with `semver::Version` gives us the standard type, closes the layer violation (external dep, not cross-crate), and leaves `VersionReq` available as a later upgrade when workflow pinning semantics need it.

## Decision

- **`ActionMetadata.version: semver::Version`** — exact version the action publishes.
- **`NodeDefinition.interface_version: Option<semver::Version>`** — exact pin, matching current engine semantics (`runtime.execute_action_versioned` does exact lookup today). `VersionReq` is a future upgrade, not in this change.
- **Delete `nebula_action::InterfaceVersion`** — the custom type goes away entirely. No re-export shim (per user standing memory — no shims).

## Scope — files touched

| File | Change |
|---|---|
| `Cargo.toml` (root workspace) | Add `semver = { version = "1", features = ["serde"] }` to `[workspace.dependencies]` |
| `crates/action/Cargo.toml` | Add `semver = { workspace = true }` |
| `crates/action/src/metadata.rs` | Delete `InterfaceVersion` struct, its `impl`, `Display`, tests. Change `ActionMetadata.version` from `InterfaceVersion` → `semver::Version`. Update `with_version(major, minor)` helper to build `semver::Version::new(major, minor, 0)` or take a `&str` and parse. |
| `crates/action/src/lib.rs` | Remove `InterfaceVersion` from `pub use` |
| `crates/action/macros/src/action_attrs.rs` | Version literal: parse via `semver::Version::parse(&version_str)` at macro expansion time. Reject invalid versions with a compile error (`compile_error!`). |
| `crates/workflow/Cargo.toml` | Remove `nebula-action` dep (closes §12.1 layer violation). Add `semver = { workspace = true }` |
| `crates/workflow/src/node.rs` | `use semver::Version` (replace `nebula_action::InterfaceVersion`). Field: `pub interface_version: Option<Version>` |
| `crates/workflow/src/sdk.rs` (if it has `with_interface_version`) | Update helper signature |
| `crates/engine/src/engine.rs` | Update imports in tests module (`use semver::Version;` or inline). `InterfaceVersion::new(1, 0)` → `Version::new(1, 0, 0)`. Core engine dispatch path continues to read `node_def.interface_version` unchanged (type is now `Option<Version>`). |
| `crates/runtime/` | If any `InterfaceVersion` reference, update. Check `execute_action_versioned` signature — likely already generic enough |
| `crates/sandbox/src/discovery.rs` | `parse_interface_version(&str) -> Option<(u32, u32)>` helper → use `semver::Version::parse(&str).ok()` directly. Downstream consumers need the whole `Version`, not a tuple |
| `crates/plugin/src/descriptor.rs`, `crates/plugin/src/plugin.rs` | Update imports + doc examples |
| `crates/sdk/` (re-exports), `apps/cli/` (doc examples) | Follow-through updates |

## JSON schema impact

**Breaking change** for persisted workflow JSON and any serialized `ActionMetadata`:

- Old: `"interface_version": {"major": 1, "minor": 0}` (nested object from `InterfaceVersion` serde)
- New: `"interface_version": "1.0.0"` (string, from `semver::Version`'s default serde impl)

Canon §5 covers this: we're alpha, breaking internal serialization is acceptable if called out. Migration note in commit body + release notes.

**Workflow JSONs created before this PR** will fail to deserialize. Two options:

1. Accept the break (alpha stage) — simplest
2. Custom `#[serde(with = "legacy_version")]` helper that accepts both `{major, minor}` and `"X.Y.Z"` — complexity, adapter-pattern-adjacent

Recommendation: **Option 1 (accept break)**. Canon §5 alpha scope + simplicity. If this becomes operator pain, add conversion tool in a follow-up (one-shot migration, not runtime accommodation).

## Semantics preservation

- Exact version matching remains (current engine dispatch behavior)
- `NodeDefinition.interface_version = Some(Version::new(1, 0, 0))` + action publishes `Version::new(1, 0, 0)` → dispatches that handler
- No version pinned (None) → falls back to latest registered (current behavior)
- `VersionReq` ("any 1.x") is a future-work item; filed as follow-up task after this lands

## Verification

- `cargo check --workspace` — zero errors, zero new warnings
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo nextest run --workspace` — all tests pass
- `cargo test --workspace --doc` — clean
- `cargo deny check` — semver is MIT/Apache-2.0, zero transitive deps problem (verify)
- `rg "nebula_action::InterfaceVersion" crates/` — zero production hits (historical in spec/docs allowed)
- `rg "InterfaceVersion" crates/ --include="*.rs"` — zero production hits
- `rg "^nebula-action" crates/workflow/Cargo.toml` — zero (dep removed)
- Knife §13 `knife_scenario_end_to_end` — still passes
- Versioned action dispatch integration test in `engine` — passes against new type

## Canon alignment

- **§12.1** one-way layering — closed (workflow no longer depends on action)
- **§14** "implement end-to-end" — version dispatch remains wired; type is standard-based
- **§5** alpha breaking — internal serialization change, documented
- **§17 DoD** — tests green, migration note in commit, no README drift

## Risks

| Risk | Mitigation |
|---|---|
| semver `Version::serde` format disagrees with our expectations | Verify: semver 1.x uses string form by default with `serde` feature. Unit test serde round-trip |
| `semver::Version::parse("1.0")` rejects non-patch strings | Macro must accept `"1.0"` but store `Version::new(1, 0, 0)`. Use `Version::new` at macro expansion, not `parse` on user-given string |
| Tests across engine/runtime build `InterfaceVersion::new(1, 0)` | Mechanical replace: `Version::new(1, 0, 0)` |
| Plugin authors had `#[action(version = "1.0")]` using the 2-part syntax | Macro should accept `"X.Y"` (parse and set patch=0), or `"X.Y.Z"` (strict). Document both |
| Workflow JSON v1 files break | Accept per canon §5 (alpha); release note |

## Out of scope

- `VersionReq` for flexible pinning (`^1.0`) — future follow-up
- Migration tooling for old workflow JSONs — future if operators hit it
- Updating `plugin-sdk` wire protocol types — separate concern (plugin version vs action version)

## Follow-ups after landing

- Task: "Upgrade `NodeDefinition.interface_version` to `Option<semver::VersionReq>` for flexible pinning" — enables `^1.0` / `~1.2` notation in workflow JSON

## Implementation approach

Single atomic PR (per canon §14 "delete over deprecate", user memory "no shims").

Agent should:
1. Read this spec
2. Execute changes top-down
3. Run verification commands
4. Commit with `refactor(workspace): replace custom InterfaceVersion with semver::Version`
5. Report STATUS: DONE with commit SHA + file list + test summary
