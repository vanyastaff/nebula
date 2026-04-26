---
name: credential П1 — trait shape scaffolding
status: draft (writing-plans skill output 2026-04-26 — awaiting execution-mode choice)
date: 2026-04-26
authors: [vanyastaff, Claude]
phase: П1
scope: cross-cutting — nebula-credential, nebula-credential-builtin (NEW), nebula-credential-macros, nebula-engine, nebula-resource, nebula-core
related:
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
  - docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/tracking/credential-concerns-register.md
---

# Credential П1 — Trait Shape Scaffolding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the validated CP5/CP6 credential trait shape — capability sub-trait split, AuthScheme sensitivity dichotomy, fatal duplicate-KEY registration, SchemeGuard/SchemeFactory refresh hook, capability-from-type authority shift, ADR-0035 phantom-shim canonical form — plus the `nebula-credential-builtin` crate scaffold and 8 mandatory landing-gate probes (7 compile-fail + 1 runtime).

**Architecture:** П1 is foundational: all later phases (П2–П10) depend on this trait shape. The work is one coherent breaking change to `nebula-credential`'s public surface, executed in a dedicated worktree with one commit per task. No backward-compat shims (per `feedback_hard_breaking_changes.md` + `feedback_no_shims.md`). Tests are mostly compile-fail probes (`trybuild`) — they ARE the verification, since the changes are largely type-level.

**Tech Stack:** Rust 1.95.0 (pinned), tokio 1.51 (async runtime), `zeroize` 1.8 (with `zeroize_derive`), `secrecy` workspace, `ahash` 0.8 (registry hash), `trybuild` 1.0 (compile-fail probes), `cargo-nextest` (test runner per CI matrix), `cargo-public-api` (ABI snapshot), `nebula-credential-macros` (proc-macro emission for `#[capability]` + `#[plugin_credential]`).

**Pre-execution requirement:** Create dedicated worktree per `superpowers:using-git-worktrees`. Plan execution agent runs inside the worktree; main branch sees only the merge commit at landing.

**Reading order for the engineer:** Tech Spec §2 (current shape), §15.3–§15.10 (CP5/CP6 canonical), ADR-0035 (phantom-shim), Strategy §3 (type system contract). Then this plan.

---

## File Map

### Created files

| Path | Purpose |
|------|---------|
| `crates/credential-builtin/Cargo.toml` | NEW crate manifest — concrete credential types live here |
| `crates/credential-builtin/src/lib.rs` | NEW crate root — re-exports + `mod sealed_caps` |
| `crates/credential-builtin/README.md` | NEW crate readme — split rationale, plugin author guide |
| `crates/credential/src/contract/interactive.rs` | `Interactive` sub-trait + `Pending` assoc type |
| `crates/credential/src/contract/refreshable.rs` | `Refreshable` sub-trait |
| `crates/credential/src/contract/revocable.rs` | `Revocable` sub-trait |
| `crates/credential/src/contract/testable.rs` | `Testable` sub-trait |
| `crates/credential/src/contract/dynamic.rs` | `Dynamic` sub-trait |
| `crates/credential/src/contract/registry.rs` | `CredentialRegistry` + `RegisterError` (fatal duplicate-KEY) |
| `crates/credential/src/secrets/scheme_guard.rs` | `SchemeGuard<'a, C>` + `SchemeFactory<C>` |
| `crates/credential/src/contract/capability_report.rs` | `plugin_capability_report::*` per-capability constants |
| `crates/credential/tests/compile_fail_state_zeroize.rs` | Probe 1 (trybuild driver) |
| `crates/credential/tests/compile_fail_scheme_sensitivity.rs` | Probe 2 |
| `crates/credential/tests/compile_fail_capability_subtrait.rs` | Probe 3 |
| `crates/credential/tests/compile_fail_engine_dispatch_capability.rs` | Probe 4 |
| `crates/credential/tests/runtime_duplicate_key_fatal.rs` | Probe 5 (runtime, not trybuild) |
| `crates/credential/tests/compile_fail_scheme_guard_retention.rs` | Probe 6 |
| `crates/credential/tests/compile_fail_scheme_guard_clone.rs` | Probe 7 |
| `crates/credential/tests/compile_fail_metadata_capability_field.rs` | Probe 8 |
| `crates/credential/tests/probes/*.rs` | Per-probe compile-fail input fixtures (one `.rs` + one `.stderr` per case) |
| `crates/credential/macros/src/capability.rs` | NEW `#[capability]` proc-macro |

### Modified files

| Path | Change |
|------|--------|
| `Cargo.toml` (workspace) | Add `crates/credential-builtin` to `members` |
| `deny.toml` | Whitelist allowed deps for `nebula-credential-builtin` |
| `crates/credential/Cargo.toml` | Bump `dev-dependencies` with `trybuild` |
| `crates/credential/src/lib.rs` | Re-exports for new sub-traits + `SchemeGuard` + `SchemeFactory` + `RegisterError` |
| `crates/credential/src/contract/credential.rs` | Strip 5 const bools + 5 default method bodies; remove `type Pending` (moved to `Interactive`); strip `metadata()`/`schema()` from base (kept on `CredentialMetadataSource`) |
| `crates/credential/src/contract/state.rs` | Add `ZeroizeOnDrop` supertrait bound |
| `crates/credential/src/contract/mod.rs` | Wire new submodules |
| `crates/credential/src/contract/any.rs` | `AnyCredential` revisit — drop assumptions about base trait having capability methods |
| `crates/credential/src/scheme/*.rs` (each scheme) | `SensitiveScheme` or `PublicScheme` impl + `ZeroizeOnDrop` derive where required |
| `crates/credential/src/scheme/connection_uri.rs` | Restructure with structured accessors per §15.5 §3295 |
| `crates/credential/src/scheme/oauth2.rs` (or wherever `OAuth2Token` lives) | `bearer_header()` returns `SecretString` |
| `crates/core/src/auth/mod.rs` (or wherever `AuthScheme` lives) | Reduce `AuthScheme` to base; add `SensitiveScheme` + `PublicScheme` sub-traits |
| `crates/credential/src/credentials/api_key.rs` | Drop `const TESTABLE = ...`; impl `Testable` if previously testable |
| `crates/credential/src/credentials/basic_auth.rs` | Same — sub-trait migration |
| `crates/credential/src/credentials/oauth2.rs` | Impl `Interactive`, `Refreshable`, `Revocable`, `Testable` as appropriate |
| `crates/credential/src/metadata.rs` | Remove `capabilities_enabled` field |
| `crates/credential/macros/src/lib.rs` | Wire new `capability` macro entry; update `Credential` derive |
| `crates/credential/macros/src/credential.rs` | Drop emission of `const INTERACTIVE/REFRESHABLE/...`; emit per-sub-trait impls + `plugin_capability_report::*` constants |
| `crates/credential/macros/src/auth_scheme.rs` | Add `sensitive` / `public` argument; field-name + field-type audit |
| `crates/engine/src/credential/rotation/mod.rs` (and scheduler / token_refresh) | Bind `where C: Refreshable` on dispatch entry points |
| `crates/engine/src/credential/registry.rs` | Replace silent overwrite with `Result<(), RegisterError>` propagation; remove `tracing::warn!` overwrite path |
| `crates/engine/src/credential/discovery.rs` (or wherever `iter_compatible` lives) | Filter via registry-computed capabilities, not metadata field |
| `crates/resource/src/contract.rs` (or equivalent — Resource trait location) | `on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, ...>, ctx: &'a CredentialContext<'a>)` signature |
| `crates/credential/README.md` | Reflect new shape — sub-traits, dichotomy, fatal duplicate, SchemeGuard, capability-from-type, plugin author obligations |
| `docs/MATURITY.md` | `nebula-credential` row update + new `nebula-credential-builtin` row |
| `docs/superpowers/specs/2026-04-24-credential-tech-spec.md` | Status flips: `complete CP6` → `П1 in-implementation` (with phase-plan + commit pointer) |
| `docs/tracking/credential-concerns-register.md` | Status flips for affected rows (see Stage 8) |

### Deleted files

| Path | Reason |
|------|--------|
| `crates/credential/src/contract/static_protocol.rs` | Subsumed by sub-trait split — static credentials are non-`Interactive`+non-`Refreshable`+non-`Revocable`+non-`Testable`+non-`Dynamic` impls of base `Credential`. Re-exports updated. |
| `crates/credential/src/contract/pending.rs` (if a separate file exists for `NoPendingState`) | `NoPendingState` removed — base `Credential` has no `Pending` after sub-trait split |

---

## Stage 0 — Foundation (worktree, baseline, builtin crate scaffold)

### Task 0.1 — Worktree + baseline

**Files:** none (worktree creation + baseline check)

- [ ] **Step 1: Create worktree**

```bash
git worktree add -b credential-p1-trait-scaffolding ../nebula-credential-p1
cd ../nebula-credential-p1
```

- [ ] **Step 2: Baseline check — full local gate**

Run: `cargo +nightly fmt --all -- --check && cargo clippy --workspace -- -D warnings && cargo nextest run -p nebula-credential -p nebula-engine -p nebula-storage --profile ci --no-tests=pass`

Expected: PASS. If any failure, halt and fix before П1 work begins.

- [ ] **Step 3: Capture pre-П1 cargo-public-api snapshot**

Run: `cargo public-api --manifest-path crates/credential/Cargo.toml > /tmp/credential-pre-p1.txt`

Hold this file aside — used in Stage 8 to confirm the breaking-change boundary is exactly the П1 surface.

- [ ] **Step 4: Commit baseline marker**

No code change — annotate the worktree starting point:

```bash
git commit --allow-empty -m "chore(credential): П1 worktree baseline marker

Pre-П1 cargo-public-api snapshot captured at /tmp/credential-pre-p1.txt
(local-only; not committed). All workspace gates green at this commit.

Refs: docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md"
```

### Task 0.2 — Create `nebula-credential-builtin` crate scaffold

**Files:**
- Create: `crates/credential-builtin/Cargo.toml`
- Create: `crates/credential-builtin/src/lib.rs`
- Create: `crates/credential-builtin/README.md`

- [ ] **Step 1: Write `crates/credential-builtin/Cargo.toml`**

```toml
[package]
name = "nebula-credential-builtin"
version.workspace = true
edition.workspace = true
keywords.workspace = true
authors.workspace = true
description = "Built-in concrete credential types for Nebula. Plugin authors depend on nebula-credential (contract); this crate ships first-party concrete types and the canonical mod sealed_caps."
license.workspace = true
repository.workspace = true

[dependencies]
nebula-credential = { path = "../credential" }
nebula-credential-macros = { path = "../credential/macros" }
nebula-core = { path = "../core" }
nebula-error = { workspace = true }
nebula-schema = { path = "../schema" }
serde = { workspace = true, features = ["derive"] }
zeroize = { version = "1.8.2", features = ["zeroize_derive"] }
secrecy = { workspace = true }
tokio = { workspace = true, features = ["sync", "macros", "rt"] }
tracing = { workspace = true }

[dev-dependencies]
trybuild = "1.0"
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }

[features]
default = []

[lints]
workspace = true
```

- [ ] **Step 2: Write `crates/credential-builtin/src/lib.rs`**

```rust
//! # nebula-credential-builtin
//!
//! Built-in concrete credential types and the canonical
//! [`sealed_caps`] module per ADR-0035. Plugin authors depend on
//! `nebula-credential` (the contract crate); first-party concrete
//! types live here.
//!
//! ## Canonical `mod sealed_caps`
//!
//! Per ADR-0035 §3 (amended 2026-04-24-B), every crate that declares
//! capability phantom traits in `dyn` positions must provide a
//! crate-private `sealed_caps` module with **per-capability** inner
//! sealed traits. This crate is the canonical home for the built-in
//! capabilities; plugin crates declare their own `mod sealed_caps`
//! at their own crate root for capabilities they introduce.
//!
//! See `README.md` for the plugin-author onboarding guide.
#![forbid(unsafe_code)]

extern crate self as nebula_credential_builtin;

/// Canonical inner sealed traits for built-in capabilities.
///
/// Crate-private. External crates cannot impl these — they declare
/// their own `mod sealed_caps` per ADR-0035 §3.
pub(crate) mod sealed_caps {
    pub trait BearerSealed {}
    pub trait BasicSealed {}
    pub trait SigningSealed {}
    pub trait TlsIdentitySealed {}
}

// Concrete credential types land here in П3 (per Tech Spec §16.1).
// П1 ships the empty scaffold so deny.toml + workspace member
// resolution settle ahead of the type-shape commits.
```

- [ ] **Step 3: Write `crates/credential-builtin/README.md`**

```markdown
# nebula-credential-builtin

Built-in concrete credential types for Nebula. Plugin authors depend on
`nebula-credential` (the contract crate); first-party concrete types
(`SlackOAuth2`, `BitbucketOAuth2`, `BitbucketPat`, `BitbucketAppPassword`,
`AnthropicApiKey`, `AwsSigV4`, ...) live here.

## Why split

Per Strategy §2.4 (frozen Checkpoint 1, commit `4316a292`):

> Plugin authors depend only on the contract crate (`nebula-credential`);
> built-in concrete types live in a separate crate so the trait-only
> dependency surface stays clean for third-party consumers and so
> built-in types can evolve (add credential types, bump dependencies,
> refactor concrete impls) without touching the contract crate's
> stability surface.

## Plugin-author onboarding

1. Depend on `nebula-credential` (contract). Do **not** depend on
   `nebula-credential-builtin`.
2. Declare your own `mod sealed_caps { pub trait MyCapSealed {} }` at
   crate root, one inner trait per capability you introduce. See
   ADR-0035 §3.
3. Use `#[plugin_credential(...)]` on the credential struct and
   `#[capability(scheme_bound = ..., sealed = ...)]` on each capability
   trait you introduce.
