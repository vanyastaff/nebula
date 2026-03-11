# Documentation Index

Complete index of all documentation in the Nebula project.

---

## 🚀 Start Here

| Document | Purpose | Time |
|----------|---------|------|
| **[README.md](../README.md)** | Project overview and entry point | 5 min |
| **[NEWCOMERS.md](../NEWCOMERS.md)** | Guide for first-time contributors | 5 min |
| **[QUICK_START.md](../QUICK_START.md)** | Quick reference for common tasks | 5 min |

---

## 📖 Project Documentation

### High-Level Overview

| Document | Contents |
|----------|----------|
| **[vision/README.md](../vision/README.md)** | Project vision, goals, workspace layout |
| **[vision/ARCHITECTURE.md](../vision/ARCHITECTURE.md)** | System design, crate dependencies, data flow |
| **[vision/CRATES.md](../vision/CRATES.md)** | Purpose and responsibility of each crate |
| **[vision/STATUS.md](../vision/STATUS.md)** | Current completion state per crate |
| **[vision/ROADMAP.md](../vision/ROADMAP.md)** | Phased development plan |
| **[vision/DECISIONS.md](../vision/DECISIONS.md)** | Architectural decision records (ADRs) |
| **[vision/DEPENDENCIES.md](../vision/DEPENDENCIES.md)** | Inter-crate dependency map |

### Project Management

| Document | Contents |
|----------|----------|
| **[docs/PROJECT_STATUS.md](../docs/PROJECT_STATUS.md)** | Current project status and progress |
| **[docs/ROADMAP.md](../docs/ROADMAP.md)** | Detailed roadmap |
| **[docs/TASKS.md](../docs/TASKS.md)** | Task tracking and planning |

---

## 🤝 Contributing

### Essential Guides

| Document | When to Read |
|----------|--------------|
| **[CONTRIBUTING.md](../CONTRIBUTING.md)** | Before your first contribution |
| **[WORKFLOW.md](../WORKFLOW.md)** | When creating branches, commits, PRs |
| **[ISSUES.md](../ISSUES.md)** | When reporting bugs or requesting features |
| **[LABELS.md](../LABELS.md)** | Understanding issue labels |
| **[PROJECT_BOARD.md](../PROJECT_BOARD.md)** | Using the GitHub Project Board |

### GitHub Templates

| Template | Use Case |
|----------|----------|
| **[Bug Report](.github/ISSUE_TEMPLATE/01-bug-report.yml)** | Reporting bugs |
| **[Feature Request](.github/ISSUE_TEMPLATE/02-feature-request.yml)** | Requesting new features |
| **[Documentation Issue](.github/ISSUE_TEMPLATE/03-documentation.yml)** | Reporting documentation gaps |
| **[Question](.github/ISSUE_TEMPLATE/04-question.yml)** | Asking questions |
| **[Pull Request](.github/pull_request_template.md)** | Submitting code changes |

### Setup Guides

| Document | Contents |
|----------|----------|
| **[.github/PROJECT_SETUP.md](.github/PROJECT_SETUP.md)** | Setting up GitHub Projects from scratch |

---

## 🏗️ Technical Documentation

### Crate-Specific Docs

Located in `docs/crates/`:

- **[action](../docs/crates/action/)** — Action trait and execution
- **[api](../docs/crates/api/)** — REST and WebSocket API
- **[config](../docs/crates/config/)** — Configuration management
- **[core](../docs/crates/core/)** — Core types (IDs, Scope)
- **[credential](../docs/crates/credential/)** — Credential management
- **[engine](../docs/crates/engine/)** — DAG scheduler
- **[eventbus](../docs/crates/eventbus/)** — Event bus system
- **[execution](../docs/crates/execution/)** — Execution state machine
- **[expression](../docs/crates/expression/)** — Expression evaluation
- **[log](../docs/crates/log/)** — Logging utilities
- **[memory](../docs/crates/memory/)** — Memory management
- **[metrics](../docs/crates/metrics/)** — Metrics collection
- **[parameter](../docs/crates/parameter/)** — Parameter handling
- **[plugin](../docs/crates/plugin/)** — Plugin system
- **[resilience](../crates/resilience/docs/)** — Resilience patterns
- **[resource](../docs/crates/resource/)** — Resource lifecycle
- **[resource-postgres](../docs/crates/resource-postgres/)** — PostgreSQL adapter
- **[runtime](../docs/crates/runtime/)** — Action runtime
- **[sdk](../docs/crates/sdk/)** — SDK for building integrations
- **[storage](../docs/crates/storage/)** — Storage abstraction
- **[system](../docs/crates/system/)** — System utilities
- **[telemetry](../docs/crates/telemetry/)** — Telemetry and observability
- **[validator](../crates/validator/docs/)** — Validation framework
- **[webhook](../docs/crates/webhook/)** — Webhook handling
- **[workflow](../docs/crates/workflow/)** — Workflow definition

