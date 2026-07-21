# Nebula

[![CI](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml/badge.svg)](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE)

**Modular, type-safe workflow automation engine written in Rust.**

Nebula is a DAG-based workflow automation engine &mdash; in the same space as n8n, Zapier, and Temporal &mdash; built as a composable Rust library rather than a monolithic platform. The goal is to give teams a foundation they can embed into their own infrastructure, extend with custom integrations, and trust with production secrets.

**Current status:** core crates are stable and well-tested; the execution engine and API layer are in active development. Not production-ready yet.

---

## Why Nebula

Most automation platforms are runtime-interpreted, dynamically typed, and treat security as an afterthought. Nebula takes a different approach.

**Credentials are a first-class concern, not a bolt-on.** Every secret is encrypted at rest with AES-256-GCM, bound to its record via AAD to prevent swapping attacks, and wiped from memory on drop. Key rotation is built into the storage layer &mdash; not a future feature.

**Types and activation validation share the work.** Integration contracts, action I/O, parameter schemas, and auth patterns are expressed as Rust types. Persisted or dynamically assembled workflow graphs are validated when activated: references, graph structure, declared schemas, and runtime capabilities must agree before execution. Compiling an integration proves its Rust contracts; it does not make arbitrary workflow data valid.

**Resilience is built in, not bolted on.** Retry with backoff, circuit breakers, rate limiting, hedged requests, and bulkhead isolation are composable building blocks in `nebula-resilience`. Every pattern returns a typed error with enough context to decide what to do next. Purpose-built for the engine's concurrency model.

**Modularity is a hard constraint.** The workspace enforces strict one-way layer dependencies via `cargo deny` in CI. Direct dependencies on lower-layer domain types and ports are normal; upward and undeclared lateral dependencies are not. Durable commands and business facts travel through persisted state or explicit outbox/inbox seams. `EventBus` is only ephemeral observation for telemetry, cache/UI invalidation, and wake-up hints, never authoritative delivery or state. You can use `nebula-credential` without touching `nebula-engine`; you can embed `nebula-resilience` in a project that has nothing to do with workflows.

## Design Principles

- **Types plus boundary validation.** Make invalid local states unrepresentable with newtypes, enums, and builders; validate dynamic graphs and external data at activation and API boundaries. Tests prove behavior and cross-component guarantees that types alone cannot establish.
- **Explicit over magic.** No global state, no hidden service locators, no ambient configuration. Actions receive everything they need via `Context`. If a dependency isn't in the function signature, it doesn't exist.
- **Delete over deprecate (internals).** For internal engine architecture, when an API is wrong, replace it. No adapters, bridges, shims, or backward-compatibility tax. `nebula-sdk` is the sole supported and branded Rust surface; its documented persona APIs receive a clear compatibility and deprecation policy.
- **Security by default.** Secrets are encrypted, zeroized, and redacted in Debug output. AAD binding is mandatory. There is no `legacy_compat` flag. The safe path is the only path.
- **Composition over inheritance.** Storage layers (encryption, cache, audit, scope) stack via trait delegation. Auth schemes are open traits, not closed enums. Resilience patterns compose into pipelines.

## Architecture

```
API / Surfaces  api (HTTP + webhook module) · sdk (supported Rust persona façade)
Exec            engine · orchestrator · worker · storage · storage-loom-probe
Business        resource · action · plugin · plugin-core · tenancy
Core/shared     core · validator · expression · workflow · execution · schema · metadata · storage-port · credential
Cross-cutting   crypto · env · log · eventbus · metrics · resilience · error
```

Plugins are **trusted, statically linked, in-process adapters** (ADR-0091): an integration author implements SDK-exposed contracts, the host binary links the integration crate, and startup registration adds its actions / credentials / resources to the registry. The engine dispatches them in-process. The retired out-of-process Plugin-Proto tier (`plugin-sdk` + `sandbox`) is not a compatibility or security boundary; process/WASM isolation is a non-goal (canon §12.6).

Each layer depends only on layers below it. Cross-cutting crates are importable at any level. Layer boundaries are enforced mechanically by `cargo deny` (see `deny.toml` `wrappers`) — a missing entry fails CI before review.

Every first-party deployment composition root in this workspace lives under `apps/`. `nebula-worker` is reusable runtime assembly that wires the engine to the orchestrator pull-loop; `apps/worker` chooses concrete storage, integrations, configuration, and process lifecycle. A downstream embedded host becomes a supported composition root only through the curated `nebula_sdk::embedded::RuntimeBuilder`; until that façade ships, embedding is not a supported deployment surface. The builder cannot replace or bypass runtime admission, aggregate write ownership, or tenant authority.

The binding architecture direction is recorded in private ADR-0116, *Adopt platform planes and profiled execution*. Versioned contracts, a pure transition kernel, the durable runtime control plane, trusted integration adapters, and user-facing surfaces have distinct ownership. The graph workflow runtime is the current flagship. Interactive, agent, and stream execution are future capability-gated profiles, not features claimed by this README.

