# Credential Runtime — Foundation (Plan 1 of 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the `nebula-credential-runtime` Exec-tier crate scaffold (+ `CredentialServiceError`), ADR-0066, and 3 net-new static reference credentials in `nebula-credential-builtin` with `register_builtins`, with `task dev:check` green.

**Architecture:** Foundation increment of the credential-subsystem completion (spec `docs/superpowers/specs/2026-05-15-credential-runtime-subsystem-design.md`). No relocation of existing types (grep-gate verdict §3.2/§9 — `OAuth2Credential`/ApiKey/Basic stay in `nebula-credential`). New empty Exec crate settles workspace + `deny.toml` ahead of Plan 2's facade (mirrors the documented `credential-builtin` П1 scaffold-first pattern). The 3 reference credentials mirror the proven `BasicAuthCredential` shape verbatim — static, `State = Scheme`, five `plugin_capability_report::Is* = false`.

**Tech Stack:** Rust 1.95 / edition 2024, `cargo` workspace, `cargo-deny` (layer wrappers), `cargo nextest`, `task` runner, `thiserror`, `nebula-error` `Classify` derive, `nebula-schema` `#[derive(Schema)]`, lefthook (typos + convco) pre-commit.

**Conventions (every commit):**
- Author identity via env: `GIT_AUTHOR_NAME=vanyastaff`, `GIT_AUTHOR_EMAIL=ivan.kondrashkin@gmail.com` (+ `GIT_COMMITTER_*`).
- Conventional Commits (validated by `convco`); scope = `credential` or `credential-runtime`.
- End message with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
- On any `Cargo.toml`/dependency change, also stage the root `Cargo.lock`.
- No `unwrap()`/`expect()`/`panic!()` in library code (tests exempt). No plan/phase IDs in committed code or comments.

---

## File Structure

| Path | Responsibility | Action |
|------|----------------|--------|
| `Cargo.toml` (root) | workspace members list | Modify — add `"crates/credential-runtime"` |
| `deny.toml` | layer-wrapper allowlist | Modify — add `nebula-credential-runtime` self-scaffold ban entry |
| `crates/credential-runtime/Cargo.toml` | new Exec crate manifest | Create |
| `crates/credential-runtime/src/lib.rs` | crate root: docs, `#![forbid(unsafe_code)]`, module wiring | Create |
| `crates/credential-runtime/src/error.rs` | `CredentialServiceError` (thiserror + `nebula_error::Classify`) | Create |
| `docs/adr/0066-credential-runtime-crate.md` | ADR: narrow supersede of ADR-0030 facade slice; B deferred ideal; ADR-0028 canon audit | Create |
| `docs/adr/README.md` | worktree ADR index + supersession table | Modify — add 0066 rows |
| `crates/credential/src/scheme/shared_key.rs` | add `identity_state!` so `SharedKey` is static-usable | Modify (additive one-liner) |
| `crates/credential/src/scheme/signing_key.rs` | add `identity_state!` so `SigningKey` is static-usable | Modify (additive one-liner) |
| `crates/credential-builtin/src/bearer_token.rs` | `BearerTokenCredential` reference impl | Create |
| `crates/credential-builtin/src/shared_key.rs` | `SharedKeyCredential` reference impl | Create |
| `crates/credential-builtin/src/signing_key.rs` | `SigningKeyCredential` reference impl | Create |
| `crates/credential-builtin/src/registry.rs` | `register_builtins(&mut CredentialRegistry)` | Create |
| `crates/credential-builtin/src/lib.rs` | wire new modules + public re-exports | Modify |

Reference patterns (read these for exact shape; do not guess):
- Credential impl + Properties + `Is*`: `crates/credential/src/credentials/basic_auth.rs:1-172`
- `identity_state!` invocation: `crates/credential/src/scheme/secret_token.rs:45`
- Scheme constructors: `SecretToken::new` `scheme/secret_token.rs:34`, `SharedKey::new` `scheme/shared_key.rs:35`, `SigningKey::new` `scheme/signing_key.rs:36`
- Registry API: `crates/credential/src/contract/registry.rs:122-167` (`register<C>(instance, registering_crate: &'static str)`)
- `Classify` derive grammar + valid categories (`internal`/`validation`/`external` only): `crates/credential/src/error.rs:171-225`
- `deny.toml` scaffold ban-entry pattern: `crates/credential-vault` block `deny.toml:158-160`

---

## Task 1: `nebula-credential-runtime` crate scaffold + workspace + deny.toml

**Files:**
- Modify: `Cargo.toml` (root, members list line 9 area)
- Create: `crates/credential-runtime/Cargo.toml`
- Create: `crates/credential-runtime/src/lib.rs`
- Modify: `deny.toml` (after the `nebula-credential-vault` ban entry, ~line 160)

- [ ] **Step 1: Add the workspace member**

In root `Cargo.toml`, the members array currently contains `  "crates/credential-vault",` on its own line. Add the new member immediately after it:

```toml
  "crates/credential-vault",
  "crates/credential-runtime",
```

- [ ] **Step 2: Create `crates/credential-runtime/Cargo.toml`**

```toml
[package]
name = "nebula-credential-runtime"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
keywords.workspace = true
authors.workspace = true
description = "Credential management runtime: the CredentialService facade, type registry, state-source resolution, and observability seam (Exec tier)."
license.workspace = true
repository.workspace = true

[dependencies]
nebula-error = { workspace = true, features = ["derive"] }
thiserror = { workspace = true }

[dev-dependencies]
pretty_assertions = { workspace = true }

[features]
default = []

[lints]
workspace = true
```

