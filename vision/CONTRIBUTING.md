# Contributing

How to work on Nebula effectively. This document covers toolchain setup, engineering conventions, the PR workflow, and the quality checklist.

---

## Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| Rust | 1.93+ | `rustup update stable` |
| `cargo-deny` | latest | Dependency policy checks |
| `cargo-audit` | latest | Security advisory checks |
| `sqlx-cli` | 0.7+ | Database migrations (`cargo install sqlx-cli`) |
| Node.js | 20 LTS | Desktop app frontend |
| `pnpm` | 8+ | Frontend package manager |
| Tauri CLI | 2.x | Desktop app builds |

Optional but useful: `cargo-watch`, `cargo-nextest`, `mold` (faster linker on Linux).

---

## Setup

```bash
# Clone and build
git clone https://github.com/vanyastaff/nebula
cd nebula
cargo build --workspace

# Run all tests
cargo test --workspace

# Run lints
cargo fmt --check
cargo clippy --workspace -- -D warnings

# Check dependency policy
cargo deny check
```

For the desktop app:
```bash
cd apps/desktop
pnpm install
pnpm tauri dev      # dev mode
pnpm tauri build    # production build
```

---

## Codebase Navigation

Start with [`vision/README.md`](./README.md) (you are in the vision folder already).

For deep crate detail, each crate has a `docs/crates/<crate>/` folder containing:

| File | Content |
|------|---------|
| `README.md` | What the crate does, quick usage |
| `ARCHITECTURE.md` | Module map, data flow, known bottlenecks |
| `CONSTITUTION.md` | Platform role, user stories, invariants |
| `API.md` | Public API surface and stability contract |
| `TASKS.md` | Concrete tasks for the crate, phased |
| `ROADMAP.md` | Phase plan with exit criteria |
| `DECISIONS.md` | Crate-level ADRs |
| `INTERACTIONS.md` | How this crate interacts with other crates |
| `SECURITY.md` | Threat model, security invariants |

---

## Engineering Conventions

### Code Style

- `rustfmt` with the project's `rustfmt.toml` — run before every commit.
- `clippy -D warnings` — zero warnings policy.
- Public API items must have doc comments (`///`).
- Error types use `thiserror`. Application errors (non-library) may use `anyhow`.

### Error Design

```rust
// Library crates: explicit, typed errors
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("key not found: {0}")]
    NotFound(String),
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

// Application crates (api, desktop backend): anyhow is fine
```

### Naming

| Item | Convention |
|------|-----------|
| Crate | `kebab-case` (`nebula-action`) |
| Struct/Enum/Trait | `PascalCase` |
| Function/method | `snake_case` |
| Constants | `SCREAMING_SNAKE_CASE` |
| Modules | `snake_case` |
| Feature flags | `kebab-case` |

### Testing

- Unit tests live in the same file as the code under `#[cfg(test)]`.
- Integration tests live in `tests/`.
- Use `pretty_assertions` for complex struct comparisons.
- Use `mockall` for mocking traits in tests.
- Use `proptest` for property-based tests on data types.
- Use `tokio-test` for async test utilities.

### Async

- Always support cancellation via `CancellationToken` from `tokio_util::sync`.
- Use `JoinSet` for scoped concurrent tasks.
- Bounded `mpsc` channels for work queues — back-pressure is intentional.
- Never hold a `Mutex`/`RwLock` across an `.await` point.

### Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(engine): add DAG cycle detection
fix(storage): handle empty value in MemoryStorage::get
docs(vision): add CONTRIBUTING guide
refactor(action): rename ActionResult::Done to ActionResult::Success
chore: update tokio to 1.49
```

Breaking changes: append `!` after the type or use `BREAKING CHANGE:` footer.

---

## PR Workflow

1. **Branch** from `main` with a descriptive name: `feat/storage-postgres`, `fix/action-context-lifetime`.
2. **Make changes** — keep PRs focused. One logical change per PR.
3. **Run checks** locally before pushing:
   ```bash
   cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
   ```
4. **Write a PR description** that explains *what* changed and *why*. Link to the relevant `docs/crates/<crate>/TASKS.md` task if applicable.
5. **Review**: PRs require at least one approval. Address all comments before merging.

---

## Quality Checklist

Before marking a PR ready for review, verify:

**Correctness**
- [ ] Core invariants listed in `CONSTITUTION.md` are preserved
- [ ] State machine transitions are correct (no invalid state reachable)
- [ ] Error categories are correct (retryable vs fatal vs validation)

**Concurrency**
- [ ] No lock held across `.await`
- [ ] Cancellation wired through all long-running operations
- [ ] Bounded queues / back-pressure strategy explicit

**Testing**
- [ ] Unit tests for all new public functions
- [ ] Integration test for new end-to-end paths
- [ ] Property tests for any new data types with complex invariants

**Documentation**
- [ ] Public API items have `///` doc comments
- [ ] If the change affects architecture: update `docs/crates/<crate>/ARCHITECTURE.md`
- [ ] If the change is a breaking API change: update `docs/crates/<crate>/MIGRATION.md`
- [ ] If the change adds a new major decision: add to `vision/DECISIONS.md` or `docs/crates/<crate>/DECISIONS.md`

**Security**
- [ ] No secrets in code or logs
- [ ] Input validation is complete and early
- [ ] Auth checks are correct and cover edge cases
- [ ] `cargo audit` passes

---

## Adding a New Crate

1. Add to `Cargo.toml` workspace members.
2. Follow the crate template: `src/lib.rs`, `src/error.rs`, basic `#[cfg(test)]` module.
3. Create `docs/crates/<name>/` with at minimum: `README.md`, `ARCHITECTURE.md`, `CONSTITUTION.md`, `TASKS.md`, `ROADMAP.md`.
4. Add to the crate reference table in `vision/ARCHITECTURE.md`.
5. Add to the status table in `vision/STATUS.md`.
6. Wire in `cargo deny` rules if the new crate should not depend on certain layers.

---

## Adding a New Action (Plugin)

1. Create a new crate (e.g. `nebula-plugin-github`) that depends on `nebula-sdk`.
2. Implement the `Action` trait (or the appropriate sub-trait: `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`).
3. Declare `ActionComponents` — list required `CredentialRef` and `ResourceRef`.
4. Return `ActionResult` variants for control flow: `Success`, `Skip`, `Retry`, `Wait`, `Branch`, etc.
5. Use `nebula-macros` `#[action]` for boilerplate reduction.
6. Write tests using `nebula-sdk`'s `TestContext`.

See `docs/crates/action/EXAMPLES.md` for worked examples.
