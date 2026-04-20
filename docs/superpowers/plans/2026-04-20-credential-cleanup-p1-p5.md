# nebula-credential cleanup — P1-P5 implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Выполнить первые 5 фаз credential architecture cleanup per spec [2026-04-20-credential-architecture-cleanup-design.md](../specs/2026-04-20-credential-architecture-cleanup-design.md): duplicate collapse + submodule grouping + partial base-dep diet + nebula-core migration + 4 ADRs landing. После P5 — hard go/no-go checkpoint до P6+ (physical crate moves).

**Architecture:** В этих 5 фазах код остаётся в `nebula-credential`. Меняется организация модулей, удаляется дупликация, мигрируют 4 типа из `nebula-core`. Никаких cross-crate moves — это P6+. Финальная фаза P5 — 4 ADR-документа без кода.

**Tech Stack:** Rust 2024 edition, `nebula-resilience` (замена для дублирующего retry), `cargo nextest` (test runner), `cargo +nightly fmt`, nightly rustfmt. Конвенциональные коммиты (`convco` enforced).

---

## Phase P1: Duplicate collapse & dead code removal

**Goal:** Удалить дубликаты и dead code внутри `nebula-credential` без трогания cross-crate границ. Подготовить почву для последующих фаз.

**Files:**
- Delete: `crates/credential/src/rotation/retry.rs`
- Delete: `crates/credential/src/rotation/metrics.rs`
- Delete: `crates/credential/src/rotation/events.rs`
- Delete: `crates/credential/src/option_serde_secret.rs`
- Modify: `crates/credential/src/serde_secret.rs` (merge contents)
- Modify: `crates/credential/src/rotation/mod.rs` (remove module declarations)
- Modify: `crates/credential/src/lib.rs` (update re-exports)
- Modify: `crates/credential/Cargo.toml` (remove `tokio-util` if unused)

### Task P1.1: Verify rotation/retry duplication

- [ ] **Step 1: Compare `retry.rs` and `rotation/retry.rs`**

Both files are thin facades over `nebula-resilience::retry::{BackoffConfig, JitterConfig, RetryConfig}`. Confirm both delegate to the same resilience types with no unique logic.

Run: `diff <(head -60 crates/credential/src/retry.rs) <(head -60 crates/credential/src/rotation/retry.rs)`

Expected: differences are only in type names (`RetryPolicy` vs `RotationRetryPolicy`) and defaults. Both import `nebula_resilience::retry::*`.

- [ ] **Step 2: Verify `nebula-resilience::retry::RetryConfig` covers rotation semantics**

Read `crates/resilience/src/retry.rs`. Verify it provides:
- `max_attempts: u32`
- `BackoffConfig` (initial + multiplier + max)
- `JitterConfig` (jittered backoff for transactional flip)
- Async retry loop helper

Run: `grep -n "pub struct RetryConfig\|pub struct BackoffConfig\|pub struct JitterConfig" crates/resilience/src/retry.rs`

Expected: all three structs present. If missing — stop, extend resilience in a separate PR before continuing.

- [ ] **Step 3: Verify no external callers of `RotationRetryPolicy`**

Run: `grep -rn "RotationRetryPolicy" crates/ --include="*.rs"`

Expected: only references inside `crates/credential/src/rotation/`. If external — plan migration of those callers to `RetryPolicy` first.

### Task P1.2: Delete `rotation/retry.rs`, consolidate to top-level `retry.rs`

- [ ] **Step 1: Collapse rotation retry into top-level retry**

Read `crates/credential/src/rotation/retry.rs`. If `RotationRetryPolicy` has rotation-specific defaults, merge them as `RetryPolicy::rotation_defaults() -> RetryPolicy` constructor in `crates/credential/src/retry.rs`. Otherwise delete rotation version entirely.

Edit `crates/credential/src/retry.rs`:

```rust
impl RetryPolicy {
    /// Defaults tuned for rotation: 5 attempts, 100ms initial, 2x backoff, 32s max.
    pub fn rotation_defaults() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 100,
            max_delay_ms: 32_000,
            multiplier: 2.0,
            jitter: true,
        }
    }
}
```

- [ ] **Step 2: Update rotation callers**

Run: `grep -rn "RotationRetryPolicy" crates/credential/src/rotation/`

For each hit, replace with `RetryPolicy::rotation_defaults()` (or appropriate constructor). Typical hit: `rotation/scheduler.rs`.

- [ ] **Step 3: Delete `rotation/retry.rs`**

Run: `rm crates/credential/src/rotation/retry.rs`

Edit `crates/credential/src/rotation/mod.rs` — remove line: `pub mod retry;`

- [ ] **Step 4: Verify build + tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/credential/src/retry.rs crates/credential/src/rotation/retry.rs crates/credential/src/rotation/mod.rs crates/credential/src/rotation/scheduler.rs
git commit -m "refactor(credential): collapse rotation/retry.rs into top-level retry.rs"
```

### Task P1.3: Merge `serde_secret.rs` + `option_serde_secret.rs`

- [ ] **Step 1: Merge into single `serde_secret.rs`**

Edit `crates/credential/src/serde_secret.rs`:

```rust
//! Serde helpers for [`SecretString`] that preserve the actual value.
//!
//! Use with `#[serde(with = "nebula_credential::serde_secret")]` for
//! `SecretString` fields or `#[serde(with = "nebula_credential::serde_secret::option")]`
//! for `Option<SecretString>` fields.

use serde::{Deserialize, Deserializer, Serializer};

use crate::SecretString;

/// Serialize the actual secret value (for encrypted-at-rest storage only).
pub fn serialize<S: Serializer>(secret: &SecretString, s: S) -> Result<S::Ok, S::Error> {
    secret.expose_secret(|v| s.serialize_str(v))
}

/// Deserialize a string into a `SecretString`.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
    String::deserialize(d).map(SecretString::new)
}

/// Serde helpers for `Option<SecretString>`. Use as:
/// `#[serde(with = "nebula_credential::serde_secret::option")]`.
pub mod option {
    use super::*;

