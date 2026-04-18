---
name: credential-security-review
description: Use when touching code that handles credentials, secrets, tokens, auth schemes, or logging that could include sensitive values. Fast checklist against Nebula's §4.2 safety invariants and §12.5 encryption rules. Complements the security-lead agent — this is the self-review pass before you'd invoke security-lead for a deeper look.
---

# credential-security-review

## When to invoke

- Editing `crates/credential/`, `crates/api/` (auth paths), `crates/plugin/`, `crates/sandbox/`, or any action crate that handles auth material.
- Adding a new auth scheme, token type, or credential variant.
- Touching `tracing::*!` in any crate that carries credential IDs, tokens, or secret material.
- Writing or modifying error types that could include a secret.
- Adding or changing `Clone` on a type that holds secret material.

## Why this exists

Credentials in Nebula are **engine-owned** (PRODUCT_CANON §4.2). The stored-state vs projected-auth-material split protects node authors from hand-rolling refresh and operators from credential theft via log leak, memory dump, or debug output. This skill catches the common mistakes before they reach `security-lead`.

## Checklist

### 1. Secret types

- [ ] Every secret value uses `SecretString` / `SecretToken` / `CredentialGuard` — never plain `String` / `Vec<u8>`.
- [ ] Types holding secrets derive `Zeroize + ZeroizeOnDrop`.
- [ ] `Debug` is redacted — `fmt::Debug` never prints the raw secret.
- [ ] `Display` is not implemented on secret types, or is redacted.
- [ ] `Clone` on a secret type: each clone is another zeroization point. Is this clone necessary? Document why if kept.

### 2. Logging and errors

- [ ] No `println!`, `eprintln!`, `dbg!` in library code (even under `#[cfg(debug_assertions)]`).
- [ ] `tracing::*!` macros that take a credential ID or token use the redacted form (canon §12.5).
- [ ] Error types never carry plaintext secrets in the message. Use redacted reasons (`CredentialError::TokenRefreshFailed { credential_id, reason: RedactedReason }`).
- [ ] `anyhow::Error` is not used in library crates — it can capture backtraces that include secret-bearing locals.

### 3. Credential lifecycle

- [ ] No direct read from the credential store. All access through `CredentialAccessor` (engine-owned).
- [ ] Refresh / rotation is engine-owned. Action authors do not implement it.
- [ ] Stored state (encrypted) vs projected auth material (to action) split is preserved.
- [ ] `CredentialRotatedEvent` is published via `EventBus<T>`, not an ad-hoc channel.

### 4. Transport and persistence (§12.5)

- [ ] At-rest encryption: AES-256-GCM. Credential ID bound as AAD.
- [ ] KDF: Argon2id. No raw hashing, no truncated KDF output.
- [ ] Intermediate plaintext lives in `Zeroizing<Vec<u8>>`, dropped before the function returns.
- [ ] No secret in URLs, query strings, or unredacted metric labels.

### 5. Async and cancellation

- [ ] Futures that hold plaintext secrets are cancel-safe. If not, document that cancellation may drop plaintext before zeroize runs.
- [ ] No `MutexGuard` holding a secret across `.await` — deadlock and leak risk.
- [ ] `spawn_blocking` for CPU-heavy crypto work so the async runtime isn't starved.

### 6. Supply chain

- [ ] Any new crypto dep: confirm allowed license in `deny.toml`, no advisories, no `unsafe` without `// SAFETY:`.
- [ ] No `openssl` (Nebula prefers `rustls`; `deny.toml` enforces).

## Severity rubric

- 🔴 **CRITICAL** — plaintext secret leaving process boundary (log, error, network, disk).
- 🟠 **HIGH** — exploitable defense-in-depth gap (timing side channel, missing validation, unredacted metric).
- 🟡 **MEDIUM** — hardening opportunity, non-ideal pattern.
- ✅ **GOOD** — positive observation worth preserving (call it out to avoid accidental removal later).

## Output format

```
## Credential security review: [change]

🔴 CRITICAL: [list, or "none"]
🟠 HIGH:     [list, or "none"]
🟡 MEDIUM:   [list, or "none"]
✅ GOOD:     [positive observations]

### Action
[proceed / fix 🔴s first / handoff: security-lead for <reason>]
```

If any 🔴 appears — STOP and hand off to `security-lead`. Do not self-approve critical security changes.