4. Register concrete types at plugin init via
   `registry.register::<MyCred>()?` — duplicate `KEY` is fatal.
5. The contract's `nebula-credential::sealed::Sealed` is emitted by
   `#[plugin_credential]`; do not impl by hand.

## What's here

П1 ships the empty scaffold. Concrete types land in П3. See the
phase plan at `docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md`.
```

- [ ] **Step 4: Wire workspace member**

Edit `Cargo.toml` at workspace root — add `"crates/credential-builtin"` to the `members` list (alphabetical; insert between `"crates/credential"` and `"crates/engine"`).

```toml
members = [
  "crates/action",
  "crates/storage",
  "crates/core",
  "crates/credential",
  "crates/credential-builtin",
  "crates/engine",
  ...
]
```

- [ ] **Step 5: Verify scaffold compiles**

Run: `cargo check -p nebula-credential-builtin`
Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/credential-builtin/ Cargo.toml
git commit -m "feat(credential-builtin): scaffold new crate per Strategy §2.4

Empty scaffold ships ahead of trait-shape commits so deny.toml +
workspace resolution settle without churn during П1.

Concrete types land in П3 per Tech Spec §16.1.

Refs: docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md §2.4
      docs/superpowers/specs/2026-04-24-credential-tech-spec.md §16.1
      docs/adr/0035-phantom-shim-capability-pattern.md §3"
```

### Task 0.3 — `deny.toml` allowlist + MATURITY row

**Files:**
- Modify: `deny.toml`
- Modify: `docs/MATURITY.md`

- [ ] **Step 1: Inspect `deny.toml` for the credential allowlist pattern**

Run: `grep -n 'nebula-credential' deny.toml`

Note the form used for `nebula-credential` (allowed source crates / prohibited targets). Mirror the same shape for `nebula-credential-builtin` directly below.

- [ ] **Step 2: Add `nebula-credential-builtin` row to `deny.toml`**

The crate may depend on `nebula-credential`, `nebula-credential-macros`, `nebula-core`, `nebula-error`, `nebula-schema`. It must NOT depend on `nebula-storage`, `nebula-engine`, `nebula-api`, `nebula-resource`, `nebula-action`, `nebula-runtime`, `nebula-sandbox`, `nebula-plugin`, or `nebula-sdk` (Business-tier ↛ Exec/API per layer ownership).

Insert the rule block in the appropriate section (alphabetical, mirroring `nebula-credential`).

- [ ] **Step 3: Add MATURITY row for `nebula-credential-builtin`**

Open `docs/MATURITY.md`, locate the `nebula-credential` row, insert directly below:

```markdown
| nebula-credential-builtin | preview | П1 scaffold; concrete types land in П3 | 2026-04-26 |
```

(Match the column conventions of the surrounding rows; values may need adjusting to the actual table schema.)

- [ ] **Step 4: Run `cargo deny check`**

Run: `cargo deny check`
Expected: PASS (no policy violations).

- [ ] **Step 5: Commit**

```bash
git add deny.toml docs/MATURITY.md
git commit -m "chore(credential-builtin): deny.toml + MATURITY row

Allowed deps: nebula-credential, -macros, -core, -error, -schema.
Prohibited: any Exec/API tier crate (deny.toml enforces).

Refs: docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Stage 0"
```

---

## Stage 1 — AuthScheme sensitivity dichotomy (§15.5)

Closes security-lead N2 + N10 + N4. `AuthScheme` reduced to base; `SensitiveScheme: AuthScheme + ZeroizeOnDrop` and `PublicScheme: AuthScheme` added. Derive macro audits fields. Existing schemes migrated.

### Task 1.1 — Reduce `AuthScheme` + add `SensitiveScheme` / `PublicScheme`

**Files:**
- Modify: `crates/core/src/auth/mod.rs` (or the actual canonical home — verify with `grep -rn 'pub trait AuthScheme'` before editing)
- Modify: `crates/credential/src/scheme/auth.rs` (re-export updates)
- Modify: `crates/credential/src/lib.rs` (top-level re-exports)

- [ ] **Step 1: Locate the canonical `AuthScheme` definition**

Run: `grep -rn 'pub trait AuthScheme' crates/`

Note the file. The `crates/credential/src/scheme/auth.rs` file currently re-exports from `nebula_core::auth`. The actual definition is in `nebula-core`.

- [ ] **Step 2: Reduce `AuthScheme` and add the two sub-traits**

In the canonical `auth` file, replace the `AuthScheme` definition:

```rust
use zeroize::ZeroizeOnDrop;

/// Base trait for runtime scheme output. Implementations are concrete
/// structs holding scheme material. Sensitivity is declared by the
/// implementing crate via the [`SensitiveScheme`] or [`PublicScheme`]
/// sub-trait.
///
/// `Clone` is NOT a supertrait — per Tech Spec §15.2, schemes opt in
/// to `Clone` only when copying plaintext is acceptable for the type.
/// Pattern: long-lived consumers receive [`SchemeGuard`] (per §15.7),
/// not raw clones.
pub trait AuthScheme: Send + Sync + 'static {}

/// Schemes that hold secret material. Mandates [`ZeroizeOnDrop`] so
/// plaintext drops from heap deterministically. Derived via
/// `#[auth_scheme(sensitive)]`; the macro audits fields at expansion
/// to forbid plain `String` for token-named slots.
pub trait SensitiveScheme: AuthScheme + ZeroizeOnDrop {}

/// Schemes that hold no secret material (provider/role/region
/// identifiers, public capability descriptors). Mutually exclusive
/// with [`SensitiveScheme`] — the derive macro forbids both.
pub trait PublicScheme: AuthScheme {}
```

- [ ] **Step 3: Update `crates/credential/src/scheme/auth.rs` re-exports**

```rust
//! Auth scheme trait and pattern classification.
//!
//! Canonical definitions live in [`nebula_core::auth`]. Re-exported here
//! for backward compatibility and discoverability.

pub use nebula_core::auth::{AuthPattern, AuthScheme, PublicScheme, SensitiveScheme};
```

- [ ] **Step 4: Update `crates/credential/src/lib.rs` re-exports**

Add to the flat re-export block:

```rust
pub use crate::scheme::{AuthPattern, AuthScheme, PublicScheme, SensitiveScheme};
```

- [ ] **Step 5: Compile-check (intentionally broken)**

Run: `cargo check -p nebula-credential 2>&1 | head -40`
Expected: FAIL — every scheme that previously impl'd `AuthScheme` still compiles, but no scheme yet impl's `SensitiveScheme` / `PublicScheme`. This is acceptable mid-stage; we fix per scheme in Tasks 1.3+.

Note: do NOT commit yet — Stage 1's commit unit is "scheme trait reduction + all schemes migrated", not a half-state.

### Task 1.2 — Probe 2: `compile_fail_scheme_sensitivity.rs`

**Files:**
- Create: `crates/credential/tests/compile_fail_scheme_sensitivity.rs`
- Create: `crates/credential/tests/probes/scheme_sensitivity_plain_string.rs`
- Create: `crates/credential/tests/probes/scheme_sensitivity_plain_string.stderr`
- Create: `crates/credential/tests/probes/scheme_sensitivity_public_with_secret.rs`
- Create: `crates/credential/tests/probes/scheme_sensitivity_public_with_secret.stderr`
- Create: `crates/credential/tests/probes/scheme_sensitivity_no_zeroize.rs`
- Create: `crates/credential/tests/probes/scheme_sensitivity_no_zeroize.stderr`

- [ ] **Step 1: Add `trybuild` to dev-deps**

Edit `crates/credential/Cargo.toml`, in `[dev-dependencies]` add:

```toml
trybuild = "1.0"
```

- [ ] **Step 2: Write the trybuild driver**

`crates/credential/tests/compile_fail_scheme_sensitivity.rs`:

```rust
//! Probe 2 — §15.5 AuthScheme sensitivity dichotomy.
//!
//! Verifies the trait shape rejects:
//! (a) `#[auth_scheme(sensitive)]` with plain `String` for token-named field
//! (b) `#[auth_scheme(public)]` with `SecretString` field
//! (c) `#[auth_scheme(sensitive)]` without `ZeroizeOnDrop` derive

#[test]
fn compile_fail_scheme_sensitivity() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/scheme_sensitivity_plain_string.rs");
    t.compile_fail("tests/probes/scheme_sensitivity_public_with_secret.rs");
    t.compile_fail("tests/probes/scheme_sensitivity_no_zeroize.rs");
}
```

- [ ] **Step 3: Write probe fixture (a) — plain String for token-named field**

`crates/credential/tests/probes/scheme_sensitivity_plain_string.rs`:

```rust
use nebula_credential::{AuthScheme, SensitiveScheme};
use nebula_credential_macros::AuthScheme;

#[derive(AuthScheme)]
#[auth_scheme(sensitive)]
struct BadScheme {
    pub token: String,  // plain String for sensitive-tagged field — REJECT
}

fn main() {}
```

- [ ] **Step 4: Generate `.stderr` snapshot**

Run: `TRYBUILD=overwrite cargo nextest run -p nebula-credential --test compile_fail_scheme_sensitivity --profile ci --no-tests=pass 2>&1 | tail -20`

Expected: trybuild emits the `.stderr` file with the expected diagnostic. Inspect the generated `.stderr` to confirm the message references `sensitive` + `token` + `String` field. The expected diagnostic (per macro implementation in Task 1.4) is something like:

```
error: field `token` on #[auth_scheme(sensitive)] struct must be SecretString or SecretBytes
```

Once the message is satisfactory, the snapshot is committed. Commit happens in Step 12 — **at the end of Task 1.2**.

- [ ] **Step 5: Write probe fixture (b) — public with SecretString**

`crates/credential/tests/probes/scheme_sensitivity_public_with_secret.rs`:

```rust
use nebula_credential::{AuthScheme, PublicScheme, SecretString};
use nebula_credential_macros::AuthScheme;

#[derive(AuthScheme)]
#[auth_scheme(public)]
struct BadPublicScheme {
    pub secret: SecretString,  // SecretString on public-tagged scheme — REJECT
}

fn main() {}
```

Expected `.stderr`: macro audit error citing `SecretString` field on `#[auth_scheme(public)]`.

- [ ] **Step 6: Write probe fixture (c) — sensitive without ZeroizeOnDrop**

`crates/credential/tests/probes/scheme_sensitivity_no_zeroize.rs`:

```rust
use nebula_credential::{AuthScheme, SecretString, SensitiveScheme};

// Manual impl that skips ZeroizeOnDrop
struct ManualScheme {
    pub token: SecretString,
}

impl AuthScheme for ManualScheme {}
impl SensitiveScheme for ManualScheme {}  // E0277 — ZeroizeOnDrop not satisfied

fn main() {}
```

Expected `.stderr`: `error[E0277]: the trait bound 'ManualScheme: ZeroizeOnDrop' is not satisfied`.

- [ ] **Step 7: Generate `.stderr` snapshots for (b) and (c)**

Run: `TRYBUILD=overwrite cargo nextest run -p nebula-credential --test compile_fail_scheme_sensitivity --profile ci --no-tests=pass`

Inspect each generated `.stderr` for content match.

- [ ] **Step 8: Verify probe passes (without overwrite)**

Run: `cargo nextest run -p nebula-credential --test compile_fail_scheme_sensitivity --profile ci --no-tests=pass`
Expected: PASS — all 3 cases compile-fail with snapshotted diagnostics.

- [ ] **Step 9: Commit**

(Probe + Task 1.4 derive macro changes are committed together in Task 1.4 because the macro emits the diagnostics the snapshots match. **Hold this commit** until Task 1.4 lands.)

### Task 1.3 — Migrate existing schemes to `SensitiveScheme` / `PublicScheme`

For each file under `crates/credential/src/scheme/*.rs`, add the appropriate sub-trait impl + ensure `ZeroizeOnDrop`. Each file is a separate small commit.

**Files:**
- Modify: `crates/credential/src/scheme/secret_token.rs` → `SensitiveScheme`
- Modify: `crates/credential/src/scheme/identity_password.rs` → `SensitiveScheme`
- Modify: `crates/credential/src/scheme/key_pair.rs` → `SensitiveScheme`
- Modify: `crates/credential/src/scheme/signing_key.rs` → `SensitiveScheme`
- Modify: `crates/credential/src/scheme/shared_key.rs` → `SensitiveScheme`
- Modify: `crates/credential/src/scheme/instance_binding.rs` → `PublicScheme`
- Modify: `crates/credential/src/scheme/connection_uri.rs` → `SensitiveScheme` + restructure (Task 1.5)
- Modify: `crates/credential/src/scheme/oauth2.rs` → `SensitiveScheme` + `bearer_header()` returns `SecretString`
- Modify: `crates/credential/src/scheme/certificate.rs` → `SensitiveScheme`
- Modify: `crates/credential/src/scheme/coercion.rs` → audit — `PublicScheme` if no embedded secret material; `SensitiveScheme` otherwise

For each scheme, the same step pattern applies:

- [ ] **Step 1: Read current file, identify sensitive vs public classification**

Use the audit table at Tech Spec §15.5 implementation impact (lines 3290-3293) as the authoritative classification. Verify field types match.

- [ ] **Step 2: Add `ZeroizeOnDrop` derive (sensitive only)**

For sensitive schemes, ensure the struct has `#[derive(Zeroize, ZeroizeOnDrop)]` with `#[zeroize(...)]` field attributes for any non-`Drop`-zeroizing fields (e.g., `String` username — `#[zeroize(skip)]` if non-secret, otherwise wrap in `SecretString`).

- [ ] **Step 3: Add the sub-trait impl**