    /// Serialize an optional secret value (for encrypted-at-rest storage only).
    pub fn serialize<S: Serializer>(
        secret: &Option<SecretString>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match secret {
            Some(secret) => secret.expose_secret(|v| s.serialize_str(v)),
            None => s.serialize_none(),
        }
    }

    /// Deserialize an optional string into an `Option<SecretString>`.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<SecretString>, D::Error> {
        Option::<String>::deserialize(d).map(|opt| opt.map(SecretString::new))
    }
}
```

- [ ] **Step 2: Delete `option_serde_secret.rs`**

Run: `rm crates/credential/src/option_serde_secret.rs`

- [ ] **Step 3: Update lib.rs**

Edit `crates/credential/src/lib.rs`:

Remove lines:
```rust
/// Serde helpers for [`Option<SecretString>`] that preserve the actual value.
pub mod option_serde_secret;
```

- [ ] **Step 4: Update all callers from `option_serde_secret` → `serde_secret::option`**

Run: `grep -rn "option_serde_secret" crates/ --include="*.rs"`

For each hit, update attribute:
- Old: `#[serde(with = "nebula_credential::option_serde_secret")]`
- New: `#[serde(with = "nebula_credential::serde_secret::option")]`

Common hits: `crates/credential/src/credentials/*.rs`, `crates/credential/src/scheme/*.rs`.

- [ ] **Step 5: Verify build + tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/credential/src/serde_secret.rs crates/credential/src/option_serde_secret.rs crates/credential/src/lib.rs crates/credential/src/credentials/ crates/credential/src/scheme/
git commit -m "refactor(credential): merge option_serde_secret into serde_secret::option submodule"
```

### Task P1.4: Delete `rotation/metrics.rs` and `rotation/events.rs`

**Gate:** Emission of `CredentialEvent` via eventbus must not be blocked by deletion. Current usage analysis:

- `rotation/metrics.rs` — defines rotation-specific metrics types (counters, histograms).
- `rotation/events.rs` — defines rotation-specific event types (emission helpers).

Both are tightly coupled to rotation orchestration. Since rotation orchestration moves to engine in P8, and observability flows through eventbus per spec §8, these files are terminal today.

However, `rotation/scheduler.rs` (which stays in credential until P8) imports from both. Replacing their use is a mini-migration.

- [ ] **Step 1: Audit usage**

Run: `grep -rn "rotation::metrics\|rotation::events\|use.*rotation::{" crates/credential/src/`

Record every file that imports from these modules. Expected: `rotation/scheduler.rs`, possibly `rotation/mod.rs`, possibly public re-exports in `lib.rs`.

- [ ] **Step 2: Define replacement strategy**

Two options per spec §8:

(a) **Stub approach**: keep `rotation/scheduler.rs` but replace metrics emission with `tracing::info!` + event struct inline. Remove dependency on metrics/events modules. Rationale: scheduler will move entirely in P8, so stub is throwaway.

(b) **Full eventbus rewire**: make scheduler emit `CredentialEvent` via `nebula-eventbus`. More work but closer to target shape.

**Choose (a) — stub.** Scheduler lives ≤ 2 weeks before moving to engine in P8. Over-investing now violates YAGNI.

- [ ] **Step 3: Replace metrics calls in scheduler.rs**

Read `crates/credential/src/rotation/scheduler.rs`. For each metric emission (`RotationMetrics::record_*`, etc.), replace with `tracing::info!(...)` call carrying the same fields.

Example transformation:
```rust
// Before
self.metrics.record_rotation_start(credential_id);

// After
tracing::info!(credential_id = %credential_id, "rotation start");
```

Remove `use crate::rotation::metrics::*;` and `use crate::rotation::events::*;`.

- [ ] **Step 4: Delete `rotation/metrics.rs` and `rotation/events.rs`**

Run:
```bash
rm crates/credential/src/rotation/metrics.rs
rm crates/credential/src/rotation/events.rs
```

Edit `crates/credential/src/rotation/mod.rs`:

Remove:
```rust
pub mod events;
pub mod metrics;
```

Edit `crates/credential/src/lib.rs` if any re-exports existed:

Remove:
```rust
pub use crate::rotation::{CredentialRotationEvent, ...};
```

(Keep `RotationError`, `RotationResult`, `GracePeriodConfig` — they're in error/state/etc., not events/metrics modules.)

- [ ] **Step 5: Verify build + tests with rotation feature**

Run: `cargo check -p nebula-credential --features rotation && cargo nextest run -p nebula-credential --features rotation`

Expected: PASS. Any test previously verifying metrics emission should now be deleted (grep for them first) or rewritten to check `tracing` output.

- [ ] **Step 6: Commit**

```bash
git add -A crates/credential/
git commit -m "refactor(credential): delete rotation/metrics.rs + events.rs, stub with tracing until P8"
```

### Task P1.5: Remove `tokio-util` dependency

- [ ] **Step 1: Verify `tokio-util` unused**

Run: `grep -rn "tokio_util\|tokio-util" crates/credential/src/`

Expected: no matches.

- [ ] **Step 2: Remove from Cargo.toml**

Edit `crates/credential/Cargo.toml`:

Remove line:
```toml
tokio-util = { workspace = true }
```

- [ ] **Step 3: Verify build**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/credential/Cargo.toml
git commit -m "chore(credential): remove unused tokio-util dependency"
```

### Task P1.6: Phase P1 close — PR or intermediate commit

- [ ] **Step 1: Run full gate**

Run:
```bash
cargo +nightly fmt --all
cargo clippy -p nebula-credential -- -D warnings
cargo nextest run -p nebula-credential
cargo test -p nebula-credential --doc
```

Expected: all green.

- [ ] **Step 2: Push branch, open PR (optional per team flow)**

```bash
git push -u origin claude/hardcore-moser-e143b9
```

PR title: `refactor(credential): P1 duplicate collapse & dead code removal`

---

## Phase P2: Submodule grouping

**Goal:** Реорганизовать `crates/credential/src/` flat layout (~30+ top-level модулей) в 6 semantic submodule групп per spec §2. Public re-exports остаются flat в `lib.rs` (per rust-senior: idiomatic). Derive macros — audited для hardcoded paths.

