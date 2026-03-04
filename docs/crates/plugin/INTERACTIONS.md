# Interactions

## Ecosystem Map

**nebula-plugin** provides plugin trait, metadata, components, and registry. Engine and API consume the registry; no other workspace crate depends on plugin (engine does; API may hold registry for list/get).

### Upstream (plugin depends on)

- **nebula-core** — PluginKey (re-exported). No other nebula-* for core types.
- **Vendor:** std, serde (if metadata serialization).

### Downstream (consume plugin)

- **nebula-engine** — holds PluginRegistry; resolves action_id or plugin key to action. Depends on plugin for registry and PluginType.
- **nebula-api** — may hold registry for GET /plugins or plugin list in UI.
- **Loader/binary** — with dynamic-loading, loads and registers plugins into registry.

### Planned

- Discovery (scan path, load by manifest) may live in plugin or in a separate loader binary.

## Downstream Consumers

- **Engine:** Expects PluginRegistry with register, get, list; resolves plugin by key to get components (actions). Contract: key → PluginType; no execution in plugin crate.
- **API:** Expects registry to list plugins (metadata) for UI; get by key for detail.

## Upstream Dependencies

- **core:** PluginKey only. Required. No fallback.

## Interaction Matrix

| This crate ↔ Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|--------------------|-----------|----------|------------|------------------|-------|
| plugin → core | in | PluginKey | sync | N/A | |
| engine → plugin | in | PluginRegistry, get, list, register | sync | PluginError | engine holds registry |
| API → plugin | in | PluginRegistry list/get | sync | N/A | |
| loader → plugin | in | PluginLoader (feature), register | sync/async | PluginLoadError | optional |

## Runtime Sequence

1. At startup, engine or API creates PluginRegistry; registers static plugins (PluginType::single(...)).
2. Optional: loader loads from path, registers into same registry.
3. Engine resolves action: looks up plugin by key, gets components, resolves action from component or action registry.
4. API lists plugins: registry.list or iterate keys; return metadata for response.

## Cross-Crate Ownership

- **plugin** owns: Plugin trait, metadata, components, registry type, error type; optional loader.
- **engine** owns: how to use registry for execution (resolve action, pass to runtime); plugin only provides registry and types.
- **action** owns: Action trait; plugin declares action refs in components, does not implement actions.

## Versioning and Compatibility

- Plugin trait and registry API are contract. Breaking = major + MIGRATION.md.
- Dynamic-loading ABI/symbol stability documented in loader; may evolve in minor with doc update.
