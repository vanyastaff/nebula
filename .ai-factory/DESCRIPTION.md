# Nebula — Project Description

> The README at the repo root is the canonical product / architecture document.
> This file is the agent-facing summary. **If they disagree, README wins** —
> update this file rather than the README.

## Overview

Nebula is a modular, type-safe **workflow automation engine** written in Rust —
in the same space as n8n, Zapier, and Temporal, but built as a composable
library rather than a monolithic platform. Teams embed Nebula into their own
infrastructure, extend it with custom integrations, and trust it with
production secrets.

**Status:** core crates are stable and well-tested; the execution engine and
API layer are in active development. Not production-ready yet.

## Tech Stack

- **Language:** Rust 1.95+ (MSRV pinned via `workspace.package.rust-version`),
  edition 2024, resolver 3.
- **Workspace:** 35+ crates under `crates/` (see `Cargo.toml` `[workspace]`
  members) — see `AGENTS.md` for the layered map.
- **Async runtime:** Tokio (multi-thread).
- **Serialization:** `serde` / `serde_json`.
- **Errors:** `thiserror` in libraries, `anyhow` in binaries.
- **Observability:** `tracing` + crate-local `nebula-log`, `nebula-telemetry`,
  `nebula-metrics`.
- **Concurrency primitives:** `dashmap`, `parking_lot`, `arc-swap`, `moka`,
  custom resilience patterns in `nebula-resilience`.
- **Storage:** abstract `nebula-storage` with `loom`-checked probes and
  PostgreSQL / SQLite backends (`crates/storage/migrations/`).
- **Sandboxing:** `nebula-sandbox` for untrusted action execution.

## Architecture (high-level)

Layered workspace, enforced mechanically by `cargo deny` `[wrappers]` in
`deny.toml`:

```
API / Public    api · sdk
Exec            engine · storage · sandbox · plugin-sdk
Business        credential · resource · action · plugin
Core            core · validator · expression · workflow · execution · schema · metadata
Cross-cutting   log · system · eventbus · telemetry · metrics · resilience · error
```

Each layer depends only on layers below; cross-cutting crates are importable
anywhere. Cross-crate communication goes through `nebula-eventbus`, not direct
imports between siblings.

Detailed architecture and design principles live in **README.md** (sections
"Why Nebula", "Design Principles", "Architecture") — `.ai-factory/ARCHITECTURE.md`
captures the agent-actionable subset.

**Pattern:** Layered Modular Workspace (Cargo wrapper-enforced).

## Core Features

- **Type-safe DAG workflows.** Workflow shape, action I/O, parameter schemas,
  and auth patterns are Rust types. If it compiles, the shape is valid.
- **First-class credentials.** AES-256-GCM encryption with AAD binding,
  zeroization on `Drop`, key rotation built into the storage layer.
- **Composable resilience.** Retry, circuit breaker, bulkhead, hedged requests,
  rate limiting — typed errors, purpose-built for the engine's concurrency
  model.
- **Strict modularity.** One-way layer dependencies enforced in CI; embed any
  individual crate without pulling the whole stack.
- **Plugin SDK + sandbox.** Third-party actions run through a typed plugin
  interface and an isolation boundary.

## Non-Functional Requirements

- **Logging:** `tracing`-based, configurable via `RUST_LOG` and crate-local
  config; structured fields preferred over string interpolation.
- **Error handling:** typed errors (`thiserror`) end-to-end in libraries; every
  new error variant must carry enough context for a recovery decision.
- **Observability as DoD:** every new state, error, or hot path ships with a
  typed error + tracing span + invariant check, not as a follow-up.
- **Security:** secrets never logged in plaintext, redacted in `Debug`, AAD
  binding mandatory; security-sensitive paths gated by `CODEOWNERS`.
- **Performance:** zero-warning clippy with raised complexity thresholds; loom
  probes for concurrency-critical storage paths; benches in
  `crates/<crate>/benches/`.

## Pointers

- `README.md` — product overview, design principles, full architecture.
- `CONTRIBUTING.md` — workflow, branch naming, commits, PR rules.
- `AGENTS.md` — workspace map and key entry points (agent-facing).
- `.ai-factory/rules/base.md` — distilled coding rules for agents.
- `.ai-factory/ARCHITECTURE.md` — agent-actionable architecture subset.
- `Cargo.toml` (root) — workspace members and pinned dependency versions.
- `deny.toml` — layer-boundary enforcement and license/advisory policy.
- `clippy.toml`, `rustfmt.toml` — lint and formatting config.
- `Taskfile.yml` — `task --list` for the full developer command catalog.
- `lefthook.yml` — local pre-commit / pre-push hooks (mirrors CI required jobs).
