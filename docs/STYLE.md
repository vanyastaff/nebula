---
name: Nebula style guide
description: Consolidated house style — idioms, antipatterns, naming table, error taxonomy, type design bets. Read by every session before proposing changes.
status: accepted
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md, GLOSSARY.md, CLAUDE.md]
---

# Nebula style guide

Read before proposing a new public type, API shape, or refactor. Cross-referenced
from `CLAUDE.md` read-order.

> **When to fight this guide:** see `docs/PRODUCT_CANON.md §0.2` — canon revision
> triggers apply to style as well. A style rule that blocks a measurable
> architectural improvement is a candidate for revision, not a blocker.

## 1. Idioms we use

- **`mem::take` / `mem::replace`** — extract owned values from `&mut self` without cloning. Pairs with `Default`.
- **Newtype wrappers** — `pub struct CredentialKey(String)`, `pub struct ActionKey(String)` — strong types for identifiers, not `String` aliases.
- **Builder pattern** — for any type with more than three fields, especially when some are optional. Consumes `self` (not `&mut self`) to enable method chaining and prevent re-use after `build()`.
- **RAII guards** — release-on-drop for resource lifecycle. Companion to explicit `.release()` when an async release path exists; the guard handles the crash path.
- **Typestate** — phantom types on state transitions where the engine can enforce them at compile time. Example: `Execution<Running>` → `Execution<Terminal>` via a transition method.
- **`#[must_use]`** — on every `Result`, every builder, every function returning a cleanup or cancellation token.
- **`Cow<'_, T>`** — prefer over premature cloning for read-mostly borrows with occasional mutation.
- **Sealed traits** for extension points — define via private supertrait when Nebula owns all implementations; opens later if we decide to allow downstream impls.
- **`dynosaur` for `dyn`-compatible async traits** — author the trait in AFIT form (`async fn` in trait), apply `#[dynosaur::dynosaur(DynFoo)]`, and let the macro generate the `dyn`-compatible sibling. Static signatures name `impl Foo`; dynamic boundaries name `dyn DynFoo`. Never introduce `#[async_trait]` in new code. See [ADR-0014](adr/0014-dynosaur-macro.md).

## 2. Antipatterns we reject

- **`.clone()` to satisfy the borrow checker without a tradeoff note.** Consider `Cow<'_, T>`, lifetime redesign, typestate, or `Arc` first. If cloning is the right answer, leave a comment explaining why (rare — usually the signal is over-application of the clone).
- **`Deref` polymorphism.** Do not use `Deref` to simulate inheritance. Prefer explicit methods or trait delegation.
- **Stringly-typed public APIs.** `fn do(thing: &str)` where `thing` has a finite set of values — use an enum.
- **`anyhow` in library crates.** Use `thiserror` with typed errors. `anyhow` is for binaries only.
- **`unwrap` outside tests.** Use `expect` with a documented invariant at minimum, typed error propagation preferred.
- **Implicit panics in async state.** `assert!` inside an async fn on a path not guarded by a type-level invariant is a latent outage. Use typed errors.
- **Orphan modules.** A module that is never imported from the crate's `lib.rs` is either dead code or a test-only module in the wrong place.
- **Direct state mutation bypassing repository.** Any field like `ns.state = X` or `let _ = transition(...)` that skips version bumps is broken — see memory `feedback_direct_state_mutation.md`.

## 3. Naming table

| Suffix / pattern | Meaning | Example |
|---|---|---|
| `*Metadata` | UI-facing description: id, display name, icon, categories | `ActionMetadata`, `CredentialMetadata` |
| `*Schema` | Typed config schema (from `nebula-schema`) | `ActionSchema`, `CredentialSchema` |
| `*Key` | Stable identifier used across layers | `ExecutionKey`, `ActionKey`, `CredentialKey` |
| `*Id` | Runtime identifier, not stable across restarts | `ExecutionId` (durable), `SessionId` |
| `*Error` / `*ErrorKind` | Typed errors from `thiserror` | `ExecutionError`, `CredentialErrorKind` |
| `*Repo` | Storage port — trait abstracting persistence | `ExecutionRepo`, `CredentialRepo` |
| `*Handle` | Borrowed reference to a managed resource | `ResourceHandle<T>` |
| `*Guard` | RAII type enforcing cleanup on drop | `LeaseGuard`, `ScopeGuard` |
| `*Token` | Capability / continuation / dedup token | `CancellationToken`, `IdempotencyToken` |
| `*Policy` | Configuration type for a behavioral decision | `CheckpointPolicy`, `DrainTimeoutPolicy` |

## 4. Error taxonomy

Library crates use `thiserror`. Error types derive `Debug`, `thiserror::Error`, and do not implement `Clone` unless a specific consumer requires it.

All errors flow through `nebula-error::NebulaError` at module boundaries. The `Classify` trait decides transient vs permanent — classification is explicit, not inferred from error message strings.

API boundary: every `ApiError` variant maps to an RFC 9457 `problem+json` response. New failure modes get a new typed variant with an explicit HTTP status — no ad-hoc `500` for business-logic mistakes.

Secret-bearing errors: never include the secret in the error string. Use a redacted indicator (e.g. `CredentialError::TokenRefreshFailed { credential_id: .., reason: RedactedReason }`).

## 5. Type design bets

Defended pre-1.0 — changing any of these is an ADR-level decision:

- **Sealed traits for integration extension points.** `Action`, `Credential`, `Resource` traits seal via `crate` supertrait; downstream crates implement via derive macros or helper traits rather than by naming the sealed bound.
- **Typestate for lifecycle enforcement.** `Execution<Planned>` → `<Running>` → `<Terminal>` — transitions are methods consuming `self`; invalid state transitions fail to compile.
- **`Zeroize` + `ZeroizeOnDrop` on secret material.** Every type containing `SecretString`, `SecretToken`, or raw key bytes implements zeroization. `Debug` is redacted — leaking is a PR-level blocker.
- **`#[non_exhaustive]` on public enums and structs we intend to grow.** Consumers must use `_` or `..` in matches / destructuring, leaving us room to add variants / fields without SemVer breakage.
- **`#[unstable(feature = "...")]`-gated public API for aspirational surface.** Anything not yet engine-honored hides behind an unstable feature flag with an issue tracker link — never ships on a stable release path.

## 6. When to fight canon

See `docs/PRODUCT_CANON.md §0.2 canon revision triggers`.
