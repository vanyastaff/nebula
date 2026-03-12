# Nebula — Vision Hub

> **One place to understand the project, its state, and where it's going.**

Nebula is a modular, open-source workflow automation engine written in Rust — think n8n or Zapier, but built for performance, type safety, and extensibility. Workflows are directed acyclic graphs (DAGs) of *actions*. Actions are typed Rust implementations that can send HTTP requests, query databases, call APIs, or trigger downstream systems.

---

## Quick Navigation

| Document | Purpose |
|----------|---------|
| **[ARCHITECTURE.md](./ARCHITECTURE.md)** | Crate layers, dependency rules, data flow, async conventions |
| **[CRATES.md](./CRATES.md)** | Purpose and responsibility of every crate in the workspace |
| **[DEPENDENCIES.md](./DEPENDENCIES.md)** | Full inter-crate dependency map, Mermaid graph, blast-radius table |
| **[DECISIONS.md](./DECISIONS.md)** | Architectural decision records (why Rust, why serde_json::Value, etc.) |

---

## What Is Nebula?

```
User defines a workflow:
  trigger (webhook / cron / event)
      ↓
  node A: HTTP Request  → node B: Transform JSON  → node C: Send Slack message
      ↓ (on error branch)
  node D: Alert on-call

Nebula runs it reliably:
  – schedules the DAG
  – injects credentials and resources
  – enforces isolation, retries, timeouts
  – persists state to PostgreSQL
  – exposes real-time progress via WebSocket
  – provides a visual editor (Tauri desktop app)
```

### Core Properties

- **Type-safe**: Rust's compiler catches wrong-ID bugs, missing credentials, invalid state transitions.
- **Modular**: 25 focused crates with one-way dependencies. Add a new action without touching the engine.
- **Async-first**: Built on Tokio. Concurrent node fan-out, bounded work queues, cooperative cancellation.
- **Storage-agnostic**: In-memory for tests; PostgreSQL for production. Same API.
- **Extensible**: First-party plugins (GitHub, Telegram) and third-party via `nebula-plugin`.

---

## Current State (March 2026)

**Phase 1 (Core Foundation) — ✅ Complete**
All foundation crates are implemented and tested.

**Phase 2 (Execution Engine) — 🔄 Active**
Action trait, resource lifecycle, DAG engine, runtime — all in progress.
Blocked on: PostgreSQL storage backend (storage Phase 1).

**Phase 3–5 — ⬜ Planned**
Credential system hardening, plugin ecosystem, SDK, Desktop app completion.

See [docs/PROJECT_STATUS.md](../docs/PROJECT_STATUS.md) for per-crate detail.

---

## Workspace Layout

```
nebula/
├── crates/                 # Rust library crates (25 members)
│   ├── core/               # IDs, scope, shared traits
│   ├── workflow/           # Workflow definition + graph model
│   ├── execution/          # Execution state machine
│   ├── action/             # Action trait + execution contract
│   ├── engine/             # DAG scheduler and orchestrator
│   ├── runtime/            # Action runner, isolation, task queue
│   ├── storage/            # KV storage abstraction
│   ├── credential/         # Encrypted secrets + rotation
│   ├── resource/           # Resource lifecycle + pooling
│   ├── auth/               # Authentication and authorization (RFC phase)
│   ├── api/                # REST + WebSocket server (axum)
│   └── ...                 # See ARCHITECTURE.md for full list
├── apps/
│   ├── desktop/            # Tauri app (React + TypeScript + Rust)
│   └── web/                # Web frontend
├── docs/                   # Per-crate detailed documentation
├── vision/                 # ← You are here: project-level navigation
├── migrations/             # SQL database migrations
└── deploy/                 # Deployment configuration
```

---

## Where to Start

**I want to understand the code structure** → [ARCHITECTURE.md](./ARCHITECTURE.md)

**I want to know what a specific crate does** → [CRATES.md](./CRATES.md)

**I want to see what depends on what** → [DEPENDENCIES.md](./DEPENDENCIES.md)

**I want to know what to work on next** → [docs/ROADMAP.md](../docs/ROADMAP.md) → "Recommended next tasks"

**I want to know if a crate is ready to use** → [docs/PROJECT_STATUS.md](../docs/PROJECT_STATUS.md)

**I want to understand a design decision** → [DECISIONS.md](./DECISIONS.md)

**I want to contribute** → [docs/contributing.md](../docs/contributing.md)

**I want deep crate-level detail** → `docs/crates/<crate>/` (README, ARCHITECTURE, API, TASKS)
