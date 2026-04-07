# Nebula

[![CI](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml/badge.svg)](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://codspeed.io/vanyastaff/nebula)

**Modular, type-safe workflow automation engine in Rust.**

Nebula is a DAG-based automation engine in the same category as n8n, Zapier, and Temporal &mdash; but built from scratch in Rust as a composable library, not a monolithic platform. It is designed for teams that want workflow automation they can embed, extend, and trust with production credentials.

Alpha stage: core crates are stable, execution engine and desktop app are in active development.

---

## Why Nebula

Most automation platforms are runtime-interpreted, dynamically typed, and treat security as an afterthought. Nebula takes a different path.

**Credentials are a first-class concern, not a bolt-on.** Every secret is encrypted at rest with AES-256-GCM, bound to its record via AAD to prevent swapping attacks, and wiped from memory on drop. Key rotation is built into the storage layer &mdash; not a future feature. The credential system went through 10 adversarial review rounds, 2 dev challenges, and SOC2 grading before shipping.

**The type system does the work.** Workflow structure, action I/O, parameter schemas, auth patterns &mdash; all expressed as Rust types. If a workflow compiles, its shape is valid. There are no stringly-typed action references, no untyped credential bags, no "any" escape hatches in the core pipeline.

**Resilience is not optional.** Retry with backoff, circuit breakers, rate limiting, hedged requests, and bulkhead isolation are composable building blocks in `nebula-resilience`. Every pattern returns `CallError<E>` with enough context to decide what to do next. These aren't wrappers around another library &mdash; they're purpose-built, audited (153 tests, 14 integration tests, 7 benchmark suites), and designed for the engine's concurrency model.

**Modularity is enforced, not aspirational.** The 24-crate workspace has strict one-way layer dependencies checked by `cargo deny` on every CI run. Cross-crate communication goes through `EventBus`, not direct imports. You can use `nebula-credential` without touching `nebula-engine`. You can embed `nebula-resilience` in a project that has nothing to do with workflows.

## Design Principles

- **Types over tests.** Make invalid states unrepresentable. Use newtypes for IDs, enums for states, builders for validated config. Tests verify behavior, not type safety &mdash; the compiler handles that.
- **Explicit over magic.** No global state, no hidden service locators, no ambient configuration. Actions receive everything they need via `Context`. If a dependency isn't in the function signature, it doesn't exist.
- **Delete over deprecate.** When an API is wrong, replace it. No adapters, bridges, shims, or backward-compatibility tax. Migration cost is acceptable if the design is right.
- **Security by default.** Secrets are encrypted, zeroized, and redacted in Debug output. AAD binding is mandatory. There is no `legacy_compat` flag. The safe path is the only path.
- **Composition over inheritance.** Storage layers (encryption, cache, audit, scope) stack via trait delegation. Auth schemes are open traits, not closed enums. Resilience patterns compose into pipelines.

## Architecture

```
API              api, webhook
Exec             engine, runtime, storage, sdk
Business         credential, resource, action, plugin
Core             core, validator, parameter, expression, workflow, execution
Cross-cutting    log, system, eventbus, telemetry, metrics, config, resilience, error
```

Each layer depends only on layers below it. Cross-cutting crates are importable at any level. Layer boundaries are enforced by `cargo deny` in CI.

### Data Flow

```
Trigger (webhook / cron / event)
  -> Engine resolves workflow DAG
    -> Runtime schedules nodes in topological order
      -> Each node: Action::execute(Context) -> serde_json::Value
        -> Context provides: encrypted credentials, resources, parameters, logger
          -> Cross-crate signals via EventBus (e.g., credential rotation events)
```

`serde_json::Value` is the universal data type. No custom value crate, no conversion layers. Dates are ISO-8601 strings, decimals use a base64 convention.

## Crate Map

| Layer | Crate | Purpose |
|-------|-------|---------|
| **Core** | `core` | IDs, domain keys, `AuthScheme` trait, `AuthPattern`, `SecretString` |
| | `validator` | Schema validation |
| | `parameter` | Typed parameter definitions, `#[derive(Parameters)]` |
| | `expression` | Template expression engine |
| | `workflow` | Workflow definition, DAG structure |
| | `execution` | Execution state machine |
| **Business** | `credential` | Encrypted storage, key rotation, 12 universal auth schemes, `#[derive(AuthScheme)]` |
| | `resource` | External service connections, typed credential refs |
| | `action` | Action trait, context-based DI |
| | `plugin` | Plugin loading and registry |
| **Exec** | `engine` | DAG resolution, orchestration |
| | `runtime` | Node scheduling, isolation routing, blob spill |
| | `storage` | Persistence abstraction (in-memory, Postgres) |
| | `sdk` | Plugin author SDK and prelude |
| **API** | `api` | REST + WebSocket server |
| | `webhook` | Inbound webhook handling, HMAC verification |
| **Cross-cutting** | `error` | `NebulaError<E>`, `Classify` trait, derive macro |
| | `resilience` | Retry, circuit breaker, rate limiter, hedge, bulkhead |
| | `log` | Structured logging infrastructure |
| | `config` | Configuration loading |
| | `eventbus` | In-memory typed pub/sub for cross-crate signals |
| | `telemetry` | Metrics registry |
| | `metrics` | Prometheus export |
| | `system` | Process monitoring, system load tracking |

**Desktop app**: `apps/desktop/` &mdash; Tauri shell with React + TypeScript frontend.

## Credential System

The credential subsystem is one of Nebula's most developed areas. Highlights:

- **12 universal auth patterns** covering real-world auth: `SecretToken`, `IdentityPassword`, `OAuth2Token`, `KeyPair`, `Certificate`, `SigningKey`, `FederatedAssertion`, `ChallengeSecret`, `OtpSeed`, `ConnectionUri`, `InstanceBinding`, `SharedKey`
- **Open `AuthScheme` trait** &mdash; plugins add custom schemes without modifying core
- **`#[derive(AuthScheme)]`** generates trait impls from a single attribute
- **Layered storage**: encryption (AES-256-GCM with key rotation) -> cache (moka) -> audit trail -> scope isolation
- **Interactive flows**: OAuth2 with PKCE, multi-step auth, challenge-response &mdash; all via a state machine (`resolve` / `continue_resolve` / `refresh`)
- **Rotation subsystem** (feature-gated): periodic, before-expiry, scheduled, manual &mdash; with blue-green and grace period support

## Quick Start

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build
cargo test --workspace
```

Requires **Rust 1.94+** (edition 2024). Uses [cargo-nextest](https://nexte.st/) for faster test runs if installed.

### Local Infrastructure

```bash
task db:up          # Start Postgres via Docker Compose
task db:migrate     # Run pending migrations
task obs:up         # Start Jaeger + OTEL collector
task desktop:dev    # Launch Tauri desktop app in dev mode
```

### CI Locally

```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace
```

## Documentation

| Doc | Description |
|-----|-------------|
| [Architecture](docs/ARCHITECTURE.md) | Layer system, crate map, data flow |
| [Project Status](docs/PROJECT_STATUS.md) | What's stable, what's in progress |
| [Roadmap](docs/ROADMAP.md) | Phases, priorities, dependencies |
| [Contributing](docs/contributing.md) | Setup, standards, PR process |

## Status

Nebula is in **active alpha development**. The core layer, credential system, resilience patterns, parameter system, and error infrastructure are stable and well-tested. The execution engine, runtime, and API layer are being wired together. The desktop app (Tauri) is in early development.

This is not production-ready yet. APIs will change. But the foundation is solid, and the direction is clear.

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).
