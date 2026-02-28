# nebula-plugin

Plugin system for the Nebula workflow engine. A **plugin** is the user-visible, versionable packaging unit — e.g. "Slack", "HTTP Request", "PostgreSQL". Action, Credential, and Resource belong to Plugin.

## Scope

- **In scope:** Plugin trait, metadata, component registration (actions, credentials), registry, versioning, optional dynamic loading.
- **Out of scope:** Workflow execution, sandbox policy, credential storage, action runtime — those live in engine/runtime/credential/action crates.

## Current State

- **Maturity:** Core types stable; `PluginComponents` has placeholder `InternalHandler` until action adapters are restored.
- **Key strengths:** Object-safe `Plugin` trait; `PluginKey` normalization; multi-version support; in-memory registry.
- **Key risks:** Action integration incomplete (process_action/stateful_action methods commented out); Resource registration not yet in `PluginComponents`.

## Target State

- **Production criteria:** Full action/credential/resource registration; stable serialized metadata; dynamic loading validated.
- **Compatibility guarantees:** Patch/minor preserve `Plugin`, `PluginMetadata`, `PluginComponents`, `PluginRegistry`; breaking changes via major version and MIGRATION.md.

## Quick Example

```rust
use nebula_plugin::{Plugin, PluginMetadata, PluginComponents, PluginRegistry, PluginType};

#[derive(Debug)]
struct SlackPlugin(PluginMetadata);

impl Plugin for SlackPlugin {
    fn metadata(&self) -> &PluginMetadata {
        &self.0
    }

    fn register(&self, components: &mut PluginComponents) {
        components.credential(/* CredentialDescription */);
        // components.process_action(slack_send_message);
    }
}

let meta = PluginMetadata::builder("slack", "Slack")
    .version(2)
    .description("Send messages to Slack")
    .group(vec!["communication".into()])
    .build()
    .unwrap();

let mut registry = PluginRegistry::new();
registry.register(PluginType::single(SlackPlugin(meta))).unwrap();
```

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material (including former Node docs):
- [`_archive/`](./_archive/)