For sensitive: `impl SensitiveScheme for T {}`. For public: `impl PublicScheme for T {}`. Mutually exclusive.

- [ ] **Step 4: Compile-check the single scheme file's crate**

Run: `cargo check -p nebula-credential`
Expected: PASS for that scheme.

- [ ] **Step 5: Commit one-liner per scheme**

Example for `BearerScheme`:

```bash
git add crates/credential/src/scheme/secret_token.rs
git commit -m "refactor(credential-scheme): SecretToken impls SensitiveScheme

Per Tech Spec §15.5 dichotomy. SecretToken holds SecretString
token; ZeroizeOnDrop already derived via SecretString.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.5"
```

Apply this pattern to **each scheme listed above**. The `coercion.rs` audit is the only one that requires a judgment call (read fields, decide). All others are mechanical from the §15.5 table.

### Task 1.4 — Update `#[derive(AuthScheme)]` macro for sensitivity audit

**Files:**
- Modify: `crates/credential/macros/src/auth_scheme.rs`
- Modify: `crates/credential/macros/src/lib.rs`

- [ ] **Step 1: Add `sensitive` / `public` argument parsing**

In `auth_scheme.rs`, the macro must accept `#[auth_scheme(sensitive)]` and `#[auth_scheme(public)]` and forbid both. Parse the attribute via `syn::parse::Parse`; cache the choice on a local `Sensitivity` enum.

- [ ] **Step 2: Implement field audit for `sensitive`**

For `Sensitivity::Sensitive`, walk the struct fields:
- For each field, the type must be `SecretString` / `SecretBytes` / a type that already impls `SensitiveScheme` (nested). Any plain `String` / `Vec<u8>` is rejected with a span-attached error.
- Field-name lint: if the field name matches `/^(token|secret|key|password|bearer)$/i` and the type is plain `String`, reject even if the user used `#[zeroize(skip)]` — the name itself implies sensitivity.
- The macro emits `impl AuthScheme for T {}` and `impl SensitiveScheme for T {}` and verifies the struct also derives `Zeroize` + `ZeroizeOnDrop` (or emits an `impl ZeroizeOnDrop` if `#[derive]` is present).

- [ ] **Step 3: Implement field audit for `public`**

For `Sensitivity::Public`, walk fields:
- Reject `SecretString` / `SecretBytes` / nested `SensitiveScheme` types.
- Emit `impl AuthScheme for T {}` + `impl PublicScheme for T {}`. No `ZeroizeOnDrop` requirement.

- [ ] **Step 4: Diagnostic message convention**

Use `syn::Error::new_spanned(field, "...")` with messages of the form:

- Sensitive + plain String: `field 'token' on #[auth_scheme(sensitive)] struct must be SecretString or SecretBytes`
- Public + SecretString: `field 'secret' on #[auth_scheme(public)] struct cannot be SecretString — declare #[auth_scheme(sensitive)] instead`
- Sensitive + missing ZeroizeOnDrop: rely on the trait-bound `E0277` from `SensitiveScheme: AuthScheme + ZeroizeOnDrop` (no extra macro emission needed).

- [ ] **Step 5: Run probe 2 to verify diagnostics**

Run: `cargo nextest run -p nebula-credential --test compile_fail_scheme_sensitivity --profile ci --no-tests=pass`
Expected: PASS — all 3 cases compile-fail with the macro-emitted diagnostics matching the snapshotted `.stderr`.

- [ ] **Step 6: Commit (combines macro + probe)**

```bash
git add crates/credential/macros/src/auth_scheme.rs \
        crates/credential/macros/src/lib.rs \
        crates/credential/tests/compile_fail_scheme_sensitivity.rs \
        crates/credential/tests/probes/scheme_sensitivity_*.rs \
        crates/credential/tests/probes/scheme_sensitivity_*.stderr
git commit -m "feat(credential-macros): sensitive/public sensitivity audit

§15.5 dichotomy enforcement at macro level. #[auth_scheme(sensitive)]
audits fields for SecretString/SecretBytes/nested-sensitive; rejects
plain String for token-named slots. #[auth_scheme(public)] audits for
absence of secret material. Probe 2 (compile-fail) covers all three
rejection paths.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.5
      docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Task 1.4"
```

### Task 1.5 — `ConnectionUri` restructure (§15.5 §3295)

**Files:**
- Modify: `crates/credential/src/scheme/connection_uri.rs`

- [ ] **Step 1: Replace the struct with the structured form**

```rust
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::AuthScheme;
use crate::scheme::SensitiveScheme;

/// Database / message-broker connection URI, structured.
///
/// Per Tech Spec §15.5 (closes N4): individual fields exposed via
/// non-secret accessors where they ARE non-secret (host, port,
/// database, username); password remains `SecretString`. The full
/// URL reconstruction returns `SecretString` so logging or
/// serialization paths cannot leak the password component.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct ConnectionUri {
    #[zeroize(skip)]
    scheme: String,
    #[zeroize(skip)]
    host: String,
    #[zeroize(skip)]
    port: Option<u16>,
    #[zeroize(skip)]
    database: String,
    #[zeroize(skip)]
    username: String,
    password: SecretString,
}

impl ConnectionUri {
    pub fn new(
        scheme: String,
        host: String,
        port: Option<u16>,
        database: String,
        username: String,
        password: SecretString,
    ) -> Self {
        Self { scheme, host, port, database, username, password }
    }

    pub fn scheme(&self) -> &str { &self.scheme }
    pub fn host(&self) -> &str { &self.host }
    pub fn port(&self) -> Option<u16> { self.port }
    pub fn database(&self) -> &str { &self.database }
    pub fn username(&self) -> &str { &self.username }
    pub fn password(&self) -> &SecretString { &self.password }

    /// Reconstruct the full URL inside `SecretString`. Driver
    /// injection sites call `.expose_secret()` on the result exactly
    /// once, at the FFI boundary.
    pub fn as_url(&self) -> SecretString {
        let port_part = self.port.map(|p| format!(":{p}")).unwrap_or_default();
        let url = format!(
            "{}://{}:{}@{}{}/{}",
            self.scheme,
            self.username,
            self.password.expose_secret(),
            self.host,
            port_part,
            self.database,
        );
        SecretString::new(url.into())
    }
}

impl AuthScheme for ConnectionUri {}
impl SensitiveScheme for ConnectionUri {}
```

- [ ] **Step 2: Update consumers (driver injection sites)**

Run: `grep -rn 'ConnectionUri' crates/`

For every consumer that previously read `.url` or similar field directly, switch to:
- `as_url().expose_secret()` for full URL (single FFI-boundary call)
- per-field accessors for non-secret components

(In П1 scope, only the trait-shape touchpoints are migrated. Real driver consumers land in П3 via `nebula-credential-builtin`.)

- [ ] **Step 3: Commit**

```bash
git add crates/credential/src/scheme/connection_uri.rs
git commit -m "refactor(credential-scheme): ConnectionUri structured accessors

Per Tech Spec §15.5 (closes security-lead N4). Individual fields
(host, port, database, username) accessed as &str / Option<u16> —
safe to log. Password stays SecretString. as_url() returns
SecretString; driver injection sites .expose_secret() at FFI boundary
exactly once.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.5"
```

### Task 1.6 — `OAuth2Token::bearer_header` returns `SecretString`

**Files:**
- Modify: `crates/credential/src/scheme/oauth2.rs`

- [ ] **Step 1: Update the accessor signature**

Find `OAuth2Token` (or whatever the OAuth2-bearer scheme is named — `grep -n 'bearer_header' crates/credential/src/`). Change:

```rust
// Before:
pub fn bearer_header(&self) -> String {
    format!("Bearer {}", self.access_token.expose_secret())
}

// After:
pub fn bearer_header(&self) -> SecretString {
    SecretString::new(format!("Bearer {}", self.access_token.expose_secret()).into())
}
```

- [ ] **Step 2: Update consumers**

Run: `grep -rn 'bearer_header()' crates/`

Each consumer must `.expose_secret()` at the FFI boundary (e.g., `header("Authorization", bearer.expose_secret())`).

- [ ] **Step 3: Run scheme-affected tests**

Run: `cargo nextest run -p nebula-credential --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/credential/src/scheme/oauth2.rs $(grep -rl 'bearer_header()' crates/ --include='*.rs')
git commit -m "refactor(credential-scheme): OAuth2Token::bearer_header returns SecretString

Per Tech Spec §15.5 (closes security-lead N4). Bearer header
contains the access token verbatim; returning SecretString forces
.expose_secret() at the FFI boundary, eliminating accidental log /
Debug leaks of the bearer string.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.5"
```

### Task 1.7 — Stage 1 gate

- [ ] **Step 1: Run full credential test suite**

Run: `cargo nextest run -p nebula-credential --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p nebula-credential -- -D warnings`
Expected: PASS.

- [ ] **Step 3: Stage 1 verification commit**

```bash
git commit --allow-empty -m "chore(credential): Stage 1 gate passed (AuthScheme dichotomy + scheme migration)"
```

---

## Stage 2 — `CredentialState: ZeroizeOnDrop` supertrait bound

Closes security-lead N1 + §15.4 amendment.

### Task 2.1 — Add `ZeroizeOnDrop` to `CredentialState`

**Files:**
- Modify: `crates/credential/src/contract/state.rs`

- [ ] **Step 1: Update the trait definition**

```rust
//! Credential state trait for stored credential data.

use serde::{Serialize, de::DeserializeOwned};
use zeroize::ZeroizeOnDrop;

/// Trait for credential state types stored in encrypted storage (v2).
///
/// `ZeroizeOnDrop` is mandatory — credential state contains decrypted
/// secret material at runtime; deterministic plaintext drop is a
/// §12.5 invariant (§15.4 amendment, Tech Spec).
pub trait CredentialState: Serialize + DeserializeOwned + Send + Sync + ZeroizeOnDrop + 'static {
    const KIND: &'static str;
    const VERSION: u32;

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        None
    }
}
```

- [ ] **Step 2: Compile-check (intentionally broken)**

Run: `cargo check -p nebula-credential 2>&1 | head -20`
Expected: FAIL — every existing `impl CredentialState for X` that does not derive `ZeroizeOnDrop` fails. We fix in Task 2.3.

### Task 2.2 — Probe 1: `compile_fail_state_zeroize.rs`

**Files:**
- Create: `crates/credential/tests/compile_fail_state_zeroize.rs`
- Create: `crates/credential/tests/probes/state_zeroize_missing.rs`
- Create: `crates/credential/tests/probes/state_zeroize_missing.stderr`

- [ ] **Step 1: Write the trybuild driver**

```rust
//! Probe 1 — §15.4 CredentialState requires ZeroizeOnDrop.

#[test]
fn compile_fail_state_zeroize() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/state_zeroize_missing.rs");
}
```

- [ ] **Step 2: Write the probe fixture**

`crates/credential/tests/probes/state_zeroize_missing.rs`:

```rust
use nebula_credential::CredentialState;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct BadState {
    pub token: String,  // no ZeroizeOnDrop
}

impl CredentialState for BadState {
    const KIND: &'static str = "bad_state";
    const VERSION: u32 = 1;
}

fn main() {}
```

Expected `.stderr`: `error[E0277]: the trait bound 'BadState: ZeroizeOnDrop' is not satisfied`.

- [ ] **Step 3: Generate `.stderr` snapshot**

Run: `TRYBUILD=overwrite cargo nextest run -p nebula-credential --test compile_fail_state_zeroize --profile ci --no-tests=pass`

Verify the generated `.stderr` cites `ZeroizeOnDrop`. Commit happens at the end of Task 2.3.

### Task 2.3 — Migrate existing `CredentialState` impls

**Files:**
- Modify: every file with `impl CredentialState for ...` (locate via grep)

- [ ] **Step 1: Locate all impls**

Run: `grep -rn 'impl CredentialState for' crates/credential/src/`

Note the list. Each impl's struct must derive `Zeroize` + `ZeroizeOnDrop`, with `#[zeroize(skip)]` for non-secret fields where appropriate.

- [ ] **Step 2: Migrate each impl**

For each struct, apply the pattern:

```rust
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct OAuth2State {
    #[zeroize(skip)]
    pub provider_id: String,
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    #[zeroize(skip)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl CredentialState for OAuth2State { /* ... */ }
```

`SecretString` already drops zeroized; `String` fields not bearing secrets get `#[zeroize(skip)]`.

- [ ] **Step 3: Verify compile**

Run: `cargo check -p nebula-credential`
Expected: PASS.

- [ ] **Step 4: Run probe 1**

Run: `cargo nextest run -p nebula-credential --test compile_fail_state_zeroize --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 5: Run full credential suite**

Run: `cargo nextest run -p nebula-credential --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/credential/src/contract/state.rs \
        crates/credential/tests/compile_fail_state_zeroize.rs \
        crates/credential/tests/probes/state_zeroize_missing.* \
        $(grep -rln 'impl CredentialState for' crates/credential/src/)
git commit -m "feat(credential): CredentialState requires ZeroizeOnDrop

Per Tech Spec §15.4 amendment + security-lead N1. CredentialState
gains ZeroizeOnDrop supertrait bound. All existing state impls
migrated to derive Zeroize + ZeroizeOnDrop with #[zeroize(skip)] on
non-secret fields. Probe 1 covers the failure path.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.4
      docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Stage 2"
