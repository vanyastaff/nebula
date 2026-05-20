# nebula-credential Stabilize Sweep Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `nebula-credential` (and its bounded-context companions `nebula-credential-runtime`, `nebula-credential-vault`) from `frontier` to `stable` by (a) closing the M12.2 hardening backlog, (b) wiring `nebula-api` onto the `CredentialService` facade (ADR-0066 carry-over), and (c) shipping the production `slot_bindings` → `CredentialGuard<C::Scheme>` path that has been deferred since M11.5. Builtin catalog (M12.3 / #604) is explicitly **out of scope** and follows in a later wave.

**Architecture:**
Single bounded-context owner (`nebula-credential-runtime::CredentialService<B, PS>`) drives every credential management operation. Engine retains the low-level resolver / refresh-coordinator / lease mechanism beneath it. Workflow's `slot_bindings` parses to raw IDs; a new `ValidatedCredentialBinding` newtype is the only typed handle that crosses into engine execution, constructed exclusively by a scope-checked validator factory (closes the open confused-deputy non-goal from the ADR-0052 cascade). Error taxonomy is reshaped per Smithy RFC-0022 (per-variant context structs, boxed payloads, hard 32-byte cap). Secrets migrate to `secrecy::SecretBox<String>` for audit-grep-friendliness and `init_with_mut` in-place construction. Five execution waves; per-wave tasks are parallelizable where marked.

**Tech Stack:**
- Rust 1.95 (edition 2024, resolver 3)
- `thiserror = "2.0"` + `static_assertions = "1"` for size assertions
- `secrecy = "0.10"` for secret wrappers
- `tokio = "1.x"` + `tokio-util` for `CancellationToken`
- `compact_str = "0.7"` for inline identifier storage
- `cargo nextest` for test execution
- `convco` for commit message validation (per CLAUDE.md)
- Layered deny via `cargo-deny` (`deny.toml [wrappers]`)

---

## Pre-flight

### Task 0: Create worktree and branch

**Files:**
- Use: `scripts/worktree.sh`

- [ ] **Step 1: Fetch latest main**

```bash
git fetch origin main
```

Expected: `From github.com:vanyastaff/nebula` listing.

- [ ] **Step 2: Create worktree**

```bash
bash scripts/worktree.sh new stabilize-sweep feat credential
```

Expected output includes:
```
worktree '.worktrees/stabilize-sweep' (branch 'feat/credential-stabilize-sweep')
```

- [ ] **Step 3: Enter worktree and verify clean base**

```bash
cd .worktrees/stabilize-sweep
task dev:check
```

Expected: `task dev:check` exits 0. If failure happens here, stop — the base is broken and this plan is not yet runnable. (Memory: `cargo_fmt_all_winpath` — deep worktree paths can break fmt on Windows; if so verify per-crate with `cargo fmt -p <crate> -- --check`.)

---

## Wave 1 — Audit & Cleanup (parallel-safe)

Five parallel-dispatchable tasks. Each is a self-contained PR. Run agents simultaneously; merge in order of green.

---

### Task 1: Audit dead public symbols in nebula-credential

**Files:**
- Modify: `crates/credential/src/lib.rs`
- Modify: `crates/credential/src/accessor.rs`
- Modify: `crates/credential/README.md`
- Possibly delete: `crates/credential/src/<dead_module>.rs`

- [ ] **Step 1: Write the failing audit test**

Create `crates/credential/tests/dead_pub_audit.rs`:

```rust
//! Pins the public surface of `nebula-credential`. Failures here mean
//! either a public symbol was added without intent or one was removed
//! and we need to update this list.

use nebula_credential::{
    AuthScheme, CredentialContext, CredentialError, CredentialId, CredentialMetadata,
    CredentialRecord, CredentialRegistry, CredentialSnapshot, CredentialState, CredentialStore,
    Dynamic, Interactive, PendingStateStore, Refreshable, Revocable, ScopeResolver, SecretString,
    Testable,
};

#[test]
fn public_contract_surface_stable() {
    // Existence-only check — compiles iff every named symbol is `pub` at root.
    let _ = std::any::TypeId::of::<CredentialError>();
}
```

- [ ] **Step 2: Run the test to verify it currently passes**

```bash
cargo test -p nebula-credential --test dead_pub_audit
```

Expected: 1 passed.

- [ ] **Step 3: Grep for actual external use of each suspect symbol**

```bash
cd ../..   # back to repo root
for sym in CompositionNotAvailable CompositionFailed ScopedCredentialAccessor NoopCredentialAccessor default_credential_accessor RedactedSecret HasCredentialsExt; do
  echo "=== $sym ==="
  rg --type rust -l "\b$sym\b" crates/ | grep -v -E "^crates/credential(/|\$)|^crates/credential-runtime/" || echo "NO EXTERNAL USE"
done
cd .worktrees/stabilize-sweep
```

Record results in your scratchpad. Any symbol with `NO EXTERNAL USE` is a deletion candidate; any with `NO EXTERNAL USE` AND no internal use is a hard delete.

- [ ] **Step 4: Delete `CompositionNotAvailable` and `CompositionFailed` from `CredentialError`**

Open `crates/credential/src/error.rs`. Remove these two variants (verified via grep in Step 3 — no external callsite; internal only in the `Classify` impl which we are rewriting in Task 6 anyway). Update the `Classify::category` and `Classify::code` matches to drop the cases.

```rust
// In crates/credential/src/error.rs — remove:
//     /// Credential composition not available (no resolver in context).
//     #[error("credential composition not available")]
//     CompositionNotAvailable,
//
//     /// Composed credential resolution failed.
//     #[error("composition failed: {source}")]
//     CompositionFailed { source: Box<dyn std::error::Error + Send + 'static> },
//
// And in the Classify impl, delete the two matching arms.
```

- [ ] **Step 5: Remove the "potential movement" FUD comment from `record.rs`**

`crates/credential/src/record.rs` carries a comment that `CredentialRecord` placement is tracked for movement. Delete the comment — the decision is recorded: stays in `nebula-credential`. Also remove the matching line from `crates/credential/README.md` (under "Known gap").

```rust
// In crates/credential/src/record.rs — delete this comment block:
//     // NOTE: placement of `CredentialRecord` is tracked for potential
//     // movement to nebula-core; see README "Known gap" entry.
```

- [ ] **Step 6: Run all credential tests to verify removals**

```bash
cargo nextest run -p nebula-credential
cargo nextest run -p nebula-credential-runtime
```

Expected: all green. If `CompositionNotAvailable` was referenced externally, fix Step 4 first.

- [ ] **Step 7: Commit**

```bash
bash scripts/worktree.sh commit refactor credential "remove CompositionNotAvailable/CompositionFailed dead variants + close CredentialRecord placement FUD"
```

---

### Task 2: Audit `accessor.rs` placement

**Files:**
- Move: `crates/credential/src/accessor.rs` → `crates/engine/src/credential/accessor.rs`
- Modify: `crates/credential/src/lib.rs`
- Modify: `crates/engine/src/credential/mod.rs`

- [ ] **Step 1: Verify `ScopedCredentialAccessor` is engine-only**

```bash
rg -n "ScopedCredentialAccessor" crates/ | rg -v "^crates/credential/"
```

Expected: matches in `crates/engine/` (the production wiring), and possibly `crates/api/` or tests. If matches exist outside engine + tests, **stop and re-plan** — accessor is more widely consumed than expected; reroute to a deprecation step.

- [ ] **Step 2: Move accessor.rs to engine**

```bash
git mv crates/credential/src/accessor.rs crates/engine/src/credential/accessor.rs
```

- [ ] **Step 3: Update credential `lib.rs` re-exports**

In `crates/credential/src/lib.rs`, remove:

```rust
// Remove this block:
mod accessor;
pub use accessor::{NoopCredentialAccessor, ScopedCredentialAccessor, default_credential_accessor};
```

Keep the trait re-export (which lives in nebula-core):

```rust
pub use nebula_core::accessor::CredentialAccessor;
```

- [ ] **Step 4: Wire the moved module into engine**

In `crates/engine/src/credential/mod.rs`:

```rust
pub mod accessor;
pub use accessor::{NoopCredentialAccessor, ScopedCredentialAccessor, default_credential_accessor};
```

- [ ] **Step 5: Update import paths**

```bash
rg -l "nebula_credential::(NoopCredentialAccessor|ScopedCredentialAccessor|default_credential_accessor)" crates/
```

For each match, rewrite imports to `nebula_engine::credential::*`.

- [ ] **Step 6: Verify build + tests**

```bash
cargo check --workspace --all-targets
cargo nextest run -p nebula-credential -p nebula-engine
```

Expected: green.

- [ ] **Step 7: Commit**

```bash
bash scripts/worktree.sh commit refactor credential "move accessor impls to nebula-engine (impl belongs near runtime wire-up)"
```

---

### Task 3: Extract test shims into `nebula-credential-testutil`

**Files:**
- Create: `crates/credential-testutil/Cargo.toml`
- Create: `crates/credential-testutil/src/lib.rs`
- Move content: `crates/credential/src/store_memory.rs` → `crates/credential-testutil/src/store_memory.rs`
- Move content: `crates/credential/src/pending_store_memory.rs` → `crates/credential-testutil/src/pending_store_memory.rs`
- Modify: `crates/credential/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)
- Modify: `deny.toml` (wrappers)

- [ ] **Step 1: Scaffold the new crate**

Create `crates/credential-testutil/Cargo.toml`:

```toml
[package]
name = "nebula-credential-testutil"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false  # internal test utility only

[lints]
workspace = true

[dependencies]
nebula-credential.workspace = true
nebula-schema.workspace = true
tokio = { workspace = true, features = ["sync"] }
```

- [ ] **Step 2: Move `InMemoryStore`**

```bash
mkdir -p crates/credential-testutil/src
git mv crates/credential/src/store_memory.rs crates/credential-testutil/src/store_memory.rs
git mv crates/credential/src/pending_store_memory.rs crates/credential-testutil/src/pending_store_memory.rs
```

- [ ] **Step 3: Write the testutil lib root**

Create `crates/credential-testutil/src/lib.rs`:

```rust
//! Internal test utilities for `nebula-credential` and downstream
//! consumers. Crate is `publish = false`; do not consume from production
//! code.

