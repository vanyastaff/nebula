# Nebula

[![CI](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml/badge.svg)](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.95%2B-orange)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE)

**Modular, type-safe workflow automation engine written in Rust.**

Nebula is a DAG-based workflow automation engine &mdash; in the same space as n8n, Zapier, and Temporal &mdash; built as a composable Rust library rather than a monolithic platform. The goal is to give teams a foundation they can embed into their own infrastructure, extend with custom integrations, and trust with production secrets.

**Current status:** core crates are stable and well-tested; the execution engine and API layer are in active development. Not production-ready yet.

---

## Why Nebula

Most automation platforms are runtime-interpreted, dynamically typed, and treat security as an afterthought. Nebula takes a different approach.

**Credentials are a first-class concern, not a bolt-on.** Every secret is encrypted at rest with AES-256-GCM, bound to its record via AAD to prevent swapping attacks, and wiped from memory on drop. Key rotation is built into the storage layer &mdash; not a future feature.

**The type system does the work.** Workflow structure, action I/O, parameter schemas, and auth patterns are all expressed as Rust types. If a workflow compiles, its shape is valid. There are no stringly-typed action references, no untyped credential bags, no `Any` escape hatches in the core pipeline.

**Resilience is built in, not bolted on.** Retry with backoff, circuit breakers, rate limiting, hedged requests, and bulkhead isolation are composable building blocks in `nebula-resilience`. Every pattern returns a typed error with enough context to decide what to do next. Purpose-built for the engine's concurrency model.

**Modularity is a hard constraint.** The workspace enforces strict one-way layer dependencies via `cargo deny` in CI. Cross-crate communication goes through `EventBus`, not direct imports. You can use `nebula-credential` without touching `nebula-engine`; you can embed `nebula-resilience` in a project that has nothing to do with workflows.

## Design Principles

- **Types over tests.** Make invalid states unrepresentable. Use newtypes for IDs, enums for states, builders for validated config. Tests verify behavior, not type safety &mdash; the compiler handles that.
- **Explicit over magic.** No global state, no hidden service locators, no ambient configuration. Actions receive everything they need via `Context`. If a dependency isn't in the function signature, it doesn't exist.
- **Delete over deprecate (internals).** For internal engine architecture, when an API is wrong, replace it. No adapters, bridges, shims, or backward-compatibility tax. However, for the public `nebula-sdk` and plugin contracts, we respect the integration author's time and provide a clear deprecation path.
- **Security by default.** Secrets are encrypted, zeroized, and redacted in Debug output. AAD binding is mandatory. There is no `legacy_compat` flag. The safe path is the only path.
- **Composition over inheritance.** Storage layers (encryption, cache, audit, scope) stack via trait delegation. Auth schemes are open traits, not closed enums. Resilience patterns compose into pipelines.

## Architecture

```
API / Public    api (HTTP + webhook module) · sdk (integration author façade)
Exec            engine · runtime · storage · sandbox · plugin-sdk
Business        credential · resource · action · plugin
Core            core · validator · expression · workflow · execution · schema · metadata
Cross-cutting   log · system · eventbus · telemetry · metrics · resilience · error
```

Each layer depends only on layers below it. Cross-cutting crates are importable at any level. Layer boundaries are enforced mechanically by `cargo deny` (see `deny.toml` `wrappers`) — a missing entry fails CI before review.

### Data Flow

```
Trigger (webhook / cron / event)
  -> Engine resolves workflow DAG
    -> Runtime schedules nodes in topological order
      -> Each node: Action::execute(Context) -> serde_json::Value
        -> Context provides: encrypted credentials, resources, parameters, logger
          -> Cross-crate signals via EventBus (e.g., credential rotation events)
```

While strict Rust typing is enforced at the boundaries (inside Actions and Credentials), `serde_json::Value` is the universal interchange data type between nodes in the DAG. No custom value crate, no conversion layers. Dates are ISO-8601 strings, decimals use a base64 convention. The `nebula-schema` runtime validation bridges the gap between the dynamic graph and strictly typed nodes.

## Crate Map

Source of truth: workspace members in `Cargo.toml`.

