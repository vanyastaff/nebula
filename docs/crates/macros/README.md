# nebula-macros

Procedural macros for the Nebula workflow engine. Reduces boilerplate for implementing Action, Resource, Plugin, Credential, Parameters, Validator, and Config. Generated code conforms to nebula-action, nebula-resource, nebula-plugin, and nebula-credential contracts.

## Derives

| Macro | Description |
|-------|-------------|
| `Action` | Implements the `Action` trait (key, name, description, credential/resource refs) |
| `Resource` | Implements the `Resource` trait |
| `Plugin` | Implements the `Plugin` trait |
| `Credential` | Implements the `Credential` trait |
| `Parameters` | Generates parameter definitions for action metadata |
| `Validator` | Implements field-based validation |
| `Config` | Loads from env and validates fields |

See crate rustdoc and [API.md](./API.md) for attributes and examples.

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md) — problem, current/target architecture
- [API.md](./API.md) — public surface, attributes, compatibility
- [ROADMAP.md](./ROADMAP.md) — phases, risks, exit criteria
- [MIGRATION.md](./MIGRATION.md) — versioning, breaking attributes/output


