# nebula-schema — Phase 3 Security & Secrets — Design Spec

**Status:** Draft — to be refined after Phase 2 lands
**Date:** 2026-04-16
**Phase:** 3 of 4
**Depends on:** Phase 1 Foundation + Phase 2 DX Layer

---

## 1. Goal

`SecretField` is already a `Field` variant in Phase 1, but its value is still carried as `FieldValue::Literal(String)` — indistinguishable in memory, debug output, and wire format from a regular string. Phase 3 fixes that: secret values become a distinct `SecretValue` type with `Zeroize`, redaction across `Debug` / `Display` / `Serialize`, explicit `.expose()` for the audit path, and an integration seam for credential storage.

---

## 2. Non-goals

- Credential encryption at rest — lives in `nebula-credential`
- Key management / KMS / SGX — orthogonal infrastructure
- Cryptographic operations beyond KDF hashing — out of scope for schema

---

## 3. Key decisions

| Question | Decision |
|----------|----------|
| Secret representation | `SecretBytes(Zeroizing<Vec<u8>>)` + `SecretString(Zeroizing<String>)` wrappers with manual redacted `Debug`/`Display`/`Serialize` |
| Default Serialize behaviour | **Redacts** to `"<redacted>"`. Wire-format serialization of secrets must go through `SecretWire::expose_serialize(&self)` — explicit, audited via grep |
| `.expose()` API | Returns `&[u8]` / `&str` but emits a `tracing::debug!` event with `#[track_caller]` for audit trail |
| Zeroize feature | `zeroize` feature is **on by default** in `nebula-schema`; can be disabled for no-std / constrained builds |
| KDF integration | `SecretField.kdf: Option<KdfParams>` — when set, the resolve step hashes the value via the configured KDF (argon2 / scrypt / pbkdf2); action receives the hash, never the plaintext |
| `ResolvedValues::get_secret` | Returns `Option<&SecretValue>` — typed accessor that enforces the "secrets don't flow through `get()`" rule |
| LoaderContext leak risk | `LoaderContext::values_redacted()` — new method returning a view where `SecretField` values are replaced with a placeholder before being exposed to loader callbacks |

---

## 4. Architecture

```
crates/schema/src/
├── secret.rs        (new)     — SecretBytes, SecretString, KdfParams, SecretWire
├── field.rs         (modify)  — SecretField gains `kdf: Option<KdfParams>`
├── value.rs         (modify)  — FieldValue::SecretLiteral(SecretValue) added (nonexhaustive-safe)
├── validated.rs     (modify)  — ResolvedValues::get_secret accessor; resolve converts SecretField values
└── loader.rs        (modify)  — LoaderContext::values_redacted()
```

### Types

```rust
#[non_exhaustive]
pub enum SecretValue {
    Bytes(SecretBytes),
    String(SecretString),
}

pub struct SecretBytes(Zeroizing<Vec<u8>>);
pub struct SecretString(Zeroizing<String>);

impl std::fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBytes(<redacted>)")
    }
}
// Display same. Serialize → "<redacted>".

impl SecretBytes {
    /// Explicit, audited access to the plaintext.
    #[track_caller]
    pub fn expose(&self) -> &[u8] {
        tracing::debug!(target: "nebula_schema::secret", caller = %std::panic::Location::caller(), "secret exposed");
        &self.0
    }
}

#[non_exhaustive]
pub enum KdfParams {
    Argon2 { memory_kb: u32, iterations: u32 },
    Scrypt { n: u32, r: u32, p: u32 },
    Pbkdf2Sha256 { iterations: u32, salt: SecretBytes },
}
```

### Wire format

Default serialization redacts. Explicit exposure:
```rust
/// Wrapper that serializes the raw value. Use only when writing to an encrypted store.
pub struct SecretWire<'a>(&'a SecretValue);

impl<'a> SecretWire<'a> {
    pub fn new(value: &'a SecretValue) -> Self { Self(value) }
}

impl<'a> Serialize for SecretWire<'a> { /* emits plaintext */ }
```

A workspace-level `clippy::disallowed_methods` rule flags `SecretWire::new` outside of `nebula-credential` and migration scripts.

---

## 5. Resolve-time behaviour

Phase 1's `ValidValues::resolve` is extended:

```
for each field in schema.fields() where field is SecretField:
    let raw = values.get(field.key)
    match (raw, field.kdf):
      (Some(FieldValue::Literal(Value::String(s))), Some(kdf)) =>
          replace with FieldValue::SecretLiteral(kdf.hash(s.as_bytes()))
      (Some(FieldValue::Literal(Value::String(s))), None) =>
          replace with FieldValue::SecretLiteral(SecretString::new(s))
      (Some(FieldValue::Expression(e)), _) =>
          resolve expression, then apply the same rule above to the result
      _ => leave as-is; type_mismatch error if required
```

`ResolvedValues::get_secret(key)` is the only accessor that returns a secret.
`ResolvedValues::get(key)` continues to work for non-secret fields; for a `SecretField` it returns `None` (forcing callers to use `get_secret`).

---

## 6. Migration

| Change | Breaks | Fix |
|--------|--------|-----|
| `FieldValue` gains `SecretLiteral` variant | exhaustive matches | existing crates have `#[non_exhaustive]` — add a catch-all |
| `ResolvedValues::get(secret_key)` returns `None` | callers that read secrets via `get` | switch to `get_secret` |
| `Serialize` for secrets redacts | round-trips that relied on plaintext JSON | use `SecretWire::new(&secret)` explicitly |

`nebula-credential` updates:
- `Credential::resolve(values: ResolvedValues)` — internals now use `values.get_secret(...)` for the secret parameters
- Store layer (which encrypts at rest) uses `SecretWire` explicitly

---

## 7. Testing

- Redaction tests: `format!("{:?}", secret)` must not contain plaintext
- Serialize tests: `serde_json::to_string(&secret)` returns `"<redacted>"`
- Explicit exposure tests: `SecretWire::new(&s)` round-trips
- KDF integration: resolve with `Argon2` params produces hash output of expected length
- No-panic proof: `Drop` for `SecretBytes` zeroizes memory (use `zeroize_derive` or manual impl; test with `MaybeUninit` trick)
- Compile-fail: `SecretValue` cannot be constructed from a public API path that doesn't go through resolve or `SecretBytes::new_exposed`

---

## 8. Open questions

1. **Zeroize on Drop for `String` inside `SecretString`** — `Zeroizing<String>` zeroes the `Vec<u8>` backing but `String`'s layout doesn't expose it cleanly. May need `SecretString(Zeroizing<Vec<u8>>)` with utf8-validated accessor.
2. **Feature gate strategy** — `zeroize` on-by-default means it's always linked. Alternative: `default-features = ["zeroize"]` with graceful fallback (`Debug` still redacts but no zeroize-on-drop). Preferred: default on.
3. **KDF crate choice** — `argon2`, `scrypt`, `pbkdf2` crates. All are `no_std`-compatible. Pick one baseline + feature-flag the others.
4. **Audit logging target** — `tracing::debug!` vs structured event. Decision candidate: `tracing` event in `nebula_schema::secret` target, structured payload.
