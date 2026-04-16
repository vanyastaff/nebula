# Spec 01 — Product positioning

> **Status:** draft
> **Canon target:** §2 (extend), §5 (scope table update)
> **Depends on:** —
> **Depended on by:** all other specs (positioning is the base assumption)

## Problem

Nebula needs a single, explicit target user answer. Without it, every architectural decision can be rationalised in two contradictory directions. Competitors that tried to serve «both solo and enterprise» without explicit positioning (Airflow, early Temporal) ended up with incoherent product stories.

## Decision

Nebula is positioned as **D — OSS self-host single-binary for solo operators and small teams, plus managed cloud service at `nebula.io` for those who want it hosted**. Reference model is n8n: the same Rust binary runs in both modes; cloud adds tenant isolation and managed infrastructure but not different features.

## Target users

**Primary (v1):**
- Solo developer / small team (1–10 people) self-hosting one binary
- 50–500 active workflows
- SQLite or Postgres on same host
- Install through `cargo install` / `docker run` / downloaded binary
- Target pain: «I need to connect 5 services without standing up Temporal infra»

**Primary (v1.5+):**
- Cloud customers at `nebula.io` (managed Postgres, our infrastructure)
- Tenant isolation via Org → Workspace model (see spec 02)
- Billing / plans / rate limiting

**Secondary (v2+):**
- Self-hosted teams with multiple internal teams — same binary, Org → multiple Workspaces
- Eventually enterprise with SSO, audit requirements, compliance

**Non-targets for v1:**
- Enterprise-first customers requiring SAML SSO from day one
- Kubernetes-only deployments (K8s is supported but not required)
- Customers requiring exactly-once semantics on side effects
- Customers who want to write workflows in Python/JS (actions are Rust)
- Customers who need real-time streaming pipelines (batch + scheduled + triggered is our sweet spot)

## Architectural implications

**Single binary.** The same compiled artefact runs as self-host, cloud worker, and eventually embedded library. No feature flags at compile time — all differences are runtime configuration.

**Single-process by default.** `cargo run` / `docker run nebula` with no extra services required. SQLite file, in-memory rate limiting, in-memory session store. Multi-process coordination is opt-in by switching to Postgres.

**Contracts ready for multi-process.** Even though v1 default is single-process, primitives (lease, CAS, claim query — see specs 16, 17) are designed to work correctly in multi-process. This means the day we scale out, no rewrites — only config change + operational setup.

**Zero external dependencies for the default path.** Canon §12.3 is explicit: no mandatory Docker, Redis, Kafka, external OIDC provider. A Nebula binary + storage backend is a complete system.

**OSS first, cloud second.** Cloud is a deployment of the same code, not a parallel product. No cloud-only features in v1. Enterprise features (SSO, RBAC custom roles, audit retention beyond 90 days) may become paid in later versions, but the core product is fully featured in OSS.

## Scope boundaries — what Nebula is NOT

These are listed in canon §8 but repeated here because positioning is the source of truth:

- **Not a BI / analytics tool** — if workflow needs to compute statistics, call an external service
- **Not a stream processor** — Kafka Streams / Flink / Apache Beam territory, not ours
- **Not a code execution sandbox** — plugins run native code under OS isolation, not WASM (canon §12.6)
- **Not a low-code platform** — authoring happens in Rust; visual editor composes existing actions, does not generate Rust
- **Not a managed CI/CD system** — overlap exists (run tests on schedule) but we are not competing with GitHub Actions on breadth
- **Not a database** — we store execution state but are not a general-purpose datastore
- **Not a message broker** — `nebula-eventbus` is in-process, not a replacement for Kafka/RabbitMQ

## Competitive positioning

| Competitor | Overlap | Differentiation |
|---|---|---|
| **n8n** | Target user, OSS + cloud model, trigger model | Typed Rust integrations, real durability contracts, honest retry/cancel semantics |
| **Windmill** | OSS + cloud, local-first ergonomics | Rust integration model (they are Python/TS scripts), typed parameter schema |
| **Airflow** | DAG execution, scheduling, batch orientation | Simpler operational story, no scheduler/worker split, no Python requirement |
| **Temporal** | Durable execution, retry, long-running workflows | Much simpler operational model, opinionated defaults, OSS-first |
| **Zapier** | Integration marketplace, no-code feel | Self-host, typed contracts, developer-first |
| **Make (Integromat)** | Visual workflow builder | Self-host, open source |

## Success criteria

**Solo developer success path (v1 target):**

1. Visits `nebula.io`, downloads binary or runs `docker run`
2. Opens `localhost:8080`, sees «Create owner account» form
3. Creates account, lands on empty dashboard in `default/default` workspace
4. Creates a workflow visually (or via YAML), wires 3 actions (e.g., webhook → transform → Slack)
5. Tests manually, sees green, activates
6. Workflow fires from webhook, runs end-to-end, visible in execution list with timeline
7. Total time from download to first live execution: **< 15 minutes**

**Cloud customer success path (v1.5 target):**

1. Signs up at `nebula.io` with email or OAuth
2. Lands in their org / workspace
3. Imports workflow from self-host OR builds new
4. Connects credentials through in-product OAuth flow
5. Activates workflow
6. Monitors executions with live-updating timeline
7. Total time from signup to first live execution: **< 5 minutes** (no infra setup)

## Configuration surface

```toml
[deployment]
mode = "self-host"  # or "cloud" or "embedded"
# Changes defaults for quotas, telemetry, auth requirements

[storage]
backend = "sqlite"  # or "postgres"
url = "sqlite:///var/lib/nebula/db.sqlite"
# or: url = "postgres://user:pass@host/nebula"

[auth]
mode = "built-in"  # or "none" for local dev
# No "external" mode in v1 — spec 03
```

Defaults differ by `mode`:

| Setting | `self-host` | `cloud` | `embedded` |
|---|---|---|---|
| Max concurrent executions per workspace | 100 | 50 (free plan) / 500 (paid) | unlimited |
| Storage quota | unlimited | 1 GB free / 100 GB paid | unlimited |
| Active workflows per workspace | unlimited | 50 free / unlimited paid | unlimited |
| Telemetry | opt-out (enabled) | disabled (we already know you) | disabled |
| Signup form | enabled after owner created | open with email verification | disabled |
| Rate limits | relaxed | strict per tenant | none |
| TLS required | false | true | n/a |

## Testing criteria

- **E2E self-host install test:** CI runs `cargo install`, creates owner, creates workflow, triggers execution, verifies completion
- **E2E Docker test:** `docker run nebula:latest` + health check + smoke test
- **E2E cloud mode test:** starts in cloud mode with Postgres + mock OAuth, walks through signup flow
- **Non-regression test:** single binary size stays < 100 MB (release build)
- **Non-regression test:** `nebula --help` completes in < 200 ms cold start

## Performance targets

These are **positioning targets**, not implementation SLOs (those live per-subsystem):

- Cold start to first request handled: **< 2 seconds** self-host
- Single-process throughput: **≥ 100 executions/second** steady state on 4-core workstation
- Memory footprint idle: **< 200 MB** RSS
- Memory per active execution: **< 500 KB** average

## Open questions

- **Embedded mode** — is Nebula usable as a library inside another Rust app, or only as a daemon? Listed as «embedded» deployment mode but not designed in detail. Deferred to v2.
- **Desktop app** — `apps/desktop` is mentioned in workspace layout; is it first-class or an experiment? Deferred.
- **Enterprise plan differentiation** — which features become paid-only? Deferred to when we have first paying customer asking.