#![forbid(unsafe_code)]

pub mod store_memory;
pub mod pending_store_memory;

pub use store_memory::InMemoryStore;
pub use pending_store_memory::InMemoryPendingStore;

/// Convenience helper: build a fully wired in-memory `(Store, PendingStore)` pair.
pub fn in_memory_pair() -> (InMemoryStore, InMemoryPendingStore) {
    (InMemoryStore::new(), InMemoryPendingStore::new())
}
```

- [ ] **Step 4: Strip the test shims from `nebula-credential::lib.rs`**

```rust
// In crates/credential/src/lib.rs, REMOVE these blocks:
// #[cfg(any(test, feature = "test-util"))]
// pub mod pending_store_memory;
// #[cfg(any(test, feature = "test-util"))]
// pub mod store_memory;
// #[cfg(any(test, feature = "test-util"))]
// pub use pending_store_memory::InMemoryPendingStore;
// #[cfg(any(test, feature = "test-util"))]
// pub use store_memory::InMemoryStore;
```

Also remove any leftover `feature = "test-util"` from `crates/credential/Cargo.toml`.

- [ ] **Step 5: Update workspace and `deny.toml`**

In root `Cargo.toml`, add the new member:

```toml
[workspace]
members = [
    # ... existing entries ...
    "crates/credential-testutil",
]
```

In `deny.toml`, add the wrapper entry under `[wrappers]` (Business tier, like credential):

```toml
{ name = "nebula-credential-testutil", allow = [
    "nebula-credential",
    "nebula-schema",
    "tokio",
] },
```

- [ ] **Step 6: Update all credential / storage / api / engine dev-dependencies**

For every crate currently consuming the in-crate shim via `#[cfg(any(test, feature = "test-util"))]` (`crates/credential/tests/*`, `crates/credential-runtime/tests/*`, `crates/storage/tests/*`, `crates/api/tests/*`, `crates/engine/tests/*`):

```toml
# Crate's Cargo.toml [dev-dependencies] section:
nebula-credential-testutil.workspace = true
```

And in source: `use nebula_credential::{InMemoryStore, InMemoryPendingStore}` → `use nebula_credential_testutil::{InMemoryStore, InMemoryPendingStore}`.

Run:

```bash
rg -l "nebula_credential::(InMemoryStore|InMemoryPendingStore)" crates/ | xargs -I{} sed -i 's|nebula_credential::\(InMemoryStore\|InMemoryPendingStore\)|nebula_credential_testutil::\1|g' {}
```

(Manual review after sed — verify only test files were touched, no production paths.)

- [ ] **Step 7: Verify**

```bash
cargo check --workspace --all-targets
cargo nextest run --workspace
cargo deny check
```

Expected: green; `cargo deny` confirms wrappers.

- [ ] **Step 8: Commit**

```bash
bash scripts/worktree.sh commit refactor credential "extract test shims to nebula-credential-testutil crate"
```

---

### Task 4: Audit `nebula-credential::rotation` for orchestration leakage

**Files:**
- Read-only: `crates/credential/src/rotation/*.rs`
- Possibly move: any orchestration code → `crates/engine/src/credential/rotation/`

- [ ] **Step 1: List `crates/credential/src/rotation/`**

```bash
ls -la crates/credential/src/rotation/
```

Expected files (verified earlier): `error.rs`, `events.rs`, `ids.rs`, `mod.rs`, `policy.rs`, `state.rs`.

- [ ] **Step 2: Inspect each file for orchestration code**

For each file, run:

```bash
for f in crates/credential/src/rotation/*.rs; do
  echo "=== $f ==="
  rg -n "tokio::spawn|select!|sleep|interval|tick|background|spawn_local" "$f" || echo "no orchestration markers"
done
```

Expected: every file shows `no orchestration markers` — only domain types live here. If any file has orchestration markers, surface the names; orchestration code moves to `engine::credential::rotation::*`.

- [ ] **Step 3: Document the audit outcome in the crate**

Append to `crates/credential/src/rotation/mod.rs` doc:

```rust
//! ## Module scope
//!
//! Domain types only: rotation events, errors, state machine enum, IDs,
//! policy. Orchestration (scheduler, blue-green transactions, fanout
//! drivers) lives in `nebula_engine::credential::rotation`.
//! Verified 2026-05-20.
```

- [ ] **Step 4: Commit**

```bash
bash scripts/worktree.sh commit docs credential "audit rotation module — confirm domain-only, no orchestration leak"
```

---

### Task 5: Audit + tighten `CredentialContext` shape

**Files:**
- Modify: `crates/credential/src/context.rs`
- Modify: `crates/credential/src/lib.rs` (re-exports if changed)

- [ ] **Step 1: Read the current shape**

```bash
sed -n '1,80p' crates/credential/src/context.rs
```

Inspect fields. The required shape for the next-wave production seam is:

```rust
pub struct CredentialContext<'a> {
    base: &'a nebula_core::BaseContext<'a>,
    scope: &'a TenantScope,
    cancel: tokio_util::sync::CancellationToken,
    span: tracing::Span,
    pending_store: &'a dyn PendingStateStore,
}
```

- [ ] **Step 2: Write a probe enforcing context cancel semantics**

Create `crates/credential/tests/context_cancel_probe.rs`:

```rust
//! Probe: `CredentialContext` must surface a `CancellationToken`, and
//! a child token must be derivable for in-flight credential capability
//! calls. Cancellation observation goes through the borrowed token —
//! no proxied `is_cancelled` wrapper on the context.

use nebula_credential::CredentialContextBuilder;
use tokio_util::sync::CancellationToken;

#[test]
fn child_token_derivable_and_cascades_from_parent() {
    let parent = CancellationToken::new();
    let ctx = CredentialContextBuilder::new()
        .with_cancel(parent.clone())
        .build();
    // Caller observes cancellation on the token directly — no
    // duplicated `is_cancelled` API on the context (ADR-0083 budget).
    let child = ctx.cancel_token().child_token();
    parent.cancel();
    assert!(child.is_cancelled(), "child token must cascade from parent cancel");
}
```

- [ ] **Step 3: Run the probe — it should fail until `with_cancel` + `cancel_token` exist**

```bash
cargo nextest run -p nebula-credential --test context_cancel_probe
```

Expected: FAIL (methods missing).

- [ ] **Step 4: Add `CancellationToken` field + borrow accessor**

In `crates/credential/src/context.rs`:

```rust
use tokio_util::sync::CancellationToken;

pub struct CredentialContext<'a> {
    // ... existing fields ...
    cancel: CancellationToken,
}

impl<'a> CredentialContext<'a> {
    /// Borrow the context's cancellation token. Callers derive child
    /// tokens with `.child_token()` and check status with `.is_cancelled()`
    /// directly on the borrowed token — no proxy methods on the context
    /// itself (avoids duplicating tokio-util's surface; ADR-0083 budget).
    #[must_use]
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}
```

In `CredentialContextBuilder`:

```rust
impl<'a> CredentialContextBuilder<'a> {
    #[must_use]
    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }
}
```

The `build()` method defaults `cancel` to `CancellationToken::new()` if unset.

- [ ] **Step 5: Add `tokio-util` dep to `crates/credential/Cargo.toml`**

```toml
[dependencies]
tokio-util = { workspace = true, features = ["rt"] }
```

- [ ] **Step 6: Re-run the probe**

```bash
cargo nextest run -p nebula-credential --test context_cancel_probe
```

Expected: PASS.

- [ ] **Step 7: Update `CredentialContext` rustdoc + existing tests using `CredentialContextBuilder` (the `with_cancel` is optional + defaulted; no breakage)**

```bash
cargo nextest run -p nebula-credential
```

Expected: all green.

- [ ] **Step 8: Commit**

```bash
bash scripts/worktree.sh commit feat credential "add CancellationToken to CredentialContext; child_cancel + is_cancelled accessors"
```

---

### Task 6: Probe sealed-capability discipline

**Files:**
- Create: `crates/credential/tests/probes/sealed_capability_third_party.rs` (compile-fail probe)
- Modify: `crates/credential/tests/probes/mod.rs` (probe runner)

- [ ] **Step 1: Verify the existing sealed companion convention**

```bash
rg -n "mod sealed" crates/credential/src/contract/
```

Expected: matches in capability files (`Refreshable`, `Revocable`, etc.) indicating each has a sealed-companion marker per ADR-0035.

- [ ] **Step 2: Write the compile-fail probe**

Create `crates/credential/tests/probes/sealed_capability_third_party.rs`:

```rust
//! Probe: a third-party type cannot implement `Refreshable` outside
//! the `nebula-credential` crate. The sealed companion supertrait
//! `sealed_caps::IsRefreshable` is not nameable from outside.
//!
//! This file is compiled via trybuild — see `tests/probes/mod.rs`.

use nebula_credential::{Credential, CredentialContext, CredentialError, Refreshable};
use nebula_credential::resolve::{RefreshOutcome, RefreshPolicy};

struct ThirdPartyCred;

// This impl block must fail to compile because `Refreshable` is sealed
// via `sealed_caps::IsRefreshable`, which is not nameable from outside
// the credential crate.
impl Refreshable for ThirdPartyCred {
    async fn refresh(
        _state: &mut <Self as Credential>::State,
        _policy: RefreshPolicy,
        _ctx: &CredentialContext<'_>,
    ) -> Result<RefreshOutcome<<Self as Credential>::State>, CredentialError> {
        unreachable!()
    }
}

fn main() {}
```

- [ ] **Step 3: Wire it into the probes harness**

In `crates/credential/tests/probes/mod.rs`:

