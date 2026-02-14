# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Nebula is a workflow automation toolkit in Rust, similar to n8n.

**Stack:**
- Rust 2024 Edition (MSRV: 1.92)
- Tokio async runtime
- 11-crate workspace
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

All crates live under `crates/` with short directory names. Package names retain the `nebula-` prefix (e.g. `crates/core/` contains package `nebula-core`).

**Core:**
- `crates/core` - Identifiers, scope system
- `crates/log` - Logging, observability

**Domain:**
- `crates/action` - Action execution
- `crates/expression` - Expression evaluation
- `crates/validator` - Validation combinators
- `crates/credential` - Credential management

**System:**
- `crates/config` - Configuration, hot-reload
- `crates/memory` - Memory management, arenas, caching
- `crates/resilience` - Circuit breaker, retry, rate limiting
- `crates/resource` - Resource lifecycle, scopes, policies
- `crates/system` - Cross-platform utilities, pressure detection

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
- Builds on Phase 2 storage providers (requires durable storage for rotation state, backups, audit logs) (004-credential-rotation)
- Rust 2024 Edition (MSRV: 1.92) + Tokio async runtime, async-trait, serde, thiserror, chrono (005-refactor-traits-validation)
- N/A (pure refactoring) (005-refactor-traits-validation)
- N/A (this feature only adds types, no persistence logic) (006-extend-core-identity)
- Rust 2024 Edition (MSRV: 1.92) + core, system, log (optional), thiserror, parking_lot, crossbeam-queue, hashbrown, dashmap, tokio (optional), winapi (Windows) (007-memory-prerelease)
- N/A (in-memory allocators) (007-memory-prerelease)
- Rust 2024 Edition (MSRV: 1.92) + serde_json, chrono, rust_decimal, bytes, thiserror (008-serde-value-migration)
- N/A (value type refactoring) (008-serde-value-migration)

## Recent Changes
- 008-serde-value-migration: Migrated from custom nebula-value to serde_json::Value (completed 2026-02-11)
- 001-credential-core-abstractions: Added Rust 2024 Edition (MSRV: 1.92)
