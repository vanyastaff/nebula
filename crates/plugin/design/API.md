# API

## Public Surface

- **Stable:** Plugin trait, PluginMetadata (builder), PluginComponents, PluginType, PluginVersions, PluginRegistry (new, register, get, contains, list), PluginError, PluginKey (re-export from core). Serialized metadata for API response is stable in patch/minor.
- **Experimental:** PluginLoader (when dynamic-loading feature enabled); ABI/symbol stability documented but may evolve.
- **Hidden/internal:** Registry internals; loader FFI details.

## Usage Patterns

- **Static registration:** Build PluginType::single(impl Plugin) or versioned; registry.register(plugin_type). Engine or API holds registry and resolves by key.
- **Dynamic loading:** Enable feature; PluginLoader::load(path) returns plugin type; register into same registry. Caller responsible for thread-safety (e.g. RwLock around registry).

## Minimal Example

```rust
let mut registry = PluginRegistry::new();
let meta = PluginMetadata::builder("echo", "Echo").build().unwrap();
let plugin_type = PluginType::single(EchoPlugin(meta));
registry.register(plugin_type).unwrap();
assert!(registry.contains(&"echo".parse().unwrap()));
```

## Error Semantics

- **PluginError::AlreadyExists(key):** Duplicate registration; caller must use different key or replace (if we add replace API later). Not retryable without changing key.
- **PluginLoadError:** Dynamic load failed (symbol, ABI, path); document in loader module. Not retryable without fixing path/lib.
- **Fatal:** Invalid metadata or component declaration; fail at register or load.

## Compatibility Rules

- **Major bump:** Breaking Plugin trait or registry API (e.g. register signature, get return type). MIGRATION.md required.
- **Minor:** Additive (new optional methods, new error variants). No removal.
- **Deprecation:** At least one minor version before removal.
