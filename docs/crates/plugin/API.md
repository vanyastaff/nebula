# API

## Public Surface

### Stable APIs

- `Plugin` — base trait; `metadata()`, `register()`, `key()`, `name()`, `version()`
- `PluginMetadata` — builder API; key, name, version, group, description, icon, docs URL, color, tags
- `PluginComponents` — `credential()`, `handler()`; `credentials()`, `handlers()`, `into_parts()`
- `PluginType` — `single()`, `versioned()`, `get_plugin()`, `latest()`, `add_version()`, `key()`, `version_numbers()`
- `PluginVersions` — `add()`, `get()`, `latest()`, `key()`, `version_numbers()`
- `PluginRegistry` — `register()`, `register_or_replace()`, `get()`, `get_by_name()`, `contains()`, `remove()`, `keys()`, `values()`
- `PluginError` — error enum for all plugin operations
- `PluginKey` — re-export from `nebula-core`

### Experimental APIs

- `PluginLoader` (feature `dynamic-loading`) — load plugins from shared libraries

### Hidden/Internal APIs

- `InternalHandler` — temporary trait until action adapters restored; not part of long-term public contract

## Usage Patterns

### Define a Plugin

```rust
use nebula_plugin::{Plugin, PluginMetadata, PluginComponents};

#[derive(Debug)]
struct MyPlugin(PluginMetadata);

impl Plugin for MyPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.0
    }

    fn register(&self, components: &mut PluginComponents) {
        components.credential(/* CredentialDescription */);
        // components.process_action(MyAction::default());
    }
}
```

### Build Metadata

```rust
let meta = PluginMetadata::builder("http_request", "HTTP Request")
    .version(2)
    .description("Make HTTP calls")
    .group(vec!["network".into(), "api".into()])
    .icon("globe")
    .documentation_url("https://docs.example.com/http")
    .build()
    .unwrap();
```

### Register and Lookup

```rust
let mut registry = PluginRegistry::new();
registry.register(PluginType::single(MyPlugin(meta))).unwrap();

let key: PluginKey = "http_request".parse().unwrap();
let plugin_type = registry.get(&key).unwrap();
let plugin = plugin_type.latest().unwrap();
```

## Minimal Example

```rust
use nebula_plugin::{Plugin, PluginMetadata, PluginComponents, PluginRegistry, PluginType};

#[derive(Debug)]
struct EchoPlugin(PluginMetadata);
impl Plugin for EchoPlugin {
    fn metadata(&self) -> &PluginMetadata { &self.0 }
    fn register(&self, _: &mut PluginComponents) {}
}

let meta = PluginMetadata::builder("echo", "Echo").build().unwrap();
let mut reg = PluginRegistry::new();
reg.register(PluginType::single(EchoPlugin(meta))).unwrap();
assert!(reg.contains(&"echo".parse().unwrap()));
```

## Advanced Example

```rust
// Multi-version plugin
let mut versions = PluginVersions::new();
versions.add(SlackPlugin(meta_v1)).unwrap();
versions.add(SlackPlugin(meta_v2)).unwrap();

let pt = PluginType::Versions(versions);
let plugin_v2 = pt.get_plugin(Some(2)).unwrap();
```

## Error Semantics

- **Retryable:** None; plugin operations are synchronous and deterministic.
- **Fatal:** `PluginError::NotFound`, `VersionNotFound`, `InvalidKey`, `KeyMismatch`, `VersionAlreadyExists`, `AlreadyExists`
- **Validation:** `PluginError::InvalidKey` from `PluginMetadata::build()` when key fails normalization

## Compatibility Rules

- **Major version bump:** Changes to `Plugin` trait, `PluginComponents` structure, or serialized metadata schema.
- **Deprecation policy:** Minimum 6 months for public API changes; document in MIGRATION.md.