- [ ] **Step 3: Create `crates/credential-runtime/src/lib.rs`**

```rust
//! # nebula-credential-runtime
//!
//! **Role:** Credential management runtime — the single owner of the
//! credential *management bounded context*. Sole public entry is
//! `CredentialService` (lands in a later increment); all
//! invariant-bearing composition is crate-private so the secure
//! construction path is the only path.
//!
//! Exec tier. Narrowly supersedes the facade-ownership slice of
//! ADR-0030 (engine retains the low-level resolver / RefreshCoordinator
//! / lease mechanism); see `docs/adr/0066-credential-runtime-crate.md`.
//!
//! This increment ships only the crate scaffold and the
//! [`CredentialServiceError`](error::CredentialServiceError) taxonomy.
#![forbid(unsafe_code)]

pub mod error;

pub use error::CredentialServiceError;
```

> Note: Step 3 references `mod error` which Task 2 creates. Build is deferred to Task 2 Step 4 (this task's build check uses `--lib` after Task 2, or temporarily comment the `pub mod error;` line — simpler: do Task 1 and Task 2 as one commit. To keep tasks bite-sized, Task 1 Step 5 below builds with `error.rs` stubbed to an empty module, then Task 2 fills it).

- [ ] **Step 3b: Create a minimal `crates/credential-runtime/src/error.rs` stub so Task 1 compiles independently**

```rust
//! Error taxonomy for the credential management runtime.
//! Filled in Task 2.
```

- [ ] **Step 4: Add the `deny.toml` ban entry**

Mirror the `nebula-credential-vault` scaffold pattern. Immediately after the closing `]` of the `nebula-credential-vault` wrapper entry (around `deny.toml:160`), add:

```toml
  # nebula-credential-runtime is the Exec-tier owner of the credential
  # management bounded context (CredentialService facade + type registry +
  # state-source resolution + observability). Narrowly supersedes the
  # facade-ownership slice of ADR-0030 (see docs/adr/0066). This scaffold
  # increment ships an empty crate; the wrapper allowlist starts at self
  # only and is widened to { nebula-api, nebula-cli } when the facade lands
  # and api depends on it (Plan 2).
  { crate = "nebula-credential-runtime", wrappers = [
    "nebula-credential-runtime",
  ], reason = "Credential management runtime is Exec-tier; only the API and CLI composition roots may depend on it (allowlist widened when the facade lands)" },
```

- [ ] **Step 5: Build the crate and verify deny passes**

Run: `cargo build -p nebula-credential-runtime`
Expected: `Compiling nebula-credential-runtime ...` then `Finished` with no errors.

Run: `cargo deny check bans`
Expected: `bans ok` (no wrapper violations — the crate has no dependents and no upper-tier deps).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock deny.toml crates/credential-runtime
git commit -m "feat(credential-runtime): scaffold Exec-tier crate + deny wrapper

Empty nebula-credential-runtime crate, workspace member, and deny.toml
self-scaffold ban entry. Mirrors the documented credential-builtin П1
scaffold-first pattern so workspace + layer-wrapper resolution settle
ahead of the facade (Plan 2). Narrowly supersedes the ADR-0030 facade
slice — see ADR-0066 (Task 3).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `CredentialServiceError` taxonomy

**Files:**
- Modify: `crates/credential-runtime/src/error.rs` (replace the Task 1 stub)
- Test: `crates/credential-runtime/src/error.rs` (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Confirm the `Classify` attribute grammar and valid category set**

Read `crates/credential/src/error.rs:171-225`. Confirm: derive is `#[derive(Debug, Error, nebula_error::Classify)]`, enum is `#[non_exhaustive]`, each variant carries `#[classify(category = "<cat>", code = "<CODE>")]` then `#[error("...")]`. The only category strings used in the codebase are `internal`, `validation`, `external` — restrict `CredentialServiceError` to these three.

- [ ] **Step 2: Write the failing test**

Append to `crates/credential-runtime/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::CredentialServiceError;

    #[test]
    fn display_messages_are_actionable() {
        let e = CredentialServiceError::NotFound {
            id: "cred-1".to_owned(),
        };
        assert_eq!(e.to_string(), "credential not found: cred-1");

        let e = CredentialServiceError::VersionConflict {
            id: "cred-1".to_owned(),
            expected: 3,
            actual: 4,
        };
        assert_eq!(
            e.to_string(),
            "version conflict for cred-1: expected 3, got 4"
        );

        let e = CredentialServiceError::CapabilityUnsupported {
            capability: "refresh".to_owned(),
            key: "bearer_token".to_owned(),
        };
        assert_eq!(
            e.to_string(),
            "credential type 'bearer_token' does not support capability 'refresh'"
        );
    }

    #[test]
    fn is_std_error() {
        fn assert_error<E: std::error::Error + Send + Sync + 'static>() {}
        assert_error::<CredentialServiceError>();
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p nebula-credential-runtime --lib`
Expected: FAIL — `cannot find type CredentialServiceError` / unresolved variants.

- [ ] **Step 4: Implement the enum**

Replace the entire contents of `crates/credential-runtime/src/error.rs` with:

```rust
//! Error taxonomy for the credential management runtime.
//!
//! `#[non_exhaustive]` so later increments add variants without breaking
//! downstream `match` exhaustiveness. Classified via
//! [`nebula_error::Classify`] using only the codebase-standard categories
//! `internal` / `validation` / `external` (mirrors
//! `crates/credential/src/error.rs`).

use thiserror::Error;

/// Failure modes of the credential management facade. The API layer maps
/// each `category` to an HTTP status; `code` is the stable machine label.
#[derive(Debug, Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum CredentialServiceError {
    /// No credential with this id in the caller's tenant scope.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:NOT_FOUND")]
    #[error("credential not found: {id}")]
    NotFound {
        /// The credential id that was not found.
        id: String,
    },

    /// Optimistic-concurrency check failed on update.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:VERSION_CONFLICT")]
    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict {
        /// Credential id under contention.
        id: String,
        /// Version the caller expected (CAS precondition).
        expected: u64,
        /// Version actually stored.
        actual: u64,
    },

    /// Property payload failed the credential type's schema validation.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:VALIDATION_FAILED")]
    #[error("credential property validation failed: {reason}")]
    ValidationFailed {
        /// Human-readable validation failure (never echoes secret values).
        reason: String,
    },

    /// The requested lifecycle op needs a capability the type lacks.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:CAPABILITY_UNSUPPORTED")]
    #[error("credential type '{key}' does not support capability '{capability}'")]
    CapabilityUnsupported {
        /// Capability name (`refresh` / `revoke` / `test`).
        capability: String,
        /// `Credential::KEY` of the target type.
        key: String,
    },

    /// No credential type registered under this key.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:TYPE_UNKNOWN")]
    #[error("unknown credential type: {key}")]
    TypeUnknown {
        /// The unregistered credential key.
        key: String,
    },

    /// Interactive acquisition token is expired or already consumed.
    #[classify(category = "validation", code = "CREDENTIAL_SERVICE:PENDING_EXPIRED")]
    #[error("pending acquisition expired or already consumed")]
    PendingExpired,

    /// An external secret provider failed.
    #[classify(category = "external", code = "CREDENTIAL_SERVICE:PROVIDER")]
    #[error("external provider error: {0}")]
    Provider(String),

    /// The persistence layer failed.
    #[classify(category = "internal", code = "CREDENTIAL_SERVICE:STORE")]
    #[error("credential store error: {0}")]
    Store(String),

    /// An invariant the runtime owns was violated.
    #[classify(category = "internal", code = "CREDENTIAL_SERVICE:INTERNAL")]
    #[error("internal credential runtime error: {0}")]
    Internal(String),
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p nebula-credential-runtime --lib`
Expected: PASS — `test result: ok. 2 passed`.

- [ ] **Step 6: Lint clean**

Run: `cargo clippy -p nebula-credential-runtime -- -D warnings`
Expected: no warnings, exit 0.

- [ ] **Step 7: Commit**

```bash
git add crates/credential-runtime/src/error.rs
git commit -m "feat(credential-runtime): CredentialServiceError taxonomy

thiserror + nebula_error::Classify, #[non_exhaustive], categories
restricted to internal/validation/external. Foundation for the API
error mapping the facade will use.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: ADR-0066

**Files:**
- Create: `docs/adr/0066-credential-runtime-crate.md`
- Modify: `docs/adr/README.md` (index table + supersession table)

- [ ] **Step 1: Create `docs/adr/0066-credential-runtime-crate.md`**

```markdown
# 0066 — Credential management runtime crate (`nebula-credential-runtime`)

- **Status:** accepted (2026-05-15)
- **Tags:** credential, runtime, layer-boundary, breaking, supersession, m11
- **Narrowly supersedes:** the facade-ownership slice of
  `C:/Users/vanya/RustroverProjects/docs/adr/0030-engine-owns-credential-orchestration.md`

## Context

The credential contract crate (`nebula-credential`) is internally
complete, but the subsystem has no owner for the *management bounded
context*: a populated `CredentialTypeRegistry`, the
validate→encrypt→CAS-store pipeline, lifecycle dispatch by capability,
store-or-external state resolution, and the observability seam. The API
service layer is 12 `503` stubs precisely because that owner does not
exist. ADR-0030 placed the low-level resolver/RefreshCoordinator/lease
mechanism in `nebula-engine` but did not create a management facade;
folding one into `nebula-engine` would conflate the workflow-execution
engine with credential management and leave the security invariants
(layered store, non-optional observer, tenant scoping) enforced by
discipline rather than by a crate boundary.

`deny.toml` facts: only `nebula-api`/`nebula-cli` may depend on both
`nebula-engine` and `nebula-storage` (both Exec). The facade needs both,
so it must be Exec tier — it cannot be a Business-tier crate.

## Decision

Introduce `nebula-credential-runtime` (Exec tier). It is the sole owner
of the credential management facade; its only public entry is
`CredentialService`, with all invariant-bearing composition crate-private
so the secure construction path is the only path. It depends on
`nebula-engine` (Exec sibling, curated) for the existing low-level
resolver/RefreshCoordinator/lease mechanism — acyclic: `nebula-engine`
does **not** depend on the runtime.

This narrowly supersedes ADR-0030's facade slice only: ADR-0030's
mechanism (resolver, RefreshCoordinator, claim repo) stays in
`nebula-engine`. ADR-0041 (durable refresh claim repo) and ADR-0051
(external provider redesign) are untouched; ADR-0051's deferred Phase-D
non-goal ("wire `ExternalProvider::resolve` into resolution") is
*fulfilled* by the runtime's `StateSource`, not worked around.

## Consequences

- `deny.toml` gains a `nebula-credential-runtime` wrapper entry; the
  allowlist widens to `{ nebula-api, nebula-cli }` when the facade lands.
- `nebula-api` depends on `nebula-credential-runtime` for credential
  management (its `nebula-engine` dep remains for workflow execution).
- Breaking: the API credential service surface changes from stubs to a
  real facade-backed implementation.

## Deferred ideal (recorded so it is not lost)

Full extraction — relocating the engine's resolver / lease / rotation /
RefreshCoordinator / claim-repo into the runtime crate so
`nebula-engine` is de-godded — is the cleaner long-term decomposition.
It is **deferred**: relocating the chaos-tested ADR-0041 claim-repo
against a "finalize to stable" goal is unacceptable risk for this
effort. Revisit as a dedicated migration ADR.

## ADR-0028 cross-crate canon-audit checklist

The runtime implementation (Plans 2–3) must satisfy all eight ADR-0028
invariants; each is gated by a test or compile-fail probe:

1. §12.5 encryption-at-rest preserved — runtime composes the layered
   store (`Scope(Audit(Cache(Encryption(raw))))`); compile-fail probe:
   raw backend unusable without layers.
2. §13.2 refresh/rotation seam integrity — no silent strand; explicit
   `ReauthRequired`.
3. Stored-state vs projected-auth-material split — responses built from
   `CredentialSnapshot` only.
4. No discard-and-log audit — audit sink refusal → `StoreError::AuditFailure`.
5. §4.5 honesty gating — MATURITY/status vocabulary respected.
6. Compat re-exports — no shims; importers updated directly.
7. No new storage behaviour without canon — runtime adds no new
   `CredentialStore` semantics, only composes existing layers.
8. Cross-crate compat cycle — acyclic (engine ⇏ runtime) verified by
   `cargo deny check bans`.
```

- [ ] **Step 2: Add the index row to `docs/adr/README.md`**

In the `## Index` table, after the `0050` row, add:

```markdown
| [0066](./0066-credential-runtime-crate.md) | Credential management runtime crate (`nebula-credential-runtime`) | accepted (2026-05-15) | credential, runtime, layer-boundary, breaking, m11 |
```

- [ ] **Step 3: Add the supersession row to `docs/adr/README.md`**

In the `## Supersession` table, add a row:

```markdown
| `0030` facade slice (`engine-owns-credential-orchestration`, external `C:/Users/vanya/RustroverProjects/docs/adr/0030-*.md`) | [0066](./0066-credential-runtime-crate.md) | Management facade ownership moves to `nebula-credential-runtime` (Exec). ADR-0030's low-level mechanism (resolver/RefreshCoordinator/claim-repo) stays in `nebula-engine`. ADR-0041/0051 untouched. |
```

- [ ] **Step 4: Verify lint hooks accept the docs**

Run: `npx --yes lefthook run pre-commit` (or rely on the commit hook in Step 5).
Expected: `typos` passes (no misspellings). If `typos` flags a token, fix the spelling — do not add an ignore.

- [ ] **Step 5: Commit**

```bash
git add docs/adr/0066-credential-runtime-crate.md docs/adr/README.md
git commit -m "docs(credential): ADR-0066 credential-runtime crate

Narrowly supersedes the ADR-0030 facade slice; records full-extract as
the deferred ideal; embeds the ADR-0028 8-invariant canon-audit
checklist for Plans 2-3.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Make `SharedKey` / `SigningKey` static-usable

`BearerTokenCredential` uses `SecretToken` which already has
`identity_state!` (`scheme/secret_token.rs:45`). `SharedKey`/`SigningKey`
need the same so they can be a credential's `type State`.

**Files:**
- Modify: `crates/credential/src/scheme/shared_key.rs`
- Modify: `crates/credential/src/scheme/signing_key.rs`

- [ ] **Step 1: Verify the macro is absent (do not double-invoke)**

Run: `grep -n "identity_state!" crates/credential/src/scheme/shared_key.rs crates/credential/src/scheme/signing_key.rs`
Expected: no matches (exit 1). If a match exists, skip the corresponding edit below — the scheme is already static-usable.

- [ ] **Step 2: Add `identity_state!` to `shared_key.rs`**

In `crates/credential/src/scheme/shared_key.rs`, the `impl SharedKey { ... }` block ends at line ~58 and the `impl std::fmt::Debug for SharedKey` begins after it. Immediately before the `impl std::fmt::Debug for SharedKey` line, insert:

```rust
// Static credentials use State = Scheme (identity projection).
identity_state!(SharedKey, "shared_key", 1);
```

Then ensure `identity_state` is in scope: the file's top `use` is
`use crate::{AuthScheme, SecretString};`. Change it to:

```rust
use crate::{AuthScheme, SecretString, identity_state};
```

- [ ] **Step 3: Add `identity_state!` to `signing_key.rs`**

In `crates/credential/src/scheme/signing_key.rs`, immediately before the
`impl std::fmt::Debug for SigningKey` line, insert:

```rust
// Static credentials use State = Scheme (identity projection).
identity_state!(SigningKey, "signing_key", 1);
```

Change the top `use crate::{AuthScheme, SecretString};` to:

```rust
use crate::{AuthScheme, SecretString, identity_state};
```

- [ ] **Step 4: Build the contract crate**

Run: `cargo build -p nebula-credential`
Expected: `Finished` — no errors. (`identity_state!` mirrors the verified
`secret_token.rs:45` invocation; `SharedKey`/`SigningKey` already derive
`Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop` which satisfy the
`CredentialState` supertrait bounds.)

- [ ] **Step 5: Run the contract crate's scheme tests**

Run: `cargo test -p nebula-credential --lib scheme::`
Expected: PASS — existing scheme tests (`pattern_is_*`, `debug_redacts_*`) still green; no new failures.

- [ ] **Step 6: Commit**

```bash
git add crates/credential/src/scheme/shared_key.rs crates/credential/src/scheme/signing_key.rs
git commit -m "feat(credential): make SharedKey/SigningKey static-usable

Additive identity_state! invocations mirroring scheme/secret_token.rs:45
so these schemes can be a credential's State (needed by the builtin
reference credentials). No behaviour change to existing consumers.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: `BearerTokenCredential` reference impl

**Files:**
- Create: `crates/credential-builtin/src/bearer_token.rs`
- Modify: `crates/credential-builtin/src/lib.rs` (add `pub mod bearer_token;` + re-export)
- Test: `crates/credential-builtin/src/bearer_token.rs` (`#[cfg(test)] mod tests`)

Import paths mirror `crates/credential/src/credentials/basic_auth.rs:6-13`
with `crate::` → `nebula_credential::`. If `cargo build` reports an
unresolved path, the contract crate re-exports that item at its root —
switch to `nebula_credential::<Item>` (rustc prints the exact path).

- [ ] **Step 1: Write the failing test**

Create `crates/credential-builtin/src/bearer_token.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_credential::{CredentialContext, SecretString};
    use nebula_schema::FieldValues;

    #[test]
    fn key_is_bearer_token() {
        assert_eq!(BearerTokenCredential::KEY, "bearer_token");
    }

    #[tokio::test]
    async fn resolve_wraps_token_into_secret_token() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("token", serde_json::Value::String("sk-abc123".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("test-user");
        let result = BearerTokenCredential::resolve(&values, &ctx)
            .await
            .expect("resolve ok");
        match result {
            nebula_credential::contract::resolve::ResolveResult::Complete(scheme) => {
                use nebula_credential::scheme::SecretToken;
                let _: &SecretToken = &scheme;
                assert_eq!(scheme.token().expose_secret(), "sk-abc123");
            }
            _ => panic!("expected Complete"),
        }
    }

    #[tokio::test]
    async fn resolve_errors_on_missing_token() {
        let values = FieldValues::new();
        let ctx = CredentialContext::for_test("test-user");
        assert!(BearerTokenCredential::resolve(&values, &ctx).await.is_err());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nebula-credential-builtin --lib bearer_token`
Expected: FAIL — `cannot find type BearerTokenCredential`.

- [ ] **Step 3: Implement the credential (prepend above the test module)**

Prepend to `crates/credential-builtin/src/bearer_token.rs` (keep the
`#[cfg(test)] mod tests` from Step 1 at the bottom):

```rust
//! Opaque bearer-token credential — static, non-interactive.
//!
//! Resolves a single secret token into [`SecretToken`]. `State = Scheme`
//! (identity projection). Reference impl mirroring the contract crate's
//! `BasicAuthCredential` shape.

use nebula_credential::contract::plugin_capability_report;
use nebula_credential::contract::resolve::ResolveResult;
use nebula_credential::metadata::CredentialMetadata;
use nebula_credential::scheme::SecretToken;
use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, SecretString,
};
use nebula_schema::{FieldValues, Schema};
use serde::Deserialize;

/// Setup-form shape for the `bearer_token` credential.
#[derive(Schema, Deserialize, Default)]
pub struct BearerTokenProperties {
    /// The opaque bearer token (API key, PAT, session token).
    #[field(secret, label = "Token")]
    #[validate(required)]
    pub token: String,
}

/// Static opaque-token credential. Projects stored state (the token)
/// directly as the auth scheme.
pub struct BearerTokenCredential;

impl Credential for BearerTokenCredential {
    type Properties = BearerTokenProperties;
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "bearer_token";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("bearer_token"))
            .name("Bearer Token")
            .description("Opaque bearer token (API key, PAT, session token).")
            .schema(Self::properties_schema())
            .pattern(AuthPattern::SecretToken)
            .icon("key")
            .build()
            .expect("bearer_token metadata is valid")
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        let token = values.get_string_by_str("token").ok_or_else(|| {
            CredentialError::Provider("missing required field 'token'".to_owned())
        })?;
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new(token.to_owned()),
        )))
    }
}

impl plugin_capability_report::IsInteractive for BearerTokenCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for BearerTokenCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for BearerTokenCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for BearerTokenCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for BearerTokenCredential {
    const VALUE: bool = false;
}
```

- [ ] **Step 4: Wire the module in `crates/credential-builtin/src/lib.rs`**

After the `pub(crate) mod sealed_caps { ... }` block and the trailing
comment, append:

```rust
pub mod bearer_token;

pub use bearer_token::{BearerTokenCredential, BearerTokenProperties};
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p nebula-credential-builtin --lib bearer_token`
Expected: PASS — `3 passed`. If a `use nebula_credential::...` path is
unresolved, apply rustc's suggested path (the type is exported by the
contract crate; only the path differs) and re-run.

