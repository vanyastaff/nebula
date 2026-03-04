# nebula-plugin Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-03

---

## Platform Role

Workflow nodes are grouped into **plugins** — user-visible, versionable units (e.g. "Slack", "HTTP Request", "PostgreSQL"). The engine and API need a single registry to resolve plugin key → metadata and components (actions, credential requirements). The plugin crate owns the plugin trait, metadata, component declaration, and registry; optional dynamic loading from shared libraries is feature-gated.

**nebula-plugin is the plugin packaging and registry layer for the Nebula platform.**

It answers: *What is a plugin (metadata + components), how is it registered, and how does the engine resolve actions by plugin/action key?*

```
Plugin (impl trait) → metadata + register(PluginComponents)
    ↓
PluginRegistry: PluginKey → PluginType (single or versioned)
    ↓
Engine / API: resolve action, list plugins, optional load from path (dynamic-loading)
```

This is the plugin contract: plugin is the packaging unit; registry is the single source of truth for key → components; engine depends on registry, not on loading mechanism.

---

## User Stories

### Story 1 — Engine Resolves Action by Key (P1)

Engine has an action_id (or plugin key + action); it looks up the plugin in the registry and gets the action implementation or metadata. Registry is populated at startup (static plugins) or via loader (dynamic).

**Acceptance:**
- PluginRegistry maps PluginKey → PluginType; engine can resolve plugin and its components.
- Actions are registered in PluginComponents; engine or action registry uses plugin registry to resolve by key.
- Register fails with clear error if key already exists (PluginError::AlreadyExists).

### Story 2 — API or CLI Lists Plugins (P1)

API or CLI needs to list installed plugins (key, name, version) for UI or discovery. Registry provides list and get by key.

**Acceptance:**
- Registry supports list (keys or metadata) and get(key); no execution in plugin crate.
- PluginMetadata is serializable for API response.

### Story 3 — Optional Load from Path (P2)

With `dynamic-loading` feature, a loader can load plugins from shared libraries (.so/.dll/.dylib); loaded plugins are registered in the same PluginRegistry. Safety and ABI constraints are documented.

**Acceptance:**
- PluginLoader (feature-gated) loads from path; register plugin type into registry.
- Load failure returns clear error; no unsafe outside gated module.
- Static (compile-time) and dynamic plugins coexist in same registry.

---

## Core Principles

### I. Plugin Is Packaging, Not Execution

**Plugin crate owns metadata and component declaration; it does not run actions or resolve credentials at runtime.**

**Rationale:** Execution is engine/runtime; credential resolution is credential crate. Plugin is the declaration layer.

**Rules:**
- Plugin::register() only declares refs (actions, credentials) into PluginComponents; no I/O or execution.
- Registry is in-memory map; thread-safety is caller's responsibility (e.g. wrap in RwLock).

### II. Registry Is Single Source of Truth

**PluginKey → PluginType is the contract for engine and API; registration is explicit (register) or via loader.**

**Rationale:** Engine and API must not diverge on how they discover plugins; one registry, one key space.

**Rules:**
- Duplicate key registration fails (AlreadyExists).
- Optional: versioned plugins (PluginVersions) for multiple versions per key; resolution policy documented.

### III. Dynamic Loading Is Optional and Safe-Gated

**Unsafe FFI is confined to dynamic-loading module; default build has no unsafe.**

**Rationale:** Most users use static plugins; dynamic loading is optional and must not compromise default safety.

**Rules:**
- `dynamic-loading` feature enables PluginLoader; loader uses allow(unsafe_code) only in that module.
- Document ABI and symbol stability constraints for dynamic plugins.

---

## Production Vision

In production, the registry is populated at startup (static plugins from engine/API composition) or by a loader (dynamic-loading). Engine resolves action_id via plugin registry and action registry; API lists plugins for UI. From the archives: `archive-crates-architecture.md` (nebula-registry) described ActionRegistry and PluginManager with load_plugin; current crate separates Plugin (trait + metadata + components), PluginRegistry (in-memory map), and optional PluginLoader. Production vision: registry is the contract; static and dynamic plugins both register into same registry.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Contract tests with engine | High | Engine resolves action via registry; test in CI |
| Discovery from path/config | Medium | Optional: scan directory, load by manifest |
| Version resolution policy | Low | PluginVersions; document latest vs pinned |

---

## Key Decisions

### D-001: Plugin Trait Is Object-Safe

**Decision:** Plugin is object-safe (metadata, register) so plugins can be stored as Arc<dyn Plugin>.

**Rationale:** Registry and loader need to hold heterogeneous plugin types.

**Rejected:** Only static plugin types — would block dynamic loading and generic registry.

### D-002: Registry Does Not Own Thread-Safety

**Decision:** PluginRegistry is not thread-safe by default; caller wraps in RwLock if shared.

**Rationale:** Keeps crate simple; engine/API typically hold registry behind RwLock or in single-threaded context.

**Rejected:** Built-in Mutex/RwLock — would force sync dependency and may not match caller's locking strategy.

### D-003: Dynamic Loading Behind Feature

**Decision:** PluginLoader and FFI are behind `dynamic-loading` feature; default build is no_std-friendly where possible and no unsafe.

**Rationale:** Security and portability; most users use static plugins.

**Rejected:** Always-on dynamic loading — would pull in libloading and unsafe on all builds.

---

## Open Proposals

### P-001: Discovery API

**Problem:** Operators want to load all plugins from a directory or from config.

**Proposal:** Optional discovery API (scan path, read manifest, load and register); document in ROADMAP Phase 3.

**Impact:** New public API; may require async or blocking file I/O.

---

## Non-Negotiables

1. **Plugin is packaging only** — metadata and component declaration; no execution or credential resolution in crate.
2. **Registry is the contract** — key → PluginType; duplicate key fails.
3. **Unsafe only in gated loader** — default build no unsafe; dynamic-loading feature documents ABI/symbol constraints.
4. **Breaking registry or Plugin trait = major + MIGRATION.md** — engine and API depend on it.

---

## Governance

- **PATCH:** Bug fixes, docs. No change to Plugin trait or registry contract.
- **MINOR:** Additive (new optional API, new error variant). No removal.
- **MAJOR:** Breaking Plugin or registry contract. Requires MIGRATION.md.