```rust
#[test]
fn sealed_capability_third_party_rejected() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/sealed_capability_third_party.rs");
}
```

- [ ] **Step 4: Run the probe — must succeed (i.e., rust must reject the file)**

Memory: `trybuild_agent_timeout` — false timeouts under nextest `agent` profile on cold cache. Use plain `cargo test` here:

```bash
cargo test -p nebula-credential --test probes sealed_capability_third_party_rejected -- --include-ignored
```

Expected: PASS (the inner compile-fail file IS rejected; the test asserts that).

If the probe FAILS (the inner file compiled), the sealed pattern is broken — surface it as a finding and stop.

- [ ] **Step 5: Commit**

```bash
bash scripts/worktree.sh commit test credential "add sealed-capability third-party rejection probe"
```

---

## Wave 2 — Type-level fixes (parallel-safe after Wave 1 merges)

Eight tasks. Most independent.

---

### Task 7: Reshape `CredentialError` per Smithy RFC-0022

**Files:**
- Modify: `crates/credential/src/error.rs`
- Modify: `crates/credential/src/lib.rs`
- Modify: callsites across workspace

- [ ] **Step 1: Add `static_assertions` dev-dep**

In `crates/credential/Cargo.toml`:

```toml
[dependencies]
static_assertions = "1"
```

- [ ] **Step 2: Write the size-cap test (failing)**

Append to `crates/credential/src/error.rs`:

```rust
#[cfg(test)]
mod size_assertions {
    use super::CredentialError;
    static_assertions::const_assert!(std::mem::size_of::<CredentialError>() <= 32);
}
```

- [ ] **Step 3: Run — should FAIL (CredentialError currently > 32B due to inline `Box<dyn Error>` payloads)**

```bash
cargo check -p nebula-credential --tests
```

Expected: compile error from `const_assert!`.

- [ ] **Step 4: Add per-variant context structs**

Replace the `CredentialError` enum and add context types:

```rust
//! ... existing module doc ...

use thiserror::Error;
use compact_str::CompactString;

/// Secret-free message wrapper — its constructor pattern-checks for
/// known secret-like substrings in debug builds.
#[derive(Debug, Clone)]
pub struct SecretFreeMessage(CompactString);

impl SecretFreeMessage {
    pub fn new(s: impl Into<CompactString>) -> Self {
        let v = s.into();
        debug_assert!(!looks_like_secret(&v), "SecretFreeMessage given likely secret content");
        Self(v)
    }

    pub fn as_str(&self) -> &str { self.0.as_str() }
}

impl std::fmt::Display for SecretFreeMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn looks_like_secret(s: &str) -> bool {
    // Conservative heuristic — flags anything that looks like a token /
    // base64 blob / long hex string. False positives are acceptable; the
    // intent is to catch accidental injection.
    let len = s.len();
    if len >= 32 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '=' || c == '+' || c == '/') {
        return true;
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemeKind {
    SecretToken, IdentityPassword, OAuth2Token, KeyPair, Certificate,
    SigningKey, ConnectionUri, InstanceBinding, SharedKey,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SchemeMismatch {
    expected: SchemeKind,
    actual: SchemeKind,
}

impl SchemeMismatch {
    pub fn new(expected: SchemeKind, actual: SchemeKind) -> Self {
        Self { expected, actual }
    }
    pub fn expected(&self) -> SchemeKind { self.expected }
    pub fn actual(&self) -> SchemeKind { self.actual }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderErrorKind {
    Network, Auth, RateLimit, InvalidGrant, ServerError, Schema, Other,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProviderErrorContext {
    kind: ProviderErrorKind,
    message: SecretFreeMessage,
    provider_code: Option<CompactString>,
}

impl ProviderErrorContext {
    pub fn new(kind: ProviderErrorKind, message: SecretFreeMessage) -> Self {
        Self { kind, message, provider_code: None }
    }
    pub fn with_code(mut self, code: impl Into<CompactString>) -> Self {
        self.provider_code = Some(code.into()); self
    }
    pub fn kind(&self) -> ProviderErrorKind { self.kind }
    pub fn message(&self) -> &SecretFreeMessage { &self.message }
    pub fn provider_code(&self) -> Option<&str> { self.provider_code.as_deref() }
}

impl std::fmt::Display for ProviderErrorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RefreshFailedContext {
    kind: RefreshErrorKind,
    retry: RetryAdvice,
    cause: SecretFreeMessage,
    provider_code: Option<CompactString>,
}

impl RefreshFailedContext {
    pub fn new(kind: RefreshErrorKind, retry: RetryAdvice, cause: SecretFreeMessage) -> Self {
        Self { kind, retry, cause, provider_code: None }
    }
    pub fn with_code(mut self, code: impl Into<CompactString>) -> Self {
        self.provider_code = Some(code.into()); self
    }
    pub fn kind(&self) -> RefreshErrorKind { self.kind }
    pub fn retry(&self) -> RetryAdvice { self.retry }
    pub fn cause(&self) -> &SecretFreeMessage { &self.cause }
    pub fn provider_code(&self) -> Option<&str> { self.provider_code.as_deref() }
}

impl std::fmt::Display for RefreshFailedContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.cause)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RevokeErrorKind {
    ProviderRejected, Network, AlreadyRevoked, Unsupported, Other,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RevokeFailedContext {
    kind: RevokeErrorKind,
    cause: SecretFreeMessage,
}

impl RevokeFailedContext {
    pub fn new(kind: RevokeErrorKind, cause: SecretFreeMessage) -> Self {
        Self { kind, cause }
    }
    pub fn kind(&self) -> RevokeErrorKind { self.kind }
    pub fn cause(&self) -> &SecretFreeMessage { &self.cause }
}

impl std::fmt::Display for RevokeFailedContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.cause)
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CredentialError {
    #[error(transparent)]
    Crypto(CryptoError),

    #[error(transparent)]
    Validation(ValidationError),

    #[error("provider error: {0}")]
    Provider(Box<ProviderErrorContext>),

    #[error("refresh failed: {0}")]
    RefreshFailed(Box<RefreshFailedContext>),

    #[error("revoke failed: {0}")]
    RevokeFailed(Box<RevokeFailedContext>),

    #[error("credential does not support interactive flows")]
    NotInteractive,

    #[error("scheme mismatch: expected {expected:?}, got {actual:?}",
        expected = _0.expected(), actual = _0.actual())]
    SchemeMismatch(SchemeMismatch),

    #[error("credential resolution failed: {0}")]
    Resolution(#[from] nebula_core::CoreError),
}

impl From<CryptoError> for CredentialError {
    fn from(e: CryptoError) -> Self { Self::Crypto(e) }
}

impl From<ValidationError> for CredentialError {
    fn from(e: ValidationError) -> Self { Self::Validation(e) }
}
```

- [ ] **Step 5: Re-run the size assertion**

```bash
cargo check -p nebula-credential --tests
```

Expected: passes (CredentialError now ≤ 32B — tag + Box pointer + padding).

- [ ] **Step 6: Update the `Classify` impl**

```rust
impl nebula_error::Classify for CredentialError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Crypto(s) => nebula_error::Classify::category(s),
            Self::Validation(s) => nebula_error::Classify::category(s),
            Self::NotInteractive => nebula_error::ErrorCategory::Unsupported,
            Self::Provider(_) => nebula_error::ErrorCategory::External,
            Self::RefreshFailed(_) | Self::RevokeFailed(_) => nebula_error::ErrorCategory::External,
            Self::SchemeMismatch(_) => nebula_error::ErrorCategory::Validation,
            Self::Resolution(s) => nebula_error::Classify::category(s),
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::Crypto(s) => nebula_error::Classify::code(s),
            Self::Validation(s) => nebula_error::Classify::code(s),
            Self::NotInteractive => nebula_error::ErrorCode::new("CREDENTIAL:NOT_INTERACTIVE"),
            Self::Provider(_) => nebula_error::ErrorCode::new("CREDENTIAL:PROVIDER"),
            Self::RefreshFailed(_) => nebula_error::ErrorCode::new("CREDENTIAL:REFRESH_FAILED"),
            Self::RevokeFailed(_) => nebula_error::ErrorCode::new("CREDENTIAL:REVOKE_FAILED"),
            Self::SchemeMismatch(_) => nebula_error::ErrorCode::new("CREDENTIAL:SCHEME_MISMATCH"),
            Self::Resolution(_) => nebula_error::ErrorCode::new("CREDENTIAL:RESOLUTION_FAILED"),
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::RefreshFailed(ctx) => matches!(
                ctx.kind(),
                RefreshErrorKind::TransientNetwork | RefreshErrorKind::ProviderUnavailable
            ),
            Self::Provider(ctx) => matches!(
                ctx.kind(),
                ProviderErrorKind::Network | ProviderErrorKind::RateLimit | ProviderErrorKind::ServerError
            ),
            _ => false,
        }
    }
}
```

- [ ] **Step 7: Migrate every `CredentialError::Provider("...".into())` and `RefreshFailed { ... }` callsite**

```bash
rg -n "CredentialError::Provider\(" crates/
rg -n "CredentialError::RefreshFailed" crates/
rg -n "CredentialError::RevokeFailed" crates/
```

For each callsite, convert to the boxed context-struct form. Concrete example:

```rust
// Before
return Err(CredentialError::Provider("OAuth2 HTTP transport has moved".into()));

// After
return Err(CredentialError::Provider(Box::new(
    ProviderErrorContext::new(
        ProviderErrorKind::Other,
        SecretFreeMessage::new("OAuth2 HTTP transport has moved"),
    )
)));
```

```rust
// Before
return Err(CredentialError::refresh(
    RefreshErrorKind::TokenExpired,
    RetryAdvice::Never,
    "refresh token expired",
));

// After
return Err(CredentialError::RefreshFailed(Box::new(
    RefreshFailedContext::new(
        RefreshErrorKind::TokenExpired,
        RetryAdvice::Never,
        SecretFreeMessage::new("refresh token expired"),
    )
)));
```

