# nebula-plugin v2 — Design Spec

## Goal

Evolve the plugin crate from a metadata-only container to a full plugin SDK: manifest format, component registration, lifecycle hooks, and integration with the plugin ecosystem strategy (one canonical plugin per service, Plugin Fund, Hub page).

## Philosophy

- **Plugin = packaging unit.** Plugin struct carries metadata + declares components. Execution logic lives in action/credential/resource crates.
- **One canonical plugin per service.** Not a marketplace — collaborative development, funded by Plugin Fund.
- **Static first, dynamic later.** v1: compiled-in Rust crates. v2: WASM loading. Phase 3: multi-language.
- **Plugin manifest as source of truth.** `nebula-plugin.toml` declares everything the Hub needs.

---

## 1. Plugin Trait — Enhanced

```rust
pub trait Plugin: Send + Sync + 'static {
    /// Plugin metadata (key, name, version, description, author).
    fn metadata(&self) -> &PluginMetadata;

    /// Actions this plugin provides.
    fn actions(&self) -> Vec<ActionDescriptor>;

    /// Credential types this plugin provides.
    fn credentials(&self) -> Vec<CredentialDescriptor>;

    /// Resource types this plugin provides (optional).
    fn resources(&self) -> Vec<ResourceDescriptor> {
        vec![]
    }

    /// DataTags this plugin registers (must be called before actions).
    fn data_tags(&self) -> Vec<DataTagDefinition> {
        vec![]
    }

    /// Called once when plugin is loaded into the engine.
    fn on_load(&self, _ctx: &PluginContext) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called when plugin is being unloaded (cleanup).
    fn on_unload(&self) -> Result<(), PluginError> {
        Ok(())
    }
}
```

### PluginMetadata — extended

```rust
pub struct PluginMetadata {
    pub key: PluginKey,
    pub name: String,
    pub version: Version,
    pub description: String,
    pub author: Option<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub icon: Option<String>,
    /// Minimum Nebula engine version required.
    pub nebula_version: Option<String>,
}
```

### Descriptors

```rust
/// Describes an action without constructing it.
pub struct ActionDescriptor {
    pub key: ActionKey,
    pub name: String,
    pub description: String,
    pub version: InterfaceVersion,
    /// Factory function — engine calls this to create the handler.
    pub factory: Box<dyn Fn() -> Arc<dyn InternalHandler> + Send + Sync>,
}

/// Describes a credential type.
pub struct CredentialDescriptor {
    pub key: CredentialKey,
    pub name: String,
    pub pattern: AuthPattern,
    pub parameters: ParameterCollection,
}

/// Describes a resource type.
pub struct ResourceDescriptor {
    pub key: ResourceKey,
    pub name: String,
    pub topologies: Vec<String>,
}
```

---

## 2. Plugin Manifest — `nebula-plugin.toml`

```toml
[plugin]
key = "slack"
name = "Slack"
version = "1.0.0"
description = "Slack messaging and workspace automation"
author = "Nebula Contributors"
license = "MIT"
homepage = "https://nebula.dev/plugins/slack"
repository = "https://github.com/nebula-plugins/slack"
icon = "slack"
nebula_version = ">=1.0, <2.0"

[actions]
"slack.send_message" = { version = "1.0", description = "Send a message to a channel" }
"slack.create_channel" = { version = "1.0", description = "Create a new channel" }
"slack.list_channels" = { version = "1.0", description = "List workspace channels" }

[credentials]
"slack_oauth2" = { pattern = "OAuth2", description = "Slack workspace OAuth2" }
"slack_bot_token" = { pattern = "SecretToken", description = "Slack bot token" }

[resources]
"slack_client" = { topology = "Resident", description = "Shared Slack HTTP client" }

[data_tags]
produces = ["comm.slack.message", "comm.slack.channel"]
consumes = ["text", "json"]
```

Hub page reads this manifest directly — no separate metadata needed.

---

## 3. Plugin Registration Flow

```rust
// In main.rs or engine setup:
let engine = Engine::new(config);

// Register plugin — engine reads descriptors, creates handlers
engine.register_plugin(SlackPlugin::new())?;

// Or register from manifest + compiled crate:
engine.register_plugin_crate::<SlackPlugin>()?;

// Internally:
// 1. Call plugin.data_tags() → register in DataTagRegistry
// 2. Call plugin.credentials() → register in CredentialRegistry
// 3. Call plugin.actions() → call factory(), register in ActionRegistry
// 4. Call plugin.on_load() → plugin-specific init
```

---

## 4. Plugin Derive Macro

```rust
#[derive(Plugin)]
#[plugin(
    key = "slack",
    name = "Slack",
    version = "1.0.0",
    description = "Slack messaging and workspace automation",
)]
pub struct SlackPlugin;

// Derive generates:
// - Plugin trait impl with metadata()
// - Empty defaults for actions(), credentials(), resources(), data_tags()
// - Developer overrides what they need:

impl SlackPlugin {
    fn actions(&self) -> Vec<ActionDescriptor> {
        vec![
            ActionDescriptor::from_action::<SlackSendMessage>(),
            ActionDescriptor::from_action::<SlackCreateChannel>(),
        ]
    }

    fn credentials(&self) -> Vec<CredentialDescriptor> {
        vec![
            CredentialDescriptor::from_credential::<SlackOAuth2>(),
        ]
    }
}
```

---

## 5. PluginContext — What Plugins Receive on Load

```rust
/// Context available to plugins during lifecycle.
pub struct PluginContext {
    /// Engine configuration (read-only).
    pub config: Arc<Config>,
    /// Logger scoped to this plugin.
    pub logger: Arc<dyn ActionLogger>,
    /// Metrics registry for plugin-specific counters.
    pub metrics: Arc<MetricsRegistry>,
}
```

Plugins do NOT get access to:
- Other plugins' state
- Raw credential store
- Engine internals

---

## 6. Integration with Ecosystem Strategy

| Ecosystem concept | Plugin crate support |
|-------------------|---------------------|
| One plugin per service | PluginKey uniqueness enforced in registry |
| nebula-plugin.toml | Manifest parsed by Hub page + CLI tooling |
| Essential 50 | Tag on PluginMetadata: `essential: bool` |
| Version constraints | `nebula_version` field in manifest |
| Backward compat CI | Plugin's test suite runs against Nebula RC |

---

## 7. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Plugin trait | metadata() only | + actions(), credentials(), resources(), data_tags(), on_load(), on_unload() |
| Metadata | key + name + version | + description, author, license, homepage, icon, nebula_version |
| Component declaration | Implicit (registered in engine separately) | Explicit via descriptors returned from Plugin trait |
| Manifest | None | `nebula-plugin.toml` for Hub + CLI |
| Derive | None | `#[derive(Plugin)]` generates trait impl |
| Lifecycle | None | on_load() / on_unload() hooks |

---

## 8. Not In Scope

- Dynamic loading via dlopen (Phase 2 — ABI fragility)
- WASM plugin loading (Phase 3)
- Plugin dependency resolution (plugin A requires plugin B)
- Plugin sandboxing (handled by runtime sandbox, not plugin crate)
- Plugin hot-reload (requires WASM or process isolation)
- Visual plugin builder
