# Implementation Plan: nebula-plugin

**Crate**: `nebula-plugin` | **Path**: `crates/plugin` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The plugin crate manages plugin discovery, registration, and loading. Plugins bundle metadata and components (actions, credentials) that the engine uses via `PluginRegistry`. It provides static (compile-time) plugins now and will support dynamic loading via `dlopen`-based FFI in Phase 2. Current focus is Phase 1: stabilizing the registry contract and aligning with engine/action.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-action`, `nebula-credential`, `nebula-core`
**Feature Flags**: `dynamic-loading` (Phase 2)
**Testing**: `cargo test -p nebula-plugin`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Registry Baseline | ⬜ Planned | Stable API, contract tests, error taxonomy |
| Phase 2: Dynamic Loading and Safety | ⬜ Planned | `PluginLoader` via shared library, validation, safety docs |
| Phase 3: Discovery and Versioning | ⬜ Planned | Path/config-based discovery, version selection |
| Phase 4: Ecosystem and DX | ⬜ Planned | Authoring guide, migration path, manifest schema |

## Phase Details

### Phase 1: Contract and Registry Baseline

**Goal**: Stable public API; contract tests for registry resolution; error taxonomy; compatibility policy.

**Deliverables**:
- `Plugin`, `PluginMetadata`, `PluginComponents`, `PluginRegistry`, `PluginType`/`PluginVersions` stable
- Contract tests: register plugin, resolve by key, list actions/credentials; engine can consume registry
- `PluginError` taxonomy and compatibility policy (patch/minor/major)

**Exit Criteria**:
- Registry is single source of truth for plugin key → components
- No undocumented breaking changes to Plugin or PluginComponents

### Phase 2: Dynamic Loading and Safety

**Goal**: `PluginLoader` from shared libraries; validated loading; safety documented.

**Deliverables**:
- `dynamic-loading` feature: `PluginLoader` loads from `.so`/`.dll`
- Validation of loaded plugin; clear failure semantics and unload behavior
- Optional sandbox/capability constraints for loaded plugins

**Exit Criteria**:
- Static and dynamic plugins both usable via same registry interface
- No unsafe outside gated module

**Risks**:
- ABI instability across Rust versions; unsound FFI

### Phase 3: Discovery and Versioning

**Goal**: Plugin discovery from paths/config; version selection policy.

**Deliverables**:
- Discovery from paths or config (scan directory, load by manifest)
- `PluginVersions` and version selection policy (latest compatible, pinned)
- Integration with engine: registry populated by API or loader

**Exit Criteria**:
- Discovery deterministic and documented; version policy clear

### Phase 4: Ecosystem and DX

**Goal**: Authoring guide; migration path; optional manifest schema.

**Deliverables**:
- Authoring guide: how to implement `Plugin` and `PluginComponents`
- `MIGRATION.md` for plugin format or registry API changes
- Optional plugin manifest schema for discovery and packaging

**Exit Criteria**:
- Plugin authors can ship a plugin that engine loads and runs; contract stable in minor

## Inter-Crate Dependencies

- **Depends on**: `nebula-action` (action component), `nebula-credential` (credential component), `nebula-core`
- **Depended by**: `nebula-engine` (consumes registry), `nebula-api` (plugin CRUD), `nebula-sdk`

## Verification

- [ ] `cargo check -p nebula-plugin`
- [ ] `cargo test -p nebula-plugin`
- [ ] `cargo clippy -p nebula-plugin -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-plugin`