- [ ] **Step 6: Lint clean**

Run: `cargo clippy -p nebula-credential-builtin -- -D warnings`
Expected: exit 0, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/credential-builtin/src/bearer_token.rs crates/credential-builtin/src/lib.rs
git commit -m "feat(credential-builtin): BearerTokenCredential reference impl

First net-new static reference credential (SecretToken scheme), mirroring
the BasicAuthCredential shape. Zero upper-tier deps; charter intact.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: `SharedKeyCredential` reference impl

**Files:**
- Create: `crates/credential-builtin/src/shared_key.rs`
- Modify: `crates/credential-builtin/src/lib.rs`
- Test: `crates/credential-builtin/src/shared_key.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/credential-builtin/src/shared_key.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_credential::CredentialContext;
    use nebula_schema::FieldValues;

    #[test]
    fn key_is_shared_key() {
        assert_eq!(SharedKeyCredential::KEY, "shared_key");
    }

    #[tokio::test]
    async fn resolve_wraps_key_into_shared_key() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("key", serde_json::Value::String("psk-xyz".into()))
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("u");
        let r = SharedKeyCredential::resolve(&values, &ctx)
            .await
            .expect("ok");
        match r {
            nebula_credential::contract::resolve::ResolveResult::Complete(s) => {
                assert_eq!(s.key().expose_secret(), "psk-xyz");
            }
            _ => panic!("expected Complete"),
        }
    }

    #[tokio::test]
    async fn resolve_errors_on_missing_key() {
        let ctx = CredentialContext::for_test("u");
        assert!(SharedKeyCredential::resolve(&FieldValues::new(), &ctx)
            .await
            .is_err());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nebula-credential-builtin --lib shared_key`
