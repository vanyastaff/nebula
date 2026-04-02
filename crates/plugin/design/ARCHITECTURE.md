# Architecture

## Problem Statement

- **Business problem:** Plugins (Slack, HTTP, etc.) must be discoverable and resolvable by engine and API; one registry, one key space, optional loading from path or shared libs.
- **Technical problem:** Plugin trait, metadata, component declaration, and registry must stay separate from action execution and credential resolution; optional FFI for dynamic loading must be gated.

## Current Architecture

- **Module map:** plugin (trait), metadata, components, plugin_type, versions, registry, error; loader (feature dynamic-loading). PluginRegistry is in-memory HashMap<PluginKey, Arc<PluginType>>.
- **Data/control flow:** Plugin impl registers metadata and components; registry.register(plugin_type); engine/API get by key or list. Loader (if enabled) loads from path and registers.
- **Known bottlenecks:** No discovery API yet; version resolution (PluginVersions) policy to document.

## Target Architecture

- **Target module map:** Same; optional discovery module (scan path, load by manifest) in ROADMAP Phase 3.
- **Public contract boundaries:** Plugin, PluginMetadata, PluginComponents, PluginRegistry, PluginType, PluginError; PluginLoader when feature enabled.
- **Internal invariants:** No execution; no credential resolution; unsafe only in loader module.

## Design Reasoning

- **Trade-off:** Registry not thread-safe by default — keeps crate simple; caller wraps in RwLock. Rejected built-in Mutex to avoid forcing sync and to match caller's locking.
- **Rejected:** Always-on dynamic loading — would pull unsafe and libloading into default build.

## Comparative Analysis

Sources: n8n (integrations), Node-RED (nodes), VS Code (extensions).

- **Adopt:** Plugin as packaging unit with metadata and component list; registry as single source of truth (n8n/Node-RED style).
- **Reject:** Plugin crate running or resolving actions/credentials.
- **Defer:** Plugin sandbox for untrusted dynamic plugins; document when adopted.

## Breaking Changes (if any)

- Plugin trait or registry contract change: major; see MIGRATION.md.

## Open Questions

- Discovery API shape (sync vs async, config format).
- PluginVersions resolution policy (latest vs pinned) and API.
