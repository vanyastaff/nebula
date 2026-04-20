# Nebula

[CI](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml)
[Rust](https://www.rust-lang.org/)
[License](LICENSE)
[CodSpeed](https://codspeed.io/vanyastaff/nebula)

**Modular, type-safe workflow automation engine in Rust.**

Nebula is a DAG-based automation engine in the same category as n8n, Zapier, and Temporal &mdash; but built from scratch in Rust as a composable library, not a monolithic platform. It is designed for teams that want workflow automation they can embed, extend, and trust with production credentials.

Alpha stage: core crates are stable, execution engine and desktop app are in active development.

---

## Why Nebula

Most automation platforms are runtime-interpreted, dynamically typed, and treat security as an afterthought. Nebula takes a different path.

**Credentials are a first-class concern, not a bolt-on.** Every secret is encrypted at rest with AES-256-GCM, bound to its record via AAD to prevent swapping attacks, and wiped from memory on drop. Key rotation is built into the storage layer &mdash; not a future feature. The credential system went through 10 adversarial review rounds, 2 dev challenges, and SOC2 grading before shipping.

**The type system does the work.** Workflow structure, action I/O, parameter schemas, auth patterns &mdash; all expressed as Rust types. If a workflow compiles, its shape is valid. There are no stringly-typed action references, no untyped credential bags, no "any" escape hatches in the core pipeline.

**Resilience is not optional.** Retry with backoff, circuit breakers, rate limiting, hedged requests, and bulkhead isolation are composable building blocks in `nebula-resilience`. Every pattern returns `CallError<E>` with enough context to decide what to do next. These aren't wrappers around another library &mdash; they're purpose-built, audited (153 tests, 14 integration tests, 7 benchmark suites), and designed for the engine's concurrency model.

**Modularity is a hard requirement.** The workspace follows one-way layer dependencies with selected checks enforced by `cargo deny` in CI and the rest enforced in review. Cross-crate communication goes through `EventBus`, not direct imports. You can use `nebula-credential` without touching `nebula-engine`. You can embed `nebula-resilience` in a project that has nothing to do with workflows.

## Design Principles

- **Types over tests.** Make invalid states unrepresentable. Use newtypes for IDs, enums for states, builders for validated config. Tests verify behavior, not type safety &mdash; the compiler handles that.
- **Explicit over magic.** No global state, no hidden service locators, no ambient configuration. Actions receive everything they need via `Context`. If a dependency isn't in the function signature, it doesn't exist.
- **Delete over deprecate.** When an API is wrong, replace it. No adapters, bridges, shims, or backward-compatibility tax. Migration cost is acceptable if the design is right.
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

`serde_json::Value` is the universal data type. No custom value crate, no conversion layers. Dates are ISO-8601 strings, decimals use a base64 convention.

## Crate Map

Source of truth: workspace members in `Cargo.toml`, status in [`docs/MATURITY.md`](docs/MATURITY.md).

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

### Apps

- `apps/cli` — `nebula` CLI (in-process one-shot runs, includes optional `--tui` viewer).
- `apps/desktop` — Tauri + React reference shell (not a release artifact).
- `apps/web` — placeholder (no implementation yet).

A production composition root (`apps/server`) for the `mode-self-hosted` deployment shape (ADR-0013) is tracked as ADR-0008 follow-up.

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
task desktop:dev    # Launch Tauri desktop app in dev mode
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

## Documentation

| Doc                                                        | Description                                                                       |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------- |
| [docs/PRODUCT_CANON.md](docs/PRODUCT_CANON.md)             | Normative core — pillars, golden path, contracts, non-negotiable invariants       |
| [docs/MATURITY.md](docs/MATURITY.md)                       | Per-crate stability dashboard (`stable` / `frontier` / `partial` / `planned`)     |
| [docs/STYLE.md](docs/STYLE.md)                             | Rust idioms, naming, error taxonomy                                               |
| [docs/INTEGRATION_MODEL.md](docs/INTEGRATION_MODEL.md)     | Integration model: Action / Credential / Resource / Schema / Plugin               |
| [docs/adr/](docs/adr/)                                     | Architectural decision records (numbered, immutable once accepted)                |
| [CLAUDE.md](CLAUDE.md)                                     | Coding-agent operational guidance + canonical commands                            |
| [CONTRIBUTING.md](CONTRIBUTING.md)                         | Setup and contribution flow                                                       |
| [docs/plans/](docs/plans/)                                 | Active implementation plans and archive                                           |


## Status

Nebula is in **active alpha development**. The core layer, credential system, resilience patterns, parameter system, and error infrastructure are stable and well-tested. The execution engine, runtime, and API layer are being wired together. The desktop app (Tauri) is in early development.

This is not production-ready yet. APIs will change. But the foundation is solid, and the direction is clear.

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).