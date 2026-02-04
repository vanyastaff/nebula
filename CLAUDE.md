# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Nebula is a workflow automation toolkit in Rust, similar to n8n.

**Stack:**
- Rust 2024 Edition (MSRV: 1.92)
- Tokio async runtime
- 16-crate workspace
- egui for UI

## Common Commands

```bash
# Build & Test
cargo build
cargo test --workspace
cargo test -p nebula-parameter -- --nocapture

# Code Quality
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-features

# CI Pipeline (all must pass)
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo check --workspace --all-targets
cargo test --workspace
cargo doc --no-deps --workspace
cargo audit
```

## Workspace Structure

**Core:**
- `nebula-core` - Identifiers, scope system
- `nebula-value` - Runtime type system (Value: Null, Bool, Number, String, Array, Object)
- `nebula-log` - Logging, observability

**Domain:**
- `nebula-parameter` - Parameter definitions with validation
- `nebula-action` - Action execution
- `nebula-expression` - Expression evaluation
- `nebula-validator` - Validation combinators
- `nebula-credential` - Credential management

**UI:**
- `nebula-ui` - Base UI framework
- `nebula-parameter-ui` - Parameter widgets

**System:**
- `nebula-config`, `nebula-memory`, `nebula-resilience`, `nebula-resource`, `nebula-system`

**Tooling:**
- `nebula-derive` - Procedural macros

## Error Handling

**Each crate defines its own error type** - do NOT use `nebula-error` dependency.

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MyError {
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
}
```

## Async Patterns

```rust
// Prefer JoinSet for scoped tasks
use tokio::task::JoinSet;
let mut set = JoinSet::new();
set.spawn(async { /* work */ });

// Always include cancellation
tokio::select! {
    result = do_work() => Ok(result),
    _ = shutdown.cancelled() => Err(Cancelled),
}
```

**Channels:**
- Work queues: bounded mpsc
- Events: broadcast (stateless only)
- Response: oneshot
- Shared state: RwLock preferred

**Timeouts:**
- Default: 30s
- Database: 5s
- HTTP: 10s

## Rust 2024 Specifics

```rust
// ❌ WRONG - unsized type
type Input = str;

// ✅ CORRECT - sized type
type Input = String;

// ✅ Explicit type annotations for complex generics
let validator: Field<User, u32, _, _> =
    named_field("age", MinValue { min: 18 }, get_age);
```

## Testing

```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_async() {
    tokio::time::pause();
    let result = operation().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    // assertions
}
```

**Pre-Commit Checklist:**
- [ ] `cargo check --all-features`
- [ ] `cargo test --workspace`
- [ ] `cargo fmt --all`
- [ ] `cargo clippy --all-features -- -D warnings`

## Git Workflow

**Branch:** `feat/`, `fix/`, `docs/`, `refactor/`

**Commit:** `type(scope): subject`

**Commit Format:**
- Use standard commit message format
- Do NOT add "Generated with [Claude Code]" footer
- Do NOT add "Co-Authored-By: Claude Sonnet 4.5" footer
- Keep commits clean and professional

**Never:** force push to main, skip hooks, commit secrets

## Additional Resources

- `.cursorrules` - Engineering rules (JSON format with CI, async, resilience patterns)
- `AGENTS.md` - Build commands, coding style, commit guidelines

## Active Technologies
- File-based local storage with encrypted credentials (Phase 2 adds cloud providers) (001-credential-core-abstractions)
- Rust 2024 Edition (MSRV: 1.92) + Tokio async runtime, async-trait, serde, thiserror, aws-sdk-secretsmanager, azure_security_keyvault, vaultrs, kube (002-storage-backends)
- Multiple pluggable backends - local encrypted filesystem (AES-256-GCM), AWS Secrets Manager (KMS-encrypted), Azure Key Vault (HSM-backed), HashiCorp Vault (KV v2), Kubernetes Secrets (namespace-isolated) (002-storage-backends)
- Abstracted via `StorageProvider` trait (implemented in Phase 2): (001-credential-manager)

## Recent Changes
- 001-credential-core-abstractions: Added Rust 2024 Edition (MSRV: 1.92)
