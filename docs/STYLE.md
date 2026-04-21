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

## 0. Universal mindset (project rules + Rust 1.95+)

This section is the **short, universal** framing for humans and agents. It does
not list every case; **`PRODUCT_CANON.md`** is normative for product rules, and
**`docs/AGENT_PROTOCOL.md`** states how agents combine evidence, boundaries,
and workflow. **Concrete thresholds** (when to stop and rethink shape, Clippy
IDs, CI gates) live in **`docs/IDIOM_REVIEW_CHECKLIST.md`** and
**`docs/QUALITY_GATES.md`** — they **operationalize** these principles, not
replace them.

- **Project first.** Layers and integrations follow **`PRODUCT_CANON.md`** and
  **`deny.toml`**. Public names and type ownership follow **`GLOSSARY.md`** and
  §3 below. Do not “solve locally” in a way that duplicates ownership or crosses
  boundaries without an **ADR**.

- **Design.** Prefer **one clear responsibility** per module and **explicit**
  domain modeling (enums, newtypes, builders) over flags and ad hoc parameters
  when the domain is not an open-ended string space — aligned with §§1–2 and
  *Rust Design Patterns* ([design principles](https://rust-unofficial.github.io/patterns/additional_resources/design-principles.html)).

- **Rust 1.95+ toolchain.** Behavior is pinned in **`rust-toolchain.toml`**. For
  **semantic** questions (patterns, lifetimes, unsafe), use the **Rust
  Reference** and `std` docs on [`doc.rust-lang.org`](https://doc.rust-lang.org/).
  House idioms and antipatterns are **§§1–2** of this file; they override generic
  blog-style Rust when they conflict.

- **Evolving shape.** When requirements add cases or cross-cutting concerns,
  **revisit** whether the existing API or data structure still fits — not only
  whether the smallest patch compiles. Agents apply the same idea via
  **`AGENT_PROTOCOL.md`** (universal principles + inspect/implement) and the
  checklist pass for pattern-heavy edits.

## 1. Idioms we use

- **`mem::take` / `mem::replace`** — extract owned values from `&mut self` without cloning. Pairs with `Default`.
- **Newtype wrappers** — `pub struct CredentialKey(String)`, `pub struct ActionKey(String)` — strong types for identifiers, not `String` aliases.
- **Builder pattern** — for any type with more than three fields, especially when some are optional. Consumes `self` (not `&mut self`) to enable method chaining and prevent re-use after `build()`.
- **RAII guards** — release-on-drop for resource lifecycle. Companion to explicit `.release()` when an async release path exists; the guard handles the crash path.
- **Typestate** — phantom types on state transitions where the engine can enforce them at compile time. Example: `Execution<Running>` → `Execution<Terminal>` via a transition method.
- **`#[must_use]`** — on every `Result`, every builder, every function returning a cleanup or cancellation token.
- **`Cow<'_, T>`** — prefer over premature cloning for read-mostly borrows with occasional mutation.
- **Sealed traits** for extension points — define via private supertrait when Nebula owns all implementations; opens later if we decide to allow downstream impls.
- **Async traits — match the seam.** For traits consumed only through generics (no `dyn`), author native AFIT: `fn foo(&self, …) -> impl Future<Output = …> + Send`. For traits stored or passed as `Arc<dyn Foo>` / `Box<dyn Foo>`, use `#[async_trait]` — native AFIT is not `dyn`-compatible on stable Rust today. Do not mix both forms on the same trait. Re-evaluate when `async_fn_in_dyn_trait` stabilizes (see [rust-lang/rust#133119](https://github.com/rust-lang/rust/issues/133119)). See [ADR-0024](adr/0024-defer-dynosaur-migration.md) (supersedes [ADR-0014](adr/0014-dynosaur-macro.md)).

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

## 6. Secret handling

Anchors [`PRODUCT_CANON.md §12.5 — Secrets and auth`](PRODUCT_CANON.md#125-secrets-and-auth)
in practical rules. Every rule below is **non-negotiable**; a PR that trips one is a blocker, not a nit.

### 6.1 What counts as "secret material"

- User-supplied credentials: API keys, OAuth tokens, passwords, client secrets, session tokens.
- Cryptographic key bytes: signing keys, shared keys, key-derivation inputs.
- Pre-decrypt ciphertext + nonce pairs *while decryption is in-flight*.
- Any value a credential scheme wraps in `SecretString` or a scheme-specific secret newtype.

### 6.2 Mandatory patterns

1. **Wrap it.** Raw `String` / `Vec<u8>` is **not** acceptable for secret material at a public API boundary. Wrap with
   `nebula_credential::SecretString` (or equivalent newtype) so `Zeroize` / `ZeroizeOnDrop` / redacted `Debug` come for free.
2. **Access via closure scope.** Use `secret.expose_secret(|s| { ... })` — do **not** store the raw `&str` in a longer-lived local.
   Scope the exposure to the smallest block that needs the plaintext.
3. **Redacted `Debug` and `Display`.** Any type carrying secret material implements `Debug` (and `Display` if it exists)
   to emit `"[REDACTED]"`. Deriving `#[derive(Debug)]` on a struct that contains a secret field is the most common bug —
   write `Debug` by hand and redact the field.
4. **Default `Serialize` redacts, too.** For a credential wrapper, default `Serialize` emits the `"[REDACTED]"` sentinel.
   If a call site needs the actual value on the wire (encrypted-at-rest storage), it uses the explicit
   `serde_secret` module — never the default impl.
5. **No `tracing::*!` of raw secrets.** Any `tracing` event that takes a secret must log the wrapper (so its redacted
   `Debug` / `Display` fires) — never `expose_secret`-extract a plaintext value and then feed it to a format string.
6. **No secret in error strings.** Error variants carry structured identifiers (credential id, token id) + a classified
   reason enum; they do not carry the secret. See §4 (Error taxonomy).

### 6.3 Right vs wrong

**Right:**

```rust
#[derive(Clone)]
pub struct ApiKey {
    key: SecretString,
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKey").field("key", &"[REDACTED]").finish()
    }
}

pub fn authorize(k: &ApiKey) -> Result<(), AuthError> {
    k.key.expose_secret(|raw| {
        // short-lived borrow; nothing leaves this closure
        send_http_header("Authorization", &format!("Bearer {raw}"));
    });
    Ok(())
}
```

**Wrong:**

```rust
#[derive(Debug, Clone)]            // ← derived Debug leaks the key
pub struct ApiKey {
    pub key: String,               // ← raw String, no zeroize, public field
}

pub fn authorize(k: &ApiKey) -> Result<(), AuthError> {
    tracing::info!("auth for key {}", k.key); // ← logs the raw key
    Err(AuthError::Forbidden(format!("bad key {}", k.key))) // ← secret in error
}
```

### 6.4 Verifying it

`crates/credential/tests/redaction.rs` ships a
**log-redaction test helper** — `assert_no_secret_in_logs(forbidden, || { ... })` — that captures
current-thread `tracing` output (all levels, including `DEBUG` / `TRACE`) while the closure runs
and fails if the forbidden substring shows up. It covers both the *positive* case (secrets
formatted as `[REDACTED]`) and a `#[should_panic]` negative case (a raw leak must fail the
assertion, so a silently-passing test cannot mask a real regression).

**Scope caveat.** The subscriber is installed via `tracing::subscriber::with_default` and is
thread-local. Events emitted from threads spawned inside the closure — or from work that an async
runtime moves onto a worker thread — will **not** be captured. Keep redaction tests on a
single-thread logging path; do not `tokio::spawn` / `std::thread::spawn` inside `body`.

New credential-adjacent types add a targeted test there:

```rust
#[test]
fn my_new_credential_never_leaks() {
    let raw = "my-unique-test-value-do-not-use-in-prod";
    let cred = MyCredential::new(SecretString::new(raw));

    assert_no_secret_in_logs(raw, || {
        tracing::info!(cred = ?cred, "sanity");
        tracing::error!("error path: {cred:?}");
    });
}
```

### 6.5 Review checklist

Paste this into a credential-touching PR's self-review:

- [ ] Every new struct carrying secret material wraps it (`SecretString` / newtype) — no raw `String` / `Vec<u8>` fields.
- [ ] Custom `Debug` (and `Display` if any) written by hand; field formatted as `"[REDACTED]"`.
- [ ] Default `Serialize` emits the redacted sentinel; explicit `serde_secret` used only where justified in code.
- [ ] No `tracing::*!` format argument is a raw plaintext secret.
- [ ] No error variant carries the secret in its payload or `Display`.
- [ ] A targeted `assert_no_secret_in_logs` test in `crates/credential/tests/redaction.rs` for the new type.

## 7. When to fight canon

See `docs/PRODUCT_CANON.md §0.2 canon revision triggers`.