**Files (target shape):**

```
crates/credential/src/
├── lib.rs                              # flat re-exports only
├── contract/                           # created
├── metadata/                           # created
├── secrets/                            # created
├── accessor/                           # created
├── scheme/                             # exists — unchanged
├── credentials/                        # exists — minor tweaks (oauth2 grouped)
├── rotation/                           # exists — contract types only after P1
├── refresh.rs                          # stays top-level (§13.2 seam)
├── error.rs                            # stays top-level
├── resolve.rs                          # stays top-level (DTOs)
├── snapshot.rs                         # stays top-level (DTO)
└── retry.rs                            # stays top-level
```

### Task P2.1: Create `contract/` submodule

- [ ] **Step 1: Create `contract/mod.rs`**

Run: `mkdir -p crates/credential/src/contract`

Write `crates/credential/src/contract/mod.rs`:

```rust
//! Credential contract surface — the Credential trait and its associated types.
//!
//! Action / resource / plugin authors bind to these types; they never touch
//! persistence, orchestration, or transport concerns.

mod any;
mod credential;
mod pending;
mod state;
mod static_protocol;

pub use any::AnyCredential;
pub use credential::Credential;
pub use pending::{NoPendingState, PendingState, PendingToken};
pub use state::CredentialState;
pub use static_protocol::StaticProtocol;
```

- [ ] **Step 2: Move files**

Run:
```bash
git mv crates/credential/src/credential.rs crates/credential/src/contract/credential.rs
git mv crates/credential/src/any.rs crates/credential/src/contract/any.rs
git mv crates/credential/src/state.rs crates/credential/src/contract/state.rs
git mv crates/credential/src/pending.rs crates/credential/src/contract/pending.rs
git mv crates/credential/src/static_protocol.rs crates/credential/src/contract/static_protocol.rs
```

- [ ] **Step 3: Update internal imports in moved files**

In each moved file, replace imports that crossed module boundaries:
- `use crate::credential::Credential` → `use super::Credential` (within contract/)
- `use crate::state::CredentialState` → `use super::CredentialState`
- `use crate::{credential::..., state::...}` → `use super::{..., ...}`

Use Grep to find all `crate::credential`, `crate::any`, `crate::state`, `crate::pending`, `crate::static_protocol` paths within contract/ files.

- [ ] **Step 4: Update lib.rs**

Edit `crates/credential/src/lib.rs`:

Remove lines:
```rust
/// Unified Credential trait.
pub mod credential;
/// Object-safe supertrait for credential dependency declaration.
pub mod any;
/// Credential state trait for stored credential data.
pub mod state;
/// Typed pending state for interactive flows.
pub mod pending;
/// Reusable protocol pattern for static credentials.
pub mod static_protocol;
```

Add:
```rust
/// Credential contract surface — trait + associated types.
pub mod contract;
```

Update re-exports to come through `contract` (keep flat paths per rust-senior):

```rust
// Flat root re-exports — idiomatic (tokio/tracing precedent)
pub use crate::any::AnyCredential;           // → replace with
pub use crate::contract::AnyCredential;

pub use credential::Credential;               // → replace with
pub use crate::contract::Credential;

pub use pending::{NoPendingState, PendingState, PendingToken};  // → replace
pub use crate::contract::{NoPendingState, PendingState, PendingToken};

pub use state::CredentialState;               // → replace
pub use crate::contract::CredentialState;

pub use static_protocol::StaticProtocol;      // → replace
pub use crate::contract::StaticProtocol;
```

- [ ] **Step 5: Update external (cross-module) imports within the crate**

Run: `grep -rn "use crate::credential::\|use crate::any::\|use crate::state::\|use crate::pending::\|use crate::static_protocol::" crates/credential/src/`

For each hit outside `contract/`, update path:
- `crate::credential::Credential` → `crate::contract::Credential` (or `crate::Credential` via flat re-export — prefer flat)
- Similarly for other types.

Prefer `crate::TypeName` via flat re-export where possible — that's the permanent public path and internal imports should use the same.

- [ ] **Step 6: Verify derive macros generated code**

Run: `grep -rn "::pending::\|::credential::\|::state::" crates/credential/macros/src/`

Expected: zero matches, OR matches use the fully-qualified new paths. Macros should emit `::nebula_credential::Credential` / `::nebula_credential::NoPendingState` (flat re-exports) or `::nebula_credential::contract::Credential`. If they emit old flat-top paths like `::nebula_credential::credential::Credential`, that would break — update macro codegen.

- [ ] **Step 7: Verify build + tests**

Run:
```bash
cargo check -p nebula-credential
cargo nextest run -p nebula-credential
cargo check -p nebula-action -p nebula-plugin -p nebula-engine
```

Expected: PASS. Consumer crates should compile without changes because flat re-exports preserved.

- [ ] **Step 8: Commit**

```bash
git add -A crates/credential/
git commit -m "refactor(credential): group contract types into contract/ submodule"
```

### Task P2.2: Create `metadata/` submodule

- [ ] **Step 1: Create `metadata/mod.rs`**

Run: `mkdir -p crates/credential/src/metadata`

Edit existing `crates/credential/src/metadata.rs` to become `crates/credential/src/metadata/metadata.rs` content (just rename file). Then create new `crates/credential/src/metadata/mod.rs`:

```rust
//! Credential metadata — static type descriptors and runtime operational state.

mod key;
mod metadata;
mod record;

pub use key::CredentialKey;
pub use metadata::{CredentialMetadata, CredentialMetadataBuilder, MetadataCompatibilityError};
pub use record::CredentialRecord;
```

- [ ] **Step 2: Move files**

```bash
mkdir -p crates/credential/src/metadata_new
git mv crates/credential/src/metadata.rs crates/credential/src/metadata_new/metadata.rs
git mv crates/credential/src/record.rs crates/credential/src/metadata_new/record.rs
git mv crates/credential/src/key.rs crates/credential/src/metadata_new/key.rs
```

Then rename the directory:
```bash
git mv crates/credential/src/metadata_new crates/credential/src/metadata
```

(Two-step rename avoids conflict with old `metadata.rs` file.)