```

---

## Stage 3 — Capability sub-trait split (§15.4)

The biggest piece. Strips 5 const bools + 5 default method bodies from `Credential`. Adds 5 sub-traits. Migrates built-in credentials. Updates engine dispatchers. Probes 3 + 4.

### Task 3.1 — Define `Interactive` sub-trait + move `Pending`

**Files:**
- Create: `crates/credential/src/contract/interactive.rs`
- Modify: `crates/credential/src/contract/mod.rs`

- [ ] **Step 1: Write `interactive.rs`**

```rust
//! `Interactive` sub-trait — credentials with multi-step resolve flows.
//!
//! Per Tech Spec §15.4 capability sub-trait split. The `Pending`
//! associated type lives here, not on the base `Credential` trait —
//! non-interactive credentials need no Pending companion type.

use std::future::Future;

use crate::Credential;
use crate::CredentialContext;
use crate::error::CredentialError;
use crate::resolve::{ResolveResult, UserInput};
use crate::contract::PendingState;

/// Credentials that require multi-step interactive resolution
/// (OAuth2 authorize→callback, device code flow, multi-step chain).
///
/// Static credentials (API keys, basic auth) do **not** impl this
/// trait. The base `Credential::resolve` returns
/// `ResolveResult<Self::State, ()>` — interactive variants go through
/// `Interactive::continue_resolve` with a typed `Pending` companion.
pub trait Interactive: Credential {
    /// Typed pending state for interactive flows.
    type Pending: PendingState + Send + Sync + ZeroizeOnDrop + 'static;

    /// Continue interactive resolve after user completes interaction.
    /// Framework loads + consumes `PendingState` before calling.
    fn continue_resolve(
        pending: &Self::Pending,
        input: &UserInput,
        ctx: &CredentialContext<'_>,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, Self::Pending>, CredentialError>>
       + Send
    where
        Self: Sized;
}

use zeroize::ZeroizeOnDrop;
```

- [ ] **Step 2: Wire submodule**

In `crates/credential/src/contract/mod.rs`, add:

```rust
pub mod interactive;
pub use interactive::Interactive;
```

- [ ] **Step 3: Compile-check (the base trait still has Pending; intentionally double-defined for now — fixed in Task 3.6)**

Run: `cargo check -p nebula-credential 2>&1 | head -20`
Expected: WARN or FAIL depending on if rustc complains about duplicate `type Pending`. Note any errors; resolve in Task 3.6.

### Task 3.2 — Define `Refreshable` sub-trait

**Files:**
- Create: `crates/credential/src/contract/refreshable.rs`
- Modify: `crates/credential/src/contract/mod.rs`

- [ ] **Step 1: Write `refreshable.rs`**

```rust
//! `Refreshable` sub-trait — credentials with refreshable State.
//!
//! Per Tech Spec §15.4. Engine `RefreshDispatcher::for_credential<C>`
//! binds `where C: Refreshable`. A non-`Refreshable` credential
//! cannot be passed — `E0277` at the dispatch site.

use std::future::Future;

use crate::Credential;
use crate::CredentialContext;
use crate::error::CredentialError;
use crate::resolve::{RefreshOutcome, RefreshPolicy};

pub trait Refreshable: Credential {
    /// Refresh timing policy — controls early refresh, retry backoff,
    /// jitter. Default `RefreshPolicy::DEFAULT` per §2.1.
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    fn refresh(
        state: &mut Self::State,
        ctx: &CredentialContext<'_>,
    ) -> impl Future<Output = Result<RefreshOutcome, CredentialError>> + Send
    where
        Self: Sized;
}
```

- [ ] **Step 2: Wire submodule**

```rust
pub mod refreshable;
pub use refreshable::Refreshable;
```

### Task 3.3 — Define `Revocable` sub-trait

**Files:**
- Create: `crates/credential/src/contract/revocable.rs`
- Modify: `crates/credential/src/contract/mod.rs`

- [ ] **Step 1: Write `revocable.rs`**

```rust
//! `Revocable` sub-trait — credentials with provider-side revocation.

use std::future::Future;

use crate::Credential;
use crate::CredentialContext;
use crate::error::CredentialError;

pub trait Revocable: Credential {
    fn revoke(
        state: &mut Self::State,
        ctx: &CredentialContext<'_>,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send
    where
        Self: Sized;
}
```

- [ ] **Step 2: Wire submodule**

```rust
pub mod revocable;
pub use revocable::Revocable;
```

### Task 3.4 — Define `Testable` sub-trait

**Files:**
- Create: `crates/credential/src/contract/testable.rs`
- Modify: `crates/credential/src/contract/mod.rs`

- [ ] **Step 1: Write `testable.rs`**

```rust
//! `Testable` sub-trait — credentials with health probe.

use std::future::Future;

use crate::Credential;
use crate::CredentialContext;
use crate::error::CredentialError;
use crate::resolve::TestResult;

pub trait Testable: Credential {
    fn test(
        scheme: &Self::Scheme,
        ctx: &CredentialContext<'_>,
    ) -> impl Future<Output = Result<TestResult, CredentialError>> + Send
    where
        Self: Sized;
}
```

Note: signature changed from current `Result<Option<TestResult>, _>` to `Result<TestResult, _>` — the `Option` was the soft "not testable" carve-out which the sub-trait split eliminates. A credential is `Testable` xor not.

- [ ] **Step 2: Wire submodule**

```rust
pub mod testable;
pub use testable::Testable;
```

### Task 3.5 — Define `Dynamic` sub-trait

**Files:**
- Create: `crates/credential/src/contract/dynamic.rs`
- Modify: `crates/credential/src/contract/mod.rs`

- [ ] **Step 1: Write `dynamic.rs`**

```rust
//! `Dynamic` sub-trait — ephemeral per-execution credentials.
//!
//! Per Tech Spec §15.4. CP6 corrected the production `release(&self,
//! ...)` vestigial `&self` receiver — `Self` is a ZST type-marker, the
//! receiver gave no access. Signature aligned with sister sub-trait
//! signatures (state + ctx, no `&self`).

use std::future::Future;
use std::time::Duration;

use crate::Credential;
use crate::CredentialContext;
use crate::error::CredentialError;

pub trait Dynamic: Credential {
    /// Lease duration. `None` means release happens only at execution
    /// end.
    const LEASE_TTL: Option<Duration> = None;

    fn release(
        state: &Self::State,
        ctx: &CredentialContext<'_>,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send
    where
        Self: Sized;
}
```

- [ ] **Step 2: Wire submodule**

```rust
pub mod dynamic;
pub use dynamic::Dynamic;
```

### Task 3.6 — Strip 5 const bools + default method bodies from base `Credential`

**Files:**
- Modify: `crates/credential/src/contract/credential.rs`

- [ ] **Step 1: Replace the base trait with the reduced shape**

Per Tech Spec §15.4 (and CP5/CP6 canonical form):

```rust
//! Unified credential trait (CP5/CP6).

use std::future::Future;

use nebula_schema::{HasSchema, ValidSchema};

use super::{CredentialState, sealed};
use crate::{
    AuthScheme, CredentialContext, CredentialMetadata,
    error::CredentialError,
    resolve::ResolveResult,
};

pub trait Credential: sealed::Sealed + Send + Sync + 'static {
    type Input: HasSchema + Send + Sync + 'static;
    type Scheme: AuthScheme;
    type State: CredentialState + Send + Sync + 'static;

    /// Stable key for this credential type (e.g., `"github_oauth2"`).
    const KEY: &'static str;

    /// Project runtime Scheme from stored State. Synchronous, pure.
    /// `where Self: Sized` excludes from object-safe vtable —
    /// dispatch goes through downcast at full type knowledge.
    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized;

    /// Build initial State from user Input. Returns
    /// `ResolveResult<State, ()>` for static credentials; interactive
    /// credentials override via the `Interactive` sub-trait.
    fn resolve(
        values: &nebula_schema::FieldValues,
        ctx: &CredentialContext<'_>,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, ()>, CredentialError>> + Send
    where
        Self: Sized;
}
```

Notes:

- `Pending` assoc type **removed** — moved to `Interactive`.
- `INTERACTIVE` / `REFRESHABLE` / `REVOCABLE` / `TESTABLE` / `DYNAMIC` const bools **removed**.
- `REFRESH_POLICY` const **removed** — moved to `Refreshable::REFRESH_POLICY`.
- `LEASE_TTL` const **removed** — moved to `Dynamic::LEASE_TTL`.
- Defaulted method bodies for `continue_resolve` / `test` / `refresh` / `revoke` / `release` **removed** — those methods now live on the corresponding sub-traits with no defaults.
- `metadata()` and `schema()` methods **removed** from the base — `metadata()` lives on `CredentialMetadataSource` (per §2.8); `schema()` is computed via `<Self::Input as HasSchema>::schema()` at the registration site, not as a trait method.

- [ ] **Step 2: Compile-check (broken — every concrete credential needs migration)**

Run: `cargo check -p nebula-credential 2>&1 | head -40`
Expected: FAIL — built-in credentials in `crates/credential/src/credentials/{api_key, basic_auth, oauth2}.rs` reference removed members. Fix in Task 3.8.

### Task 3.7 — Probe 3: `compile_fail_capability_subtrait.rs`

**Files:**
- Create: `crates/credential/tests/compile_fail_capability_subtrait.rs`
- Create: `crates/credential/tests/probes/capability_subtrait_refreshable_no_method.rs`
- Create: `crates/credential/tests/probes/capability_subtrait_refreshable_no_method.stderr`
- Create: `crates/credential/tests/probes/capability_subtrait_revocable_no_method.rs`
- Create: `crates/credential/tests/probes/capability_subtrait_revocable_no_method.stderr`
- Create: `crates/credential/tests/probes/capability_subtrait_testable_no_method.rs`
- Create: `crates/credential/tests/probes/capability_subtrait_testable_no_method.stderr`
- Create: `crates/credential/tests/probes/capability_subtrait_dynamic_no_method.rs`
- Create: `crates/credential/tests/probes/capability_subtrait_dynamic_no_method.stderr`

- [ ] **Step 1: Write the trybuild driver**

```rust
//! Probe 3 — §15.4 sub-traits require method bodies (no silent default).

#[test]
fn compile_fail_capability_subtrait() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/capability_subtrait_refreshable_no_method.rs");
    t.compile_fail("tests/probes/capability_subtrait_revocable_no_method.rs");
    t.compile_fail("tests/probes/capability_subtrait_testable_no_method.rs");
    t.compile_fail("tests/probes/capability_subtrait_dynamic_no_method.rs");
}
```

- [ ] **Step 2: Write fixture for `Refreshable` missing method**

`tests/probes/capability_subtrait_refreshable_no_method.rs`:

```rust
use nebula_credential::{Credential, Refreshable};

struct DummyState;
struct DummyScheme;
struct Dummy;

// (Credential impl elided — assume valid.)

impl Refreshable for Dummy {}  // E0046 — refresh missing

fn main() {}
```

Expected `.stderr`: `error[E0046]: not all trait items implemented, missing: 'refresh'`.

- [ ] **Step 3: Repeat for Revocable / Testable / Dynamic**

Mechanical repetition with the corresponding trait + missing method name (`revoke`, `test`, `release`).

- [ ] **Step 4: Generate `.stderr` snapshots**

Run: `TRYBUILD=overwrite cargo nextest run -p nebula-credential --test compile_fail_capability_subtrait --profile ci --no-tests=pass`

Inspect `.stderr` files; confirm `E0046` + correct method name in each.

- [ ] **Step 5: Verify probe passes**

Run: `cargo nextest run -p nebula-credential --test compile_fail_capability_subtrait --profile ci --no-tests=pass`
Expected: PASS — all 4 cases compile-fail.

(Commit at end of Stage 3.)

### Task 3.8 — Migrate built-in credentials to sub-traits

**Files:**
- Modify: `crates/credential/src/credentials/api_key.rs`
- Modify: `crates/credential/src/credentials/basic_auth.rs`
- Modify: `crates/credential/src/credentials/oauth2.rs`

- [ ] **Step 1: `api_key.rs`**

API keys are static — non-interactive, non-refreshable, non-revocable, possibly testable.

```rust
// Drop: const INTERACTIVE/REFRESHABLE/REVOCABLE/TESTABLE/DYNAMIC = ...
// Drop: type Pending
// Drop: defaulted continue_resolve / refresh / revoke / test / release method bodies

impl Credential for ApiKeyCredential {
    type Input = ApiKeyInput;
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "api_key";