Expected: FAIL — `cannot find type SharedKeyCredential`.

- [ ] **Step 3: Implement (prepend above the test module)**

```rust
//! Pre-shared symmetric-key credential — static, non-interactive.
//! Resolves a secret key into [`SharedKey`]; `State = Scheme`.

use nebula_credential::contract::plugin_capability_report;
use nebula_credential::contract::resolve::ResolveResult;
use nebula_credential::metadata::CredentialMetadata;
use nebula_credential::scheme::SharedKey;
use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, SecretString,
};
use nebula_schema::{FieldValues, Schema};
use serde::Deserialize;

/// Setup-form shape for the `shared_key` credential.
#[derive(Schema, Deserialize, Default)]
pub struct SharedKeyProperties {
    /// The pre-shared symmetric key material.
    #[field(secret, label = "Pre-shared key")]
    #[validate(required)]
    pub key: String,
}

/// Static pre-shared-key credential.
pub struct SharedKeyCredential;

impl Credential for SharedKeyCredential {
    type Properties = SharedKeyProperties;
    type Scheme = SharedKey;
    type State = SharedKey;

    const KEY: &'static str = "shared_key";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("shared_key"))
            .name("Pre-shared Key")
            .description("Pre-shared symmetric key (TLS-PSK, WireGuard, IoT).")
            .schema(Self::properties_schema())
            .pattern(AuthPattern::SharedSecret)
            .icon("key")
            .build()
            .expect("shared_key metadata is valid")
    }

    fn project(state: &SharedKey) -> SharedKey {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SharedKey, ()>, CredentialError> {
        let key = values.get_string_by_str("key").ok_or_else(|| {
            CredentialError::Provider("missing required field 'key'".to_owned())
        })?;
        Ok(ResolveResult::Complete(SharedKey::new(SecretString::new(
            key.to_owned(),
        ))))
    }
}

impl plugin_capability_report::IsInteractive for SharedKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for SharedKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for SharedKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for SharedKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for SharedKeyCredential {
    const VALUE: bool = false;
}
```

