# nebula-plugin

Plugin packaging and registry for the Nebula workflow engine. A **plugin** is a user-visible, versionable unit (e.g. "Slack", "HTTP Request") that bundles metadata and component refs (actions, credentials). The engine and API use a **PluginRegistry** to resolve plugins by key.

## Scope

- **In scope:** `Plugin` trait, `PluginMetadata`, `PluginComponents`, `PluginType`/`PluginVersions`, `PluginRegistry`, `PluginError`. Optional `PluginLoader` (feature `dynamic-loading`) for loading from shared libraries.
- **Out of scope:** Action implementation; credential resolution; execution. Plugin declares refs only.

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md) — problem, current/target architecture
- [API.md](./API.md) — public surface, registry, compatibility
- [ROADMAP.md](./ROADMAP.md) — phases, risks, exit criteria
- [MIGRATION.md](./MIGRATION.md) — versioning, breaking changes