- [ ] **Step 3: Update internal imports**

In `metadata/metadata.rs`, `metadata/record.rs`, `metadata/key.rs` — fix any `use crate::{metadata, record, key}::...` → `use super::{...}`.

- [ ] **Step 4: Update lib.rs**

Remove:
```rust
pub mod metadata;
pub mod record;
pub mod key;
```

Add (after contract):
```rust
pub mod metadata;
```

Re-exports stay flat:
```rust
pub use crate::metadata::{CredentialKey, CredentialMetadata, CredentialMetadataBuilder, MetadataCompatibilityError, CredentialRecord};
```

- [ ] **Step 5: Update cross-module imports**

Run: `grep -rn "use crate::{record::\|use crate::record::\|use crate::key::\|crate::metadata::CredentialMetadata" crates/credential/src/`

For each hit outside `metadata/`, prefer flat path: `crate::CredentialRecord`, `crate::CredentialMetadata`, `crate::CredentialKey`.

- [ ] **Step 6: Verify build + tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A crates/credential/
git commit -m "refactor(credential): group metadata/record/key into metadata/ submodule"
```

### Task P2.3: Create `secrets/` submodule

- [ ] **Step 1: Create `secrets/mod.rs`**

Run: `mkdir -p crates/credential/src/secrets`

Write `crates/credential/src/secrets/mod.rs`:

```rust
//! §12.5 primitives — AES-256-GCM, Argon2id KDF, PKCE, zeroizing secret wrappers.
//!
//! Canon-level secret-handling primitives. Every plaintext buffer here is
//! wrapped in `Zeroizing<T>` or a zeroize-on-drop newtype. Primitives are
//! called by `nebula-storage::credential::layer::encryption` (which holds
//! the layer impl) after the P6 move.

mod crypto;
mod guard;
mod secret_string;
mod serde_secret;

pub use crypto::{EncryptedData, EncryptionKey, decrypt, encrypt};
pub use guard::CredentialGuard;
pub use secret_string::SecretString;
pub use serde_secret as serde_secret_module; // if re-exporting module
```

Actually: serde_secret is used as `#[serde(with = "...")]` path, which needs to be reachable as a module path. Plan: keep `serde_secret` as module-level in `secrets/`, and re-export the module from `lib.rs` under the old path for backward-compat.

- [ ] **Step 2: Move files**

```bash
git mv crates/credential/src/crypto.rs crates/credential/src/secrets/crypto.rs
git mv crates/credential/src/guard.rs crates/credential/src/secrets/guard.rs
git mv crates/credential/src/secret_string.rs crates/credential/src/secrets/secret_string.rs
git mv crates/credential/src/serde_secret.rs crates/credential/src/secrets/serde_secret.rs
```

- [ ] **Step 3: Update internal imports**

In `secrets/crypto.rs`, `guard.rs`, `secret_string.rs`, `serde_secret.rs`: fix imports per submodule.

- [ ] **Step 4: Update lib.rs**

Remove:
```rust
pub mod crypto;
pub mod guard;
pub mod secret_string;
pub mod serde_secret;
```

Add:
```rust
pub mod secrets;

/// Back-compat alias for `#[serde(with = "nebula_credential::serde_secret")]`.
pub use crate::secrets::serde_secret;
```

Flat re-exports:
```rust
pub use crate::secrets::{EncryptedData, EncryptionKey, decrypt, encrypt, CredentialGuard, SecretString};
```

- [ ] **Step 5: Update cross-module imports**

Run: `grep -rn "crate::crypto::\|crate::guard::\|crate::secret_string::" crates/credential/src/`

Prefer flat path via re-export: `crate::SecretString`, `crate::CredentialGuard`, `crate::encrypt`, etc.

- [ ] **Step 6: Verify derive macros**

Run: `grep -rn "::crypto::\|::guard::\|::secret_string::" crates/credential/macros/src/`

Expected: zero or updated to new paths.

- [ ] **Step 7: Verify build + tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential && cargo test -p nebula-credential --doc`

Expected: PASS (doc tests matter here — serde attribute paths).

- [ ] **Step 8: Commit**

```bash
git add -A crates/credential/
git commit -m "refactor(credential): group §12.5 primitives into secrets/ submodule"
```

### Task P2.4: Create `accessor/` submodule

- [ ] **Step 1: Create `accessor/mod.rs`**

Run: `mkdir -p crates/credential/src/accessor`

Write `crates/credential/src/accessor/mod.rs`:

```rust
//! Consumer-facing accessor surface.
//!
//! Action / resource code imports from here to obtain credentials via
//! `CredentialAccessor`. `CredentialHandle` is the typed handle returned
//! by resolution.

mod access_error;
mod accessor;
mod context;
mod handle;

pub use access_error::CredentialAccessError;
pub use accessor::{
    CredentialAccessor, NoopCredentialAccessor, ScopedCredentialAccessor,
    default_credential_accessor,
};
pub use context::{CredentialContext, CredentialResolverRef};
pub use handle::CredentialHandle;
```

- [ ] **Step 2: Move files**

```bash
git mv crates/credential/src/accessor.rs crates/credential/src/accessor/accessor.rs
git mv crates/credential/src/access_error.rs crates/credential/src/accessor/access_error.rs
git mv crates/credential/src/handle.rs crates/credential/src/accessor/handle.rs
git mv crates/credential/src/context.rs crates/credential/src/accessor/context.rs
```

(Note: `accessor.rs` file inside `accessor/` directory — valid Rust, but consider renaming to `accessor/impls.rs` if confusing.)

- [ ] **Step 3: Update internal imports**

Within `accessor/` files, fix `crate::accessor::... → super::...`, etc.

- [ ] **Step 4: Update lib.rs**

Remove:
```rust
pub mod accessor;
pub mod access_error;
pub mod handle;
pub mod context;
```

Add:
```rust
pub mod accessor;
```

Flat re-exports:
```rust
pub use crate::accessor::{CredentialAccessor, NoopCredentialAccessor, ScopedCredentialAccessor, default_credential_accessor, CredentialAccessError, CredentialHandle};
pub use crate::accessor::{CredentialContext, CredentialResolverRef};
```