- [ ] **Step 4: Wire in `lib.rs`**

Append to `crates/credential-builtin/src/lib.rs`:

```rust
pub mod shared_key;

pub use shared_key::{SharedKeyCredential, SharedKeyProperties};
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p nebula-credential-builtin --lib shared_key`
Expected: PASS — `3 passed`.

- [ ] **Step 6: Lint clean**

Run: `cargo clippy -p nebula-credential-builtin -- -D warnings`
Expected: exit 0.

- [ ] **Step 7: Commit**

```bash
git add crates/credential-builtin/src/shared_key.rs crates/credential-builtin/src/lib.rs
git commit -m "feat(credential-builtin): SharedKeyCredential reference impl

Second net-new static reference credential (SharedKey scheme).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: `SigningKeyCredential` reference impl

**Files:**
- Create: `crates/credential-builtin/src/signing_key.rs`
- Modify: `crates/credential-builtin/src/lib.rs`
- Test: `crates/credential-builtin/src/signing_key.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/credential-builtin/src/signing_key.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_credential::CredentialContext;
    use nebula_schema::FieldValues;

    #[test]
    fn key_is_signing_key() {
        assert_eq!(SigningKeyCredential::KEY, "signing_key");
    }

    #[tokio::test]
    async fn resolve_wraps_key_and_algorithm() {
        let mut values = FieldValues::new();
        values
            .try_set_raw("key", serde_json::Value::String("whsec_1".into()))
            .expect("test-only known-good key");
        values
            .try_set_raw(
                "algorithm",
                serde_json::Value::String("hmac-sha256".into()),
            )
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("u");
        let r = SigningKeyCredential::resolve(&values, &ctx)
            .await
            .expect("ok");
        match r {
            nebula_credential::contract::resolve::ResolveResult::Complete(s) => {
                assert_eq!(s.key().expose_secret(), "whsec_1");
                assert_eq!(s.algorithm(), "hmac-sha256");
            }
            _ => panic!("expected Complete"),
        }
    }

    #[tokio::test]
    async fn resolve_errors_on_missing_key() {
        let mut values = FieldValues::new();
        values
            .try_set_raw(
                "algorithm",
                serde_json::Value::String("hmac-sha256".into()),
            )
            .expect("test-only known-good key");
        let ctx = CredentialContext::for_test("u");
        assert!(SigningKeyCredential::resolve(&values, &ctx)
            .await
            .is_err());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nebula-credential-builtin --lib signing_key`
Expected: FAIL — `cannot find type SigningKeyCredential`.

- [ ] **Step 3: Implement (prepend above the test module)**

```rust
//! Request-signing-key credential — static, non-interactive.
//! Resolves a secret key + algorithm id into [`SigningKey`]; `State = Scheme`.

use nebula_credential::contract::plugin_capability_report;
use nebula_credential::contract::resolve::ResolveResult;
use nebula_credential::metadata::CredentialMetadata;
use nebula_credential::scheme::SigningKey;
use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, SecretString,
};
use nebula_schema::{FieldValues, Schema};
use serde::Deserialize;

