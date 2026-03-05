# Nebula — Workflow Automation Engine

[![Rust](https://img.shields.io/badge/rust-1.93%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-Active%20Development-brightgreen.svg)](docs/PROJECT_STATUS.md)

> **A modular, type-safe, high-performance workflow automation engine** — think n8n or Zapier, but built with Rust for performance, type safety, and extensibility.

---

## 🚀 What Is Nebula?

Nebula lets you:

- **Define workflows** as directed acyclic graphs (DAGs) of composable actions
- **Execute reliably** with built-in retries, timeouts, and error handling
- **Store safely** with encryption, versioning, and audit logging
- **Extend easily** via first-party plugins and third-party SDKs
- **Monitor visually** with a Tauri desktop app or REST API

```
trigger (webhook / cron / event)
    ↓
node A: HTTP Request  →  node B: Transform JSON  →  node C: Send Slack message
    ↓ (on error)
node D: Alert on-call
```

---

## ✨ Key Features

| Feature | Description |
|---------|-------------|
| **Type-Safe** | Rust's compiler catches invalid state transitions, missing credentials, and type mismatches at compile time |
| **Async-First** | Built on Tokio for concurrent execution, bounded work queues, and backpressure |
| **Modular** | 26 focused crates with one-way dependencies; add features without touching the engine |
| **Storage-Agnostic** | In-memory for tests; PostgreSQL for production; same API everywhere |
| **Extensible** | Action trait system + plugin ecosystem for custom integrations |
| **Observable** | Detailed execution logs, telemetry, WebSocket real-time progress, audit trails |

---

## 📖 Documentation Map

| Document | Purpose |
|----------|---------|
| **[vision/README.md](vision/README.md)** | Project vision, high-level overview, workspace layout |
| **[vision/ARCHITECTURE.md](vision/ARCHITECTURE.md)** | Crate layers, dependency rules, data flow patterns |
| **[vision/CRATES.md](vision/CRATES.md)** | Purpose and responsibility of every crate |
| **[vision/STATUS.md](vision/STATUS.md)** | Current completion state per crate |
| **[vision/ROADMAP.md](vision/ROADMAP.md)** | Phased plan: what's done, what's next |
| **[vision/DECISIONS.md](vision/DECISIONS.md)** | Architectural decision records (ADRs) |
| **[CONTRIBUTING.md](CONTRIBUTING.md)** | Contributor guidelines, code standards, review process |
| **[ISSUES.md](ISSUES.md)** | Issue templates, labels, triage process |
| **[WORKFLOW.md](WORKFLOW.md)** | Branch naming, commit conventions, PR process |
| **[PROJECT_BOARD.md](PROJECT_BOARD.md)** | GitHub Project Board structure and usage |
| **[LABELS.md](LABELS.md)** | Issue label hierarchy and definitions |

---

## 🏁 Quick Start

### Prerequisites

- **Rust 1.93+** ([Install](https://rustup.rs/))
- **Cargo** (comes with Rust)
- **Git**

### Clone & Build

```bash
git clone https://github.com/vanyastaff/nebula.git
cd nebula

# Build all crates
cargo build

# Run tests
cargo test

# Check code quality
cargo clippy -- -D warnings
cargo fmt --check
```

### First Steps

1. **New contributor?** Start with [NEWCOMERS.md](NEWCOMERS.md) (5 min guide)
2. **Understand the project**: Read [vision/README.md](vision/README.md)
3. **Explore the architecture**: Check [vision/ARCHITECTURE.md](vision/ARCHITECTURE.md)
4. **Find something to work on**: Browse [Good First Issues](https://github.com/vanyastaff/nebula/issues?q=label:difficulty:good-first-issue)
5. **Set up your environment**: Follow [CONTRIBUTING.md](CONTRIBUTING.md#development-setup)

### Repository Setup (for maintainers)

**Automated setup with scripts:**
```bash
# Create all GitHub labels automatically
python scripts/setup-github-api.py

# Or with GitHub CLI
gh auth login
./scripts/setup-github.sh  # Linux/macOS
# or
.\scripts\setup-github.ps1  # Windows
```

See [scripts/README.md](scripts/README.md) for details.

---

## 🏗️ Project Structure

```
nebula/
├── crates/                    # 26 Rust library crates
│   ├── core/                  # Fundamental types (IDs, Scope)
│   ├── workflow/              # Workflow definition & DAG model
│   ├── execution/             # Execution state machine
│   ├── action/                # Action trait & contract
│   ├── engine/                # DAG scheduler & orchestrator
│   ├── runtime/               # Action runner, isolation, work queue
│   ├── storage/               # KV storage abstraction
│   ├── credential/            # Encrypted secrets & rotation
│   ├── resource/              # Resource lifecycle & pooling
│   ├── api/                   # REST + WebSocket server (Axum)
│   └── ... (18 more)          # See vision/CRATES.md for full list
├── apps/
│   ├── desktop/               # Tauri desktop app (React + Rust)
│   └── web/                   # Web frontend
├── docs/                      # Per-crate documentation
├── vision/                    # Project strategy & architecture
├── migrations/                # SQL database migrations
└── deploy/                    # Deployment config (Docker, K8s)
```

See [vision/ARCHITECTURE.md](vision/ARCHITECTURE.md) for detailed layer breakdown.

---

## 🎯 Current Status (March 2026)

| Phase | Status | Details |
|-------|--------|---------|
| **Phase 1: Core Foundation** | ✅ Complete | All foundation crates implemented and tested |
| **Phase 2: Execution Engine** | 🔄 Active | Action trait, DAG engine, runtime in progress |
| **Phase 3: Credential System** | ⬜ Planned | Hardening, rotation policies, audit |
| **Phase 4: Plugin Ecosystem** | ⬜ Planned | First-party & third-party plugin SDKs |
| **Phase 5: Desktop App** | ⬜ Planned | Visual editor, workflow debugging, monitoring |

See [vision/ROADMAP.md](vision/ROADMAP.md) for detailed timelines and exit criteria.

---

## 🤝 Contributing

We welcome contributions! Before you start:

1. **Read**: [CONTRIBUTING.md](CONTRIBUTING.md) — Code of conduct, development setup, style guide
2. **Understand**: [WORKFLOW.md](WORKFLOW.md) — Branch naming, commit conventions, PR process
3. **Choose**: [Issues](https://github.com/vanyastaff/nebula/issues) — Filter by `good-first-issue` or `help-wanted`
4. **Discuss**: Open an issue or discussion before major changes

**Quick Checklist Before Submitting a PR:**
- [ ] Tests pass: `cargo test`
- [ ] Clippy passes: `cargo clippy -- -D warnings`
- [ ] Code formatted: `cargo fmt`
- [ ] Commit messages follow conventions (see [WORKFLOW.md](WORKFLOW.md))
- [ ] PR description references issue (e.g., "Closes #123")
- [ ] No breaking changes without discussion

---

## 📊 Stats

- **Language**: Rust 🦀
- **Crates**: 26
- **Edition**: 2024
- **MSRV**: 1.93
- **License**: MIT OR Apache-2.0

---

## 🐛 Reporting Issues

Found a bug? Have a feature idea? Please file an issue!

- **Bug Reports**: [ISSUES.md#bug-report](ISSUES.md#bug-report)
- **Feature Requests**: [ISSUES.md#feature-request](ISSUES.md#feature-request)
- **Documentation**: [ISSUES.md#documentation](ISSUES.md#documentation)

---

## 📚 Learning Resources

- **[Architecture Deep Dive](vision/ARCHITECTURE.md)** — Understand how components interact
- **[Crates Overview](vision/CRATES.md)** — Learn what each crate does
- **[Decision Records](vision/DECISIONS.md)** — Understand the "why" behind key choices
- **[Design Documents](docs/)** — In-depth per-crate specifications
- **[Status Report](vision/STATUS.md)** — Track progress across crates

---

## 💬 Community

- **GitHub Issues**: [vanyastaff/nebula/issues](https://github.com/vanyastaff/nebula/issues)
- **Discussions**: [vanyastaff/nebula/discussions](https://github.com/vanyastaff/nebula/discussions)
- **Code of Conduct**: [CONTRIBUTING.md#code-of-conduct](CONTRIBUTING.md#code-of-conduct)

---

## 📄 License

This project is dual-licensed under MIT OR Apache-2.0. See [LICENSE](LICENSE) for details.

---

## 🙏 Acknowledgments

Built with ❤️ by the Nebula team and contributors. Thanks for being part of this journey!

---

**Questions?** Start with [vision/README.md](vision/README.md) or open a [discussion](https://github.com/vanyastaff/nebula/discussions).