- [ ] **Step 5: Verify build + tests**

Run: `cargo check --workspace && cargo nextest run -p nebula-credential -p nebula-action`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A crates/credential/
git commit -m "refactor(credential): group consumer-facing accessor types into accessor/ submodule"
```

### Task P2.5: Group `credentials/oauth2*` into `credentials/oauth2/`

- [ ] **Step 1: Create `credentials/oauth2/` directory**

Run: `mkdir -p crates/credential/src/credentials/oauth2`

- [ ] **Step 2: Move files**

```bash
git mv crates/credential/src/credentials/oauth2.rs crates/credential/src/credentials/oauth2/credential.rs
git mv crates/credential/src/credentials/oauth2_config.rs crates/credential/src/credentials/oauth2/config.rs
git mv crates/credential/src/credentials/oauth2_flow.rs crates/credential/src/credentials/oauth2/flow.rs
```

- [ ] **Step 3: Create `credentials/oauth2/mod.rs`**

```rust
//! OAuth2 credential type.
//!
//! Type definition, state shape, and configuration. HTTP flow machinery
//! (reqwest client) lives in `flow.rs` and is scheduled to move to
//! `nebula-api` / `nebula-engine` in P10. Until then, flow stays here
//! (credential crate retains `reqwest` dep).

mod config;
mod credential;
mod flow;

pub use config::*;
pub use credential::{OAuth2Credential, OAuth2Pending, OAuth2State};
pub use flow::*;
```

Inspect exact public surface of each sub-file before writing `mod.rs`. Names like `OAuth2Config`, `OAuth2Flow`, etc. — list them accurately.

- [ ] **Step 4: Update `credentials/mod.rs`**

Edit `crates/credential/src/credentials/mod.rs`:

Remove:
```rust
pub mod oauth2;
pub mod oauth2_config;
pub mod oauth2_flow;
```

Add:
```rust
pub mod oauth2;
```

Re-exports stay flat (public surface preserved).

- [ ] **Step 5: Update internal imports in oauth2/{credential,config,flow}.rs**

Fix `use crate::credentials::oauth2_config::*` → `use super::config::*`.

- [ ] **Step 6: Verify build + tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A crates/credential/
git commit -m "refactor(credential): group oauth2 {credential,config,flow} into credentials/oauth2/"
```

### Task P2.6: Review `lib.rs` — ensure flat re-exports idiomatic

- [ ] **Step 1: Read final `lib.rs`**

Read `crates/credential/src/lib.rs`. Verify:

- Top-level `pub mod` declarations are the 6 semantic groups: `contract`, `metadata`, `secrets`, `accessor`, `scheme`, `credentials`, plus utility modules (`rotation`, `refresh`, `error`, `resolve`, `snapshot`, `retry`).
- Flat re-exports: `pub use crate::{SecretString, CredentialGuard, Credential, ...}`. Consumers continue to `use nebula_credential::SecretString;` — not `use nebula_credential::secrets::SecretString;`.
- Module docs point to spec & canon.

- [ ] **Step 2: Add lib.rs top-level doc note**

Ensure module-level doc block mentions:

```rust
//! ## Canonical import paths
//!
//! This crate follows the tokio/tracing idiom: submodules are `pub` for
//! escape hatches, but the canonical public surface is flat re-exports at
//! the root. Prefer `nebula_credential::SecretString` over
//! `nebula_credential::secrets::SecretString`.
```

- [ ] **Step 3: Final build + clippy**

Run:
```bash
cargo +nightly fmt --all
cargo clippy -p nebula-credential -- -D warnings
cargo nextest run -p nebula-credential
cargo test -p nebula-credential --doc
cargo check --workspace
```

Expected: all green.

- [ ] **Step 4: P2 CredentialGuard Copy-derive grep (rust-senior ask)**

Run: `grep -rn "#\[derive.*Copy.*\]" crates/credential/src/`

For each hit, verify type does NOT carry secret material. `CredentialGuard` / `SecretString` / `Zeroizing` must not be `Copy` (implicit copy = zeroize bypass).

Expected: no offending derives. If found, remove `Copy`.

- [ ] **Step 5: Commit (if any fix needed)**

```bash
git add -A crates/credential/
git commit -m "chore(credential): final P2 polish — lib.rs docs + Copy-derive audit"
```

### Task P2.7: Phase P2 close

- [ ] **Step 1: Full gate**

Run:
```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
```

Expected: all green.

- [ ] **Step 2: Push + optional PR**

```bash
git push
```

PR title: `refactor(credential): P2 submodule grouping (contract/metadata/secrets/accessor)`

---

## Phase P3: Base-dep diet (partial)

**Goal:** Удалить dead base deps, которые уже unused после P1 (tokio-util уже удалён в P1.5). Дополнительно audit tokio features. Большая часть dep-диеты отложена до P6-P10, потому что rotation/scheduler.rs (использует nebula-metrics/telemetry) переезжает только в P8, а reqwest используется oauth2_flow до P10.

**Files:**
- Modify: `crates/credential/Cargo.toml` (audit tokio features)

### Task P3.1: Audit tokio features

- [ ] **Step 1: Current tokio features**

Read `crates/credential/Cargo.toml`. Current: `tokio = { workspace = true, features = ["time", "sync", "macros", "rt"] }`.

- [ ] **Step 2: Check usage of each feature**

For each feature:
- `time` — search `tokio::time::`, `Duration`, `sleep`, `timeout`, `Interval`:
  `grep -rn "tokio::time\|tokio::time::sleep\|tokio::time::timeout" crates/credential/src/`
- `sync` — search `tokio::sync::`:
  `grep -rn "tokio::sync\|oneshot\|Mutex\|Semaphore" crates/credential/src/ | grep "tokio::"`
- `macros` — search `#[tokio::test]`, `#[tokio::main]`:
  `grep -rn "#\[tokio::test\]\|#\[tokio::main\]" crates/credential/src/ crates/credential/tests/`
- `rt` — search `tokio::runtime::`:
  `grep -rn "tokio::runtime::" crates/credential/src/`