/// Setup-form shape for the `signing_key` credential.
#[derive(Schema, Deserialize, Default)]
pub struct SigningKeyProperties {
    /// The signing secret (HMAC key, webhook signing secret).
    #[field(secret, label = "Signing key")]
    #[validate(required)]
    pub key: String,
    /// Algorithm identifier (e.g. `hmac-sha256`, `sigv4`).
    #[field(label = "Algorithm")]
    #[validate(required)]
    pub algorithm: String,
}

/// Static request-signing-key credential.
pub struct SigningKeyCredential;

impl Credential for SigningKeyCredential {
    type Properties = SigningKeyProperties;
    type Scheme = SigningKey;
    type State = SigningKey;

    const KEY: &'static str = "signing_key";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("signing_key"))
            .name("Signing Key")
            .description("Request-signing secret (HMAC, SigV4, webhook signatures).")
            .schema(Self::properties_schema())
            .pattern(AuthPattern::RequestSigning)
            .icon("key")
            .build()
            .expect("signing_key metadata is valid")
    }

    fn project(state: &SigningKey) -> SigningKey {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SigningKey, ()>, CredentialError> {
        let key = values.get_string_by_str("key").ok_or_else(|| {
            CredentialError::Provider("missing required field 'key'".to_owned())
        })?;
        let algorithm = values.get_string_by_str("algorithm").ok_or_else(|| {
            CredentialError::Provider("missing required field 'algorithm'".to_owned())
        })?;
        Ok(ResolveResult::Complete(SigningKey::new(
            SecretString::new(key.to_owned()),
            algorithm.to_owned(),
        )))
    }
}

