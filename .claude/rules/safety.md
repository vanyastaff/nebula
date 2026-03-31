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
