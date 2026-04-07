# Nebula

[![CI](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml/badge.svg)](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://codspeed.io/vanyastaff/nebula)

Modular, type-safe workflow automation engine in Rust. Think n8n/Zapier, but as a composable library with strong guarantees.

Workflows are DAGs of actions with encrypted credentials, resilience patterns (retry, circuit breaker, rate limiting), and a plugin system. Alpha stage: core crates stable, execution engine in active development.

## Architecture

```
API layer          api, webhook
Exec layer         engine, runtime, storage, sdk
Business layer     credential, resource, action, plugin
Core layer         core, validator, parameter, expression, memory, workflow, execution
Cross-cutting      log, system, eventbus, telemetry, metrics, config, resilience, error
```

Each layer depends only on layers below it. Cross-cutting crates are importable at any level.

```
Trigger (webhook/cron/event)
  -> Engine resolves workflow DAG
    -> Runtime schedules nodes (topological order)
      -> Action::execute(Context) -> serde_json::Value
```

## Crate Map

| Layer | Crate | Purpose |
|-------|-------|---------|
| **Core** | `core` | IDs, domain keys, `AuthScheme` trait, `SecretString` |
| | `validator` | Schema validation |
| | `parameter` | Typed parameter definitions, `#[derive(Parameters)]` |
| | `expression` | Template expression engine |
| | `memory` | Shared execution memory |
| | `workflow` | Workflow definition, DAG structure |
| | `execution` | Execution state machine |
| **Business** | `credential` | Encrypted storage, key rotation, 12 universal auth schemes, `#[derive(AuthScheme)]` |
| | `resource` | External service connections |
| | `action` | Action trait, context DI |
| | `plugin` | Plugin loading and registry |
| **Exec** | `engine` | DAG resolution, orchestration |
| | `runtime` | Node scheduling, isolation, blob spill |
| | `storage` | Persistence abstraction (in-memory, Postgres) |
| | `sdk` | Plugin author SDK, prelude |
| **API** | `api` | REST + WebSocket server |
| | `webhook` | Inbound webhook handling, HMAC verification |
| **Cross-cutting** | `error` | `NebulaError<E>`, `Classify` trait, derive macro |
| | `resilience` | Retry, circuit breaker, rate limiter, hedge, bulkhead |
| | `log` | Structured logging infrastructure |
| | `config` | Configuration loading |
| | `eventbus` | In-memory pub/sub for cross-crate signals |
| | `telemetry` | Metrics registry |
| | `metrics` | Prometheus export |
| | `system` | Process monitoring, system load |

**Desktop app**: `apps/desktop/` — Tauri (React + TypeScript)

## Quick Start

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build
cargo test --workspace
```

Requires Rust 1.94+ (edition 2024). Uses [cargo-nextest](https://nexte.st/) for faster test runs if installed.

For local Postgres and observability:

```bash
task db:up        # Start Postgres via Docker Compose
task db:migrate   # Run migrations
task obs:up       # Start Jaeger + OTEL collector
```

## Key Design Choices

- **`serde_json::Value`** as the universal data type — no custom value crate
- **Context-based DI** — actions receive credentials, resources, logger via `Context`, never construct them
- **EventBus** for cross-crate signals — prevents circular dependencies
- **Typed errors** — `thiserror` in libraries, `anyhow` in binaries, `Classify` trait for error categorization
- **Encryption at rest** — AES-256-GCM with key rotation, mandatory AAD binding, `Zeroizing` plaintext buffers
- **12 universal auth patterns** — `SecretToken`, `IdentityPassword`, `OAuth2Token`, `KeyPair`, `Certificate`, `SigningKey`, and more

## Documentation

| Doc | Description |
|-----|-------------|
| [Architecture](docs/ARCHITECTURE.md) | Layering, crate map, data flow |
| [Project Status](docs/PROJECT_STATUS.md) | Current implementation status |
| [Roadmap](docs/ROADMAP.md) | Phases and priorities |
| [Contributing](docs/contributing.md) | Setup, standards, PR process |

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).