    fn project(state: &Self::State) -> Self::Scheme {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext<'_>,
    ) -> Result<ResolveResult<Self::State, ()>, CredentialError> {
        let token = values.get_string("api_key").unwrap_or_default();
        Ok(ResolveResult::Complete(SecretToken::new(SecretString::new(token.into()))))
    }
}

// If api_key is testable (per its current production shape), add:
// impl Testable for ApiKeyCredential { ... }
```

- [ ] **Step 2: `basic_auth.rs`**

Same pattern — `impl Credential` only, no sub-trait impls (basic auth is static + non-testable).

- [ ] **Step 3: `oauth2.rs`**

OAuth2 is interactive, refreshable, revocable, testable. Apply all four sub-trait impls:

```rust
impl Credential for OAuth2Credential { /* base resolve = initial step kicks off interaction */ }
impl Interactive for OAuth2Credential {
    type Pending = OAuth2Pending;
    async fn continue_resolve(...) -> Result<ResolveResult<Self::State, Self::Pending>, _> {
        // implementation per current oauth2.rs
    }
}
impl Refreshable for OAuth2Credential {
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;
    async fn refresh(state: &mut Self::State, ctx: &CredentialContext<'_>) -> Result<RefreshOutcome, CredentialError> {
        // implementation per current oauth2.rs
    }
}
impl Revocable for OAuth2Credential {
    async fn revoke(state: &mut Self::State, ctx: &CredentialContext<'_>) -> Result<(), CredentialError> {
        // implementation per current oauth2.rs
    }
}
impl Testable for OAuth2Credential {
    async fn test(scheme: &Self::Scheme, ctx: &CredentialContext<'_>) -> Result<TestResult, CredentialError> {
        // implementation per current oauth2.rs (drop the Option wrapper)
    }
}
```

- [ ] **Step 4: Compile-check**

Run: `cargo check -p nebula-credential`
Expected: PASS.

### Task 3.9 — Update engine dispatchers — `where C: Refreshable` etc.

**Files:**
- Modify: `crates/engine/src/credential/rotation/mod.rs` and `scheduler.rs` and `token_refresh.rs`
- Modify: any other engine site that calls `C::refresh` / `C::revoke` / `C::test` / `C::release`

- [ ] **Step 1: Locate dispatch entry points**

Run: `grep -rn 'C::refresh\|C::revoke\|C::test\|C::release\|<C as Credential>::refresh' crates/engine/`

For each entry point, replace the `where C: Credential` bound with `where C: Refreshable` (or the appropriate sub-trait).

- [ ] **Step 2: Pattern — refresh dispatcher**

```rust
pub(crate) fn for_credential<C: Refreshable>() -> Self {
    Self { /* ... */ }
}

pub(crate) async fn dispatch_refresh<C: Refreshable>(
    state: &mut C::State,
    ctx: &CredentialContext<'_>,
) -> Result<RefreshOutcome, CredentialError> {
    C::refresh(state, ctx).await
}
```

- [ ] **Step 3: Repeat for Revocable / Testable / Dynamic**

Same shape — bind `C: Revocable` for revocation, `C: Testable` for test, `C: Dynamic` for release.

- [ ] **Step 4: Engine-level capability gating**

Where the engine previously read `C::REFRESHABLE` (a const bool) and branched, the branch is now structurally absent — non-`Refreshable` credentials are not passed to refresh paths. The engine routes via the registry's computed capability set (Stage 7) and the dispatcher signature (`where C: Refreshable`).

Search: `grep -rn 'REFRESHABLE\|REVOCABLE\|TESTABLE\|DYNAMIC' crates/engine/`
Each call site must be rewritten — usually one of:
- Drop the branch entirely (the dispatcher bound enforces).
- Replace with capability-set check from registry (`registry.capabilities_of(key).contains(Capability::Refreshable)`) for runtime decisions.

- [ ] **Step 5: Compile-check**

Run: `cargo check -p nebula-engine`
Expected: PASS.

### Task 3.10 — Probe 4: `compile_fail_engine_dispatch_capability.rs`

**Files:**
- Create: `crates/credential/tests/compile_fail_engine_dispatch_capability.rs`
- Create: `crates/credential/tests/probes/engine_dispatch_non_refreshable.rs`
- Create: `crates/credential/tests/probes/engine_dispatch_non_refreshable.stderr`

(Note: the probe exercises engine dispatch; the test is hosted in `nebula-credential` because it imports the engine path. Alternatively, the probe lives in `nebula-engine/tests/`. Per Tech Spec §16.1.1 the probe lives at `crates/credential/tests/`. Use `nebula-engine` as a `dev-dependency` of `nebula-credential` ONLY for tests, **OR** mirror the dispatcher type into a probe-local stub. Use the stub approach — keeps `nebula-credential` dev-deps minimal.)

- [ ] **Step 1: Write trybuild driver**

```rust
#[test]
fn compile_fail_engine_dispatch_capability() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/engine_dispatch_non_refreshable.rs");
}
```

- [ ] **Step 2: Write fixture (stub dispatcher to keep cross-crate import out)**

`tests/probes/engine_dispatch_non_refreshable.rs`:

```rust
use nebula_credential::{Credential, Refreshable};

// Stand-in for engine's RefreshDispatcher::for_credential.
struct RefreshDispatcher;
impl RefreshDispatcher {
    fn for_credential<C: Refreshable>() -> Self { Self }
}

// Static, non-refreshable credential.
struct ApiKeyCred;
// Credential impl elided (assume valid).

fn main() {
    let _ = RefreshDispatcher::for_credential::<ApiKeyCred>();
    // E0277 — `ApiKeyCred: Refreshable` not satisfied.
}
```

Expected `.stderr`: `error[E0277]: the trait bound 'ApiKeyCred: Refreshable' is not satisfied`.

- [ ] **Step 3: Generate `.stderr` snapshot**

Run: `TRYBUILD=overwrite cargo nextest run -p nebula-credential --test compile_fail_engine_dispatch_capability --profile ci --no-tests=pass`

Inspect; confirm message.

- [ ] **Step 4: Verify probe passes**

Run: `cargo nextest run -p nebula-credential --test compile_fail_engine_dispatch_capability --profile ci --no-tests=pass`
Expected: PASS.

### Task 3.11 — Stage 3 commit + gate

- [ ] **Step 1: Run full credential + engine tests**

Run: `cargo nextest run -p nebula-credential -p nebula-engine --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p nebula-credential -p nebula-engine -- -D warnings`
Expected: PASS.

- [ ] **Step 3: Commit Stage 3 in one batch**

```bash
git add crates/credential/src/contract/{interactive,refreshable,revocable,testable,dynamic,credential,mod}.rs \
        crates/credential/src/credentials/ \
        crates/credential/src/lib.rs \
        crates/credential/tests/compile_fail_capability_subtrait.rs \
        crates/credential/tests/compile_fail_engine_dispatch_capability.rs \
        crates/credential/tests/probes/capability_subtrait_*.{rs,stderr} \
        crates/credential/tests/probes/engine_dispatch_*.{rs,stderr} \
        crates/engine/src/credential/
git commit -m "feat(credential)!: capability sub-trait split (§15.4)

BREAKING. 5 const bools + 5 default method bodies removed from base
Credential trait. New sub-traits: Interactive, Refreshable, Revocable,
Testable, Dynamic. Pending assoc type moves to Interactive.
REFRESH_POLICY moves to Refreshable. LEASE_TTL moves to Dynamic.

Engine dispatchers bind where C: Refreshable etc. — silent-downgrade
vector (REFRESHABLE = true + refresh() defaults to NotSupported)
structurally impossible.

Built-in credentials migrated:
- ApiKeyCredential — static, no sub-traits.
- BasicAuthCredential — static, no sub-traits.
- OAuth2Credential — Interactive + Refreshable + Revocable + Testable.

Probes:
- 3: compile_fail_capability_subtrait — E0046 missing method body
- 4: compile_fail_engine_dispatch_capability — E0277 sub-trait bound

Closes security-lead N1+N3+N5.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.4
      docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Stage 3"
```

---

## Stage 4 — Phantom-shim canonical form (ADR-0035)

`mod sealed_caps` + `#[capability]` proc-macro + `#[action]` rewrite. Bonus probes for `dyn Credential` const-KEY block + Pattern 2 service reject.

### Task 4.1 — Implement `#[capability]` proc-macro

**Files:**
- Create: `crates/credential/macros/src/capability.rs`
- Modify: `crates/credential/macros/src/lib.rs`

- [ ] **Step 1: Write `capability.rs`**

The macro accepts:

```rust
#[capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
pub trait BitbucketBearer: BitbucketCredential {}
```

And expands to (per Tech Spec §2.6 hand-expanded equivalent):

```rust
pub trait BitbucketBearer: BitbucketCredential {}

impl<T> BitbucketBearer for T
where
    T: BitbucketCredential,
    T::Scheme: AcceptsBearer,
{}

impl<T: BitbucketBearer> sealed_caps::BearerSealed for T {}

pub trait BitbucketBearerPhantom: sealed_caps::BearerSealed + Send + Sync {}

impl<T: BitbucketBearer> BitbucketBearerPhantom for T {}
```

Implementation skeleton:

```rust
//! `#[capability]` macro per ADR-0035 + Tech Spec §2.6.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Ident, ItemTrait, Meta, parse_macro_input};

pub fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as CapabilityArgs);
    let trait_def = parse_macro_input!(input as ItemTrait);

    let real_name = &trait_def.ident;
    let phantom_name = Ident::new(&format!("{real_name}Phantom"), real_name.span());
    let scheme_bound = args.scheme_bound;
    let sealed = args.sealed;
    // service_supertrait: extract supertraits from trait_def.supertraits;
    // expect exactly one Credential-rooted supertrait (e.g., BitbucketCredential).
    let service_supertrait = extract_service_supertrait(&trait_def);

    let expanded: TokenStream2 = quote! {
        #trait_def

        impl<T> #real_name for T
        where
            T: #service_supertrait,
            <T as ::nebula_credential::Credential>::Scheme: #scheme_bound,
        {}

        impl<T: #real_name> sealed_caps::#sealed for T {}

        pub trait #phantom_name: sealed_caps::#sealed + ::core::marker::Send + ::core::marker::Sync {}

        impl<T: #real_name> #phantom_name for T {}
    };

    expanded.into()
}

struct CapabilityArgs {
    scheme_bound: syn::Path,
    sealed: Ident,
}

impl syn::parse::Parse for CapabilityArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Parse `scheme_bound = ...,  sealed = ...`
        unimplemented!("see Step 2 — parsing helper")
    }
}

fn extract_service_supertrait(t: &ItemTrait) -> &syn::Path {
    // First Credential-rooted supertrait. Tech Spec §2.5 example uses
    // BitbucketCredential as the service supertrait.
    unimplemented!("see Step 2")
}
```

- [ ] **Step 2: Fill in argument parser + supertrait extractor**

Implement `CapabilityArgs::parse` to consume `scheme_bound = <Path>, sealed = <Ident>`. Implement `extract_service_supertrait` to walk `trait_def.supertraits` and return the first non-marker bound.

- [ ] **Step 3: Wire entry in `lib.rs`**

```rust
mod capability;

#[proc_macro_attribute]
pub fn capability(args: TokenStream, input: TokenStream) -> TokenStream {
    capability::expand(args, input)
}
```

- [ ] **Step 4: Document the macro contract**

The macro does NOT emit `mod sealed_caps` (per ADR-0035 §4.2 and Tech Spec §2.6 line 470). The crate author declares it once at crate root with one inner trait per capability; missing → `E0433`.

- [ ] **Step 5: Compile-check the macro crate**

Run: `cargo check -p nebula-credential-macros`
Expected: PASS.

### Task 4.2 — Update `#[action]` macro to rewrite `dyn X` → `dyn XPhantom`

**Files:**
- Modify: `crates/action/macros/src/` (locate the `#[action]` impl entry)

- [ ] **Step 1: Locate `#[action]` macro emission for `CredentialRef` fields**

Run: `grep -rn 'CredentialRef' crates/action/macros/src/`

The macro currently parses `CredentialRef<T>` field types. We need to rewrite `T = dyn X` to `T = dyn XPhantom` where the user-facing identifier `X` resolves to a phantom-paired capability trait.

- [ ] **Step 2: Heuristic — append `Phantom` suffix to `dyn X`**

Per Tech Spec §2.7 line 487 ("rewrites silently"): if the field type is `CredentialRef<dyn X>`, generate `CredentialRef<dyn XPhantom>`. The macro does NOT need to verify `XPhantom` exists — if it doesn't, the user gets an `E0405: cannot find trait XPhantom` at the rewritten span, which is the expected diagnostic.

```rust
// In the action macro's field-type rewriter:
fn rewrite_credential_ref_dyn(ty: &mut Type) {
    // Match `CredentialRef<dyn X>` and rewrite the inner to `dyn XPhantom`.
    // Pattern 1 (`CredentialRef<ConcreteCred>`) is a pass-through — no rewrite.
}
```

- [ ] **Step 3: Add a unit test for the rewriter**

In `crates/action/macros/tests/`, add a snapshot test (use `insta`) verifying:
- `CredentialRef<dyn BitbucketBearer>` → `CredentialRef<dyn BitbucketBearerPhantom>`
- `CredentialRef<SlackOAuth2Credential>` → unchanged.

- [ ] **Step 4: Compile-check**

Run: `cargo check -p nebula-action-macros`
Expected: PASS.

### Task 4.3 — Bonus Probe: `compile_fail_dyn_credential_const_key.rs`

Per spike iter-3 finding 5 (Tech Spec §15.4 line 3237). Documents the structural reason `dyn Credential` cannot exist (`const KEY` blocks `E0038`).

**Files:**
- Create: `crates/credential/tests/compile_fail_dyn_credential_const_key.rs`
- Create: `crates/credential/tests/probes/dyn_credential_const_key.rs`
- Create: `crates/credential/tests/probes/dyn_credential_const_key.stderr`

- [ ] **Step 1: Write trybuild driver**

```rust
#[test]
fn compile_fail_dyn_credential_const_key() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/dyn_credential_const_key.rs");
}
```

- [ ] **Step 2: Write fixture**

```rust
use nebula_credential::Credential;

fn _take(_c: &dyn Credential) {}  // E0038 — Credential not dyn-compatible

fn main() {}
```

Expected `.stderr`: `error[E0038]: the trait 'Credential' cannot be made into an object` citing `const KEY` (and the assoc-type set).

- [ ] **Step 3: Generate snapshot, verify, commit at end of Stage 4**

```bash
TRYBUILD=overwrite cargo nextest run -p nebula-credential --test compile_fail_dyn_credential_const_key --profile ci --no-tests=pass
```

### Task 4.4 — Bonus Probe: `compile_fail_pattern2_service_reject.rs`

**Files:**
- Create: `crates/credential/tests/compile_fail_pattern2_service_reject.rs`
- Create: `crates/credential/tests/probes/pattern2_service_reject.rs`
- Create: `crates/credential/tests/probes/pattern2_service_reject.stderr`

This probe is best hosted in `nebula-credential-builtin/tests/` once the canonical `mod sealed_caps` + service trait + capability traits exist. For П1 we ship a minimal in-test stub.

- [ ] **Step 1: Write fixture**