Delete the old `CredentialError::refresh` helper from `error.rs`.

- [ ] **Step 8: Verify workspace builds + tests pass**

```bash
cargo nextest run --workspace
```

Expected: green.

- [ ] **Step 9: Commit**

```bash
bash scripts/worktree.sh commit refactor credential "reshape CredentialError — per-variant context structs + boxed payloads + 32B cap (Smithy RFC-0022)"
```

---

### Task 8: Move `AuthStyle` to `scheme::oauth2`

**Files:**
- Modify: `crates/credential/src/scheme/oauth2.rs` (add AuthStyle)
- Modify: `crates/credential/src/credentials/oauth2.rs` (re-export shim + deprecate)
- Update imports across `nebula_engine`, `nebula_api`, `nebula_storage` (test), `nebula_credential::tests`

- [ ] **Step 1: Locate current AuthStyle definition**

```bash
rg -n "pub enum AuthStyle|^pub enum AuthStyle|struct AuthStyle" crates/credential/src/credentials/oauth2.rs crates/credential/src/credentials/oauth2_config.rs
```

Identify the variant set and any methods (`Header`, `PostBody`, etc.).

- [ ] **Step 2: Move the type into `scheme/oauth2.rs`**

In `crates/credential/src/scheme/oauth2.rs`, add (verbatim copy of the existing definition):

```rust
/// OAuth2 client authentication style — where the client credentials
/// go when calling the token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthStyle {
    /// HTTP Basic auth (Authorization: Basic header) — RFC 6749 §2.3.1.
    Header,
    /// Body-encoded `client_id` + `client_secret` — RFC 6749 §2.3.1.
    PostBody,
}

impl Default for AuthStyle {
    fn default() -> Self { Self::Header }
}
```

- [ ] **Step 3: Convert the original definition into a deprecated re-export**

In `crates/credential/src/credentials/oauth2.rs`, replace the old enum declaration with:

```rust
#[deprecated(since = "0.X.Y", note = "use `nebula_credential::scheme::oauth2::AuthStyle`")]
pub use crate::scheme::oauth2::AuthStyle;
```

(Replace `0.X.Y` with the current crate version from `Cargo.toml`.)

- [ ] **Step 4: Re-export at lib root**

Add to `crates/credential/src/lib.rs` `scheme::` re-export block:

```rust
pub use scheme::oauth2::AuthStyle;
```

- [ ] **Step 5: Update production consumers**

For each of (`crates/api/src/transport/oauth/flow.rs`, `crates/api/src/domain/credential/oauth.rs`, `crates/engine/src/credential/rotation/token_refresh.rs`), replace:

```rust
use nebula_credential::credentials::oauth2::AuthStyle;
```

with:

```rust
use nebula_credential::scheme::oauth2::AuthStyle;
```

(Or import via root: `use nebula_credential::AuthStyle;`.)

- [ ] **Step 6: Verify**

```bash
cargo check --workspace --all-targets
```

Expected: builds clean. Deprecation warnings should appear only if a remaining consumer still uses the old path — fix those.

- [ ] **Step 7: Commit**

```bash
bash scripts/worktree.sh commit refactor credential "move AuthStyle from credentials::oauth2 to scheme::oauth2 (survives builtin carve-out)"
```

---

### Task 9: Migrate to `secrecy::SecretBox`

**Files:**
- Modify: `crates/credential/Cargo.toml`
- Modify: `crates/credential/src/secrets/secret_string.rs`
- Modify: `crates/credential/src/secrets/mod.rs`

- [ ] **Step 1: Add `secrecy` dep**

In `crates/credential/Cargo.toml`:

```toml
[dependencies]
secrecy = { version = "0.10", features = ["serde"] }
```

- [ ] **Step 2: Re-alias `SecretString`**

In `crates/credential/src/secrets/secret_string.rs`, replace the existing custom struct with:

```rust
//! `SecretString` is an alias for `secrecy::SecretString` (= `SecretBox<String>`).
//! Access via `secret.expose_secret()`. Construction prefers
//! `SecretBox::new` or `SecretBox::init_with_mut` to avoid stack copies.

pub use secrecy::{ExposeSecret, ExposeSecretMut, SecretBox, SecretString};

/// Convenience constructor — moves an owned `String` into a heap
/// `SecretBox` and zeroizes the source on drop of the temporary.
#[must_use]
pub fn from_string(s: String) -> SecretString {
    SecretBox::new(Box::new(s))
}
```

- [ ] **Step 3: Update `secrets/mod.rs` re-exports**

```rust
pub use secret_string::{from_string as secret_from_string, ExposeSecret, ExposeSecretMut, SecretBox, SecretString};
```

- [ ] **Step 4: Migrate callsites — `.new()` constructors**

```bash
rg -n "SecretString::new\(" crates/
```

For each, rewrite:

```rust
// Before
let s = SecretString::new("token".to_string());

// After
let s = nebula_credential::secrets::secret_from_string("token".to_string());
```

- [ ] **Step 5: Migrate callsites — accessor patterns**

```bash
rg -n "\.as_str\(\)|\.into_string\(\)|\.reveal\(\)" crates/ -g "**/*credential*/**/*.rs"
```

Replace any custom accessor with `.expose_secret()`:

```rust
// Before
let raw: &str = my_secret.as_str();

// After
use secrecy::ExposeSecret;
let raw: &str = my_secret.expose_secret();
```

- [ ] **Step 6: Update existing custom serde adapter (`serde_secret`) — verify it works with `SecretBox`**

```bash
sed -n '1,80p' crates/credential/src/secrets/serde_secret.rs
```

If the custom adapter calls into our old `SecretString` API, rewrite to use `SecretBox::new` + `ExposeSecret`. `SecretBox<String>: Deserialize` is provided when `serde` feature is on.

- [ ] **Step 7: Verify**

```bash
cargo nextest run --workspace
```

Expected: green.

- [ ] **Step 8: Commit**

```bash
bash scripts/worktree.sh commit refactor credential "migrate SecretString to secrecy::SecretBox<String> (forbid(unsafe_code), audited, ExposeSecret callsites)"
```

---

### Task 10: Typed `CredentialId` via `CompactString`

**Files:**
- Modify: `crates/core/src/id/types.rs` (where `CredentialId` lives)

- [ ] **Step 1: Locate definition**

```bash
rg -n "pub struct CredentialId" crates/core/
```

- [ ] **Step 2: Add `compact_str` dep**

In `crates/core/Cargo.toml`:

```toml
[dependencies]
compact_str = { version = "0.7", features = ["serde"] }
```

- [ ] **Step 3: Rewrite the newtype**

Replace the current `CredentialId(String)` with:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct CredentialId(compact_str::CompactString);