| Layer             | Crate           | Purpose                                                                              |
| ----------------- | --------------- | ------------------------------------------------------------------------------------ |
| **Core**          | `core`          | IDs, domain keys, prefixed-ULID, shared vocabulary                                   |
|                   | `validator`     | Validation rule engine                                                               |
|                   | `schema`        | Typed field definitions + `#[derive(HasSchema)]` (was `nebula-parameter`)            |
|                   | `metadata`      | `Metadata` trait + helpers shared by Action / Credential / Resource / Plugin         |
|                   | `expression`    | Template expression engine                                                           |
|                   | `workflow`      | `WorkflowDefinition`, DAG structure, activation-time validator                       |
|                   | `execution`     | Execution state machine + transitions                                                |
| **Business**      | `credential`    | Encrypted storage (AES-256-GCM + AAD), key rotation, 12 universal auth schemes       |
|                   | `resource`      | External service lifecycle, typed credential refs                                    |
|                   | `action`        | Action trait family (Stateless / Stateful / Trigger / Resource / Control)            |
|                   | `plugin`        | In-process plugin trait + registry                                                   |
| **Exec**          | `engine`        | Frontier loop, lease lifecycle, control consumer (ADR-0008)                          |
|                   | `runtime`       | Node scheduling, dispatch, blob spill                                                |
|                   | `storage`       | Persistence trait family + in-memory + Postgres (SQLite local path planned)          |
|                   | `sandbox`       | Process-isolated action execution (capability allowlist planned)                     |
|                   | `plugin-sdk`    | Out-of-process plugin protocol (`run_duplex`)                                        |
| **API / Public**  | `api`           | REST server, webhook transport, middleware                                           |
|                   | `sdk`           | **Integration author façade** — re-exports + `WorkflowBuilder` + `TestRuntime`       |
| **Cross-cutting** | `error`         | `NebulaError<E>`, `Classify` trait, derive macro                                     |
|                   | `resilience`    | Retry, circuit breaker, rate limiter, hedge, bulkhead                                |
|                   | `log`           | Structured logging infrastructure                                                    |
|                   | `eventbus`      | In-memory typed pub/sub for cross-crate signals                                      |
|                   | `telemetry`     | Lock-free metrics primitives + label interning                                       |
|                   | `metrics`       | `nebula_*` naming, cardinality allowlist, Prometheus export                          |
|                   | `system`        | Process monitoring, system load tracking                                             |

## Credential System

The credential subsystem is one of Nebula's most developed areas. Highlights:

- **12 universal auth patterns** covering real-world auth: `SecretToken`, `IdentityPassword`, `OAuth2Token`, `KeyPair`, `Certificate`, `SigningKey`, `FederatedAssertion`, `ChallengeSecret`, `OtpSeed`, `ConnectionUri`, `InstanceBinding`, `SharedKey`
- **Open `AuthScheme` trait** &mdash; plugins add custom schemes without modifying core
- `#[derive(AuthScheme)]` generates trait impls from a single attribute
- **Layered storage**: encryption (AES-256-GCM with key rotation) -> cache (moka) -> audit trail -> scope isolation
- **Interactive flows**: OAuth2 with PKCE, multi-step auth, challenge-response &mdash; all via a state machine (`resolve` / `continue_resolve` / `refresh`)
- **Rotation subsystem** (feature-gated): periodic, before-expiry, scheduled, manual &mdash; with blue-green and grace period support

## Quick Start

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build
cargo nextest run --workspace
```

Requires **Rust 1.95+** (edition 2024). Uses [cargo-nextest](https://nexte.st/) for test runs.

### Local Infrastructure

```bash
task db:up          # Start Postgres via Docker Compose
task db:migrate     # Run pending migrations
task obs:up         # Start Jaeger + OTEL collector
```

### CI Locally

```bash
task dev:check
```

### Optional Local Hooks

```bash
cargo install --locked lefthook
lefthook install
```

This enables local hooks from `lefthook.yml`: fast checks on `pre-commit` and full `nextest` on `pre-push`.

## Status

Nebula is in **active alpha development**. The core layer, credential system, resilience patterns, schema system, and error infrastructure are stable and well-tested. The execution engine, runtime, and API layer are actively being wired together.

APIs will change. Not production-ready yet. See [CONTRIBUTING.md](CONTRIBUTING.md) to get involved.

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).