`tests/probes/pattern2_service_reject.rs`:

```rust
use nebula_credential::{Credential, AcceptsBearer, BearerScheme, BasicScheme, AuthScheme};

mod sealed_caps {
    pub trait BearerSealed {}
}

// Fake service supertrait
pub trait BitbucketCredential: Credential {}

#[nebula_credential_macros::capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
pub trait BitbucketBearer: BitbucketCredential {}

// Credential type with the wrong scheme — Basic, not Bearer.
struct BitbucketAppPassword;
// (Credential + BitbucketCredential impls elided — assume Scheme = BasicScheme.)

fn _wire(_: &dyn BitbucketBearerPhantom) {}

fn main() {
    let cred = BitbucketAppPassword;
    _wire(&cred);  // E0277 — BitbucketAppPassword: BitbucketBearerPhantom not satisfied.
}
```

Expected `.stderr`: chain `E0277: BasicScheme: AcceptsBearer not satisfied → required for BitbucketAppPassword: BitbucketBearer → required for BitbucketAppPassword: BitbucketBearerPhantom`.

- [ ] **Step 2: Generate snapshot + verify**

Same TRYBUILD command as previous probes.

### Task 4.5 — Stage 4 commit

- [ ] **Step 1: Run full suite**

Run: `cargo nextest run -p nebula-credential -p nebula-credential-macros -p nebula-action-macros --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 2: Commit**

```bash
git add crates/credential/macros/src/capability.rs \
        crates/credential/macros/src/lib.rs \
        crates/action/macros/ \
        crates/credential/tests/compile_fail_dyn_credential_const_key.rs \
        crates/credential/tests/compile_fail_pattern2_service_reject.rs \
        crates/credential/tests/probes/dyn_credential_const_key.* \
        crates/credential/tests/probes/pattern2_service_reject.*
git commit -m "feat(credential-macros): #[capability] proc-macro + #[action] phantom rewrite

Per ADR-0035 §4 + Tech Spec §2.6/§2.7. #[capability(scheme_bound, sealed)]
expands to real + sealed-blanket + phantom (canonical form per ADR-0035
amendments 2026-04-24-B). #[action] silently rewrites
CredentialRef<dyn X> to CredentialRef<dyn XPhantom> in generated code.

Bonus probes:
- compile_fail_dyn_credential_const_key — E0038 Credential not dyn-compatible
- compile_fail_pattern2_service_reject — E0277 BasicScheme: AcceptsBearer

Refs: docs/adr/0035-phantom-shim-capability-pattern.md
      docs/superpowers/specs/2026-04-24-credential-tech-spec.md §2.6 §2.7
      docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Stage 4"
```

---

## Stage 5 — Fatal duplicate-KEY registration (§15.6)

Closes security-lead N7. `register::<C>()` returns `Result<(), RegisterError>`. Probe 5 (runtime).

### Task 5.1 — Define `CredentialRegistry` + `RegisterError`

**Files:**
- Create: `crates/credential/src/contract/registry.rs`
- Modify: `crates/credential/src/contract/mod.rs`
- Modify: `crates/credential/src/lib.rs`

- [ ] **Step 1: Write `registry.rs`**

Per Tech Spec §15.6 + §3.1 (registry shape):

```rust
//! Credential registry — keyed by `Credential::KEY`. Append-only after
//! startup. Fatal duplicate-KEY per Tech Spec §15.6 (closes
//! security-lead N7).

use std::any::{Any, TypeId};
use std::sync::Arc;

use ahash::AHashMap;

use super::any::AnyCredential;
use super::credential::Credential;
use super::capability_report::Capabilities;

#[derive(Debug, Clone, thiserror::Error)]
pub enum RegisterError {
    #[error("duplicate credential key '{key}': existing={existing_crate}, new={new_crate}")]
    DuplicateKey {
        key: &'static str,
        existing_crate: &'static str,
        new_crate: &'static str,
    },
}

pub struct CredentialRegistry {
    entries: AHashMap<Arc<str>, RegistryEntry>,
}

struct RegistryEntry {
    instance: Box<dyn AnyCredential>,
    capabilities: Capabilities,
    registering_crate: &'static str,
}

impl CredentialRegistry {
    pub fn new() -> Self {
        Self { entries: AHashMap::new() }
    }

    /// Register a concrete credential. Fatal on duplicate KEY.
    pub fn register<C>(
        &mut self,
        instance: C,
        registering_crate: &'static str,
    ) -> Result<(), RegisterError>
    where
        C: Credential + crate::CredentialMetadataSource,
    {
        let key = C::KEY;
        if let Some(existing) = self.entries.get(key) {
            return Err(RegisterError::DuplicateKey {
                key,
                existing_crate: existing.registering_crate,
                new_crate: registering_crate,
            });
        }
        let capabilities = crate::compute_capabilities::<C>();  // Stage 7
        let arc_key: Arc<str> = key.into();
        self.entries.insert(
            arc_key,
            RegistryEntry {
                instance: Box::new(instance),
                capabilities,
                registering_crate,
            },
        );
        tracing::info!(key, registering_crate, "credential registered");
        Ok(())
    }

    pub fn resolve_any(&self, key: &str) -> Option<&(dyn AnyCredential + 'static)> {
        self.entries.get(key).map(|e| &*e.instance)
    }

    pub fn resolve<C: Credential + 'static>(&self, key: &str) -> Option<&C> {
        let entry = self.entries.get(key)?;
        entry.instance.as_any().downcast_ref::<C>()
    }

    pub fn capabilities_of(&self, key: &str) -> Option<Capabilities> {
        self.entries.get(key).map(|e| e.capabilities)
    }
}

impl Default for CredentialRegistry {
    fn default() -> Self { Self::new() }
}
```

Note: `compute_capabilities::<C>()` is filled in Stage 7. Stage 5 stubs it as `Capabilities::empty()` to keep the build green.

- [ ] **Step 2: Wire submodule + re-export**

```rust
// contract/mod.rs
pub mod registry;
pub use registry::{CredentialRegistry, RegisterError};
```

```rust
// lib.rs root re-export
pub use crate::contract::registry::{CredentialRegistry, RegisterError};
```

- [ ] **Step 3: Stub `compute_capabilities` to keep build green**

Add to `crates/credential/src/lib.rs` or a new `crates/credential/src/contract/capability_report.rs`:

```rust
//! Capability detection at registration. Filled out in Stage 7.
//! Stage 5 stubs `compute_capabilities` to return `Capabilities::empty()`
//! so the registry compiles standalone.

use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
    pub struct Capabilities: u8 {
        const INTERACTIVE = 1 << 0;
        const REFRESHABLE = 1 << 1;
        const REVOCABLE   = 1 << 2;
        const TESTABLE    = 1 << 3;
        const DYNAMIC     = 1 << 4;
    }
}

#[doc(hidden)]
pub fn compute_capabilities<C>() -> Capabilities {
    // Stage 7 fills in real detection via plugin_capability_report::*
    // constants. Stage 5 returns empty to bootstrap registry compile.
    Capabilities::empty()
}
```

Add `bitflags = "2"` to `crates/credential/Cargo.toml` `[dependencies]`.

- [ ] **Step 4: Compile-check**

Run: `cargo check -p nebula-credential`
Expected: PASS.

### Task 5.2 — Replace engine's silent-overwrite registry with the contract one

**Files:**
- Modify: `crates/engine/src/credential/registry.rs`

- [ ] **Step 1: Delete the engine's local `register` implementation**

The engine should **use** `nebula_credential::CredentialRegistry`, not redefine it. Locate `crates/engine/src/credential/registry.rs`, read its current content. If the engine's registry is just a thin wrapper, replace it with a re-export:

```rust
pub use nebula_credential::{CredentialRegistry, RegisterError};
```

If the engine has additional fields (e.g., orchestration state), refactor: keep the engine wrapper but delegate the `register` to `CredentialRegistry::register`, propagating the `Result`. **No silent overwrite path may remain.**

- [ ] **Step 2: Update all callers to handle `Result<(), RegisterError>`**

Run: `grep -rn '\.register::' crates/`

For each call site, switch from infallible to `?` propagation (or explicit handle if the caller cannot propagate, but in plugin init paths `?` to a startup error is the canonical shape).

- [ ] **Step 3: Compile-check**

Run: `cargo check -p nebula-engine`
Expected: PASS.

### Task 5.3 — Probe 5: `runtime_duplicate_key_fatal.rs`

**Files:**
- Create: `crates/credential/tests/runtime_duplicate_key_fatal.rs`

Note: this is a **runtime** probe, not compile-fail. Duplicate KEY across two distinct crates is data-dependent — not statically detectable.

- [ ] **Step 1: Write the test**

```rust
//! Probe 5 — §15.6 fatal duplicate-KEY registration (runtime).
//!
//! Per Tech Spec §16.1.1: duplicate keys across crates are not
//! statically detectable by rustc alone — this probe is runtime.

use nebula_credential::{CredentialRegistry, RegisterError};

// Two credential types sharing the same KEY.
struct CredA;
struct CredB;

// (Credential + CredentialMetadataSource impls — minimal, both with
// const KEY: &'static str = "shared.duplicate".)

#[test]
fn duplicate_key_returns_error_not_panic() {
    let mut registry = CredentialRegistry::new();
    let r1 = registry.register(CredA, env!("CARGO_CRATE_NAME"));
    assert!(r1.is_ok(), "first registration should succeed");

    let r2 = registry.register(CredB, env!("CARGO_CRATE_NAME"));
    let err = r2.expect_err("second registration must error, not overwrite");

    match err {
        RegisterError::DuplicateKey { key, .. } => {
            assert_eq!(key, "shared.duplicate");
        }
    }
}

#[test]
fn duplicate_key_first_wins() {
    let mut registry = CredentialRegistry::new();
    let _ = registry.register(CredA, env!("CARGO_CRATE_NAME"));
    let _ = registry.register(CredB, env!("CARGO_CRATE_NAME"));  // rejected

    // Resolve by KEY — must return CredA (the first registration), not CredB.
    let resolved = registry.resolve::<CredA>("shared.duplicate");
    assert!(resolved.is_some(), "first registration must remain authoritative");
}
```

- [ ] **Step 2: Provide minimal `Credential` + `CredentialMetadataSource` impls for `CredA`/`CredB`**

Inside the test file or a `tests/support/` module. They share `KEY` — the whole point of the probe.

- [ ] **Step 3: Run the probe**

Run: `cargo nextest run -p nebula-credential --test runtime_duplicate_key_fatal --profile ci --no-tests=pass`
Expected: PASS.

### Task 5.4 — Stage 5 commit

```bash
git add crates/credential/src/contract/registry.rs \
        crates/credential/src/contract/capability_report.rs \
        crates/credential/src/contract/mod.rs \
        crates/credential/src/lib.rs \
        crates/credential/Cargo.toml \
        crates/credential/tests/runtime_duplicate_key_fatal.rs \
        crates/engine/src/credential/registry.rs
git commit -m "feat(credential)!: fatal duplicate-KEY registration (§15.6)

BREAKING. CredentialRegistry::register::<C>() returns
Result<(), RegisterError>. Silent overwrite (registry.rs:31 warn+
overwrite path) replaced with fail-closed startup error. Closes
security-lead N7 — supply-chain credential takeover via duplicate
KEY collision now blocks startup with operator-actionable error.

Probe 5 (runtime, not compile-fail per §16.1.1):
- runtime_duplicate_key_fatal — DuplicateKey returned, not panic;
  first registration remains authoritative, second rejected.

Long-term: arch-signing-infra (queue #7) closes the supply-chain
provenance gap entirely. §15.6 is interim mitigation.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.6
      docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Stage 5"
```

---

## Stage 6 — `SchemeGuard` + `SchemeFactory` for refresh hook (§15.7)

Closes security-lead N8 + tech-lead gap (i). Probes 6 + 7.

### Task 6.1 — Define `SchemeGuard` with lifetime pinning

**Files:**
- Create: `crates/credential/src/secrets/scheme_guard.rs`
- Modify: `crates/credential/src/secrets/mod.rs`
- Modify: `crates/credential/src/lib.rs`

- [ ] **Step 1: Write `scheme_guard.rs`**

Per Tech Spec §15.7 + spike iter-3 secondary finding (refined lifetime form):

```rust
//! `SchemeGuard<'a, C>` — borrowed wrapper for refreshed Scheme
//! material. `!Clone` + `ZeroizeOnDrop` + lifetime-pinned via shared
//! `'a` with a `CredentialContext<'a>` borrow at the engine call site.
//!
//! Per Tech Spec §15.7 (closes security-lead N8 + tech-lead gap (i))
//! and spike iter-3 secondary finding (lifetime-gap refinement).

use std::marker::PhantomData;
use std::ops::Deref;
use zeroize::ZeroizeOnDrop;

use crate::Credential;

/// Borrowed wrapper for a refreshed `Scheme`. The `'a` lifetime is
/// shared with a `&'a CredentialContext<'a>` at the engine call site;
/// any attempt to retain the guard past the call site forces the
/// engine borrow to also outlive the struct, which the engine blocks.
///
/// **Invariants enforced at compile time:**
/// - `!Clone` — see [`SchemeGuard::clone`] inherent shadow (no impl).
/// - Drop-zeroizes via `ZeroizeOnDrop` derive on the inner Scheme.
/// - `!Send` if `Scheme: !Send` — propagates through `PhantomData`.
pub struct SchemeGuard<'a, C: Credential> {
    scheme: <C as Credential>::Scheme,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a, C: Credential> SchemeGuard<'a, C> {
    /// Crate-private constructor — only the engine creates these.
    pub(crate) fn new(scheme: <C as Credential>::Scheme) -> Self {
        Self { scheme, _lifetime: PhantomData }
    }
}

