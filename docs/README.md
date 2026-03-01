# Nebula

High-performance workflow automation engine written in Rust. Build, run, and manage complex
automation workflows with type-safe, composable nodes — similar to n8n but designed for
extensibility and production performance from the ground up.

**Stack:** Rust 1.93+ · Tokio · egui · axum · PostgreSQL

## Documentation Index

### Overview

| Doc | Description |
|-----|-------------|
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Layer diagram, crate map, data flow |
| [ROADMAP.md](./ROADMAP.md) | Phase-by-phase development plan |
| [PROJECT_STATUS.md](./PROJECT_STATUS.md) | Current component status |
| [TECHNICAL_NOTES.md](./TECHNICAL_NOTES.md) | Architecture decision records |

### Development Guides

| Guide | Description |
|-------|-------------|
| [development/getting-started.md](./development/getting-started.md) | Installation and first workflow |
| [development/node-dev.md](./development/node-dev.md) | Building custom nodes |
| [development/cicd.md](./development/cicd.md) | CI/CD pipeline reference |
| [development/migrations.md](./development/migrations.md) | Database migration guide |

### Crate Reference

| Doc | Description |
|-----|-------------|
| [crates/README.md](./crates/README.md) | Complete crate dependency map |
| [crates/core.md](./crates/core.md) | Core identifiers and traits |
| [crates/action.md](./crates/action.md) | Action execution model |
| [crates/resource.md](./crates/resource.md) | Resource lifecycle management |
| [crates/credential.md](./crates/credential.md) | Credential storage and encryption |
| [crates/sdk.md](./crates/sdk.md) | Developer SDK reference |
| [../crates/log/docs/README.md](../crates/log/docs/README.md) | `nebula-log` internal technical docs |

## Quick Commands

```bash
cargo build                              # Build workspace
cargo test --workspace                   # Run all tests
cargo clippy --workspace -- -D warnings  # Lint
cargo fmt --all                          # Format
cargo doc --no-deps --workspace          # Generate rustdoc
cargo audit                              # Security audit
```

## Key Design Decisions

- **Values** — All workflow runtime data uses `serde_json::Value`; no separate nebula-value crate.
- **API** — REST + WebSocket (axum). GraphQL is not planned.
- **Error handling** — Each crate defines its own error type via `thiserror`; no shared error crate.

See [TECHNICAL_NOTES.md](./TECHNICAL_NOTES.md) for full rationale.