- [ ] **Step 3: Trim features**

Keep only features used. Typical outcome for a contract crate (post-P1): `["sync", "macros"]` (oneshot + test attribute; no `time` or `rt` if not used). If grep shows `time` usage — keep.

Edit `crates/credential/Cargo.toml`:

```toml
tokio = { workspace = true, features = ["sync", "macros"] }  # adjust per grep results
```

- [ ] **Step 4: Verify build + tests**

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`

Expected: PASS. If FAIL on missing feature — add it back and record in commit message why.

- [ ] **Step 5: Commit**

```bash
git add crates/credential/Cargo.toml
git commit -m "chore(credential): trim tokio features to minimum used"
```

### Task P3.2: Phase P3 close

- [ ] **Step 1: Full gate**

Run:
```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

Expected: all green.

- [ ] **Step 2: Push + optional PR**

PR title: `chore(credential): P3 base-dep diet (partial — metrics/telemetry deferred to P8, reqwest to P10)`

---

## Phase P4: nebula-core → nebula-credential migration

**Goal:** Переместить 4 credential-specific типа из `nebula-core` (frontier) в `nebula-credential`. Clean break — без deprecated shim (alpha + breaking разрешён per spec).

**Files:**
- Delete: from `crates/core/src/`: definitions of `AuthPattern`, `AuthScheme`, `CredentialEvent`, `CredentialId` (and their modules)
- Create/Modify: `crates/credential/src/scheme/auth_pattern.rs`, `crates/credential/src/scheme/auth_scheme.rs` (or embed in existing mod.rs)
- Create: `crates/credential/src/event.rs` (for `CredentialEvent`)
- Modify: `crates/credential/src/metadata/key.rs` (host `CredentialId` — or separate file)
- Modify: `crates/core/src/lib.rs` (remove re-exports)
- Modify: `crates/credential/src/lib.rs` (remove `nebula_core::...` re-exports; add local re-exports)
- Modify: consumers (action, plugin, sandbox, engine, runtime, sdk) — update imports
- Modify: `docs/MATURITY.md` — reflect nebula-core slimdown

### Task P4.1: Locate source definitions in nebula-core

- [ ] **Step 1: Find definitions**

Run:
```bash
grep -rn "pub struct AuthPattern\|pub enum AuthPattern\|pub struct AuthScheme\|pub trait AuthScheme\|pub struct CredentialEvent\|pub enum CredentialEvent\|pub struct CredentialId" crates/core/src/
```

Record each file + line. Typical locations: `crates/core/src/auth.rs` or `crates/core/src/credential.rs`.

- [ ] **Step 2: Find each type's dependencies**

For each of the 4 types, list:
- Other types they import from `nebula-core`
- Derive macros used (`Serialize`, `Deserialize`, `Debug`)
- Trait impls (`Display`, `From`, `PartialEq`)

Goal: understand what travels with the type.

### Task P4.2: Copy `CredentialId` to credential

- [ ] **Step 1: Read source**

Read the file containing `CredentialId` in `crates/core/src/`.

- [ ] **Step 2: Write to credential**

Create `crates/credential/src/metadata/id.rs` (or append to existing `key.rs` if thematically fits):

```rust
//! CredentialId newtype — identifies a stored credential instance.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a stored credential instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CredentialId(pub Uuid);

// ... copy impl blocks from nebula-core
```

Exact content: copy verbatim from `nebula-core`.

- [ ] **Step 3: Update `metadata/mod.rs`**

```rust
mod id;
pub use id::CredentialId;
```

- [ ] **Step 4: Flat re-export in lib.rs**

Edit `crates/credential/src/lib.rs` — remove `pub use nebula_core::CredentialId;`, add `pub use crate::metadata::CredentialId;`.

- [ ] **Step 5: Verify credential builds standalone**

Run: `cargo check -p nebula-credential`

Expected: PASS. Tests deferred until all 4 types moved.

### Task P4.3: Copy `CredentialEvent` to credential

- [ ] **Step 1: Read source**

Read the file containing `CredentialEvent` in `crates/core/src/`.

- [ ] **Step 2: Write to credential**

Create `crates/credential/src/event.rs`:

```rust
//! CredentialEvent — emitted through nebula-eventbus for observability subscribers.

use serde::{Deserialize, Serialize};

use crate::CredentialId;

/// Event emitted by credential operations for observability subscribers.
///
/// Fire-and-forget via `nebula-eventbus`; not a durability channel
/// (see spec §8 audit path for in-line durable semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CredentialEvent {
    // ... copy variants from nebula-core
}
```

- [ ] **Step 3: Update lib.rs**

```rust
pub mod event;
pub use crate::event::CredentialEvent;
```

Remove `pub use nebula_core::CredentialEvent;`.

- [ ] **Step 4: Verify credential builds**

Run: `cargo check -p nebula-credential`

### Task P4.4: Copy `AuthPattern` + `AuthScheme` to credential

- [ ] **Step 1: Read source**

Read files containing `AuthPattern` / `AuthScheme` in `crates/core/src/`.

- [ ] **Step 2: Write to credential**

Create `crates/credential/src/scheme/auth.rs` (or extend `scheme/mod.rs`):

```rust
//! AuthPattern classification + AuthScheme open trait.

use serde::{Deserialize, Serialize};

/// Classification of credential authentication patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthPattern {
    // ... copy variants
}

/// Open trait for authentication schemes.
pub trait AuthScheme {
    // ... copy trait body
}
```

- [ ] **Step 3: Update `scheme/mod.rs`**

```rust
mod auth;
pub use auth::{AuthPattern, AuthScheme};
```

- [ ] **Step 4: Update lib.rs**

Remove `pub use nebula_core::{AuthPattern, AuthScheme};`.
Add `pub use crate::scheme::{AuthPattern, AuthScheme};`.

- [ ] **Step 5: Verify credential builds**

Run: `cargo check -p nebula-credential`

### Task P4.5: Remove types from nebula-core

- [ ] **Step 1: Delete source files or remove definitions**

Edit files in `crates/core/src/` — remove all 4 type definitions. If a file only contained one of these, delete it entirely; update `crates/core/src/lib.rs` to drop the module.

