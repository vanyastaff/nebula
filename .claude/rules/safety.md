# Safety & Security Rules — Nebula

## Rust Safety

### `unsafe` Code
- `#![deny(unsafe_code)]` on every library crate — no exceptions without team approval
- If approved: `// SAFETY:` comment MUST explain the exact invariant, not just "this is safe"
- Every `unsafe` block must have a test that would fail (miri or loom) if the invariant broke

### No Panics in Library Code
- No `unwrap()` / `expect()` outside `#[cfg(test)]`
- Use `?` operator, `map_or_else`, or typed error variants
- For invariants provable at construction: `debug_assert!` (off in release) + safe fallback
- Convenience functions that could panic (e.g., `retry(n=0)`) → accept `NonZeroU32` instead
- `panic!` / `unreachable!` only when the condition is TRULY impossible (comment why)

### RAII Guards
- Use `defused: bool` flag pattern — NOT `mem::forget`
- `mem::forget` is panic-unsafe: panic between creation and forget → guard drops → double action
- Drop impl checks `if !self.defused` before executing cleanup
- Every guard must be tested: assert cleanup runs on normal drop AND after defuse

### Numeric Safety
- `saturating_add` / `saturating_sub` for counters — never overflow silently
- `f64` arithmetic: cap result with `.min(max)` BEFORE passing to `Duration::from_secs_f64`
  (infinity and very large values cause panic)
- `as` casts: add `#[allow(clippy::cast_*)]` only with `// Reason:` comment
- Prefer `TryFrom` / `TryInto` for fallible conversions at API boundaries

### Atomics
- `Ordering::Relaxed` — only for counters where exact ordering doesn't matter
- `Ordering::Acquire` / `Release` — for synchronization pairs (load-after-store)
- `Ordering::SeqCst` — almost never needed; if you think you need it, document why
- Atomic + Mutex double-check pattern: read lock → check → drop → write lock → re-check

## Application Security

### Secrets
- Credentials encrypted at rest: AES-256-GCM, `SecretString` zeroizes on drop
- `Debug` impls on types containing secrets MUST redact: `[REDACTED]`, never print values
- No secrets in `tracing` spans or log messages — use field-level redaction
- No secrets in error messages — wrap in opaque error type

### Input Validation
- Validate at system boundaries: API handlers, plugin interfaces, deserialization
- Config types: validate in constructor (`new() -> Result`), not in setters
- `serde::Deserialize` on configs — consider `#[serde(try_from = "RawConfig")]` for validated deserialization
- Size/depth limits on untrusted deserialization (prevent DoS via deeply nested JSON)
- Path traversal: never `Path::join` with user input without canonicalizing first

### Error Information Leakage
- Internal errors → generic message to external callers, full details to logs
- Stack traces: never expose to API consumers
- Config validation errors: OK to show field name + constraint, NOT the value if sensitive

### Dependencies
- `cargo deny check` must pass — advisories, licenses, bans
- `cargo audit` in CI — fail on known vulnerabilities
- New deps: check for `unsafe`, review transitive tree size
- Pin versions: `"1.2"` not `"*"` — reproducible builds

## Concurrency Safety

### Mutex/Lock Patterns
- `parking_lot::Mutex` — OK in async if no `.await` under lock (short critical sections)
- `tokio::sync::Mutex` — required if holding across `.await` points
- `tokio::sync::RwLock` — for async read-heavy access patterns
- Lock ordering: if taking multiple locks, always same order to prevent deadlock
- Drop guards explicitly (`drop(guard)`) before calling functions that might take other locks

### Cancel Safety
- Document `# Cancel safety` on every async method using `select!`
- If not cancel-safe: state what leaks when the future is dropped
- RAII guards (`ProbeGuard`, `WaitCountGuard`, `GateGuard`) must release on drop
- `select!` with `biased;` — document why the priority order matters

### Shared State
- `Arc<T>` where `T: Send + Sync` — always verify both bounds
- `Arc<Mutex<T>>` — consider whether a channel (`mpsc`, `watch`) would be simpler
- Interior mutability: prefer `AtomicU64` for counters over `Mutex<u64>`
- State machines: all transitions must be tested, especially error → recovery paths

## Threat Model & Attack Surface

Nebula is a workflow engine executing user-defined workflows with plugins, webhooks, API endpoints, and external service integrations. Attack vectors:

### Injection Attacks

**Command Injection**
- NEVER pass user input to `std::process::Command` without allowlisting
- Expression engine (`nebula-expression`): sandbox evaluation, no access to `std::process`, `std::fs`, `std::net`
- Plugin names, action keys: validate against `[a-zA-Z0-9_-]` regex, reject everything else
- Workflow node parameters: treat as DATA, never as CODE

**SQL Injection**
- All database queries via parameterized statements (`sqlx::query!` / `query_as!`)
- NEVER string-format user input into SQL — `format!("SELECT * FROM {table}")` is forbidden
- `nebula-storage` PostgreSQL backend: all queries parameterized at compile time