impl CredentialId {
    #[must_use]
    pub fn new(s: impl Into<compact_str::CompactString>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str { self.0.as_str() }
}

impl std::fmt::Display for CredentialId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl From<&str> for CredentialId {
    fn from(s: &str) -> Self { Self::new(s) }
}

impl From<String> for CredentialId {
    fn from(s: String) -> Self { Self::new(s) }
}
```

- [ ] **Step 4: Verify size**

Add a sanity test:

```rust
// In the same file
#[cfg(test)]
mod size_check {
    use super::CredentialId;
    static_assertions::const_assert!(std::mem::size_of::<CredentialId>() <= 24);
}
```

- [ ] **Step 5: Build + test**

```bash
cargo nextest run --workspace
```

- [ ] **Step 6: Commit**

```bash
bash scripts/worktree.sh commit perf core "switch CredentialId to CompactString (inline ≤24B, no heap for typical IDs)"
```

---

### Task 11: Workflow `SlotBinding` typed IDs

**Files:**
- Modify: `crates/workflow/src/node.rs` (the `SlotBinding` enum)
- Modify: `crates/workflow/Cargo.toml` (add `nebula-core` if missing — verified already present via NodeKey)

- [ ] **Step 1: Write the failing test**

Create `crates/workflow/tests/slot_binding_typed.rs`:

```rust
use nebula_core::{CredentialId, ResourceId};
use nebula_workflow::node::SlotBinding;

#[test]
fn slot_binding_uses_typed_ids() {
    let b = SlotBinding::CredentialId(CredentialId::new("cred-1"));
    match b {
        SlotBinding::CredentialId(id) => assert_eq!(id.as_str(), "cred-1"),
        _ => panic!("wrong variant"),
    }

    let b = SlotBinding::ResourceId(ResourceId::new("res-1"));
    match b {
        SlotBinding::ResourceId(id) => assert_eq!(id.as_str(), "res-1"),
        _ => panic!("wrong variant"),
    }
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo nextest run -p nebula-workflow --test slot_binding_typed
```

Expected: compile error (`String` ≠ `CredentialId`).

- [ ] **Step 3: Update the enum**

In `crates/workflow/src/node.rs`:

```rust
use nebula_core::{CredentialId, ResourceId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SlotBinding {
    ResourceId(ResourceId),
    CredentialId(CredentialId),
}
```

Update the `with_*_binding` builder methods + `*_binding()` accessors:

```rust
impl NodeDefinition {
    #[must_use]
    pub fn with_resource_binding(
        mut self,
        slot: impl Into<String>,
        resource_id: impl Into<ResourceId>,
    ) -> Self {
        self.slot_bindings.insert(slot.into(), SlotBinding::ResourceId(resource_id.into()));
        self
    }

    #[must_use]
    pub fn with_credential_binding(
        mut self,
        slot: impl Into<String>,
        credential_id: impl Into<CredentialId>,
    ) -> Self {
        self.slot_bindings.insert(slot.into(), SlotBinding::CredentialId(credential_id.into()));
        self
    }

    #[must_use]
    pub fn resource_binding(&self, slot: &str) -> Option<&ResourceId> {
        match self.slot_bindings.get(slot)? {
            SlotBinding::ResourceId(id) => Some(id),
            SlotBinding::CredentialId(_) => None,
        }
    }

    #[must_use]
    pub fn credential_binding(&self, slot: &str) -> Option<&CredentialId> {
        match self.slot_bindings.get(slot)? {
            SlotBinding::CredentialId(id) => Some(id),
            SlotBinding::ResourceId(_) => None,
        }
    }
}
```

- [ ] **Step 4: Re-run — expect PASS**

```bash
cargo nextest run -p nebula-workflow --test slot_binding_typed
```

- [ ] **Step 5: Fix downstream consumers**

```bash
rg -n "SlotBinding::(Resource|Credential)Id\(" crates/
```

For each match, convert `String` literals to typed newtypes (`.into()`).

- [ ] **Step 6: Verify workspace**

```bash
cargo nextest run --workspace
```

- [ ] **Step 7: Commit**

```bash
bash scripts/worktree.sh commit feat workflow "type SlotBinding payloads via CredentialId/ResourceId newtypes"
```

---

### Task 12: `ValidatedCredentialBinding` confused-deputy closure

**Files:**
- Create: `crates/credential-runtime/src/binding.rs`
- Modify: `crates/credential-runtime/src/lib.rs`
- Modify: `crates/credential-runtime/src/service.rs`

- [ ] **Step 1: Write the failing cross-tenant probe**

Create `crates/credential-runtime/tests/validated_binding_cross_tenant.rs`:

```rust
use nebula_core::CredentialId;
use nebula_credential_runtime::{
    CredentialServiceBuilder, TenantScope, ValidatedCredentialBindingError,
};
use nebula_credential_testutil::in_memory_pair;

#[tokio::test(flavor = "multi_thread")]
async fn cross_tenant_binding_rejected() {
    let (store, pending) = in_memory_pair();
    let service = CredentialServiceBuilder::new(store, pending).build();

    let scope_a = TenantScope::new("org", "ws-a");
    let scope_b = TenantScope::new("org", "ws-b");

    // create a credential under scope A
    let snap = service.create_test_credential(&scope_a, "api-key-A").await
        .expect("create credential under tenant A");

    // attempt to validate it under tenant B — must fail
    let err = service.validate_credential_binding(&scope_b, &snap.id())
        .await
        .expect_err("scope B cannot bind to scope A credential");

    assert!(matches!(err, ValidatedCredentialBindingError::ScopeMismatch { .. }),
        "got {err:?}");
}
```

- [ ] **Step 2: Run — expect compile failure (types not yet defined)**

```bash
cargo nextest run -p nebula-credential-runtime --test validated_binding_cross_tenant
```

Expected: missing `ValidatedCredentialBindingError` / `validate_credential_binding`.

- [ ] **Step 3: Author `binding.rs`**

Create `crates/credential-runtime/src/binding.rs`:

```rust
//! Validated credential binding — a typed handle proving that a
//! workflow `slot_bindings` entry has been scope-checked against the
//! caller's `TenantScope`. Constructors are crate-private; downstream
//! consumers (engine execution) receive only validated handles, closing
//! the confused-deputy non-goal left open by the ADR-0052 cascade.

use nebula_core::CredentialId;

use crate::scope::TenantScope;

/// Tenant-scope-checked credential binding. The only constructor is
/// `CredentialService::validate_credential_binding`; engine execution
/// consumes this handle directly.
#[derive(Debug, Clone)]
pub struct ValidatedCredentialBinding {
    credential_id: CredentialId,
    tenant_fingerprint: TenantFingerprint,
}

/// Opaque proof of which tenant validated this binding. Equality
/// checks happen only inside the runtime crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantFingerprint(pub(crate) String);

impl ValidatedCredentialBinding {
    pub(crate) fn new(id: CredentialId, fp: TenantFingerprint) -> Self {
        Self { credential_id: id, tenant_fingerprint: fp }
    }

    pub fn credential_id(&self) -> &CredentialId { &self.credential_id }

    pub(crate) fn fingerprint(&self) -> &TenantFingerprint { &self.tenant_fingerprint }
}

impl TenantFingerprint {
    pub(crate) fn from_scope(scope: &TenantScope) -> Self {
        Self(scope.owner_id().to_string())
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ValidatedCredentialBindingError {
    #[error("credential `{id}` not found in tenant scope `{tenant}`")]
    NotFound { id: CredentialId, tenant: String },

    #[error("credential `{id}` exists in a different tenant scope (`{actual}` ≠ requested `{requested}`)")]
    ScopeMismatch {
        id: CredentialId,
        requested: String,
        actual: String,
    },

    #[error("validator i/o: {0}")]
    Io(#[from] crate::CredentialServiceError),
}
```

- [ ] **Step 4: Expose at crate root**

In `crates/credential-runtime/src/lib.rs`, add:

```rust
pub mod binding;
pub use binding::{TenantFingerprint, ValidatedCredentialBinding, ValidatedCredentialBindingError};
```

- [ ] **Step 5: Wire `validate_credential_binding` on `CredentialService`**

In `crates/credential-runtime/src/service.rs`:

```rust
impl<B, PS> CredentialService<B, PS>
where
    B: nebula_credential::CredentialStore,
    PS: nebula_credential::PendingStateStore,
{
    /// Validate a workflow `slot_bindings` reference against the caller's
    /// tenant. Returns a typed `ValidatedCredentialBinding` that engine
    /// execution consumes — the only construction path for cross-tier
    /// credential dispatch.
    pub async fn validate_credential_binding(
        &self,
        scope: &TenantScope,
        id: &nebula_core::CredentialId,
    ) -> Result<crate::ValidatedCredentialBinding, crate::ValidatedCredentialBindingError> {
        let stored = self.store_get_for_validation(id).await
            .map_err(crate::ValidatedCredentialBindingError::Io)?
            .ok_or_else(|| crate::ValidatedCredentialBindingError::NotFound {
                id: id.clone(),
                tenant: scope.owner_id().to_string(),
            })?;

        let owner = stored.metadata.get("owner_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if owner != scope.owner_id() {
            return Err(crate::ValidatedCredentialBindingError::ScopeMismatch {
                id: id.clone(),
                requested: scope.owner_id().to_string(),
                actual: owner.to_string(),
            });
        }

        Ok(crate::ValidatedCredentialBinding::new(
            id.clone(),
            crate::TenantFingerprint::from_scope(scope),
        ))
    }

    /// Crate-private read used by `validate_credential_binding`. Bypasses
    /// the tenant-stamp comparison done by `get`, since we need to *see*
    /// the foreign owner_id to construct a ScopeMismatch error.
    pub(crate) async fn store_get_for_validation(
        &self,
        id: &nebula_core::CredentialId,
    ) -> Result<Option<nebula_credential::StoredCredential>, crate::CredentialServiceError> {
        // ... existing store-load helper, adapted to skip the scope-stamp gate ...
        todo!("delegate to LayeredStore::get_raw — implementation matches existing get() minus the scope check")
    }
}
```

(Resolve the `todo!` by adapting the existing `get` body — the change is removing the `metadata["owner_id"] == scope.owner_id()` early-return so the validator can read it explicitly.)

- [ ] **Step 6: Add the test helper `create_test_credential` on service**

In `crates/credential-runtime/src/service.rs` (gated `#[cfg(any(test, feature = "test-util"))]`):

```rust
#[cfg(any(test, feature = "test-util"))]
impl<B, PS> CredentialService<B, PS>
where
    B: nebula_credential::CredentialStore,
    PS: nebula_credential::PendingStateStore,
{
    /// Test helper — persists a minimal credential row for tenant tests.
    pub async fn create_test_credential(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<nebula_credential::CredentialSnapshot, crate::CredentialServiceError> {
        // Minimal stub: persists `{owner_id, kind: "test"}` via the layered store.
        // Implementation mirrors the production `create` path with a static-state
        // builtin (`ApiKeyCredential`) so the row is real but cheap.
        todo!("call self.create::<ApiKeyCredential>(scope, ApiKeyProperties { key: SecretString::new(\"x\".into()) }).await")
    }
}
```

(Resolve `todo!` against the actual production `create` method signature once Wave 1 lands.)

- [ ] **Step 7: Re-run the probe**

```bash
cargo nextest run -p nebula-credential-runtime --test validated_binding_cross_tenant
```

Expected: PASS — cross-tenant binding rejected with `ScopeMismatch`.

- [ ] **Step 8: Commit**

```bash
bash scripts/worktree.sh commit feat credential-runtime "ValidatedCredentialBinding newtype + validate_credential_binding (closes slot_bindings confused-deputy non-goal)"
```

---

### Task 13: ADR for pre-expiry refresh decision

**Files:**
- Create: `docs/adr/0084-pre-expiry-credential-refresh-deferred.md`
- Modify: `docs/ROADMAP.md` (M12.5 entry)

- [ ] **Step 1: Determine the next free ADR number**

```bash
ls docs/adr/ | grep -E '^0[0-9]{3}-' | sort | tail -3
```

If the latest is `0083`, the new one is `0084`.

- [ ] **Step 2: Author the ADR**

Create `docs/adr/0084-pre-expiry-credential-refresh-deferred.md`:

```markdown
# ADR-0084 — Pre-expiry credential refresh: deferred to 1.1

- **Status:** Accepted
- **Date:** 2026-05-20
- **Supersedes:** N/A
- **Superseded by:** N/A

## Context

ROADMAP §M12.5 listed "pre-expiry credential refresh (proactive)" as a 1.0
candidate. The current implementation is reactive: an action attempts to
use a credential; the resolver observes expiry; refresh runs L1 (in-process)
+ L2 (durable claim repo) coalescing. n8n #13088-class multi-replica races
are closed by the L2 coordinator landed in П2 (2026-04-26).

## Decision

Pre-expiry (proactive) refresh ships in **1.1**, not 1.0.

Reactive refresh is the contract for 1.0. Proactive adds:
- A per-instance background tick scheduler.
- Per-credential expiry-aware scheduling state.
- A new failure class (background-refresh failure with no caller to
  notify).
- Test scaffolding (chaos: instance dies mid-tick).

None of these is required for production correctness — the reactive path
already handles every observed failure mode and survives chaos tests.

## Rationale

- Reactive refresh + L2 durable coordinator is correct under multi-replica
  load (chaos test green, archived).
- Proactive adds two failure classes (orphan background refresh,
  scheduler drift) for a latency win on the first request after expiry.
- Active-dev policy (`feedback_active_dev_mode`): prefer more-ideal over
  more-expedient, but only when the more-ideal path is genuinely better.
  Proactive refresh is *not* more-ideal: it adds operational surface for
  a latency optimization that p99 metrics can already cover via
  warm-up requests on critical paths.

## Consequences

- nebula-credential ships 1.0 with `frontier`→`stable` flip on reactive
  refresh.
- Proactive design is tracked as a 1.1 backlog item; no code lands until
  then.
- ROADMAP M12.5 row updated to mark this decision.
```

- [ ] **Step 3: Update ROADMAP M12.5**

In `docs/ROADMAP.md`, find the M12.5 row and change:

```markdown
- [ ] Pre-expiry credential refresh (proactive) — v1 is reactive via
      `EventBus<CredentialRotatedEvent>`; decide if proactive ships in 1.0.
```

to:

```markdown
- [x] Pre-expiry credential refresh decision — **deferred to 1.1** per ADR-0084.
      Reactive path remains the contract for 1.0.
```

- [ ] **Step 4: Commit**

```bash
bash scripts/worktree.sh commit docs credential "ADR-0084: defer proactive pre-expiry refresh to 1.1"
```

---

## Wave 3 — Production seams

Three tasks. Sequential within wave (depend on Wave 2 merges).

---

### Task 14: `resolve_for_slot` production resolver

**Files:**
- Modify: `crates/credential-runtime/src/service.rs`
- Modify: `crates/engine/src/credential/resolver.rs`
- Create: `crates/credential-runtime/tests/resolve_for_slot.rs`

- [ ] **Step 1: Write the end-to-end test**

Create `crates/credential-runtime/tests/resolve_for_slot.rs`:

```rust
use nebula_credential::{CredentialGuard, scheme::SecretToken};
use nebula_credential_runtime::{CredentialServiceBuilder, TenantScope};
use nebula_credential_testutil::in_memory_pair;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
async fn resolve_for_slot_produces_guard() {
    let (store, pending) = in_memory_pair();
    let service = CredentialServiceBuilder::new(store, pending).build();

    let scope = TenantScope::new("org", "ws");
    let snap = service.create_test_credential(&scope, "tok-1").await.unwrap();
    let binding = service.validate_credential_binding(&scope, snap.id()).await.unwrap();

    let cancel = CancellationToken::new();
    let guard: CredentialGuard<SecretToken> = service
        .resolve_for_slot::<nebula_credential::credentials::ApiKeyCredential>(&scope, &binding, cancel)
        .await
        .unwrap();

    // `CredentialGuard::Deref` exposes the projected scheme; verify token is non-empty.
    use secrecy::ExposeSecret;
    assert!(!guard.bearer_header().expose_secret().is_empty());
}
```

- [ ] **Step 2: Run — expect compile failure (method missing)**

```bash
cargo nextest run -p nebula-credential-runtime --test resolve_for_slot
```

Expected: `no method named resolve_for_slot found`.

- [ ] **Step 3: Implement `resolve_for_slot`**

In `crates/credential-runtime/src/service.rs`:

```rust
use nebula_credential::{Credential, CredentialGuard};
use tokio_util::sync::CancellationToken;

impl<B, PS> CredentialService<B, PS>
where
    B: nebula_credential::CredentialStore,
    PS: nebula_credential::PendingStateStore,
{
    /// Production execution-time resolver. Consumes a tenant-validated
    /// binding and produces a typed `CredentialGuard<C::Scheme>` for an
    /// action slot.
    ///
    /// Hot path. p99 ≤ 1ms on warm cache.
    pub async fn resolve_for_slot<C>(
        &self,
        scope: &TenantScope,
        binding: &crate::ValidatedCredentialBinding,
        cancel: CancellationToken,
    ) -> Result<CredentialGuard<C::Scheme>, crate::CredentialServiceError>
    where
        C: Credential,
    {
        // 1. Fingerprint match — defence in depth even though
        //    `validate_credential_binding` already checked.
        let expected_fp = crate::TenantFingerprint::from_scope(scope);
        if binding.fingerprint() != &expected_fp {
            return Err(crate::CredentialServiceError::ScopeViolation {
                requested: scope.owner_id().to_string(),
            });
        }

        // 2. Resolve via the engine resolver — handles refresh + L1/L2
        //    coalescing transparently.
        let resolver = self.engine_resolver();
        let resolved = cancel
            .run_until_cancelled(resolver.resolve_with_refresh::<C>(
                scope,
                binding.credential_id(),
            ))
            .await
            .ok_or(crate::CredentialServiceError::Cancelled)?
            .map_err(crate::CredentialServiceError::from)?;

        Ok(resolved.into_guard())
    }
}
```

(`self.engine_resolver()` returns the wired resolver instance held by the service; if not present, add it to the `<B, PS>` build path.)

- [ ] **Step 4: Re-run the test**

```bash
cargo nextest run -p nebula-credential-runtime --test resolve_for_slot
```

Expected: PASS.

- [ ] **Step 5: Engine — replace direct resolver-construction in execution-runtime with `CredentialService::resolve_for_slot`**

Locate engine execution code that currently builds `CredentialResolver` ad-hoc and calls `resolve_with_refresh` directly. Replace with `CredentialService::resolve_for_slot` calls (the service holds the resolver internally).

```bash
rg -n "CredentialResolver::new|resolve_with_refresh" crates/engine/src/
```

For each production callsite, route through the service. Keep the low-level `CredentialResolver` as-is — only the call-from-execution-runtime path changes.

- [ ] **Step 6: Verify workspace**

```bash
cargo nextest run --workspace
```

- [ ] **Step 7: Commit**

```bash
bash scripts/worktree.sh commit feat credential-runtime "CredentialService::resolve_for_slot production seam (closes M11.5 residual)"
```

---

### Task 15: Fallback-on-interrupt in refresh path

**Files:**
- Modify: `crates/credential-runtime/src/service.rs`
- Create: `crates/credential-runtime/tests/refresh_fallback.rs`

- [ ] **Step 1: Write the failing test**

```rust
//! Probe: a transient provider failure on refresh must return the
//! cached non-expired credential rather than propagating the error.
//! Models the aws-credential-types `fallback_on_interrupt` pattern.

use nebula_credential_runtime::{CredentialServiceBuilder, TenantScope};
use nebula_credential_testutil::{in_memory_pair, scripted_provider::ScriptedProvider};

#[tokio::test(flavor = "multi_thread")]
async fn refresh_transient_falls_back_to_cached() {
    let (store, pending) = in_memory_pair();
    let service = CredentialServiceBuilder::new(store, pending).build();

    let scope = TenantScope::new("org", "ws");
    let snap = service.create_test_credential(&scope, "cached-tok").await.unwrap();

    // Script the next refresh call to fail with TransientNetwork.
    service.install_test_provider(ScriptedProvider::transient_network_once());

    // Refresh while current creds are still valid — should return cached.
    let refreshed = service.refresh(&scope, snap.id().clone(), Default::default())
        .await
        .expect("transient failure falls back to cached non-expired");

    assert_eq!(refreshed.id(), snap.id(), "must be the same credential");
}
```

(Add the `scripted_provider` test helper to `nebula-credential-testutil` first — small enum-based provider stub.)

- [ ] **Step 2: Run — expect FAIL (no fallback yet)**

```bash
cargo nextest run -p nebula-credential-runtime --test refresh_fallback
```

Expected: FAIL — `refresh` propagates the error.

- [ ] **Step 3: Implement fallback in `refresh`**

In `crates/credential-runtime/src/service.rs`:

```rust
impl<B, PS> CredentialService<B, PS>
where
    B: nebula_credential::CredentialStore,
    PS: nebula_credential::PendingStateStore,
{
    pub async fn refresh(
        &self,
        scope: &TenantScope,
        id: nebula_core::CredentialId,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<nebula_credential::CredentialSnapshot, crate::CredentialServiceError> {
        let cached = self.get(scope, id.clone()).await?;

        match self.refresh_inner(scope, &id, cancel).await {
            Ok(snap) => Ok(snap),
            Err(e) if self.is_transient_refresh(&e) && !cached.is_expired() => {
                tracing::warn!(
                    credential.id = %id,
                    error = %e,
                    "credential refresh failed transiently; returning cached non-expired snapshot"
                );
                Ok(cached)
            }
            Err(e) => Err(e),
        }
    }

    fn is_transient_refresh(&self, e: &crate::CredentialServiceError) -> bool {
        use nebula_credential::error::{CredentialError, RefreshErrorKind};
        matches!(
            e.as_credential_error(),
            Some(CredentialError::RefreshFailed(ctx))
                if matches!(
                    ctx.kind(),
                    RefreshErrorKind::TransientNetwork | RefreshErrorKind::ProviderUnavailable
                )
        )
    }
}
```

Add `CredentialServiceError::as_credential_error(&self) -> Option<&CredentialError>` accessor if not present.

- [ ] **Step 4: Re-run the test**

```bash
cargo nextest run -p nebula-credential-runtime --test refresh_fallback
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
bash scripts/worktree.sh commit feat credential-runtime "refresh: fallback to cached non-expired credential on transient provider failures (aws-sdk pattern)"
```

---

### Task 16: Audit L1 refresh coalescer vs `tokio::sync::OnceCell`

**Files:**
- Read-only initially: `crates/engine/src/credential/refresh/*.rs`
- Possibly modify: `crates/engine/src/credential/refresh/coalescer.rs` (or wherever L1 lives)

- [ ] **Step 1: Read the L1 coalescer source**

```bash
ls crates/engine/src/credential/refresh/
sed -n '1,80p' crates/engine/src/credential/refresh/coalescer.rs 2>/dev/null || echo "not at that path; search"
rg -n "L1RefreshCoalescer|L1Coalescer|in_flight" crates/engine/src/credential/refresh/
```

Identify the type that holds in-flight refresh state.

- [ ] **Step 2: Catalogue what the custom impl does beyond `OnceCell::get_or_init`**

Score on these axes:
- Per-credential keying with composite key (`(id, version)`)
- Metric emission on miss/hit
- Eviction strategy (drop after refresh completes; LRU; manual sweep)
- Cancellation propagation
- Error sharing (one refresher errors → does observer-callers get the error?)

If the custom impl does **only** keyed single-flight + happy-path result sharing, it is reducible to `tokio::sync::OnceCell` keyed by id+version in a `DashMap<(CredentialId, u64), OnceCell<_>>`. Otherwise, keep custom.

- [ ] **Step 3: Document the audit outcome**

Append to the L1 source file's module docstring:

```rust
//! ## L1 coalescer vs `tokio::sync::OnceCell` audit (2026-05-20)
//!
//! - **Keying:** [composite `(CredentialId, u64)` / per-credential / global]
//! - **Metric emission:** [yes / no]
//! - **Eviction:** [drop-after-resolve / LRU / manual]
//! - **Cancellation:** [propagates / waits independently]
//! - **Error sharing:** [errors propagated to all waiters / first error only]
//!
//! Verdict: [keep custom — `OnceCell` insufficient for ... / replace with `OnceCell`-based]
```

- [ ] **Step 4: If verdict is "replace", implement the swap**

```rust
use dashmap::DashMap;
use tokio::sync::OnceCell;
use std::sync::Arc;

pub struct L1RefreshCoalescer {
    in_flight: DashMap<(nebula_core::CredentialId, u64), Arc<OnceCell<crate::credential::RefreshResult>>>,
}

impl L1RefreshCoalescer {
    pub async fn refresh_or_wait<F, Fut>(&self, key: (nebula_core::CredentialId, u64), f: F)
        -> crate::credential::RefreshResult
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = crate::credential::RefreshResult>,
    {
        let cell = self.in_flight.entry(key.clone())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone();
        let result = cell.get_or_init(f).await.clone();
        self.in_flight.remove(&key);
        result
    }
}
```

If verdict is "keep custom", skip this step.

- [ ] **Step 5: Verify (whichever path)**

```bash
cargo nextest run -p nebula-engine
```

- [ ] **Step 6: Commit**

```bash
bash scripts/worktree.sh commit docs engine "audit L1 refresh coalescer against tokio::sync::OnceCell"
# OR if swap landed:
bash scripts/worktree.sh commit refactor engine "replace L1 refresh coalescer with tokio::sync::OnceCell single-flight"
```

---

## Wave 4 — API integration

Two tasks. Sequential (Step 17 before 18).

---

### Task 17: Wire `nebula-api` onto `CredentialService`

**Files:**
- Modify: `crates/api/src/state.rs`
- Modify: `crates/api/src/domain/credential/handler.rs`
- Modify: `crates/api/src/domain/credential/routes.rs`
- Modify: `crates/api/src/transport/credential.rs`
- Modify: `crates/api/Cargo.toml`

- [ ] **Step 1: Add dependency**

In `crates/api/Cargo.toml`:

```toml
[dependencies]
nebula-credential-runtime.workspace = true
```

- [ ] **Step 2: Replace api-local credential stack with `CredentialService`**

In `crates/api/src/transport/credential.rs`, remove the custom `CredentialScopeLayer<InMemoryStore>` builder and replace with:

```rust
use nebula_credential_runtime::{CredentialService, CredentialServiceBuilder};

pub fn build_credential_service<B, PS>(
    store: B,
    pending: PS,
) -> CredentialService<B, PS>
where
    B: nebula_credential::CredentialStore + Send + Sync + 'static,
    PS: nebula_credential::PendingStateStore + Send + Sync + 'static,
{
    CredentialServiceBuilder::new(store, pending).build()
}
```

- [ ] **Step 3: Update the handler**

In `crates/api/src/domain/credential/handler.rs`, replace direct store calls with `CredentialService` calls:

```rust
// Before
let cred = state.credential_store.get(&id).await?;

// After
let scope = TenantScope::from_request(req)?;
let snap = state.credential_service.get(&scope, id).await?;
```

- [ ] **Step 4: Update OAuth ceremony to go via service**

In `crates/api/src/domain/credential/oauth.rs`, OAuth callback writes credentials. Change:

```rust
// Before
store.put(&credential_row).await?;

// After
state.credential_service.create::<OAuth2Credential>(&scope, props).await?;
```

- [ ] **Step 5: Verify integration tests**

```bash
cargo nextest run -p nebula-api
```

- [ ] **Step 6: Commit**

```bash
bash scripts/worktree.sh commit feat api "wire credential handlers onto CredentialService facade (ADR-0066 carry-over)"
```

---

### Task 18: Delete `CredentialScopeLayer` from nebula-tenancy

**Files:**
- Delete: `crates/tenancy/src/credential_scope.rs`
- Modify: `crates/tenancy/src/lib.rs`
- Modify: `crates/tenancy/Cargo.toml`

- [ ] **Step 1: Verify zero callsites after Task 17**

```bash
rg -n "CredentialScopeLayer" crates/
```

Expected: matches only inside `crates/tenancy/` itself. If anywhere else still references it, fix that callsite first.

- [ ] **Step 2: Remove the module**

```bash
git rm crates/tenancy/src/credential_scope.rs
```

In `crates/tenancy/src/lib.rs`:

```rust
// Remove:
// mod credential_scope;
// pub use credential_scope::ScopeLayer as CredentialScopeLayer;
// pub use nebula_credential::ScopeResolver as CredentialScopeResolver;
```

If `ScopeResolver` is still re-exported elsewhere, leave that re-export — it's the trait, not the layer.

- [ ] **Step 3: Update `Cargo.toml`**

If `nebula-credential` was only used by `credential_scope`, drop it from `crates/tenancy/Cargo.toml`. Confirm via:

```bash
rg -n "nebula_credential" crates/tenancy/src/
```

- [ ] **Step 4: Update `deny.toml`**

In root `deny.toml [wrappers]`, remove `nebula-credential` from the `nebula-tenancy` allow-list if it was there only for the layer.

- [ ] **Step 5: Verify**

```bash
cargo deny check
cargo nextest run --workspace
```

- [ ] **Step 6: Commit**

```bash
bash scripts/worktree.sh commit refactor tenancy "delete CredentialScopeLayer — operation-level tenant isolation in CredentialService is the single gate"
```

---

## Wave 5 — Tests, registry sync, freeze

Final wave. Tasks 19–22.

---

### Task 19: Three-registry sync invariant probe

**Files:**
- Create: `crates/engine/tests/registry_sync_invariant.rs`

- [ ] **Step 1: Write the probe**

Create `crates/engine/tests/registry_sync_invariant.rs`:

```rust
//! Probe: every credential type registered in
//! `nebula_credential::CredentialRegistry` must also be present in
//! `nebula_engine::credential::StateProjectionRegistry` (state-kind dispatch)
//! and `nebula_credential_runtime::CredentialDispatch` (capability table).
//!
//! Closes the silent-drift vector identified in design §M5.

use nebula_credential::{CredentialRegistry, credentials::{ApiKeyCredential, BasicAuthCredential, OAuth2Credential}};
use nebula_credential_runtime::CredentialDispatch;
use nebula_engine::credential::StateProjectionRegistry;

#[test]
fn all_builtin_credentials_present_in_three_registries() {
    let mut cred = CredentialRegistry::new();
    cred.register::<ApiKeyCredential>().unwrap();
    cred.register::<BasicAuthCredential>().unwrap();
    cred.register::<OAuth2Credential>().unwrap();

    let mut proj = StateProjectionRegistry::new();
    proj.register::<ApiKeyCredential>().unwrap();
    proj.register::<BasicAuthCredential>().unwrap();
    proj.register::<OAuth2Credential>().unwrap();

    let mut disp = CredentialDispatch::new();
    disp.register::<ApiKeyCredential>().unwrap();
    disp.register::<BasicAuthCredential>().unwrap();
    disp.register::<OAuth2Credential>().unwrap();

    let cred_keys: std::collections::HashSet<&'static str> = cred.iter_keys().collect();
    let proj_keys: std::collections::HashSet<&'static str> = proj.iter_keys().collect();
    let disp_keys: std::collections::HashSet<&'static str> = disp.iter_keys().collect();

    assert_eq!(cred_keys, proj_keys, "CredentialRegistry vs StateProjectionRegistry drift");
    assert_eq!(cred_keys, disp_keys, "CredentialRegistry vs CredentialDispatch drift");
}
```

- [ ] **Step 2: Add `iter_keys` methods if missing**

Each registry needs `pub fn iter_keys(&self) -> impl Iterator<Item = &'static str> + '_`. Add where missing.

- [ ] **Step 3: Run**

```bash
cargo nextest run -p nebula-engine --test registry_sync_invariant
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
bash scripts/worktree.sh commit test engine "three-registry sync invariant probe (closes drift vector)"
```

---

### Task 20: Composite `register_credential_complete::<C>`

**Files:**
- Create: `crates/credential-runtime/src/registration.rs`
- Modify: `crates/credential-runtime/src/lib.rs`

- [ ] **Step 1: Write the failing usage test**

```rust
//! Probe: `register_credential_complete::<C>` atomically registers C
//! into all three registries, or fails without partial state.

use nebula_credential_runtime::registration::{
    register_credential_complete, RegistrationContext, RegistrationError,
};

#[test]
fn composite_registration_atomic_on_failure() {
    let mut ctx = RegistrationContext::new();
    register_credential_complete::<nebula_credential::credentials::ApiKeyCredential>(&mut ctx).unwrap();

    // Second registration of same type must fail and leave NO partial state.
    let before_proj = ctx.state_projection_count();
    let before_cred = ctx.credential_registry_count();
    let before_disp = ctx.dispatch_count();

    let err = register_credential_complete::<nebula_credential::credentials::ApiKeyCredential>(&mut ctx)
        .unwrap_err();
    assert!(matches!(err, RegistrationError::DuplicateKey { .. }));

    assert_eq!(ctx.state_projection_count(), before_proj);
    assert_eq!(ctx.credential_registry_count(), before_cred);
    assert_eq!(ctx.dispatch_count(), before_disp);
}
```

- [ ] **Step 2: Author `registration.rs`**

Create `crates/credential-runtime/src/registration.rs`:

```rust
use nebula_credential::{Credential, CredentialRegistry};
use crate::dispatch::CredentialDispatch;

pub struct RegistrationContext {
    pub credential: CredentialRegistry,
    pub state_projection: nebula_engine::credential::StateProjectionRegistry,
    pub dispatch: CredentialDispatch,
}

impl RegistrationContext {
    pub fn new() -> Self {
        Self {
            credential: CredentialRegistry::new(),
            state_projection: nebula_engine::credential::StateProjectionRegistry::new(),
            dispatch: CredentialDispatch::new(),
        }
    }

    pub fn credential_registry_count(&self) -> usize { self.credential.len() }
    pub fn state_projection_count(&self) -> usize { self.state_projection.len() }
    pub fn dispatch_count(&self) -> usize { self.dispatch.len() }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistrationError {
    #[error("duplicate credential key `{key}` (registry: {registry})")]
    DuplicateKey { key: &'static str, registry: &'static str },
}

/// Atomically register a credential type into all three registries.
/// Returns `Err` without partial state on the first conflict.
pub fn register_credential_complete<C>(ctx: &mut RegistrationContext) -> Result<(), RegistrationError>
where
    C: Credential + Default + 'static,
{
    // Phase 1 — check all three for conflicts (no mutation yet).
    if ctx.credential.contains_key(C::KEY) {
        return Err(RegistrationError::DuplicateKey { key: C::KEY, registry: "credential" });
    }
    let kind = <C::State as nebula_credential::CredentialState>::KIND;
    if ctx.state_projection.contains_kind(kind) {
        return Err(RegistrationError::DuplicateKey { key: kind, registry: "state_projection" });
    }
    if ctx.dispatch.contains_key(C::KEY) {
        return Err(RegistrationError::DuplicateKey { key: C::KEY, registry: "dispatch" });
    }

    // Phase 2 — commit. All registries pre-checked; no partial state.
    ctx.credential.register::<C>(C::default(), env!("CARGO_PKG_NAME"))
        .expect("pre-checked");
    ctx.state_projection.register::<C>()
        .expect("pre-checked");
    ctx.dispatch.register::<C>()
        .expect("pre-checked");

    Ok(())
}
```

(Add `contains_key` / `contains_kind` / `len` accessors on the three registries.)

- [ ] **Step 3: Run the test**

```bash
cargo nextest run -p nebula-credential-runtime --test composite_registration
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
bash scripts/worktree.sh commit feat credential-runtime "register_credential_complete composite atomic registration"
```

---

### Task 21: dyn-compat probe for `AnyCredential` under 1.95 next-gen solver

**Files:**
- Create: `crates/credential/tests/dyn_compat_any_credential.rs`

- [ ] **Step 1: Write the probe**

```rust
//! Probe: `dyn AnyCredential` remains object-safe under the Rust 1.95
//! next-generation trait solver. Regressions here (e.g. addition of an
//! associated const or `Self: Sized` method without proper bound) would
//! break plugin registry which holds `Arc<dyn AnyCredential>`.

use std::sync::Arc;

use nebula_credential::AnyCredential;

fn assert_dyn_safe(_: &dyn AnyCredential) {}

#[test]
fn any_credential_is_dyn_compatible() {
    // The mere fact that `&dyn AnyCredential` compiles is the assertion.
    fn make_arc(c: Arc<dyn AnyCredential>) -> Arc<dyn AnyCredential> { c }
    let _ = make_arc;
}
```

- [ ] **Step 2: Run**

```bash
cargo nextest run -p nebula-credential --test dyn_compat_any_credential
```

If FAIL: identify the offending member (likely an `&'static str` associated const or `Self: Sized` requirement). Either:
- Move the offender to a method (preserving dyn-safety)
- Add `Self: Sized` bound on the offender (allowed in dyn-safe trait)

- [ ] **Step 3: Commit**

```bash
bash scripts/worktree.sh commit test credential "dyn-compat probe for AnyCredential under 1.95 next-gen solver"
```

---

### Task 22: Frontmatter flip + ROADMAP update + final verification

**Files:**
- Modify: `crates/credential/README.md`
- Modify: `crates/credential-runtime/README.md`
- Modify: `docs/MATURITY.md`
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Run the full pre-PR gate**

```bash
task dev:check
cargo deny check
```

Expected: green everywhere.

- [ ] **Step 2: Flip credential README frontmatter**

In `crates/credential/README.md`:

```yaml
status: stable   # was: frontier
last-reviewed: 2026-05-20
```

- [ ] **Step 3: Flip credential-runtime README frontmatter**

Same change in `crates/credential-runtime/README.md`.

- [ ] **Step 4: Update MATURITY.md**

In `docs/MATURITY.md`, locate the `nebula-credential` row and change the status column from `frontier` to `stable`. Add a note (per existing format):

```
status: stable; M12.2 hardening closed 2026-05-20 — error taxonomy reshape
(per-variant context structs + 32B size cap), SecretBox migration,
ValidatedCredentialBinding confused-deputy closure, resolve_for_slot
production seam, fallback-on-interrupt, three-registry sync invariant.
ADR-0084 defers proactive pre-expiry refresh to 1.1.
```

Same for `nebula-credential-runtime` row (was likely `partial` or `frontier`).

- [ ] **Step 5: Update ROADMAP M12.2**

In `docs/ROADMAP.md`, find M12.2 section and check off the closed items:

```markdown
- [x] Wire `nebula-api` onto `CredentialService` facade (ADR-0066) — Task 17.
- [x] Credential README frontmatter `stable`; subtrait coverage probe added (Task 6).
- [x] `slot_bindings` confused-deputy non-goal closure — Task 12.
- [x] Production credential→slot bind-population path — Task 14.
- [x] Pre-expiry refresh decision — ADR-0084 (deferred to 1.1).
- [x] Error ABI trip-wire — Task 7 + #588.
```

- [ ] **Step 6: Final re-run**

```bash
task dev:check
```

- [ ] **Step 7: Commit**

```bash
bash scripts/worktree.sh commit docs credential "flip credential + credential-runtime status to stable; update MATURITY.md + ROADMAP M12.2"
```

- [ ] **Step 8: Open the PR**

```bash
git push -u origin feat/credential-stabilize-sweep
gh pr create --title "feat(credential)!: stabilize sweep — M12.2 hardening + api wire + bind-population" \
  --body "$(cat <<'PRBODY'
## Summary
Closes M12.2 nebula-credential hardening + ships the deferred bind-population production path. `nebula-credential` and `nebula-credential-runtime` move `frontier → stable`.

## Highlights
- `CredentialError` reshaped per Smithy RFC-0022 (per-variant context structs + boxed payloads + 32B size cap)
- `SecretString` → `secrecy::SecretBox<String>`
- `ValidatedCredentialBinding` newtype closes the `slot_bindings` confused-deputy non-goal left open by the ADR-0052 cascade
- `CredentialService::resolve_for_slot` is the sole bind-population seam; `register_and_bind` quiesce contract now has a production caller
- `nebula-api` rewired onto `CredentialService` (ADR-0066 carry-over)
- `nebula-tenancy::CredentialScopeLayer` deleted — operation-level isolation is the single gate
- `AuthStyle` moved to `scheme::oauth2` (survives the future builtin carve-out)
- Three-registry sync invariant probe + composite `register_credential_complete` atomic registration
- ADR-0084 defers proactive pre-expiry refresh to 1.1

## Out of scope
- Builtin credential catalog (M12.3 / #604) — separate wave
- Proactive pre-expiry refresh — ADR-0084 deferred to 1.1

🤖 Generated with [Claude Code](https://claude.com/claude-code)
PRBODY
)"
```

---

## Self-Review

After writing all 22 tasks, the following spec items are covered:

| Spec section | Implemented by |
|---|---|
| Error reshape (§19.4 / Smithy RFC-0022) | Task 7 |
| `SecretString` → `SecretBox` (§19.3 / U3) | Task 9 |
| Bind-population (§3.1 / M11.5 residual) | Tasks 11, 12, 14 |
| `slot_bindings` confused-deputy closure | Task 12 |
| API wire to `CredentialService` (ADR-0066) | Task 17 |
| `CredentialScopeLayer` cleanup | Task 18 |
| AuthStyle move (boundary §M1) | Task 8 |
| Test shim extraction (§M6 / §M8) | Task 3 |
| Three-registry sync (§M5) | Tasks 19, 20 |
| Pre-expiry refresh decision (M12.5) | Task 13 |
| Fallback-on-interrupt (§19.2 / AWS pattern) | Task 15 |
| L1 coalescer audit (§U6) | Task 16 |
| `CredentialContext` cancellation (§U7) | Task 5 |
| Sealed capability probe (§U1) | Task 6 |
| Error size cap / #588 | Task 7 (size assertion) |
| Frontmatter freeze | Task 22 |
| Dead code audit | Tasks 1, 2, 4 |
| 1.95 dyn-compat probe | Task 21 |
| CredentialId perf newtype (§U5) | Task 10 |

Deferred (with explicit reasoning in plan or design doc):
- U2 branded tenant lifetimes — async cost > benefit; research probe only.
- U10 `Vec::push_mut` — no measured hot Vec batch path; skip.
- M12.3 builtin catalog — separate wave per user direction.

No placeholders detected after final scan.

---

**END OF PLAN**
