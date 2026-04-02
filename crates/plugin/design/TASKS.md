# Tasks: nebula-plugin

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `PLG`

---

## Phase 1: Contract and Registry Baseline 🔄

**Goal**: Stable public API; registry contract tests; error taxonomy; compatibility policy.

- [x] PLG-T000 Fix `PluginMetadataBuilder::build()` and `PluginRegistry::get_by_name()` to normalize keys (spaces → underscores, uppercase → lowercase) — was causing 2 CI test failures
- [ ] PLG-T001 [P] Stabilize `Plugin`, `PluginMetadata`, `PluginComponents` structs — add stability annotations to API.md
- [ ] PLG-T002 [P] Stabilize `PluginRegistry`, `PluginType`, `PluginVersions`
- [ ] PLG-T003 Write contract test: register plugin → resolve by key → list actions/credentials in `tests/registry.rs`
- [ ] PLG-T004 [P] Write contract test: engine can consume PluginRegistry to list available actions
- [ ] PLG-T005 Define `PluginError` taxonomy in `src/error.rs` — RegistryConflict, NotFound, LoadError, ValidationError
- [ ] PLG-T006 Document compatibility policy in README — patch/minor/major change rules
- [ ] PLG-T007 Verify PluginRegistry is single source of truth — remove any duplicate resolution paths

**Checkpoint**: Registry resolves plugins and components correctly; engine integration test passes; no undocumented breaking changes.

---

## Phase 2: Dynamic Loading and Safety ⬜

**Goal**: `PluginLoader` via shared libraries; validated loading; no unsafe outside gated module.

- [ ] PLG-T008 Implement `PluginLoader` behind `dynamic-loading` feature flag in `src/loader.rs`
- [ ] PLG-T009 Implement plugin validation on load — check metadata, component count, ABI version
- [ ] PLG-T010 [P] Document safety constraints: FFI stability, symbol naming, Rust version compatibility
- [ ] PLG-T011 Implement unload behavior — cleanup on plugin removal
- [ ] PLG-T012 Add `#[cfg(feature = "dynamic-loading")]` gating; all `unsafe` code in single module
- [ ] PLG-T013 Write tests: load/fail/unload paths for dynamic plugin
- [ ] PLG-T014 Verify static and dynamic plugins use same registry interface

**Checkpoint**: Dynamic loading functional; no unsafe outside gated module; load/fail/unload tested.

---

## Phase 3: Discovery and Versioning ⬜

**Goal**: Path/config-based plugin discovery; version selection policy.

- [ ] PLG-T015 Implement discovery from directory path — scan for plugin manifests/shared libraries
- [ ] PLG-T016 [P] Implement discovery from config — load plugins listed in config file
- [ ] PLG-T017 Implement `PluginVersions` version selection — latest compatible, pinned
- [ ] PLG-T018 Integrate engine with populated registry — API or loader fills registry at startup
- [ ] PLG-T019 Write test: discovery finds plugins deterministically; conflicting keys error clearly

**Checkpoint**: Discovery deterministic; version selection policy documented and tested.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Authoring guide; migration path; optional manifest schema.

- [ ] PLG-T020 Write plugin authoring guide in `docs/crates/plugin/AUTHORING.md`
- [ ] PLG-T021 [P] Update MIGRATION.md — document migration for plugin format and registry API changes
- [ ] PLG-T022 [P] Define optional plugin manifest schema (TOML/JSON) for discovery and packaging
- [ ] PLG-T023 Write example first-party plugin demonstrating full flow

**Checkpoint**: External authors can ship a plugin; authoring guide and manifest schema published.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- Phase 1 depends on `nebula-action` Phase 2 (action component shape) and `nebula-credential` Phase 1
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-plugin --all-features`
- [ ] `cargo test -p nebula-plugin`
- [ ] `cargo clippy -p nebula-plugin -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-plugin`