impl plugin_capability_report::IsInteractive for SigningKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRefreshable for SigningKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsRevocable for SigningKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsTestable for SigningKeyCredential {
    const VALUE: bool = false;
}
impl plugin_capability_report::IsDynamic for SigningKeyCredential {
    const VALUE: bool = false;
}
```

- [ ] **Step 4: Wire in `lib.rs`**

Append to `crates/credential-builtin/src/lib.rs`:

```rust
pub mod signing_key;

pub use signing_key::{SigningKeyCredential, SigningKeyProperties};
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p nebula-credential-builtin --lib signing_key`
Expected: PASS — `3 passed`.

- [ ] **Step 6: Lint clean**

Run: `cargo clippy -p nebula-credential-builtin -- -D warnings`
Expected: exit 0.

- [ ] **Step 7: Commit**

```bash
git add crates/credential-builtin/src/signing_key.rs crates/credential-builtin/src/lib.rs
git commit -m "feat(credential-builtin): SigningKeyCredential reference impl

Third net-new static reference credential (SigningKey scheme).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: `register_builtins`

**Files:**
- Create: `crates/credential-builtin/src/registry.rs`
- Modify: `crates/credential-builtin/src/lib.rs`
- Test: `crates/credential-builtin/src/registry.rs`

`CredentialRegistry::register<C>(instance, registering_crate: &'static str)`
requires `C: Credential + IsInteractive + IsRefreshable + IsRevocable +
IsTestable + IsDynamic` (verified `contract/registry.rs:122-134`). All
three reference credentials satisfy this (Tasks 5–7 impl the five `Is*`).

- [ ] **Step 1: Write the failing test**

Create `crates/credential-builtin/src/registry.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::register_builtins;
    use nebula_credential::CredentialRegistry;

    #[test]
    fn registers_all_three_reference_credentials() {
        let mut reg = CredentialRegistry::new();
        register_builtins(&mut reg).expect("register_builtins ok");
        assert_eq!(reg.len(), 3);
        assert!(reg.contains("bearer_token"));
        assert!(reg.contains("shared_key"));
        assert!(reg.contains("signing_key"));
    }

    #[test]
    fn register_builtins_is_idempotent_safe_on_fresh_registry() {
        // Two fresh registries must each succeed independently.
        let mut a = CredentialRegistry::new();
        let mut b = CredentialRegistry::new();
        register_builtins(&mut a).expect("a ok");
        register_builtins(&mut b).expect("b ok");
        assert_eq!(a.len(), 3);
        assert_eq!(b.len(), 3);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nebula-credential-builtin --lib registry`
Expected: FAIL — `cannot find function register_builtins`.

- [ ] **Step 3: Implement (prepend above the test module)**

