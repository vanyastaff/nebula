# AGENTS.md

> Map of the Nebula workspace for AI agents and new contributors. Keep factual.
> Update when crates / top-level layout / required files change. Detailed
> product / architecture content lives in `README.md` — do not duplicate it
> here.

## Project Overview

Nebula is a modular, type-safe **workflow automation engine** in Rust. See
`README.md` for the product overview and `.ai-factory/DESCRIPTION.md` for the
agent-facing summary.

## Tech Stack

- **Language:** Rust 1.95+ (edition 2024, resolver 3)
- **Async:** Tokio
- **Errors:** `thiserror` (libs) / `anyhow` (bins)
- **Storage backends:** PostgreSQL, SQLite (`crates/storage/migrations/`)
- **Testing:** `cargo nextest` + doctests
- **Build orchestration:** `Taskfile.yml` (`task --list`)
- **Local hooks:** `lefthook.yml` (mirrors CI required jobs)

## Workspace Layout

```
nebula/
├── Cargo.toml                  # workspace root, pinned deps
├── README.md                   # product overview, architecture, principles
├── CONTRIBUTING.md             # branch / commit / PR rules
├── AGENTS.md                   # this file
├── CODE_OF_CONDUCT.md
├── LICENSE                     # MIT OR Apache-2.0
├── clippy.toml                 # lint config (msrv 1.95)
├── rustfmt.toml                # nightly rustfmt config
├── rust-toolchain.toml         # pinned toolchain
├── deny.toml                   # cargo-deny: layer wrappers + advisories
├── lefthook.yml                # pre-commit / pre-push hooks
├── Taskfile.yml                # task runner (developer commands)
├── typos.toml, _typos.toml     # typo-checker config
├── .taplo.toml                 # TOML formatter
├── .coderabbit.yaml            # CodeRabbit review config
├── .editorconfig
├── .ai-factory/                # AI Factory artifacts (config, rules, plans)
│   ├── config.yaml             # AI Factory configuration
│   ├── DESCRIPTION.md          # agent-facing project summary
│   ├── ARCHITECTURE.md         # agent-actionable architecture subset
│   └── rules/base.md           # distilled coding rules for agents
├── .ai-factory.json            # AI Factory install manifest (skills/agents)
├── .claude/                    # Claude Code skills + agents
│   ├── skills/                 # /aif-* skills
│   └── agents/                 # subagents (sidecars, workers, loop roles)
├── .github/
│   ├── CODEOWNERS              # auto-reviewer mapping
│   ├── PULL_REQUEST_TEMPLATE.md
│   ├── SECURITY.md
│   ├── PROJECT_SETUP.md
│   ├── copilot-instructions.md
│   ├── workflows/              # CI pipelines
│   ├── ISSUE_TEMPLATE/
│   ├── labeler.yml
│   └── dependabot.yml
└── crates/                     # 35+ workspace members
    ├── core/                   # Core    — primitives, IDs, traits
    ├── error/  +/macros/       # Cross   — error taxonomy
    ├── log/                    # Cross   — tracing sinks / formatters
    ├── system/                 # Cross   — process / runtime utilities
    ├── eventbus/               # Cross   — typed cross-crate event bus
    ├── telemetry/              # Cross   — OTLP / OpenTelemetry
    ├── metrics/                # Cross   — counters / gauges / histograms
    ├── resilience/             # Cross   — retry, circuit breaker, hedged, …
    ├── validator/  +/macros/   # Core    — value validation
    ├── expression/             # Core    — expression language
    ├── workflow/               # Core    — DAG workflow definition
    ├── execution/              # Core    — execution model
    ├── schema/  +/macros/      # Core    — typed schemas
    ├── metadata/               # Core    — workflow metadata
    ├── credential/ +/macros/   # Business — secrets, AES-256-GCM + AAD
    ├── credential-builtin/     # Business — built-in credential types
    ├── resource/  +/macros/    # Business — resource pools / lifecycle
    ├── action/    +/macros/    # Business — action contract + builtins
    ├── plugin/    +/macros/    # Business — plugin contract
    ├── plugin-sdk/             # Exec    — third-party plugin SDK surface
    ├── engine/                 # Exec    — orchestration / execution engine
    ├── storage/                # Exec    — storage abstraction (PG / SQLite)
    ├── storage-loom-probe/     # Exec    — loom-checked concurrency probe
    ├── sandbox/                # Exec    — isolation for untrusted actions
    ├── api/                    # API     — HTTP + webhook layer
    └── sdk/       +/macros-support/  # API — integration-author façade
```

## Layered Dependency Map

