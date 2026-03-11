# Nebula — Agent Guidelines

This file documents conventions and commands for AI coding agents working in this repository.

---

## Build & Check Commands

```bash
cargo build                                    # build all crates
cargo check --workspace --all-targets          # fast type-check (no codegen)
cargo fmt                                      # format all code in place
cargo fmt --all -- --check                     # CI: fail if not formatted
cargo clippy --workspace -- -D warnings        # CI: lint (warnings treated as errors)
cargo doc --no-deps --workspace                # build docs
```

## Test Commands

```bash
cargo test --workspace                         # run all tests
cargo test -p nebula-action                    # tests for a single crate
cargo test -p nebula-action test_name          # tests matching a name substring
cargo test -p nebula-action -- --exact retryable_error_is_retryable  # exact test name
cargo test -- --nocapture                      # show stdout during tests
cargo test --workspace --all-features          # with all feature flags
```

## Benchmark Commands

```bash
cargo bench -p nebula-log --bench log_hot_path  # run a single bench target
```

---

## Project Structure

This is a **Cargo workspace** (`resolver = "3"`, `edition = "2024"`, MSRV `1.93`) with ~25 crates
in `crates/`, all named `nebula-<noun>`. The Tauri desktop app lives in `apps/desktop/src-tauri/`
and is excluded from the workspace.

Key crates:
- `nebula-core` — IDs, keys, scopes (foundational types)
- `nebula-action` — action trait system
- `nebula-engine` — DAG scheduler
- `nebula-runtime` — action runner
- `nebula-storage` — KV abstraction
- `nebula-api` — Axum REST layer
- `nebula-resilience` — circuit breaker / retry
- `nebula-log` — tracing-based logging
- `nebula-parameter`, `nebula-workflow`, etc.

### Adding a New Crate

```bash
cargo new --lib crates/nebula-newname
```

Then add `"crates/nebula-newname"` to workspace `members` in the root `Cargo.toml`.
All `[package]` fields (version, edition, authors, license) must use `workspace = true` inheritance.

---

## Code Style

### Formatting

- `max_width = 100` (enforced by `rustfmt.toml`)
- `newline_style = "Unix"`
- `edition = "2024"`
- `reorder_imports = true`
- Run `cargo fmt` before every commit; CI fails on unformatted code.

### Imports

Order: `std::` → external crates → workspace/`crate::` items. Blank line between groups.
No wildcard imports in production code (`warn-on-all-wildcard-imports = true` in `clippy.toml`).

```rust
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::instrument;

use nebula_core::id::WorkflowId;
use crate::error::MyError;
```

### Naming Conventions

| Item | Convention |
|---|---|
| Types, traits, enums | `PascalCase` |
| Functions, methods, variables, modules | `snake_case` |
| Constants, const generics | `SCREAMING_SNAKE_CASE` |
| Crate names (`Cargo.toml`) | `kebab-case` |
| Action / plugin keys | `dot.notation` lowercase (`"http.request"`, `"math.add"`) |

### Module Structure (`lib.rs` template)

```rust
//! Crate-level doc comment.
//!
//! ## Quick Start
//! ...
//!
//! ## Core Types
//! ...

#![forbid(unsafe_code)]
#![warn(missing_docs)]

// ── Public modules ──────────────────────────────────────────────────────────
/// Brief description.
pub mod error;
pub mod prelude;

// ── Re-exports ───────────────────────────────────────────────────────────────
pub use error::MyError;

// ── Private modules ──────────────────────────────────────────────────────────
mod internal;
```

- Unit tests go at the bottom of each source file in `#[cfg(test)] mod tests { use super::*; }`.
- Integration tests → `tests/`, examples → `examples/`, benchmarks → `benches/`.
- Every crate exposes a `pub mod prelude` with star-importable common types.
- Section dividers use: `// ── Section Name ─────...`

---

## Error Handling

- Use **`thiserror`** for all library error types. Never use `anyhow` in library crates.
  (`anyhow` is acceptable only in application entry points or examples.)
- Every error enum must be `#[non_exhaustive]`.
- Provide factory constructors that accept `impl Into<String>`.
- Define per-crate type aliases: `pub type ActionResult<T> = Result<T, ActionError>;`
- Propagate errors with `?`; avoid `.unwrap()` / `.expect()` except in tests or genuinely
  unreachable branches (document why with a comment).
