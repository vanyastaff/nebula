# nebula-plugin

Plugin packaging and registry for the Nebula workflow engine. A **plugin** is a user-visible, versionable unit (e.g. "Slack", "HTTP Request") that bundles metadata and component refs (actions, credentials). The engine and API use a **PluginRegistry** to resolve plugins by key.

## Scope

- **In scope:** `Plugin` trait, `PluginMetadata`, `PluginComponents`, `PluginType`/`PluginVersions`, `PluginRegistry`, `PluginError`. Optional `PluginLoader` (feature `dynamic-loading`) for loading from shared libraries.
- **Out of scope:** Action implementation; credential resolution; execution. Plugin declares refs only.

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md) — problem, current/target architecture
- [API.md](./API.md) — public surface, registry, compatibility
- [INTERACTIONS.md](./INTERACTIONS.md) — ecosystem, engine/API contract
- [DECISIONS.md](./DECISIONS.md) — object-safe trait, registry thread-safety, dynamic-loading
- [ROADMAP.md](./ROADMAP.md) — phases, risks, exit criteria
- [PROPOSALS.md](./PROPOSALS.md) — discovery, version resolution
- [SECURITY.md](./SECURITY.md) — threat model, dynamic loading safety
- [RELIABILITY.md](./RELIABILITY.md) — failure modes, registry consistency
- [TEST_STRATEGY.md](./TEST_STRATEGY.md) — pyramid, contract tests
- [MIGRATION.md](./MIGRATION.md) — versioning, breaking changes
- [_archive/README.md](./_archive/README.md) — legacy doc preservation

## Archive

Legacy material: [\_archive/](./_archive/)
