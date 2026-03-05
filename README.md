<div align="center">

# 🌌 Nebula

**A modular, high-performance workflow automation engine — built in Rust.**

[![CI](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml/badge.svg)](https://github.com/vanyastaff/nebula/actions/workflows/ci.yml)
[![Security Audit](https://github.com/vanyastaff/nebula/actions/workflows/security-audit.yml/badge.svg)](https://github.com/vanyastaff/nebula/actions/workflows/security-audit.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.93%2B-orange.svg)](https://www.rust-lang.org)

[Getting Started](#getting-started) · [Architecture](#architecture) · [Roadmap](docs/ROADMAP.md) · [Contributing](CONTRIBUTING.md) · [Security](.github/SECURITY.md)

</div>

---

## What is Nebula?

Nebula is a **workflow automation engine** — think n8n or Zapier, but built from the ground up in Rust for performance, type safety, and extensibility.

Workflows are **directed acyclic graphs (DAGs)** of *actions*. Actions are typed Rust implementations that can send HTTP requests, query databases, call APIs, trigger external systems, or do any custom computation.

```
User defines a workflow:

  trigger (webhook / cron / event)
      │
      ▼
  [HTTP Request] ──▶ [Transform JSON] ──▶ [Send Slack message]
      │
      ▼ (on error)
  [Alert on-call]

Nebula runs it reliably:
  ✔ Schedules the DAG
  ✔ Injects credentials and resources
  ✔ Enforces isolation, retries, and timeouts
  ✔ Persists state to PostgreSQL
  ✔ Exposes real-time progress via WebSocket
  ✔ Provides a visual editor (Tauri desktop app)
```

### Core Properties

| Property | Description |
|----------|-------------|
| **Type-safe** | Rust's compiler catches wrong-ID bugs, missing credentials, and invalid state transitions at compile time |
| **Modular** | 26 focused crates with strict one-way dependency rules — add a new action without touching the engine |
| **Async-first** | Built on Tokio: concurrent node fan-out, bounded work queues, cooperative cancellation |
| **Storage-agnostic** | In-memory for tests; PostgreSQL for production — same API |
| **Extensible** | First-party plugins (GitHub, Telegram) and third-party via `nebula-plugin` |

---

## Getting Started

### Prerequisites

| Tool | Version |
|------|---------|
| [Rust](https://rustup.rs) | 1.93 or later |
| [Cargo](https://doc.rust-lang.org/cargo/) | bundled with Rust |
| [PostgreSQL](https://www.postgresql.org/) | 14+ (for production; optional for dev) |

### Build

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula

# Build all crates
cargo build

# Run tests
cargo test --workspace

# Check formatting and lints
cargo fmt --check
cargo clippy -- -D warnings
```

### Development Quick Start

```bash
# Copy environment config
cp deploy/.env.example deploy/.env

# Run a specific crate's tests
cargo test -p nebula-validator

# Build documentation
cargo doc --no-deps --open
```

---

## Architecture

Nebula is organised into **five layers** — each layer only depends on layers below it:

```
┌─────────────────────────────────────────────────────────┐
│  Applications  │  api · desktop (Tauri) · web           │
├─────────────────────────────────────────────────────────┤
│  Engine        │  engine · runtime · scheduler          │
├─────────────────────────────────────────────────────────┤
│  Business      │  action · credential · plugin          │
├─────────────────────────────────────────────────────────┤
│  Infrastructure│  storage · resource · resilience       │
├─────────────────────────────────────────────────────────┤
│  Foundation    │  core · workflow · execution · memory  │
└─────────────────────────────────────────────────────────┘
```

Full architecture details → [`vision/ARCHITECTURE.md`](vision/ARCHITECTURE.md)  
Crate responsibilities → [`vision/CRATES.md`](vision/CRATES.md)  
Dependency graph → [`vision/DEPENDENCIES.md`](vision/DEPENDENCIES.md)

---

## Workspace Structure

```
nebula/
├── crates/              # Rust library crates (26 members)
│   ├── core/            # Identifiers, scope, shared traits
│   ├── workflow/        # Workflow definition + DAG model
│   ├── execution/       # Execution state machine
│   ├── action/          # Action trait + execution contract
│   ├── engine/          # DAG scheduler and orchestrator
│   ├── runtime/         # Action runner, isolation, task queue
│   ├── storage/         # KV storage abstraction
│   ├── credential/      # Encrypted secrets + rotation
│   ├── resource/        # Resource lifecycle + pooling
│   ├── api/             # REST + WebSocket server (axum)
│   └── ...              # See vision/CRATES.md for full list
├── apps/
│   ├── desktop/         # Tauri app (React + TypeScript + Rust)
│   └── web/             # Web frontend
├── docs/                # Detailed documentation per crate
├── vision/              # Project-level navigation hub
├── migrations/          # SQL database migrations
└── deploy/              # Deployment configuration
```

---

## Project Status

**Current phase:** 🔄 Phase 2 — Execution Engine (active development)

| Phase | Status | Description |
|-------|--------|-------------|
| 1 — Core Foundation | ✅ Complete | Base crates: core, workflow, execution, memory, validator, etc. |
| 2 — Execution Engine | 🔄 Active | Action trait, DAG engine, runtime, PostgreSQL storage backend |
| 3 — Credential & Plugin | ⬜ Planned | Secure credential storage, plugin loading, webhooks |
| 4 — Developer Experience | ⬜ Planned | SDK, CLI, OpenAPI, testing framework |
| 5 — API & UI | ⬜ Planned | REST/WebSocket API, visual workflow editor (Tauri) |

Full roadmap → [`docs/ROADMAP.md`](docs/ROADMAP.md)  
Per-crate status → [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md)

---

## Contributing

Contributions of all kinds are welcome — bug reports, feature requests, documentation, and code.

1. Read the [Contributing Guide](CONTRIBUTING.md)
2. Browse [open issues](https://github.com/vanyastaff/nebula/issues)
3. Check the [Roadmap](docs/ROADMAP.md) for planned work
4. Open an issue before starting large changes

Please follow the [commit convention](CONTRIBUTING.md#commit-messages) (`feat`, `fix`, `docs`, etc.) and ensure all CI checks pass before requesting review.

---

## License

Nebula is licensed under the **MIT License** — see [LICENSE](LICENSE) for details.