- [ ] **Step 2: Remove from nebula-core public surface**

Edit `crates/core/src/lib.rs`:

Remove any `pub use` that exposed the 4 types. Remove `pub mod` for files that were deleted.

- [ ] **Step 3: Verify nebula-core builds**

Run: `cargo check -p nebula-core`

Expected: PASS if `nebula-core` has no internal callers of the 4 types. If FAIL — the type was actually used inside nebula-core, not just re-exported; revisit §10 of spec (should the type stay in core with a different name, or did we miss an internal use).

### Task P4.6: Update consumer imports (leaf-first)

Leaf-first order per spec §11: action → plugin → sandbox → engine → runtime → sdk.

- [ ] **Step 1: Update nebula-action**

Run: `grep -rn "use nebula_core::{.*AuthPattern\|use nebula_core::{.*AuthScheme\|use nebula_core::{.*CredentialEvent\|use nebula_core::{.*CredentialId\|use nebula_core::AuthPattern\|use nebula_core::AuthScheme\|use nebula_core::CredentialEvent\|use nebula_core::CredentialId" crates/action/src/`

For each hit, change `nebula_core::X` → `nebula_credential::X`. Batch-replace via sed/editor.

Run: `cargo check -p nebula-action`

Expected: PASS.

- [ ] **Step 2: Update nebula-plugin**

Same pattern — grep, replace, check.

Run: `cargo check -p nebula-plugin`

- [ ] **Step 3: Update nebula-sandbox**

Same pattern.

Run: `cargo check -p nebula-sandbox`

- [ ] **Step 4: Update nebula-engine**

Same pattern.

Run: `cargo check -p nebula-engine`

- [ ] **Step 5: Update nebula-runtime**

Same pattern.

Run: `cargo check -p nebula-runtime`

- [ ] **Step 6: Update nebula-sdk**

Same pattern. SDK is facade, may need re-export tweaks.

Run: `cargo check -p nebula-sdk`

- [ ] **Step 7: Full workspace check**

Run: `cargo check --workspace && cargo nextest run --workspace`

Expected: PASS.

- [ ] **Step 8: Commit migration**

```bash
git add -A
git commit -m "refactor!: move AuthPattern/AuthScheme/CredentialEvent/CredentialId from nebula-core to nebula-credential"
```

### Task P4.7: Update MATURITY.md

- [ ] **Step 1: Update MATURITY row for nebula-core**

Edit `docs/MATURITY.md`. `nebula-core` row may shift columns if surface narrowed (likely still `frontier` — depends on remaining frontier-ness).

- [ ] **Step 2: Append to "Last targeted revision" log**

Add entry:

```markdown
Prior: 2026-04-20 — P4 of credential cleanup: AuthPattern, AuthScheme,
CredentialEvent, CredentialId migrated from nebula-core to nebula-credential
(credential-specific types no longer polluting core). Consumers (action,
plugin, sandbox, engine, runtime, sdk) updated. Spec:
docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md.
```

- [ ] **Step 3: Commit**

```bash
git add docs/MATURITY.md
git commit -m "docs(maturity): reflect P4 credential types migration out of nebula-core"
```

### Task P4.8: Phase P4 close

- [ ] **Step 1: Full gate**

Run:
```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
cargo test --workspace --doc
```

Expected: all green.

- [ ] **Step 2: Push + optional PR**

PR title: `refactor!: P4 migrate credential types from nebula-core to nebula-credential`

---

## Phase P5: ADR-0028..0031 landing

**Goal:** Написать и принять 4 ADR'а. Это ADR-only PR без кода. **Hard go/no-go checkpoint** — перед P6+ (physical crate moves) нужно acceptance всех 4 ADR. Если хоть один блокер — revisit design.

**Files:**
- Create: `docs/adr/0028-cross-crate-credential-invariants.md`
- Create: `docs/adr/0029-storage-owns-credential-persistence.md`
- Create: `docs/adr/0030-engine-owns-credential-orchestration.md`
- Create: `docs/adr/0031-api-owns-oauth-flow.md`
- Modify: `docs/adr/0023-keyprovider-trait.md` (add `superseded_by: [0029]` to frontmatter when landing)
- Modify: `docs/adr/README.md` (add 4 new entries to ADR index)

### Task P5.1: Write ADR-0028 (umbrella)

- [ ] **Step 1: Check ADR template and numbering**

Read `docs/adr/README.md` — confirm numbering conventions, required frontmatter.

Run: `ls docs/adr/ | tail -5` — verify 0025, 0026, 0027 exist and 0028+ free.

- [ ] **Step 2: Write ADR-0028**

Create `docs/adr/0028-cross-crate-credential-invariants.md`:

```markdown
---
id: 0028
title: cross-crate-credential-invariants
status: proposed
date: 2026-04-20
supersedes: []
superseded_by: []
tags: [credential, storage, engine, api, security, canon-12.5, canon-13.2]
related:
  - docs/adr/0023-keyprovider-trait.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0031-api-owns-oauth-flow.md
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
linear: []
---

# 0028. Cross-crate credential invariants

## Context

(Write based on spec §4. Motivates the redistribution of nebula-credential
responsibilities across 4 crates. Cites §12.5 / §13.2 / §3.5 / §14 / §4.5
as anchoring canon sections. References ADR-0023 as precedent for
KeyProvider seam, to be superseded in part by ADR-0029.)

## Decision

(Paste verbatim or adapt spec §4 — 8 invariants: §12.5 preservation,
§13.2 seam integrity, §3.5 split, §14 no discard-and-log, §4.5 honest
MATURITY, cross-crate compat, zeroize at boundaries, versioning in alpha.)

## Consequences

**Positive**
- (from spec §14 "Positive" considerations)

**Negative / accepted costs**
- (from spec risks)

**Neutral**
- (from spec)

## Alternatives considered

(See spec model A / C revised / rejected variants.)

## Seam / verification

(References files that carry the invariants — spec §13 testing.)

## Follow-ups

(References ADR-0029/0030/0031 as downstream.)
```

Write the full ADR body using the spec's §0, §1, §4, §14 content as source.

