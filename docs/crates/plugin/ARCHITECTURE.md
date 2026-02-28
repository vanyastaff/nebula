# Architecture

## Problem Statement

- **Business problem:** A workflow platform (n8n-class) needs a packaging unit for integrations (Slack, HTTP, PostgreSQL). Users see "nodes" in the editor; the runtime needs a stable way to discover, version, and load Actions, Credentials, and Resources.
- **Technical problem:** How to define a single, versionable unit that bundles actions, credentials, and resources without coupling to execution, storage, or sandbox logic.

## Role in the Workspace

`nebula-plugin` is the **packaging and discovery layer** for user-visible integrations.

Dependency direction:

- `nebula-plugin` → `nebula-core`, `nebula-action`, `nebula-credential`
- `nebula-runtime`, `nebula-engine`, `nebula-sdk`, `nebula-registry` → `nebula-plugin`

Plugin owns the concept "one integration = one Plugin"; action/credential/resource crates define the component contracts.

## Current Architecture

### Module Map

| Module | File | Responsibility |
|--------|------|----------------|
| `plugin` | `plugin.rs` | `Plugin` trait — metadata, `register(components)` |
| `metadata` | `metadata.rs` | `PluginMetadata`, builder, key/name/version/group/icon/docs |
| `components` | `components.rs` | `PluginComponents` — credentials, handlers (actions); placeholder `InternalHandler` |
| `plugin_type` | `plugin_type.rs` | `PluginType` — single or versioned set |
| `versions` | `versions.rs` | `PluginVersions` — multi-version container keyed by `u32` |
| `registry` | `registry.rs` | `PluginRegistry` — in-memory map `PluginKey` → `PluginType` |
| `error` | `error.rs` | `PluginError` taxonomy |
| `loader` | `loader.rs` | `PluginLoader` — dynamic `.so`/`.dll` loading (feature-gated) |

### Data/Control Flow

- **Data:** Plugin metadata and component descriptors flow from plugin → registry → runtime/engine. No execution data flows through plugin.
- **Control:** Plugin is passive; `register()` is called by registry/loader during discovery. No callbacks into plugin from runtime.

### Known Bottlenecks

- `PluginComponents` uses `InternalHandler` placeholder; typed `process_action`/`stateful_action` are commented out until action adapters are restored.
- Resource registration not yet in `PluginComponents` (planned).
- `PluginRegistry` is not thread-safe by design; caller must wrap in `RwLock` if shared.

## Target Architecture

- **Target module map:** Same structure; add `resource()` to `PluginComponents`; restore typed action registration.
- **Public contract boundaries:** `Plugin`, `PluginMetadata`, `PluginComponents`, `PluginRegistry`, `PluginType`, `PluginVersions`. Loader is feature-gated and optional.
- **Internal invariants:** All public types `Serialize + Deserialize` where applicable; `Plugin` is object-safe; no heap in hot lookup paths.

## Design Reasoning

- **Plugin vs Node:** Adopt Plugin. "Node" implied graph vertex; Plugin is the packaging unit. Action, Credential, Resource belong to Plugin.
- **Object-safe Plugin:** Adopt. Enables `Arc<dyn Plugin>`, registry storage, dynamic loading.
- **u32 versioning:** Adopt. Simpler than semver for internal versioning; semver can be in metadata if needed for UI.

### Rejected Alternatives

- **Node as parent of Action:** Rejected. Plugin is the user-facing unit; Action is an execution contract.
- **Plugin depending on engine/runtime:** Rejected. Would create cycles; plugin is discovery-only.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces/Activeflow.

| Pattern | Decision | Rationale |
|---------|----------|-----------|
| Plugin as packaging unit | **Adopt** | n8n/Node-RED use node = integration; Plugin clarifies packaging vs execution |
| Versioned plugin sets | **Adopt** | Enables gradual migration; workflow can pin version |
| In-memory registry | **Adopt** | Runtime populates from static + dynamic; no persistence in plugin |
| Dynamic loading | **Adopt** | Feature-gated; enables external integrations without recompile |

## Breaking Changes (if any)

- Restoring typed action registration may change `PluginComponents` API — minor if additive.
- Adding `resource()` to `PluginComponents` — additive, non-breaking.

## Open Questions

- Q1: Should `PluginComponents` support resource descriptions (similar to `CredentialDescription`)?
- Q2: Should Plugin metadata include capability/sandbox hints for runtime policy?