Durable write authority is aggregate-scoped rather than concentrated in a god service. Runtime control owns the execution aggregate, its journal and queues, execution outbox/inbox, and operation ledger. Credential runtime owns credential/refresh/lease state; resource lifecycle owns resource/binding/fan-out state. Cross-aggregate work crosses durable persisted seams. `EventBus` can wake or inform an owner, but never commits a business fact.

Private ADR-0117, *Support one Rust SDK surface with lockstep dependency packages*, defines one persona-scoped `nebula-sdk`: workflow/authoring, integration, schema, testing, client, and embedded façades. Client and embedded support are curated safe surfaces when their documented features ship; they do not expose raw storage, durable mutation, admission, claim, or tenant-proof capabilities. Transport-contract and implementation crates remain technical lockstep dependencies, not additional supported Rust products.

### Data Flow

```
Trigger (webhook / cron / event)
  -> Activation validates and pins the workflow contract
    -> Runtime control accepts durable work
      -> Graph runtime schedules ready nodes
        -> Each node: Action::execute(Context) -> serde_json::Value
          -> Context provides guarded credentials, resources, parameters, and observability
            -> Durable effects persist; optional EventBus observations may wake readers
```

While strict Rust typing is enforced at the boundaries (inside Actions and Credentials), `serde_json::Value` is the universal interchange data type between nodes in the DAG. Nebula does not add a second universal value crate or permit ad hoc conversion chains: canonical, revision-pinned converters are allowed only at validated contract boundaries. Dates are ISO-8601 strings, decimals use a base64 convention. The `nebula-schema` runtime validation bridges the gap between the dynamic graph and strictly typed nodes.

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
|                   | `storage-port`  | Object-safe storage seam: row-model traits every storage consumer depends on (ADR-0072) |
|                   | `credential`    | Shared-infra credential subsystem (ADR-0092): contract + runtime (resolver/refresh/lease/rotation-state) + `CredentialService` facade + builtin types; 12 universal auth schemes |
| **Business**      | `resource`           | External service lifecycle, typed credential refs, per-slot rotation fan-out         |
|                   | `action`             | Action trait family (Stateless / Stateful / Trigger / Resource / Control)            |
|                   | `plugin`             | In-process plugin trait + registry                                                   |
|                   | `plugin-core`        | First-party `core` plugin: filter/sort/aggregate, reshaping, branching, datetime, durable delay |
|                   | `tenancy`            | Scope-enforcing decorator wrapping `storage-port` so a tenant scope is substituted on every call (ADR-0072) |
| **Exec**          | `engine`             | Frontier loop, lease lifecycle, node scheduling, control consumer (ADR-0008)         |
|                   | `orchestrator`       | Capability-routed job-dispatch pull loop (ADR-0095)                                  |
|                   | `worker`             | Reusable runtime assembly wiring a `WorkflowEngine` into the orchestrator pull-loop (ADR-0095 D1); not a deployment composition root |
|                   | `storage`            | Persistence adapters: SQLite/Postgres deployment backends plus internal InMemory test/reference conformance adapter |
|                   | `storage-loom-probe` | `loom`-checked concurrency probe for storage paths                                   |
| **API / Surfaces** | `api`               | REST server, webhook transport, middleware                                           |
|                    | `sdk`               | **Sole supported Rust façade** — persona-scoped workflow, integration, schema, testing, client, and embedded APIs as they ship |
| **Cross-cutting** | `error`              | `NebulaError<E>`, `Classify` trait, derive macro                                     |
|                   | `crypto`             | AES-256-GCM + Argon2id + `Cipher`/`Kdf` ports + `EncryptedData`/`key_id` envelope (extracted from credential per ADR-0088/0092) |
|                   | `env`                | Cross-cutting typed environment reader (ADR-0086)                                    |
|                   | `resilience`         | Retry, circuit breaker, rate limiter, hedge, bulkhead                                |
|                   | `log`                | Structured logging infrastructure                                                    |
|                   | `eventbus`           | Ephemeral typed observations for telemetry, cache/UI invalidation, and wake hints; never durable truth |
|                   | `metrics`            | Lock-free primitives + label interning + `nebula_*` naming + Prometheus export (absorbs the former `nebula-telemetry` crate per ADR-0046) |

## Quick Start

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build
cargo nextest run --workspace
```

Requires **Rust 1.96+** (edition 2024). Uses [cargo-nextest](https://nexte.st/) for test runs.

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

Nebula is in **active alpha development**. The core layer, credential system, resilience patterns, schema system, and error infrastructure are stable and well-tested. The execution engine and API layer are actively being wired together.

APIs will change. Not production-ready yet. See [AGENTS.md](AGENTS.md) for the workspace map, common commands, and contribution workflow.

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).
