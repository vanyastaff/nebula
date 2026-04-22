# nebula-schema Phase 3 Security & Secrets — Implementation Plan (skeleton)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. **This is a skeleton plan** — refine after Phase 1 + Phase 2 land.

**Goal:** `SecretField` values become `SecretValue` with `Zeroize`, redaction, explicit exposure API, and optional KDF hashing at resolve time.

**Spec:** `docs/superpowers/specs/2026-04-16-nebula-schema-phase3-security-design.md`

**Estimated tasks:** 10.

---

## Skeleton tasks

### Task 1 — Crate prep: zeroize dep + feature flag

**Files:** `crates/schema/Cargo.toml`, `Cargo.toml` (workspace)

- [ ] Add `zeroize = { version = "1", features = ["zeroize_derive"] }` as workspace dep.
- [ ] Schema Cargo: `zeroize = { workspace = true }` under default features.
- [ ] Add `tracing = { workspace = true }` to schema (for audit events).
- [ ] Commit.

### Task 2 — `SecretBytes` + `SecretString` types

**Files:** `crates/schema/src/secret.rs` (new)

- [ ] TDD: tests for `Debug`/`Display`/`Serialize` all redact to `<redacted>`.
- [ ] TDD: `SecretBytes::new(v).expose() == v`.
- [ ] Implement using `Zeroizing<Vec<u8>>` / `Zeroizing<String>`.
- [ ] Add `#[track_caller]` + `tracing::debug!` inside `expose()`.
- [ ] Commit.

### Task 3 — `SecretValue` wrapper + `SecretWire` explicit-serialize path

**Files:** `crates/schema/src/secret.rs`

- [ ] TDD: `serde_json::to_string(&SecretValue::...)` returns `"\"<redacted>\""`.
- [ ] TDD: `SecretWire::new(&secret)` round-trips plaintext.
- [ ] Implement.
- [ ] Commit.

### Task 4 — `KdfParams` + hash method

**Files:** `crates/schema/src/secret.rs`

- [ ] Pick baseline KDF (decision: `argon2` per spec §8).
- [ ] TDD: `KdfParams::Argon2 {...}.hash(b"pass").len() > 0` and different for different inputs.
- [ ] Add other KDF variants behind feature flags (`kdf-scrypt`, `kdf-pbkdf2`).
- [ ] Commit.

### Task 5 — Extend `FieldValue` with `SecretLiteral`

**Files:** `crates/schema/src/value.rs`

- [ ] TDD: `FieldValue::from_json(Value::String(s))` on a `SecretField` context (needs reference from schema) converts to `SecretLiteral`.
- [ ] But wait — `from_json` is schema-blind. Alternative: `resolve()` handles conversion, `from_json` always produces `Literal(String)` and resolve substitutes. Prefer this — no schema coupling in parse.
- [ ] Add `FieldValue::SecretLiteral(SecretValue)` variant (non-exhaustive enum so no breakage).
- [ ] Commit.

### Task 6 — `SecretField.kdf` + builder wiring

**Files:** `crates/schema/src/field.rs`, `crates/schema/src/builder/secret.rs`

- [ ] Add `kdf: Option<KdfParams>` field to `SecretField`.
- [ ] Builder method: `.kdf(KdfParams::Argon2 { ... })`.
- [ ] TDD: builder produces expected kdf config.
- [ ] Commit.

### Task 7 — Resolve-time secret conversion

**Files:** `crates/schema/src/validated.rs`

- [ ] During `ValidValues::resolve`, when a field is `SecretField`:
  - Literal String → `SecretLiteral(SecretString)`
  - If `kdf` set → hash the string, store hash as `SecretLiteral(SecretBytes)`
  - Expression → resolve, then same rule
- [ ] Integration test: schema with `Field::secret(...).kdf(KdfParams::Argon2{...})`; resolve produces hash, `ResolvedValues::get_secret` returns `SecretBytes`.
- [ ] Commit.

### Task 8 — `ResolvedValues::get_secret` + `ResolvedValues::get` returns `None` for secret fields

**Files:** `crates/schema/src/validated.rs`

- [ ] TDD: `rv.get(&secret_key)` → `None`, `rv.get_secret(&secret_key)` → `Some(&SecretValue)`.
- [ ] Commit.

### Task 9 — `LoaderContext::values_redacted()`

**Files:** `crates/schema/src/loader.rs`

- [ ] Add method returning a view of `FieldValues` where `SecretField` entries are replaced with `Literal(String("<redacted>"))`.
- [ ] Needs schema reference — `LoaderContext` extended to carry `&ValidSchema`.
- [ ] TDD: construct context with a secret value, assert `values_redacted().get(secret_key)` returns placeholder.
- [ ] Commit.

### Task 10 — nebula-credential migration + integration sweep

**Files:** `crates/credential/src/credentials/{api_key,basic_auth,oauth2}.rs`, `crates/credential/src/credential.rs`

- [ ] Update `Credential::resolve` implementations to use `values.get_secret(...)` for secret params.
- [ ] Where creds were previously storing plaintext, now wrap in `SecretWire::new` explicitly.
- [ ] End-to-end test: api_key credential with `api_key` param — redacted in logs, round-trips via `SecretWire` to encrypted store, decrypted back.
- [ ] Workspace-wide `cargo test --workspace`.
- [ ] Commit + CHANGELOG.

---

## Acceptance (Phase 3)

- [ ] `format!("{:?}", secret_value)` never contains plaintext
- [ ] `serde_json::to_string(&resolved_values)` redacts all secret fields
- [ ] `SecretBytes` / `SecretString` zero their memory on drop (verified with `MaybeUninit` trick)
- [ ] `ResolvedValues::get_secret` is the only read path for secrets
- [ ] `nebula-credential` builtins pass all existing tests
- [ ] `cargo test --workspace && cargo clippy --workspace -- -D warnings` green