```rust
//! First-party credential registration entry point.
//!
//! `nebula-credential-runtime` (and any composition root) calls
//! [`register_builtins`] to add the first-party reference credential
//! types to a [`CredentialRegistry`] alongside the contract crate's own
//! types and plugin-discovered types.

use nebula_credential::CredentialRegistry;
use nebula_credential::contract::registry::RegisterError;

use crate::{BearerTokenCredential, SharedKeyCredential, SigningKeyCredential};

/// Register every first-party reference credential into `registry`.
///
/// First-wins on duplicate KEY (Tech Spec §15.6) — a `RegisterError`
/// surfaces the colliding key to the operator; the registry is unchanged
/// for the rejected entry.
///
/// # Errors
///
/// Returns [`RegisterError::DuplicateKey`] if any reference KEY is
/// already present in `registry` (e.g. a plugin shipped a colliding KEY).
pub fn register_builtins(registry: &mut CredentialRegistry) -> Result<(), RegisterError> {
    let crate_name = env!("CARGO_CRATE_NAME");
    registry.register(BearerTokenCredential, crate_name)?;
    registry.register(SharedKeyCredential, crate_name)?;
    registry.register(SigningKeyCredential, crate_name)?;
    Ok(())
}
```

> If `nebula_credential::contract::registry::RegisterError` is unresolved,
> it is re-exported at the crate root — use `nebula_credential::RegisterError`
> (rustc prints the exact path; the type is `contract/registry.rs:54`).

- [ ] **Step 4: Wire in `lib.rs`**

Append to `crates/credential-builtin/src/lib.rs`:

```rust
pub mod registry;

pub use registry::register_builtins;
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p nebula-credential-builtin --lib registry`
Expected: PASS — `2 passed`.

- [ ] **Step 6: Run the full builtin test suite**

Run: `cargo nextest run -p nebula-credential-builtin`
Expected: all green — the 3 credentials' tests (Tasks 5–7) + the 2
registry tests + any existing scaffold tests.

- [ ] **Step 7: Commit**

```bash
git add crates/credential-builtin/src/registry.rs crates/credential-builtin/src/lib.rs
git commit -m "feat(credential-builtin): register_builtins entry point

Registers the 3 reference credentials into a CredentialRegistry via
CredentialRegistry::register with CARGO_CRATE_NAME attribution. Consumed
by the credential-runtime composition root in Plan 2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 9: Workspace pre-PR gate

**Files:** none (verification only)

- [ ] **Step 1: Format**

Run: `task fmt`
Expected: exit 0; if it reformats files, `git add -A` the formatting-only
changes and amend them into the most recent relevant commit OR make a
`style(credential-runtime): cargo fmt` commit.

- [ ] **Step 2: Full pre-PR gate**

Run: `task dev:check`
Expected: green across fmt + clippy (`-D warnings`) + nextest + doctests
+ `cargo-deny`. In particular:
- `cargo deny check bans` — no layer-wrapper violation (the new crate has
  no upper-tier deps; acyclic).
- doctests — the spec-referenced scheme doctests (`SecretToken`,
  `SharedKey`, `SigningKey`) still pass after the Task 4 `identity_state!`
  additions.

- [ ] **Step 3: If `dev:check` fails**

Triage by the failing stage. Most likely:
- unresolved `nebula_credential::...` path → apply rustc's suggested
  root re-export path (the type exists; only the path differs).
- `identity_state!` macro error → re-read `scheme/secret_token.rs:45`
  and match its argument arity exactly (`Type, "kind", version`).
- clippy `missing_errors_doc` / `must_use` → add the `# Errors` doc or
  `#[must_use]` (mirror `basic_auth.rs`).
Fix, re-run `task dev:check`, do not proceed until green.

- [ ] **Step 4: Final foundation commit (if any triage fixes were made)**

```bash
git add -A
git commit -m "chore(credential-runtime): green task dev:check for foundation

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage (against `2026-05-15-credential-runtime-subsystem-design.md`):**
- §15 Phase 1 (crate scaffold + layer wiring + ADR-0066) → Tasks 1, 3. ✓
- §15 Phase 2 (3 net-new static reference creds + `register_builtins`, no relocation) → Tasks 4–8. ✓
- §4 deny.toml runtime entry → Task 1 Step 4 (self-scaffold; widened in Plan 2). ✓
- §9 concrete reference set (SecretToken/SharedKey/SigningKey, exact KEYs/patterns) → Tasks 5–7. ✓
- §11 ADR-0066 (narrow supersede + B deferred ideal + ADR-0028 checklist) → Task 3. ✓
- Out of scope for Plan 1 (correctly deferred to Plans 2–3): facade/builder, observability seam, StateSource, API wiring, adversarial e2e tests. Not gaps — sequenced.

**2. Placeholder scan:** No "TBD"/"implement later"/"add error handling". Every code step has complete code. The two "if unresolved path → use rustc's suggestion" notes are deterministic compile-fix instructions against types that *are* defined (in `nebula-credential`), not vague placeholders.

**3. Type consistency:** `BearerTokenCredential`/`SharedKeyCredential`/`SigningKeyCredential` + `*Properties` names are consistent across their create task, the `lib.rs` re-export, and Task 8's `register_builtins` imports. `register_builtins(&mut CredentialRegistry) -> Result<(), RegisterError>` signature matches the verified `CredentialRegistry::register` bounds (all three impl the five `Is*`). `CredentialServiceError` is self-contained in Task 2.

**Plan 2 (facade/observability/StateSource) and Plan 3 (API wiring/e2e) are written after this plan merges**, when their bite-sized steps can reference concrete merged module paths and signatures (avoids spec-forbidden placeholders).