impl<'a, C: Credential> Deref for SchemeGuard<'a, C> {
    type Target = <C as Credential>::Scheme;
    fn deref(&self) -> &Self::Target {
        &self.scheme
    }
}

// Zeroize on drop — relies on the wrapped Scheme being SensitiveScheme
// (which mandates ZeroizeOnDrop) or PublicScheme (no zeroize needed,
// no harm).
impl<'a, C: Credential> Drop for SchemeGuard<'a, C> {
    fn drop(&mut self) {
        // Scheme's own ZeroizeOnDrop fires here — no extra work.
    }
}

// IMPORTANT: NO `Clone` impl. Probe 7 verifies `clone()` is rejected.
```

- [ ] **Step 2: Define `SchemeFactory<C>`**

Append to the same file:

```rust
use std::sync::Arc;
use std::pin::Pin;
use std::future::Future;

use crate::error::CredentialError;

type AcquireFuture<'a, C> = Pin<Box<dyn Future<Output = Result<SchemeGuard<'a, C>, CredentialError>> + Send + 'a>>;

/// Factory for fresh `SchemeGuard` acquisition. Long-lived resources
/// invoke `.acquire()` per request rather than retaining a guard.
pub struct SchemeFactory<C: Credential> {
    inner: Arc<dyn for<'a> Fn() -> AcquireFuture<'a, C> + Send + Sync>,
}

impl<C: Credential> SchemeFactory<C> {
    pub(crate) fn new<F>(f: F) -> Self
    where
        F: for<'a> Fn() -> AcquireFuture<'a, C> + Send + Sync + 'static,
    {
        Self { inner: Arc::new(f) }
    }

    pub async fn acquire(&self) -> Result<SchemeGuard<'_, C>, CredentialError> {
        (self.inner)().await
    }
}

impl<C: Credential> Clone for SchemeFactory<C> {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}
```

Note the deliberate divergence: `SchemeFactory: Clone` (cheap `Arc` bump for sharing across resource pools); `SchemeGuard: !Clone` (no copying of plaintext scheme material).

- [ ] **Step 3: Wire submodule + re-export**

```rust
// secrets/mod.rs
pub mod scheme_guard;
pub use scheme_guard::{SchemeFactory, SchemeGuard};
```

```rust
// lib.rs root re-export
pub use crate::secrets::{SchemeFactory, SchemeGuard};
```

- [ ] **Step 4: Compile-check**

Run: `cargo check -p nebula-credential`
Expected: PASS.

### Task 6.2 — Update `Resource::on_credential_refresh` signature

**Files:**
- Modify: `crates/resource/src/contract.rs` (or wherever the Resource trait lives — verify)

- [ ] **Step 1: Locate the Resource trait**

Run: `grep -rn 'pub trait Resource' crates/resource/src/`

- [ ] **Step 2: Update `on_credential_refresh` signature**

Per Tech Spec §15.7 spike iter-3 refinement:

```rust
use std::future::Future;
use nebula_credential::{Credential, CredentialContext, SchemeGuard};

pub trait Resource: Send + Sync {
    type Credential: Credential;
    type Error: std::error::Error;

    /// Notification hook fired by engine when Self::Credential is
    /// refreshed. Resources MUST NOT retain the SchemeGuard past this
    /// call — the lifetime parameter shared with `ctx` enforces this
    /// at compile time (see Probe 6).
    ///
    /// Per Tech Spec §15.7. Default no-op for resources that
    /// re-acquire via SchemeFactory per request rather than caching.
    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext<'a>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        async move {
            let _ = new_scheme;
            let _ = ctx;
            Ok(())
        }
    }
}
```

- [ ] **Step 3: Update existing Resource impls**

Run: `grep -rn 'on_credential_refresh' crates/`

Each existing impl gets the new signature. Most likely just signature edit; default body if no special handling.

- [ ] **Step 4: Compile-check**

Run: `cargo check -p nebula-resource`
Expected: PASS.

### Task 6.3 — Probe 6: `compile_fail_scheme_guard_retention.rs`

**Files:**
- Create: `crates/credential/tests/compile_fail_scheme_guard_retention.rs`
- Create: `crates/credential/tests/probes/scheme_guard_retention.rs`
- Create: `crates/credential/tests/probes/scheme_guard_retention.stderr`

- [ ] **Step 1: Write trybuild driver**

```rust
#[test]
fn compile_fail_scheme_guard_retention() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/scheme_guard_retention.rs");
}
```

- [ ] **Step 2: Write fixture (retention via field-store)**

```rust
use nebula_credential::{Credential, CredentialContext, SchemeGuard};

struct DummyCred;
// (Credential impl elided.)

struct LeakyResource<'a> {
    stored: Option<SchemeGuard<'a, DummyCred>>,
}

impl<'a> LeakyResource<'a> {
    async fn on_credential_refresh(
        &mut self,
        new_scheme: SchemeGuard<'a, DummyCred>,
        ctx: &'a CredentialContext<'a>,
    ) {
        let _ = ctx;
        self.stored = Some(new_scheme);  // E0597 — guard tied to 'a, but `self` may outlive it
    }
}

fn main() {}
```

Expected `.stderr`: `error[E0597]: 'new_scheme' does not live long enough` (or equivalent — exact diagnostic depends on lifetime elision; what matters is the lifetime-error class).

- [ ] **Step 3: Generate `.stderr` snapshot**

Run: `TRYBUILD=overwrite cargo nextest run -p nebula-credential --test compile_fail_scheme_guard_retention --profile ci --no-tests=pass`

If the diagnostic differs from `E0597`, refine the fixture so the failing form is the one we want to document (retention into a struct field with conflicting lifetime). Multiple variants may be needed:

- variant a: store into self struct
- variant b: return SchemeGuard from the fn (forces caller to outlive)
- variant c: spawn into 'static task

Pick whichever cleanest demonstrates "guard cannot outlive ctx" and snapshot.

- [ ] **Step 4: Verify probe passes**

Run: `cargo nextest run -p nebula-credential --test compile_fail_scheme_guard_retention --profile ci --no-tests=pass`
Expected: PASS.

### Task 6.4 — Probe 7: `compile_fail_scheme_guard_clone.rs`

**Files:**
- Create: `crates/credential/tests/compile_fail_scheme_guard_clone.rs`
- Create: `crates/credential/tests/probes/scheme_guard_clone.rs`
- Create: `crates/credential/tests/probes/scheme_guard_clone.stderr`

- [ ] **Step 1: Write trybuild driver**

```rust
#[test]
fn compile_fail_scheme_guard_clone() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/scheme_guard_clone.rs");
}
```

- [ ] **Step 2: Write fixture**

```rust
use nebula_credential::{Credential, SchemeGuard};

struct DummyCred;
// (Credential impl elided.)

fn try_clone<'a>(g: SchemeGuard<'a, DummyCred>) -> SchemeGuard<'a, DummyCred> {
    g.clone()  // E0599 — no method named `clone`
}

fn main() {}
```

Expected `.stderr`: `error[E0599]: no method named 'clone' found for ...`.

- [ ] **Step 3: Generate snapshot, verify, run**

Same TRYBUILD pattern.

### Task 6.5 — Stage 6 commit

```bash
git add crates/credential/src/secrets/scheme_guard.rs \
        crates/credential/src/secrets/mod.rs \
        crates/credential/src/lib.rs \
        crates/credential/tests/compile_fail_scheme_guard_*.rs \
        crates/credential/tests/probes/scheme_guard_*.{rs,stderr} \
        crates/resource/src/contract.rs
git commit -m "feat(credential): SchemeGuard + SchemeFactory for refresh hook (§15.7)

Closes security-lead N8 + tech-lead gap (i). SchemeGuard<'a, C> is the
borrowed wrapper resources receive at on_credential_refresh:
- !Clone (probe 7 verifies)
- ZeroizeOnDrop (via wrapped SensitiveScheme)
- Lifetime-pinned via shared 'a with engine ctx borrow (probe 6 verifies)

SchemeFactory<C> is the re-acquisition mechanism for long-lived
resources — pool calls factory.acquire() per request rather than
caching the guard.

Probes 6 + 7. Resource::on_credential_refresh signature updated per
spike iter-3 refined lifetime form.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.7
      docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Stage 6"
```

---

## Stage 7 — Capability-from-type authority shift (§15.8)

Closes security-lead N6. `CredentialMetadata::capabilities_enabled` removed; capabilities computed at registration via per-credential `plugin_capability_report::*` constants emitted by `#[plugin_credential]`.

### Task 7.1 — Remove `capabilities_enabled` from `CredentialMetadata`

**Files:**
- Modify: `crates/credential/src/metadata.rs`

- [ ] **Step 1: Locate the field**

Run: `grep -n 'capabilities_enabled' crates/credential/src/metadata.rs`

- [ ] **Step 2: Delete the field + any builder method**

Remove the `capabilities_enabled: Capabilities` field from the struct. Remove any `with_capabilities(...)` / `set_capabilities(...)` builder method. Remove serde serialization fields that referenced it.

- [ ] **Step 3: Update consumers**

Run: `grep -rn 'capabilities_enabled' crates/`

Every consumer must move from "read field on metadata" to "read from registry" (per Task 7.4).

### Task 7.2 — Implement `plugin_capability_report::*` per-capability constants

**Files:**
- Modify: `crates/credential/src/contract/capability_report.rs` (created in Stage 5 with stub)

- [ ] **Step 1: Replace stub with real detection**

```rust
//! Capability detection at registration. Per-credential blanket
//! constants emitted by #[plugin_credential] macro.

use bitflags::bitflags;
use crate::contract::credential::Credential;
use crate::contract::{Interactive, Refreshable, Revocable, Testable, Dynamic};

bitflags! {
    #[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
    pub struct Capabilities: u8 {
        const INTERACTIVE = 1 << 0;
        const REFRESHABLE = 1 << 1;
        const REVOCABLE   = 1 << 2;
        const TESTABLE    = 1 << 3;
        const DYNAMIC     = 1 << 4;
    }
}

#[doc(hidden)]
pub mod plugin_capability_report {
    pub trait IsInteractive { const VALUE: bool; }
    pub trait IsRefreshable { const VALUE: bool; }
    pub trait IsRevocable { const VALUE: bool; }
    pub trait IsTestable { const VALUE: bool; }
    pub trait IsDynamic { const VALUE: bool; }
}

pub fn compute_capabilities<C>() -> Capabilities
where
    C: Credential
        + plugin_capability_report::IsInteractive
        + plugin_capability_report::IsRefreshable
        + plugin_capability_report::IsRevocable
        + plugin_capability_report::IsTestable
        + plugin_capability_report::IsDynamic,
{
    let mut caps = Capabilities::empty();
    if <C as plugin_capability_report::IsInteractive>::VALUE { caps.insert(Capabilities::INTERACTIVE); }
    if <C as plugin_capability_report::IsRefreshable>::VALUE { caps.insert(Capabilities::REFRESHABLE); }
    if <C as plugin_capability_report::IsRevocable>::VALUE   { caps.insert(Capabilities::REVOCABLE); }
    if <C as plugin_capability_report::IsTestable>::VALUE    { caps.insert(Capabilities::TESTABLE); }
    if <C as plugin_capability_report::IsDynamic>::VALUE     { caps.insert(Capabilities::DYNAMIC); }
    caps
}
```

The credential macro (Task 7.3) emits **all five** `IsX` impls per credential, with `VALUE = true` only for the sub-traits actually impl'd. This avoids relying on specialization.

### Task 7.3 — Update `#[derive(Credential)]` macro to emit capability_report constants

**Files:**
- Modify: `crates/credential/macros/src/credential.rs`

- [ ] **Step 1: Add an opt-in attribute syntax**

The macro accepts:

```rust
#[derive(Credential)]
#[credential(key = "oauth2_github", capabilities(interactive, refreshable, revocable, testable))]
struct GitHubOAuth2;
```

The `capabilities(...)` list declares which sub-traits the user separately impls. The macro emits:

```rust
impl plugin_capability_report::IsInteractive for GitHubOAuth2 { const VALUE: bool = true; }
impl plugin_capability_report::IsRefreshable for GitHubOAuth2 { const VALUE: bool = true; }
impl plugin_capability_report::IsRevocable  for GitHubOAuth2 { const VALUE: bool = true; }
impl plugin_capability_report::IsTestable   for GitHubOAuth2 { const VALUE: bool = true; }
impl plugin_capability_report::IsDynamic    for GitHubOAuth2 { const VALUE: bool = false; }
```

For unspecified capabilities, emit `const VALUE: bool = false`. The user is responsible for matching `capabilities(...)` declarations with actual `impl Refreshable for X` etc. — mismatch surfaces:
- Macro emits `VALUE = true`, no `impl Refreshable for X` exists → engine `RefreshDispatcher::for_credential::<X>()` fails to compile (probe 4 territory).
- Macro emits `VALUE = false`, but user wrote `impl Refreshable for X` → registry under-reports the capability; `iter_compatible` filter excludes the credential. Operator-visible bug, not a security failure.

To prevent the latter, the **strict** form is to also walk the source for `impl <SubTrait> for <X>` patterns at expansion. Stable proc-macros cannot do this (no source visibility beyond input). The opt-in declaration is the practical compromise.

- [ ] **Step 2: Update existing derives**

For `api_key.rs` / `basic_auth.rs` / `oauth2.rs`, the `#[credential(...)]` attribute gains `capabilities(...)` lists matching their sub-trait impls (Stage 3).

- [ ] **Step 3: Compile-check**

Run: `cargo check -p nebula-credential -p nebula-credential-macros`
Expected: PASS.

### Task 7.4 — Update engine `iter_compatible` filter

