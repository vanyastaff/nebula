# Quick Reference

Fast answers for common tasks in Nebula development.

---

## 🚀 Getting Started (5 min)

```bash
# Clone and build
git clone https://github.com/vanyastaff/nebula.git
cd nebula
cargo build

# Run tests
cargo test

# Check code quality
cargo clippy -- -D warnings
cargo fmt --check
```

**Next:** Read [vision/README.md](vision/README.md) for architecture overview.

---

## 🎯 Common Tasks

### I want to contribute

1. **Find an issue:** [GitHub Issues](https://github.com/vanyastaff/nebula/issues?q=label:difficulty:good-first-issue)
2. **Set up:** Follow [CONTRIBUTING.md#development-setup](CONTRIBUTING.md#development-setup)
3. **Branch:** Follow [WORKFLOW.md#branch-naming](WORKFLOW.md#branch-naming)
4. **Code:** Make changes, run tests, format code
5. **Submit:** Open a PR with [this template](.github/ISSUE_TEMPLATE/pull_request_template.md)

### I found a bug

→ [Report it here](ISSUES.md#bug-reports) | [Template](https://github.com/vanyastaff/nebula/issues/new?template=01-bug-report.yml)

### I have a feature idea

→ [Request it here](ISSUES.md#feature-requests) | [Template](https://github.com/vanyastaff/nebula/issues/new?template=02-feature-request.yml)

### I have a question

→ [Ask here](https://github.com/vanyastaff/nebula/discussions) | [Issue Template](https://github.com/vanyastaff/nebula/issues/new?template=04-question.yml)

### I want to understand the codebase

| Document | For Learning... |
|----------|-----------------|
| [vision/README.md](vision/README.md) | What Nebula is, 30-sec overview |
| [vision/ARCHITECTURE.md](vision/ARCHITECTURE.md) | How crates fit together |
| [vision/CRATES.md](vision/CRATES.md) | What each crate does |
| [docs/crates/](docs/crates/) | Deep dive into specific crates |

---

## 📝 Writing Code

### Format

```bash
cargo fmt
```

### Lint

```bash
cargo clippy -- -D warnings
```

### Test (Before Committing!)

```bash
# All tests
cargo test

# Specific crate
cargo test -p nebula-engine

# Single test
cargo test test_action_execution_success
```

### Commit (Follow Conventions)

```
feat(runtime): add action timeout
^    ^         ^
|    |         └─ subject (imperative, no period)
|    └────────────── scope (crate name, optional)
└─────────────────── type (feat, fix, docs, etc.)

Body (optional):
- Explain why, not how
- Wrap at 72 chars
- References: Closes #42
```

See [WORKFLOW.md#commit-conventions](WORKFLOW.md#commit-conventions) for full guide.

---

## 🔀 Branching

### Create a Branch

```bash
# From main, latest code
git checkout main
git pull upstream main

# Create branch with convention
git checkout -b feat/your-feature-name
```

### Push and Open PR

```bash
git push origin feat/your-feature-name

# Then open PR on GitHub
```

See [WORKFLOW.md#branch-naming](WORKFLOW.md#branch-naming) for naming rules.

---

## 🧪 Testing

### Write a Test

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_works() {
        let result = do_something();
        assert_eq!(result, expected);
    }
}
```

### Run Tests

```bash
cargo test                    # All tests
cargo test -p nebula-api     # Single crate
cargo test test_name         # Single test
cargo test -- --nocapture    # Show println! output
```

### Check Coverage

```bash
cargo tarpaulin
```

---

## 📦 Dependencies

### Add a Dependency

```bash
cargo add some_crate -p nebula-core
```

### Update Lockfile

```bash
cargo update
```

### Check for Vulnerabilities

```bash
cargo audit
```

---

## 🐛 Debugging

### Print Values

```rust
println!("debug: {:?}", my_value);
```

### Use Logs

```rust
log::info!("Workflow started: {}", workflow_id);
log::error!("Execution failed: {}", err);
```

### Run with Debug Output

```bash
RUST_LOG=debug cargo run
RUST_LOG=nebula-engine=trace cargo test
```

### Use LLDB (macOS/Linux)

```bash
rust-lldb target/debug/nebula
(lldb) b nebula-engine/src/lib.rs:42
(lldb) run
(lldb) frame variable
```

---

## 📊 Project Structure at a Glance

```
nebula/
├── crates/              ← Rust libraries (26 crates)
│   ├── core/            ← Fundamental types
│   ├── workflow/        ← Workflow definition
│   ├── engine/          ← DAG scheduler
│   ├── runtime/         ← Action execution
│   └── ...
├── apps/                ← Desktop & web UI
├── docs/                ← Per-crate documentation
├── vision/              ← Architecture & roadmap
├── migrations/          ← SQL schemas
└── deploy/              ← Docker, Kubernetes config
```

See [vision/ARCHITECTURE.md](vision/ARCHITECTURE.md) for the full picture.

---

## 🔍 Finding Things

### Find Files

```bash
find . -name "*.rs" -type f | grep action
```

### Search Code

```bash
grep -r "pub fn execute" --include="*.rs"
grep -r "TODO" crates/
```

### Using Your IDE

Most IDEs have "Go to Definition," "Find References," "Find Files" built-in.

---

## 🚨 Common Issues

### "Failed to build" → Check error output

```bash
cargo build 2>&1 | head -50
```

### "Tests failing" → Rerun with nocapture

```bash
cargo test -- --nocapture | head -100
```

### "Merge conflicts" → Rebase

```bash
git fetch upstream
git rebase upstream/main
# Resolve conflicts, then:
git add .
git rebase --continue
```

### "Clippy errors" → Fix automatically (if possible)

```bash
cargo clippy --fix
cargo fmt
```

---

## 📚 More Help

| Topic | Link |
|-------|------|
| Contributing Guidelines | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Development Workflow | [WORKFLOW.md](WORKFLOW.md) |
| Reporting Issues | [ISSUES.md](ISSUES.md) |
| Label System | [LABELS.md](LABELS.md) |
| Project Board | [PROJECT_BOARD.md](PROJECT_BOARD.md) |
| Architecture | [vision/ARCHITECTURE.md](vision/ARCHITECTURE.md) |
| Crate Docs | [vision/CRATES.md](vision/CRATES.md) |
| Roadmap | [vision/ROADMAP.md](vision/ROADMAP.md) |

---

**Stuck?** Open a [discussion](https://github.com/vanyastaff/nebula/discussions) or comment on an issue!