**Template / Expression Injection**
- Workflow expressions (e.g., `{{input.name}}`) must be evaluated in a sandboxed context
- No access to environment variables, filesystem, or network from expression evaluation
- Depth limit on nested expression evaluation (prevent stack overflow via `{{{{...}}}}`)
- Output encoding: expressions producing HTML must be escaped before rendering

**Header Injection (CRLF)**
- `nebula-webhook`: validate all user-provided header values — reject `\r\n`
- HTTP response headers: never include raw user input without sanitization

### Authentication & Authorization

**Credential Storage**
- All credentials encrypted at rest: AES-256-GCM with per-credential random nonce
- `SecretString` zeroizes memory on drop — no plaintext in heap after use
- Key derivation: use `argon2` or `scrypt`, never raw SHA/MD5
- OAuth2 tokens: store `access_token` + `refresh_token` encrypted, never in logs

**Timing Attacks**
- Secret comparison (API keys, tokens, HMAC): use `constant_time_eq` or `subtle::ConstantTimeEq`
- NEVER use `==` for secret comparison — timing side-channel leaks prefix length
- OAuth2 state parameter: compare with constant-time equality

**Session / Token Security**
- Webhook signatures: HMAC-SHA256, verify before processing payload
- API tokens: sufficient entropy (≥256 bits), rotate on compromise
- CSRF: webhook endpoints validate `Content-Type` + signature, not cookies

### Deserialization & Input Handling

**Deserialization of Untrusted Data**
- `serde_json::from_str` on user input: set `serde_json::StreamDeserializer` with size limits
- Workflow definitions: validate schema + depth limit before deserializing
- Plugin manifests: validate against known schema, reject unknown fields
- Max sizes: JSON body ≤ 10MB, nested depth ≤ 32 levels, array length ≤ 10,000 items

**Denial of Service via Input**
- Rate limit all public endpoints (`nebula-resilience::RateLimiter`)
- Workflow execution: timeout per node, total workflow timeout, max node count
- Expression evaluation: max recursion depth, max output size
- Regex in user input: use `regex` crate (guaranteed linear time), never `fancy-regex` on untrusted patterns

**Path Traversal**
- NEVER `Path::join(user_input)` without canonicalizing and checking prefix
- Plugin file access: restrict to plugin's sandbox directory
- `std::fs::canonicalize` THEN check `starts_with(allowed_root)`
- Reject paths containing `..`, null bytes, or non-UTF8 sequences

### Supply Chain & Dependencies

**Dependency Attacks**
- `cargo deny check` in CI — advisories, license compliance, ban list
- `cargo audit` — known CVE detection
- New deps: review source, check for `unsafe`, assess transitive tree
- Lock file (`Cargo.lock`) committed — reproducible builds
- Typosquatting: verify crate name matches expected publisher on crates.io

**Build Security**
- CI runs in isolated containers — no access to production secrets
- Release binaries: reproducible builds where possible
- No `build.rs` that downloads external code at compile time
- Feature flags: `default = []` — opt-in to capabilities, not opt-out

### Plugin Sandboxing

**Current: `InProcessSandbox`** (Phase 2)
- Plugins run in-process — no OS-level isolation yet
- Resource limits enforced via `Bulkhead` (concurrency) and `Timeout`
- Plugin I/O: mediated through `ActionContext` — no direct filesystem/network access
- Credential access: through `CredentialAccessor` with scope-based filtering

**Future: OS-process / WASM isolation** (Phase 3, ADR 008)
- Each plugin in separate process or WASM sandbox
- Capability-based: explicit permissions for network, filesystem, secrets
- Memory limits, CPU time limits enforced by runtime

### Logging & Observability Security

- NEVER log credential values, tokens, API keys, passwords
- `SecretString` fields: `Debug` impl prints `[REDACTED]`
- `tracing` spans: use `.redacted()` for fields that might contain PII
- Error messages to external callers: generic ("internal error"), details to internal logs only
- Audit trail: log WHO accessed WHICH credential WHEN (via `AuditLayer`)

## Patterns That Had Real Bugs (Nebula History)

These are NOT theoretical — each caused an actual bug in production-quality code:

| Pattern | Bug | Fix |
|---------|-----|-----|
| `mem::forget(guard)` | Panic between creation and forget → double cleanup | `defused: bool` flag |
| `Duration::from_secs_f64(uncapped)` | Exponential backoff → infinity → panic | `.min(max_secs)` before conversion |
| `count_X_as_Y = false` → early return | Skipped total counter AND probe slot release | Release slot before early return |
| `Rng::with_seed(s)` per call | Same seed → same output → no jitter variance | Mix seed with attempt number |
| `Pipeline` new config field | `total_budget` silently dropped in retry step | Propagate explicitly: `inner.field = config.field` |
| `current_rate()` returns stale value | Doesn't account for elapsed refill/leak | Recompute at observation time |
| `with_burst(n)` vs `capacity` | Only one of two related fields updated | Update both, or make one derived from the other |
| `Operation(()) → Cancelled` | Wrong variant — `is_cancellation()` returns false positive | Dedicated `FallbackFailed` variant |
| `HashMap` for 9 keys | Hashing overhead > linear scan | `Vec<(K, V)>` for small key spaces |
| `RecordingSink::count()` | Clones entire Vec to filter | Filter under lock |
