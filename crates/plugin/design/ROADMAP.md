# Roadmap

Phased path to production-ready plugin discovery, registration, and optional dynamic loading. Aligned with platform role: plugins bundle metadata and components (actions, credentials); engine uses PluginRegistry.

## Phase 1: Contract and Registry Baseline

- **Deliverables:**
  - Stable public API: `Plugin`, `PluginMetadata`, `PluginComponents`, `PluginRegistry`, `PluginType`/`PluginVersions`; document in API.md and align with engine/action contract.
  - Contract tests: register plugin, resolve by key, list actions/credentials; engine can consume registry.
  - Error taxonomy (`PluginError`) and compatibility policy (patch/minor/major).
- **Risks:**
  - Engine or action crate expecting different component shape; registry contract drift.
- **Exit criteria:**
  - Registry is the single source of truth for plugin key → components; engine depends on it.
  - No undocumented breaking changes to Plugin or PluginComponents.

## Phase 2: Dynamic Loading and Safety

- **Deliverables:**
  - `dynamic-loading` feature: `PluginLoader` loads from shared libraries; documented safety and platform constraints (FFI, symbol stability).
  - Validation of loaded plugin (metadata, component count); clear failure semantics and unload behavior.
  - Optional: sandbox or capability constraints for loaded plugins (if adopted).
- **Risks:**
  - ABI instability across Rust versions; unsound FFI if contract is wrong.
- **Exit criteria:**
  - Dynamic loading documented; load/fail/unload paths tested; no unsafe outside gated module.
  - Static (compile-time) and dynamic plugins both usable via same registry interface.

## Phase 3: Discovery and Versioning

- **Deliverables:**
  - Plugin discovery from paths or config (e.g. scan directory, load by manifest); optional discovery API.
  - Versioning: `PluginVersions` and version selection policy (latest compatible, pinned); document in API.md.
  - Integration with nebula-engine: engine receives registry populated by API or loader.
- **Risks:**
  - Discovery order or version resolution ambiguous; conflicting plugin keys.
- **Exit criteria:**
  - Discovery (if implemented) is deterministic and documented; version policy clear.
  - Engine and API can rely on registry for "list plugins" and "get action by id".

## Phase 4: Ecosystem and DX

- **Deliverables:**
  - Authoring guide: how to implement `Plugin` and `PluginComponents`; how to register with engine.
  - Migration path for plugin format or registry API changes (MIGRATION.md).
  - Optional: plugin manifest schema (file format) for discovery and packaging.
- **Risks:**
  - First-party vs third-party plugin lifecycle and compatibility fragmentation.
- **Exit criteria:**
  - Plugin authors can ship a plugin that engine loads and runs; contract stable in minor.

## Metrics of Readiness

- **Correctness:** Registry resolves plugins and components correctly; engine integration tests pass.
- **Stability:** Plugin and registry API stable in patch/minor; breaking = major + MIGRATION.
- **Operability:** Static and (if enabled) dynamic plugins observable; errors actionable.