- Use `#[from]` for `From` conversions in aggregator errors.
- Add helper predicate methods where useful: `is_retryable()`, `is_fatal()`, `error_code()`.

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ActionError {
    #[error("execution failed: {message}")]
    ExecutionFailed { message: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ActionError {
    pub fn execution_failed(msg: impl Into<String>) -> Self {
        Self::ExecutionFailed { message: msg.into() }
    }
}

pub type ActionResult<T> = Result<T, ActionError>;
```

---

## Traits & Async

- Use **RPITIT** (`-> impl Future<Output = ...> + Send`) for non-object-safe async traits
  (Rust 2024 edition supports this natively).
- Use **`async-trait`** only when the trait must be object-safe (`dyn Trait`).
- Use `#[tokio::test]` for async tests.
- Cancellation is handled via `tokio_util::sync::CancellationToken` accessed through
  `Context::cancellation()`. Actions may poll it cooperatively but are not required to.

---

## Types & Patterns

- **Builder pattern**: consuming `self`, returning `Self`, every builder method marked `#[must_use]`.
- **Shared capabilities**: inject as `Arc<dyn Trait + Send + Sync>`.
- **Newtype IDs**: use the `domain-key` crate macros:
  - `define_uuid!(Domain => TypeName)` for UUID-based IDs
  - `key_type!(KeyName, Domain)` for string-scoped keys
- **Const generics** for compile-time configuration (see `nebula-resilience`).
- **`PhantomData`** for zero-cost type markers.

---

## Lints

Every library crate must have at the crate root:

```rust
#![forbid(unsafe_code)]
#![warn(missing_docs)]
```

Suppress lints with `#[expect(lint, reason = "...")]`, never `#[allow(...)]` without a reason.

`clippy.toml` thresholds:
- `too-many-lines-threshold = 100` (keep functions short)
- `cognitive-complexity-threshold = 25`
- `max-trait-bounds = 3`

---

## Documentation

- All public items require a `///` doc comment.
- `lib.rs` gets a `//!` module-level doc with `## Quick Start` and `## Core Types` sections.
- Use `# Examples`, `# Errors`, and `# Panics` sections in doc comments where applicable.
- Verify docs build cleanly with `cargo doc --no-deps --workspace`.

---

## Change Policy (Current Stage)

- The project is in an active development stage where **breaking changes are allowed** if they
  materially improve architecture, API clarity, or long-term maintainability.
- Prefer coherent API surfaces over temporary compatibility shims when a cleaner design is clear.
- When making a breaking change, update:
  - local crate docs and examples,
  - direct wrappers/adapters in dependent crates,
  - tests that validate the changed contract.
- Keep breaking changes intentional and well-scoped; avoid unrelated churn.

---

## CI Checklist

Before opening a PR, ensure all of the following pass locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
cargo doc --no-deps --workspace
```

CI additionally runs Miri (`miri.yml`) for UB checks and `cargo deny check` for supply-chain
auditing. Benchmark regression thresholds are validated against Criterion JSON output.

---

## AI Project Map

This section is setup-time context for AI agents and references `.ai-factory/DESCRIPTION.md`
and `.ai-factory/ARCHITECTURE.md` for deeper details.

### Project Overview

Nebula is a Rust-based workflow automation engine built as a layered Cargo workspace with
plugin-oriented extensibility and multiple interfaces (REST/WebSocket + Tauri desktop).

### Tech Stack

- **Language:** Rust (workspace, edition 2024)
- **Framework:** Axum + Tokio
- **Database:** PostgreSQL (migration-driven)
- **Desktop Frontend:** Tauri v2 + React + TypeScript (Vite)

### Project Structure

```text
nebula/
├── crates/                  # Workspace crates (core, business, runtime, API, etc.)
├── apps/                    # App surfaces (desktop Tauri app, web placeholder)
├── deploy/                  # Docker/Kubernetes deployment stacks
├── docs/                    # Technical docs, RFCs, roadmap material
├── migrations/              # PostgreSQL schema migrations
├── scripts/                 # Benchmark and utility scripts
├── vision/                  # Architecture/decision/status references
├── .ai-factory/             # AI Factory context artifacts
│   ├── DESCRIPTION.md
│   └── ARCHITECTURE.md
└── AGENTS.md                # Agent guidance + project map
```

### Key Entry Points

| File | Purpose |
|------|---------|
| `Cargo.toml` | Workspace definition and shared dependencies |
| `crates/api/` | HTTP/WebSocket API layer (Axum) |
| `crates/engine/` | DAG scheduling/orchestration logic |
| `crates/runtime/` | Runtime execution and trigger lifecycle |
| `apps/desktop/package.json` | Desktop frontend scripts and Tauri integration |
| `.mcp.json` | Project MCP server configuration |

### Documentation

| Document | Path | Description |
|----------|------|-------------|
| README | `README.md` | Project landing page |
| Getting Started | `docs/getting-started.md` | Installation, onboarding, first run |
| Architecture | `docs/ARCHITECTURE.md` | Layering, crate map, data flow |
| Project Status | `docs/PROJECT_STATUS.md` | Current implementation status |
| Roadmap | `docs/ROADMAP.md` | Phases, priorities, dependencies |
| Tasks | `docs/TASKS.md` | Cross-crate execution backlog |
| Contributing | `docs/contributing.md` | Contribution standards and setup |
| Workflow | `docs/workflow.md` | Branching, commits, PR process |
| Issues | `docs/issues.md` | Issue templates and triage |
| Labels | `docs/labels.md` | Label taxonomy and conventions |
| Project Board | `docs/project-board.md` | Board workflow and policies |

### AI Context Files

| File | Purpose |
|------|---------|
| `AGENTS.md` | Agent instructions + setup-time project map |
| `.ai-factory/DESCRIPTION.md` | Project specification and detected stack |
| `.ai-factory/ARCHITECTURE.md` | Architecture pattern and dependency rules |
| `CLAUDE.md` | Working memory, ADRs, and active constraints |

### Agent Rules

- Never combine shell commands with `&&`, `||`, or `;` when executing workflow steps.
- Keep dependency changes aligned with the one-way layer architecture.
- Prefer eventbus-based decoupling for cross-crate signals.