---

## 🚢 Deployment

| Document | Contents |
|----------|----------|
| **[deploy/README.md](../deploy/README.md)** | Deployment overview |
| **[deploy/STACKS.md](../deploy/STACKS.md)** | Technology stacks |
| **[deploy/docker/](../deploy/docker/)** | Docker configurations |
| **[deploy/kubernetes/](../deploy/kubernetes/)** | Kubernetes manifests |

---

## 🧪 Development

### Configuration Files

| File | Purpose |
|------|---------|
| **[Cargo.toml](../Cargo.toml)** | Workspace and dependencies |
| **[clippy.toml](../clippy.toml)** | Clippy lint configuration |
| **[rustfmt.toml](../rustfmt.toml)** | Code formatting rules |
| **[deny.toml](../deny.toml)** | Cargo-deny configuration |
| **[commitlint.config.cjs](../commitlint.config.cjs)** | Commit message linting |

### Scripts & Tools

| Location | Contents |
|----------|----------|
| **[examples/](../examples/)** | Example code and usage |
| **[migrations/](../migrations/)** | Database migrations |

---

## 📱 Applications

| Location | Contents |
|----------|----------|
| **[apps/desktop/](../apps/desktop/)** | Tauri desktop application |
| **[apps/web/](../apps/web/)** | Web frontend |

---

## 📋 Reference Documents

### Compliance & Legal

| Document | Contents |
|----------|----------|
| **[LICENSE](../LICENSE)** | Project license (MIT OR Apache-2.0) |
| **[CONTRIBUTING.md#code-of-conduct](../CONTRIBUTING.md#code-of-conduct)** | Code of conduct |

### Meta Documentation

| Document | Contents |
|----------|----------|
| **[UNIVERSAL_ENGINEERING_CHECKLIST.md](../UNIVERSAL_ENGINEERING_CHECKLIST.md)** | Engineering best practices |
| **[AGENTS.md](../AGENTS.md)** | AI agent instructions |
| **[CLAUDE.md](../CLAUDE.md)** | Claude-specific instructions |

---

## 🔍 Finding Information

### By Task

**I want to...**

- **Understand the project** → [README.md](../README.md), [vision/README.md](../vision/README.md)
- **Contribute code** → [NEWCOMERS.md](../NEWCOMERS.md), [CONTRIBUTING.md](../CONTRIBUTING.md)
- **Report a bug** → [ISSUES.md](../ISSUES.md), [Bug Template](https://github.com/vanyastaff/nebula/issues/new?template=01-bug-report.yml)
- **Request a feature** → [Feature Template](https://github.com/vanyastaff/nebula/issues/new?template=02-feature-request.yml)
- **Learn the architecture** → [vision/ARCHITECTURE.md](../vision/ARCHITECTURE.md)
- **Work on a specific crate** → [docs/crates/](../docs/crates/)
- **Deploy the project** → [deploy/README.md](../deploy/README.md)
- **Set up GitHub Projects** → [.github/PROJECT_SETUP.md](.github/PROJECT_SETUP.md)

### By Role

**I am a...**

- **New Contributor** → [NEWCOMERS.md](../NEWCOMERS.md)
- **Experienced Developer** → [vision/ARCHITECTURE.md](../vision/ARCHITECTURE.md), [vision/CRATES.md](../vision/CRATES.md)
- **Maintainer** → [WORKFLOW.md](../WORKFLOW.md), [PROJECT_BOARD.md](../PROJECT_BOARD.md)
- **User** → [README.md](../README.md), [apps/](../apps/)
- **Plugin Developer** → [docs/crates/plugin/](../docs/crates/plugin/), [docs/crates/sdk/](../docs/crates/sdk/)

---

## 📊 Documentation Statistics

- **Total Documents**: 50+
- **Crate READMEs**: 26
- **Process Guides**: 7
- **Templates**: 5
- **Vision Docs**: 7

---

## 🔄 Keeping Documentation Updated

Documentation is maintained alongside code changes:

- **New feature** → Update relevant crate docs
- **Breaking change** → Update ROADMAP.md and affected guides
- **Process change** → Update WORKFLOW.md or CONTRIBUTING.md
- **New phase** → Update STATUS.md and ROADMAP.md

See [CONTRIBUTING.md#documentation](../CONTRIBUTING.md#documentation) for guidelines.

---

**Questions about documentation?** Open a [discussion](https://github.com/vanyastaff/nebula/discussions).

