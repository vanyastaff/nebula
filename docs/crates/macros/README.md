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

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md) — problem, current/target architecture
- [API.md](./API.md) — public surface, attributes, compatibility
- [INTERACTIONS.md](./INTERACTIONS.md) — ecosystem, action/plugin/credential/resource contract
- [DECISIONS.md](./DECISIONS.md) — no unsafe, single crate, attribute versioning
- [ROADMAP.md](./ROADMAP.md) — phases, risks, exit criteria
- [PROPOSALS.md](./PROPOSALS.md) — expansion debugging, diagnostics
- [SECURITY.md](./SECURITY.md) — threat model, no unsafe
- [RELIABILITY.md](./RELIABILITY.md) — compile-time only, no runtime
- [TEST_STRATEGY.md](./TEST_STRATEGY.md) — pyramid, contract tests
- [MIGRATION.md](./MIGRATION.md) — versioning, breaking attributes/output
- [_archive/README.md](./_archive/README.md) — legacy doc preservation

## Archive

Legacy material: [\_archive/](./_archive/) and parent directory (archive-*.md, from-archive/, from-core-full/).