**Files:**
- Modify: `crates/engine/src/credential/discovery.rs` (or wherever `iter_compatible` lives — verify with grep)

- [ ] **Step 1: Locate `iter_compatible`**

Run: `grep -rn 'iter_compatible\|capabilities_enabled' crates/engine/src/`

- [ ] **Step 2: Replace metadata read with registry read**

```rust
// Before:
fn iter_compatible(metadata: &CredentialMetadata, capability: Capability) -> bool {
    metadata.capabilities_enabled.contains(capability)
}

// After:
fn iter_compatible(registry: &CredentialRegistry, key: &str, capability: Capability) -> bool {
    registry.capabilities_of(key)
        .map(|caps| caps.contains(Capabilities::from_capability(capability)))
        .unwrap_or(false)
}
```

(`Capability` enum vs `Capabilities` bitflags — provide a tiny `from_capability` constructor or match conversion.)

- [ ] **Step 3: Compile-check**

Run: `cargo check -p nebula-engine`
Expected: PASS.

### Task 7.5 — Probe 8: `compile_fail_metadata_capability_field.rs`

**Files:**
- Create: `crates/credential/tests/compile_fail_metadata_capability_field.rs`
- Create: `crates/credential/tests/probes/metadata_capability_field.rs`
- Create: `crates/credential/tests/probes/metadata_capability_field.stderr`

- [ ] **Step 1: Write trybuild driver**

```rust
#[test]
fn compile_fail_metadata_capability_field() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/metadata_capability_field.rs");
}
```

- [ ] **Step 2: Write fixture**

```rust
use nebula_credential::CredentialMetadata;

fn make_meta() -> CredentialMetadata {
    CredentialMetadata {
        key: "foo".into(),
        // ... other fields ...
        capabilities_enabled: Default::default(),  // E0560 — field does not exist
    }
}

fn main() {}
```

Expected `.stderr`: `error[E0560]: struct 'CredentialMetadata' has no field named 'capabilities_enabled'`.

- [ ] **Step 3: Generate snapshot + verify**

Same TRYBUILD pattern.

### Task 7.6 — Stage 7 commit

```bash
git add crates/credential/src/metadata.rs \
        crates/credential/src/contract/capability_report.rs \
        crates/credential/src/lib.rs \
        crates/credential/macros/src/credential.rs \
        crates/credential/src/credentials/ \
        crates/engine/src/credential/discovery.rs \
        crates/credential/tests/compile_fail_metadata_capability_field.rs \
        crates/credential/tests/probes/metadata_capability_field.*
git commit -m "feat(credential)!: capability-from-type authority shift (§15.8)

BREAKING. CredentialMetadata::capabilities_enabled removed. Plugin
authors no longer self-attest capabilities — the registry computes
the capability set at registration from sub-trait membership via
plugin_capability_report::* per-credential constants emitted by
#[derive(Credential)] / #[plugin_credential] from a capabilities(...)
declaration.

iter_compatible filter now reads registry.capabilities_of(key); a
plugin lying about capabilities can no longer appear in slot pickers
it doesn't satisfy. Closes security-lead N6.

Probe 8: compile_fail_metadata_capability_field — E0560 field absent.

Refs: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.8
      docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Stage 7"
```

---

## Stage 8 — Doc sync + landing-gate verify

### Task 8.1 — Run all 8+2 probes in one shot

- [ ] **Step 1: Probe summary command**

```bash
cargo nextest run -p nebula-credential --profile ci --no-tests=pass \
    --test compile_fail_state_zeroize \
    --test compile_fail_scheme_sensitivity \
    --test compile_fail_capability_subtrait \
    --test compile_fail_engine_dispatch_capability \
    --test runtime_duplicate_key_fatal \
    --test compile_fail_scheme_guard_retention \
    --test compile_fail_scheme_guard_clone \
    --test compile_fail_metadata_capability_field \
    --test compile_fail_dyn_credential_const_key \
    --test compile_fail_pattern2_service_reject
```

Expected: **all 10 tests PASS** (8 mandatory per §16.1.1 + 2 bonus from spike iter-3).

- [ ] **Step 2: Capture probe report for the PR**

Run the command above, capture stdout to `/tmp/p1-probe-report.txt`. Reference in PR description.

### Task 8.2 — Full local gate

- [ ] **Step 1: Format check**

Run: `cargo +nightly fmt --all -- --check`
Expected: PASS.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: PASS.

- [ ] **Step 3: Doc-tests**

Run: `cargo test --workspace --doc`
Expected: PASS.

- [ ] **Step 4: Workspace nextest**

Run: `cargo nextest run --workspace --profile ci --no-tests=pass`
Expected: PASS.

If any failures surface in non-credential crates, fix at the call site (capability bound, registration call, scheme accessor) — do not paper over.

### Task 8.3 — `cargo-public-api` snapshot

- [ ] **Step 1: Capture post-П1 surface**

Run: `cargo public-api --manifest-path crates/credential/Cargo.toml > /tmp/credential-post-p1.txt`
Run: `cargo public-api --manifest-path crates/credential-builtin/Cargo.toml > /tmp/credential-builtin-post-p1.txt`

- [ ] **Step 2: Diff against pre-П1**

Run: `diff /tmp/credential-pre-p1.txt /tmp/credential-post-p1.txt > /tmp/p1-public-api-diff.txt`

Inspect: every change must correspond to a §15.x amendment. No silent surface change. Attach diff to PR description.

- [ ] **Step 3: Commit the snapshots into the repo**

If the project uses `cargo-public-api` snapshot files (e.g., `public-api.txt` in each crate), update them. If not, just attach the diff in the PR.

### Task 8.4 — Update Tech Spec status + register

**Files:**
- Modify: `docs/superpowers/specs/2026-04-24-credential-tech-spec.md` (frontmatter `status:`)
- Modify: `docs/tracking/credential-concerns-register.md` (status flips on affected rows)

- [ ] **Step 1: Tech Spec status flip**

Edit frontmatter:

```yaml
status: complete CP6 (active-dev endorse-phased 2026-04-24 Round 7) — П1 in-implementation 2026-04-26 (commit <SHA>).
```

(Replace `<SHA>` with the П1 merge commit SHA after merge — leave as `<SHA>` placeholder during the merge PR.)

Add a §16.5.1 entry below §16.5:

```markdown
### §16.5.1 П1 implementation tracker (2026-04-26)

Phase plan: [`docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md`](../plans/2026-04-26-credential-p1-trait-scaffolding.md).

Landing checklist (verified at merge):
- [ ] All 8 mandatory probes (per §16.1.1) PASS — runtime_duplicate_key_fatal + 7 compile-fail.
- [ ] Bonus probes from spike iter-3 PASS — compile_fail_dyn_credential_const_key + compile_fail_pattern2_service_reject.
- [ ] Full local gate PASS (fmt nightly + clippy + nextest + doc-test).
- [ ] cargo-public-api diff every change accounted for by §15.x amendment.
- [ ] MATURITY rows updated for nebula-credential + nebula-credential-builtin.
- [ ] Register status flips landed (see register).
- [ ] CHANGELOG / READMEs reflect new shape.
```

- [ ] **Step 2: Register row updates**

In `docs/tracking/credential-concerns-register.md`, for each row affected by Stage 1–7, update status:
- `arch-capability-subtrait-split` — `decided` → `in-implementation` (or `done` after merge); add commit pointer.
- `arch-scheme-sensitivity-dichotomy` — same.
- `arch-registry-duplicate-fail-closed` — same.
- `arch-scheme-guard-factory` — same.
- `arch-metadata-capability-authority` — same.
- `arch-phantom-shim-convention` — confirm `decided` (still authoritative; П1 uses canonical form).
- Type-system rows that locked-post-spike → `done` with commit pointer.

(Run an audit: every row mentioning a §15.x or П1-touched concern must have a fresh resolution pointer.)

### Task 8.5 — Update READMEs + MATURITY

**Files:**
- Modify: `crates/credential/README.md`
- Modify: `docs/MATURITY.md`

- [ ] **Step 1: `crates/credential/README.md`**

Add a "П1 trait shape" section reflecting:
- Sub-trait split (no `const REFRESHABLE` bool — use `impl Refreshable`)
- AuthScheme dichotomy (`SensitiveScheme` vs `PublicScheme`)
- `CredentialRegistry::register` returns `Result<(), RegisterError>`
- `SchemeGuard` + `SchemeFactory` for refresh hook
- Capability-from-type — no plugin-authored `capabilities_enabled` field
- Phantom-shim canonical form — link to ADR-0035

- [ ] **Step 2: `docs/MATURITY.md`**

Update the `nebula-credential` row to reflect the new trait shape. Update the `nebula-credential-builtin` row from "scaffold" to "preview — П1 trait scaffolding landed; concrete types in П3".

### Task 8.6 — Final commit + handoff to merge

- [ ] **Step 1: Run full local gate one final time**

```bash
cargo +nightly fmt --all -- --check && \
cargo clippy --workspace -- -D warnings && \
cargo test --workspace --doc && \
cargo nextest run --workspace --profile ci --no-tests=pass
```

Expected: PASS.

- [ ] **Step 2: Commit doc + register updates**

```bash
git add crates/credential/README.md \
        docs/MATURITY.md \
        docs/superpowers/specs/2026-04-24-credential-tech-spec.md \
        docs/tracking/credential-concerns-register.md
git commit -m "docs(credential): П1 doc sync + register status flips

Tech Spec status → 'П1 in-implementation 2026-04-26'. Register rows
flipped to in-implementation for affected concerns. crate README
reflects new trait shape (sub-trait split, dichotomy, fatal duplicate,
SchemeGuard, capability-from-type). MATURITY rows updated.

Refs: docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md Stage 8"
```

- [ ] **Step 3: Open the merge PR**

```bash
git push -u origin credential-p1-trait-scaffolding
gh pr create --title "feat(credential)!: П1 — trait shape scaffolding" \
             --body "$(cat <<'EOF'
## Summary

П1 lands the validated CP5/CP6 credential trait shape per Tech Spec §15.4–§15.10:

- Capability sub-trait split (Interactive / Refreshable / Revocable / Testable / Dynamic) — closes security-lead N1+N3+N5
- AuthScheme sensitivity dichotomy (SensitiveScheme / PublicScheme) — closes N2+N4+N10
- Fatal duplicate-KEY registration — closes N7
- SchemeGuard + SchemeFactory refresh hook — closes N8 + tech-lead gap (i)
- Capability-from-type authority shift — closes N6
- ADR-0035 phantom-shim canonical form (per-capability inner Sealed)
- New nebula-credential-builtin crate scaffold

8 mandatory probes (per Tech Spec §16.1.1) + 2 bonus from spike iter-3 — all PASS.

## Test plan

- [x] All 10 probes pass (`cargo nextest run -p nebula-credential --profile ci`)
- [x] Full local gate (fmt nightly + clippy + nextest + doc-test)
- [x] cargo-public-api diff: every change tied to §15.x amendment
- [x] MATURITY rows updated
- [x] Register rows flipped

## Refs

- Plan: docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md
- Tech Spec: docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.3–§15.12 + §16.1
- Strategy: docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md (frozen CP3)
- ADR-0035: docs/adr/0035-phantom-shim-capability-pattern.md (amendments 2026-04-24-B + -C)
- Spike iter-3: commit f36f3739 worktree worktree-agent-afe8a4c6

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**1. Spec coverage** — every §15.x compile-time amendment has a Stage:
- §15.4 sub-trait split → Stage 3 (probes 3 + 4)
- §15.5 dichotomy → Stage 1 (probe 2) + §15.5 ConnectionUri restructure (Task 1.5) + bearer_header SecretString (Task 1.6)
- §15.6 fatal duplicate-KEY → Stage 5 (probe 5)
- §15.7 SchemeGuard + SchemeFactory → Stage 6 (probes 6 + 7)
- §15.8 capability-from-type → Stage 7 (probe 8)
- §15.4 amendment (CredentialState ZeroizeOnDrop) → Stage 2 (probe 1)
- ADR-0035 canonical form → Stage 4 (bonus probes for `dyn Credential` + Pattern 2 reject)
- `nebula-credential-builtin` scaffold → Stage 0
- `mod sealed_caps` → Stage 0 (`crates/credential-builtin/src/lib.rs`)

§15.10 PendingStore atomicity is **out of П1 scope** per Tech Spec §15.3 (runtime-gated, П-later) — correctly excluded.

**2. Placeholder scan** — every step has concrete code or exact command. No `TODO`/`TBD`. Macro internals reference Tech Spec §2.6 hand-expanded form.

**3. Type consistency** — `SchemeGuard<'a, C: Credential>` consistent across §15.7 + Stage 6 + Probe 6 + Resource trait signature. `RegisterError::DuplicateKey` consistent §15.6 + Stage 5 registry + Probe 5. `Capabilities` bitflags consistent Stage 5 stub + Stage 7 real impl.

**Known gap (acknowledged):** macro detection of "user impl'd `Refreshable` but didn't declare `capabilities(refreshable)`" is **not** statically detected — the macro can't see `impl X for Y` outside its input. Mitigation: opt-in attribute + downstream operator-visible bug (registry under-reports capability → slot picker excludes credential). Documented in Task 7.3 Step 1. If post-П1 review demands stricter enforcement, candidates: (a) require `#[plugin_credential]` to wrap the entire credential decl including sub-trait impls (single source of truth); (b) add a runtime assert at registration that re-checks via specialization-style dispatch. Both deferred to post-П1 follow-up if needed.

---

**Plan complete.**

## Execution Handoff

Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Best fit for П1 because each Stage is mostly self-contained and probe-verified.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints for review. Higher context cost but tighter feedback loop on tricky lifetime / macro work.

Which approach?