- [ ] **Step 3: Verify ADR lints**

Run (if there's an ADR linter): `cargo run -p adr-lint` or manually verify frontmatter + section headers match existing ADR style (see `docs/adr/0025-sandbox-broker-rpc-surface.md` as reference).

- [ ] **Step 4: Add to ADR index**

Edit `docs/adr/README.md`, add entry for 0028 following existing list format.

### Task P5.2: Write ADR-0029 (storage owns persistence, supersedes ADR-0023)

- [ ] **Step 1: Write ADR-0029**

Create `docs/adr/0029-storage-owns-credential-persistence.md` using spec §5 as source.

Frontmatter notes:
- `supersedes: [0023]` (partial — location of `KeyProvider`/`EncryptionLayer`)
- Mark invariants from §12.5 preserved bit-for-bit

- [ ] **Step 2: Update ADR-0023 frontmatter**

Edit `docs/adr/0023-keyprovider-trait.md`:

Change `superseded_by: []` → `superseded_by: [0029]`.

Add to related if not present.

- [ ] **Step 3: Add to ADR index**

### Task P5.3: Write ADR-0030 (engine owns orchestration)

- [ ] **Step 1: Write ADR-0030**

Create `docs/adr/0030-engine-owns-credential-orchestration.md` using spec §6 as source.

Critical sections: RefreshCoordinator stays concrete-not-trait; token_refresh no-logs policy; reqwest as engine base dep.

- [ ] **Step 2: Add to ADR index**

### Task P5.4: Write ADR-0031 (api owns OAuth flow)

- [ ] **Step 1: Write ADR-0031**

Create `docs/adr/0031-api-owns-oauth-flow.md` using spec §7 as source.

Critical sections: Security invariants (PKCE S256 mandatory, CSRF HMAC+TTL, URL allowlist, zeroize on partial failure), feature gate `credential-oauth`.

- [ ] **Step 2: Add to ADR index**

### Task P5.5: Cross-reference validation

- [ ] **Step 1: Verify all ADRs reference each other correctly**

Each of 0028/0029/0030/0031 should have `related:` entry for the other 3.
ADR-0023 should now have `superseded_by: [0029]`.

Run: `grep -l "0028\|0029\|0030\|0031" docs/adr/00{28,29,30,31,23}*.md`

Expected: all 5 files appear in each relevant ADR's related list.

- [ ] **Step 2: Update ADR index alphabetically/numerically**

Verify `docs/adr/README.md` lists ADR 0028..0031 in order with correct titles.

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0028-cross-crate-credential-invariants.md \
        docs/adr/0029-storage-owns-credential-persistence.md \
        docs/adr/0030-engine-owns-credential-orchestration.md \
        docs/adr/0031-api-owns-oauth-flow.md \
        docs/adr/0023-keyprovider-trait.md \
        docs/adr/README.md
git commit -m "docs(adr): add 0028-0031 for credential architecture cleanup; 0023 superseded"
```

### Task P5.6: PR & go/no-go checkpoint

- [ ] **Step 1: Full gate**

Run:
```bash
cargo +nightly fmt --all
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace
```

Expected: all green (no code changes since P4, but paranoid check).

- [ ] **Step 2: Open ADR PR**

```bash
git push
gh pr create --title "docs(adr): 0028-0031 credential cross-crate invariants (supersedes 0023)" --body "$(cat <<'EOF'
## Summary
- ADR-0028: umbrella "cross-crate credential invariants" (§12.5/§13.2/§14 anchors)
- ADR-0029: nebula-storage owns credential persistence (supersedes ADR-0023 §KeyProvider location)
- ADR-0030: nebula-engine owns credential orchestration + token refresh
- ADR-0031: nebula-api owns OAuth flow HTTP ceremony

Reviewed spec: docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md

## Test plan
- [ ] ADR linter passes (or frontmatter manually verified)
- [ ] Cross-references between 0028-0031 consistent
- [ ] ADR-0023 frontmatter updated with superseded_by: [0029]

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Await review**

**Handoffs per spec §15:**
- security-lead — final nod on §12.5/§13.2 seam boundaries + OAuth §7 invariants
- tech-lead — confirm P5 as hard checkpoint
- rust-senior — confirm P2/P3 refactor landed clean

- [ ] **Step 4: Go/no-go decision**

If all ADRs accepted: P6+ unblocked. Proceed to next plan.
If any ADR blocked: stop, revisit spec, do not start P6.

---

## Self-review

**1. Spec coverage (P1-P5 only):**

- Spec §1 DoD items 1-4 + 8 (partial 9-11 deferred to P6+) — covered by P1, P2, P3, P4 tasks.
- Spec §3 migrations — only P1 deletions in scope (retry dup, metrics, events). Physical moves P6+.
- Spec §4 ADR-0028 — P5.1.
- Spec §5 ADR-0029 — P5.2.
- Spec §6 ADR-0030 — P5.3.
- Spec §7 ADR-0031 — P5.4.
- Spec §9 dep diet — partial in P3.1 (tokio audit); full after P8/P10.
- Spec §10 core migration — P4.
- Spec §11 consumer order — P4.6 (in P4 scope).
- Spec §12 phases P1-P5 — all 5 phases mapped.
- Spec §13 CI gates — partially introduced (P2.6 Copy grep, P2.1 derive macros audit); full gates appear with P6+ when cross-crate moves land.
- Spec §15 handoffs — P5.6 dispatches.

**2. Placeholder scan:** no "TBD", "TODO", "implement later", or unqualified "handle edge cases". ADR bodies are described in terms of spec sections to copy from (acceptable since spec is the source of truth).

**3. Type consistency:** `CredentialId`, `CredentialEvent`, `AuthPattern`, `AuthScheme` names stable across P4 tasks. `RetryPolicy`/`RotationRetryPolicy` handled in P1.2 with rename guidance. `serde_secret::option` submodule vs old `option_serde_secret` explicit.

**4. Scope check:** P1-P5 are self-contained (no cross-crate physical moves, no code in P5). Fits in ~1 week of engineering if unblocked.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-20-credential-cleanup-p1-p5.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