Mechanically enforced by `cargo deny check` against `deny.toml` `[wrappers]`.
Each layer depends only on layers below; cross-cutting crates are importable at
any level.

| Layer        | Crates |
|--------------|--------|
| API / Public | `api`, `sdk` |
| Exec         | `engine`, `storage`, `storage-loom-probe`, `sandbox`, `plugin-sdk` |
| Business     | `credential`, `credential-builtin`, `resource`, `action`, `plugin` |
| Core         | `core`, `validator`, `expression`, `workflow`, `execution`, `schema`, `metadata` |
| Cross-cutting| `log`, `system`, `eventbus`, `telemetry`, `metrics`, `resilience`, `error` |

Cross-crate communication goes through `nebula-eventbus`, **not** direct imports
between siblings at the same layer.

## Key Entry Points

| File                          | Purpose |
|-------------------------------|---------|
| `Cargo.toml`                  | Workspace members, pinned deps, `[workspace.lints]` |
| `deny.toml`                   | Layer wrappers, licenses, advisories — CI gate |
| `clippy.toml`                 | Lint thresholds (msrv 1.95) |
| `rustfmt.toml`                | Nightly rustfmt config |
| `rust-toolchain.toml`         | Pinned toolchain |
| `lefthook.yml`                | Local pre-commit / pre-push (mirrors CI) |
| `Taskfile.yml`                | `task dev:check` = full pre-PR gate; `task --list` for catalog |
| `.github/workflows/ci.yml`    | CI required jobs: fmt, clippy, nextest, doctests, MSRV, deny |
| `.github/CODEOWNERS`          | Auto-reviewer + security-sensitive path gates |
| `crates/<crate>/README.md`    | Per-crate human entry point |
| `crates/<crate>/Cargo.toml`   | Per-crate features, deps, lints |

## Documentation Index

| Document                       | Path                          | Description |
|--------------------------------|-------------------------------|-------------|
| Product overview               | `README.md`                   | What Nebula is, design principles, architecture |
| Contribution guide             | `CONTRIBUTING.md`             | Quick start, workflow, branch / commit / PR rules |
| Code of conduct                | `CODE_OF_CONDUCT.md`          | Community standards |
| Security policy                | `.github/SECURITY.md`         | Reporting vulnerabilities |
| Per-crate READMEs              | `crates/<crate>/README.md`    | Crate-level usage and design notes |
| Per-crate design docs          | `crates/<crate>/docs/`        | Where present (e.g. `log`, `resilience`, `validator`, `resource`, `api`, `action`, `workflow`, `execution`) |
| Resource topology plans        | `crates/resource/plans/`      | Resource subsystem design plans |
| GitHub project setup           | `.github/PROJECT_SETUP.md`    | Repo / project board configuration |

## AI Context Files

| File                          | Purpose |
|-------------------------------|---------|
| `AGENTS.md`                   | This file — project map for any AI agent |
| `.ai-factory/config.yaml`     | AI Factory settings (language, paths, git, rules) |
| `.ai-factory/DESCRIPTION.md`  | Agent-facing project summary |
| `.ai-factory/ARCHITECTURE.md` | Agent-actionable architecture subset |
| `.ai-factory/rules/base.md`   | Distilled coding rules for agents |
| `.ai-factory.json`            | AI Factory install manifest (managed by tooling) |
| `.github/copilot-instructions.md` | GitHub Copilot guidance |
| `.claude/skills/`             | Claude Code `/aif-*` skill definitions |
| `.claude/agents/`             | Subagent definitions (sidecars, workers, loop roles) |

## Agent Rules

- **Decompose chained shell commands.** Run them as separate steps so each step
  has a clear pass/fail. Do not chain unrelated git operations.
  - Wrong: `git checkout main && git pull`
  - Right: first `git checkout main`, then `git pull origin main`
- **Branch from `main`, squash-merge to `main`.** Never force-push or rewrite
  shared history without explicit confirmation.
- **Conventional Commits, validated by `convco`.** Scope = crate name without
  `nebula-` prefix, or top-level area (`docs`, `ci`).
- **No `unwrap()` / `expect()` / `panic!()` in library code.** Use typed
  `thiserror` errors. Tests, `const`, and binaries are exempt per `clippy.toml`.
- **Cross-crate communication goes through `nebula-eventbus`** — never reach
  across layer boundaries with direct imports.
- **Observability is part of Definition of Done.** New state / error / hot path
  must ship with a typed error variant + tracing span + invariant check.
- **`lefthook pre-push` mirrors CI required jobs.** Keep them in sync; if you
  change one, update the other.
- **Runnable examples** live in a root-level `examples/` workspace member, not
  per-crate `examples/`.
