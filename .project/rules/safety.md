# Safety & Security Rules

Universal rules for safe, secure Rust code. Apply to every crate, every PR.

## Rust Memory & Type Safety

### `unsafe`
- `#![deny(unsafe_code)]` on every library crate
- If approved: `// SAFETY:` comment explains the EXACT invariant, not "this is safe"
- Every `unsafe` block has a test (miri or loom) proving the invariant

### No Panics in Libraries
- No `unwrap()` / `expect()` outside `#[cfg(test)]`
- Accept `NonZero*` types instead of panicking on zero
- `debug_assert!` for provable invariants — provide safe fallback for release mode
- `panic!` / `unreachable!` only when the condition is logically impossible (comment why)

### RAII Guards
- Use `defused: bool` flag — NOT `mem::forget` (panic between creation and forget = double action)
- Drop impl: `if !self.defused { cleanup() }`
- Test both paths: normal drop AND defused drop

### Numeric Safety
- Counters: `saturating_add` / `saturating_sub` — never silent overflow
- `f64` → `Duration`: cap with `.min(max)` BEFORE `from_secs_f64` (infinity panics)
- `as` casts: `#[allow(clippy::cast_*)]` only with `// Reason:` comment
- API boundaries: `TryFrom` / `TryInto` for fallible conversions

### Atomics
- `Relaxed` — counters only (ordering doesn't matter)
- `Acquire` / `Release` — synchronization pairs (load-after-store)
- `SeqCst` — almost never; if used, document why lesser ordering is insufficient

---

## Injection Attacks

### Command Injection
- NEVER pass user input to `std::process::Command` without strict allowlisting
- Expression/template engines: sandbox evaluation, no access to `process`, `fs`, `net`
- Identifiers from user input: validate `[a-zA-Z0-9_-]`, reject everything else
- User-provided data is DATA, never CODE — no `eval`, no `format!` into executable strings

### SQL Injection
- All queries via parameterized statements — `query!` / `query_as!` with bind params
- NEVER `format!("SELECT * FROM {}", user_input)` — this is always a vulnerability
- Even table/column names: use allowlists, not string interpolation

### Template / Expression Injection
- Sandbox expression evaluation — no ambient authority (env vars, filesystem, network)
- Depth limit on nested evaluation (prevent stack overflow via recursive templates)
- Output encoding: escape for target context (HTML, URL, SQL, shell)
- Max output size limit to prevent memory exhaustion

### Header Injection (CRLF)
- Reject `\r\n` in all user-provided HTTP header values
- Never include raw user input in response headers without validation

### Path Traversal
- NEVER `Path::join(user_input)` without canonicalizing and prefix-checking
- `canonicalize()` THEN `starts_with(allowed_root)` — in that order
- Reject: `..`, null bytes (`\0`), non-UTF8 sequences
- Symlink resolution: canonicalize resolves symlinks — check result, not input

---

## Authentication & Secrets

### Storage
- Credentials encrypted at rest — AES-256-GCM or equivalent AEAD
- `SecretString` / `Zeroize` — wipe plaintext from memory on drop
- Key derivation: `argon2` / `scrypt` / `bcrypt` — never raw SHA/MD5 for passwords
- Tokens: store encrypted, never in logs, never in error messages

### Timing Attacks
- Secret comparison: `constant_time_eq` or `subtle::ConstantTimeEq`
- NEVER `==` for comparing secrets — timing side-channel leaks prefix length
- HMAC verification: constant-time compare of digests

### Token Security
- Webhook signatures: HMAC-SHA256, verify BEFORE processing payload
- API tokens: ≥256 bits entropy, rotate on compromise
- OAuth2 state parameter: constant-time comparison, single-use, expiring

---

## Deserialization & Input Validation

### Untrusted Data
- Size limits: max body size, max string length, max collection size
- Depth limits: max nesting depth (prevent stack overflow on deeply nested structures)
- Schema validation: reject unknown fields where strict mode is appropriate
- Type coercion: be explicit — don't silently convert `"123"` to `123`

### Denial of Service
- Rate limit all public endpoints
- Timeouts: per-operation, per-request, per-workflow — no unbounded execution
- Regex on user input: guaranteed linear-time engine only (`regex` crate, not PCRE)
- Allocation limits: reject inputs that would cause >N MB of allocations

### Validation Pattern
- Validate at construction (`new() -> Result`), not at use time
- For serde: `#[serde(try_from = "RawType")]` validates during deserialization
- Config types: all constraints checked in `validate()`, called by constructor
- Invalid state must be unrepresentable — use types (enums, newtypes, NonZero) over runtime checks

---

## Supply Chain

- `cargo deny check` in CI — advisories, licenses, bans, duplicate detection
- `cargo audit` — known CVE detection, fail the build
- New dependencies: review for `unsafe`, check transitive tree size, verify publisher
- `Cargo.lock` committed — reproducible builds
- No `build.rs` that downloads or executes external code at compile time
- Feature flags: `default = []` — opt-in to capabilities, not opt-out

---

## Logging & Observability

- NEVER log secrets, tokens, API keys, passwords, PII
- `Debug` impls on secret-bearing types: print `[REDACTED]`
- Error messages to external callers: generic ("internal error")
- Error details: internal logs only, with request correlation ID
- Audit trail: log WHO accessed WHAT resource WHEN (separate from debug logs)

---

## Concurrency Safety

### Locks
- `parking_lot::Mutex` — OK in async if no `.await` under lock
- `tokio::sync::Mutex` — required when holding across `.await`
- Lock ordering: if multiple locks, always acquire in same global order
- Drop guards before calling functions that might take other locks

### Cancel Safety
- Document `# Cancel safety` on every async method using `select!`
- RAII guards must release resources on drop (cancel = drop)
- State machines: every transition tested, especially error → recovery

### Shared State
- `Arc<T>` — verify `T: Send + Sync`
- Prefer channels (`mpsc`, `watch`, `broadcast`) over `Arc<Mutex<T>>` when data flows one way
- Counters: `AtomicU64` over `Mutex<u64>`

---

## Checklist for Security Review

When reviewing code touching external input, auth, or secrets:

- [ ] Can user input reach `Command::new`, `format!` into SQL/shell, or `Path::join`?
- [ ] Are secrets compared with constant-time equality?
- [ ] Does `Debug` output redact sensitive fields?
- [ ] Is deserialized input bounded (size, depth, count)?
- [ ] Are timeouts set on all external calls?
- [ ] Do error messages leak internal details to callers?
- [ ] Are new dependencies audited (`cargo deny`, `cargo audit`)?
- [ ] Is credential access scoped and logged?
