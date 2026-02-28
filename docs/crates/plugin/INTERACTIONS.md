# Interactions

## Ecosystem Map (Current + Planned)

`nebula-plugin` is the packaging and discovery layer. Plugin is the parent concept; Action, Credential, and Resource belong to Plugin. Dependency direction: plugin → core, action, credential; runtime/engine/registry/sdk → plugin.

## Existing Crates

- **core:** `PluginKey`, domain primitives; plugin depends for key normalization
- **action:** `ActionMetadata`, `NodeContext`, `ActionResult`, `ActionError`; plugin registers action handlers via `PluginComponents`
- **credential:** `CredentialDescription`; plugin registers credential requirements via `PluginComponents`
- **parameter:** Used indirectly via action metadata; plugin does not depend directly
- **resource:** Planned; plugin will register resource descriptions (not yet implemented)
- **runtime / engine:** Consume `PluginRegistry`; build execution context from plugin components
- **registry:** Wraps or extends `PluginRegistry`; discovery, loading, caching
- **sdk:** Re-exports plugin types; builders for plugin authoring

## Planned Crates

- **workflow:** Will consume plugin metadata for graph node compatibility
- **api / cli / ui:** Will use plugin metadata for palette, forms, docs links

## Downstream Consumers

- **runtime/engine:** Expect `PluginRegistry` or equivalent; `Plugin::register()` to obtain handlers and credential descriptions
- **sdk:** Expect `Plugin`, `PluginMetadata`, `PluginComponents` for authoring
- **registry:** Expect `PluginRegistry`, `PluginType`, `PluginLoader` for discovery
- **ui/editor:** Expect `PluginMetadata` (key, name, group, icon, docs) for palette and properties

## Upstream Dependencies

- **nebula-core:** `PluginKey`; hard contract on key format and normalization
- **nebula-action:** `ActionMetadata`, `NodeContext`, `ActionResult`, `ActionError`, `InternalHandler`-compatible types; plugin registers handlers
- **nebula-credential:** `CredentialDescription`; plugin registers credential requirements

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| plugin -> core | out | PluginKey | sync | InvalidKey on parse fail | core owns key semantics |
| plugin -> action | out | ActionMetadata, InternalHandler | sync | N/A | plugin wraps actions in handlers |
| plugin -> credential | out | CredentialDescription | sync | N/A | plugin collects credential reqs |
| runtime/engine -> plugin | in | PluginRegistry, Plugin::register | sync | PluginError on lookup fail | plugin is discovery-only |
| registry -> plugin | in | PluginRegistry, PluginLoader | sync/async | PluginError, PluginLoadError | loader is async |
| sdk -> plugin | in | Plugin, PluginMetadata, PluginComponents | sync | N/A | sdk re-exports and extends |

## Runtime Sequence

1. Registry/loader discovers plugins (static registration or dynamic load).
2. For each plugin, registry calls `plugin.register(&mut components)`.
3. Registry extracts `credentials()` and `handlers()` from `PluginComponents`.
4. Runtime/engine builds action registry and credential requirements from components.
5. At execution time, runtime looks up action by key, resolves credentials, runs handler.

## Cross-Crate Ownership

- **plugin owns:** Plugin packaging, metadata, component collection, registry, versioning
- **action owns:** Action execution contract, handlers, context
- **credential owns:** Credential lifecycle, storage, resolution
- **resource owns:** Resource lifecycle, pooling (when integrated)
- **runtime/engine own:** Orchestration, scheduling, context building

## Failure Propagation

- `PluginError` on registry operations (NotFound, AlreadyExists, VersionNotFound, etc.)
- `PluginLoadError` (dynamic-loading) on library load failure
- No retries; plugin operations are synchronous and deterministic

## Versioning and Compatibility

- **Compatibility promise:** Patch/minor preserve `Plugin`, `PluginMetadata`, `PluginComponents`, `PluginRegistry`, `PluginType`, `PluginVersions`
- **Breaking-change protocol:** Major version bump; MIGRATION.md; migration path for registry/loader consumers
- **Deprecation window:** Minimum 6 months for public API changes

## Contract Tests Needed

- plugin/action: Handler registration; metadata compatibility
- plugin/credential: CredentialDescription registration
- plugin/core: PluginKey normalization in metadata
- registry/plugin: Register, lookup, version resolution
