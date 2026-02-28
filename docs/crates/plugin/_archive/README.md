# Plugin Archive

Legacy documentation preserved during the docs refactor per SPEC.md.

## Contents

- **archive-crates-architecture.md** — Original registry/plugin architecture from crates-architecture.md
- **archive-node-execution.md** — Legacy "Node" concept (superseded by Plugin)
- **archive-node-development.md** — Node development guide (concepts now apply to Plugin)
- **archive-node-crates-dependencies.md** — Node layer dependencies (now Plugin layer)
- **archive-nebula-all-docs__docs_guides_node-development.md.md** — Full node development guide

## Conceptual Migration: Node → Plugin

The platform evolved from **Node** (grouping Actions + Credentials) to **Plugin** as the canonical packaging unit. Plugin is the parent concept; Action, Credential, and Resource belong to Plugin.

- Node = user-visible packaging unit (Slack, HTTP Request, etc.)
- Plugin = same concept, extended with versioning, registry, and Resources
- Actions, Credentials, Resources are **components** of a Plugin
