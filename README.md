# Nebula - Workflow Automation Engine

[![Rust](https://img.shields.io/badge/rust-1.93%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://codspeed.io/vanyastaff/nebula?utm_source=badge)
[![Status](https://img.shields.io/badge/status-Active%20Development-brightgreen.svg)](docs/PROJECT_STATUS.md)

> Modular, type-safe workflow automation for Rust-first teams.

Nebula is a DAG-based automation engine (in the n8n/Zapier category) built as a Rust workspace.
It focuses on strong typing, composable action plugins, reliable execution, and clear separation of
core, runtime, API, and infrastructure concerns.

## Quick Start

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build
cargo test
```

## Key Features

- Type-safe workflow and execution models across 26 crates
- Async-first runtime built on Tokio
- Storage abstraction with in-memory and PostgreSQL paths
- Encrypted credentials and rotation-oriented resource integration
- REST plus WebSocket API layer and Tauri desktop surface

## Example

```text
trigger (webhook/cron/event)
    -> http.request
    -> transform.json
    -> notify.slack
```

## Documentation

| Guide | Description |
|-------|-------------|
| [Getting Started](docs/getting-started.md) | Installation, onboarding, first run |
| [Architecture](docs/ARCHITECTURE.md) | Layering, crate map, data flow |
| [API Reference](docs/api.md) | Routes, auth, request flow |
| [Configuration](docs/configuration.md) | Environment variables and defaults |
| [Deployment](docs/deployment.md) | Local infra and runtime startup |
| [Project Status](docs/PROJECT_STATUS.md) | Current implementation status |
| [Roadmap](docs/ROADMAP.md) | Phases, priorities, dependencies |
| [Tasks](docs/TASKS.md) | Cross-crate execution backlog |
| [Contributing](docs/contributing.md) | Contribution standards and setup |
| [Workflow](docs/workflow.md) | Branching, commits, PR process |

## Additional References

- [docs/crates/README.md](docs/crates/README.md)
- [vision/README.md](vision/README.md)
- [vision/ARCHITECTURE.md](vision/ARCHITECTURE.md)
- [vision/CRATES.md](vision/CRATES.md)
- [vision/DECISIONS.md](vision/DECISIONS.md)

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).

Built with ❤️ by the Nebula team and contributors. Thanks for being part of this journey!

---

**Questions?** Start with [vision/README.md](vision/README.md) or open a [discussion](https://github.com/vanyastaff/nebula/discussions).


