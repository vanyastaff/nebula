# Open-Source Quality Standards — Nebula

## Documentation

- Every public item must have a doc comment (`///`) — CI enforces `missing_docs`
- Doc comments describe **what** and **why**, not **how** (the code shows how)
- Include a `# Examples` section for non-trivial public APIs
- `# Errors` section for fallible functions listing when each error variant is returned
- `# Panics` section if the function can panic (should be rare outside tests)
- `# Cancel safety` section on async methods using `select!` or cancellation

## API Design (Rust API Guidelines)

- Public API surface is a contract — treat additions as permanent
- Mark experimental APIs with `#[doc(hidden)]` or gate behind a feature flag
- Deprecate before removing: `#[deprecated(since = "0.x.0", note = "use Y instead")]`
- Re-exports in `lib.rs` define the public API — internal modules stay `pub(crate)`
- Exhaustive enums get `#[non_exhaustive]` if they may grow
- All public types implement `Debug` — manual impl with `finish_non_exhaustive()` for types with closures/`Arc<dyn>`
- Config types implement `serde::Serialize` + `Deserialize`
- Common traits derived eagerly: `Clone`, `PartialEq`, `Eq`, `Hash`, `Default`, `Copy` where applicable
- Constructors have `#[must_use]` (unless return type like `Result` already has it)
- Builder methods have `#[must_use = "builder methods must be chained or built"]`

## Naming (Rust API Guidelines)

- Getters: no `get_` prefix — `state()` not `get_state()`
- `can_*` → returns `bool`; `try_*` → returns `Result` — never mix
- Conversions: `as_` (free, borrowed→borrowed), `to_` (expensive), `into_` (consuming)
- Consistent verb: all patterns use `.call()`, not mixed `.call()`/`.execute()`
- No module name stuttering with crate name
- Feature flags: `governor` not `use-governor` or `with-governor`
- Word order consistent within crate (same struct: `max_half_open_operations` not `half_open_max_ops`)

## Error Handling

- Libraries: `thiserror` with typed error enums
- No `unwrap()` / `expect()` outside tests — use `?`, `map_or_else`, or `NonZero*` types
- No `panic!` in library code — return `Result` or use `debug_assert`
- RAII guards: use `defused: bool` flag, not `mem::forget` (panic-safe)
- Error types are `#[non_exhaustive]` — use `flat_map_inner` pattern for variant remapping
- Extension traits: document "new methods will always have default impls"

## Clippy & Formatting

- `cargo clippy --workspace -- -D warnings` must pass (zero warnings policy)
- `cargo fmt --all` with `rustfmt.toml` config (max_width=100, edition 2024)
- Clippy config in `clippy.toml`: cognitive-complexity ≤25, nesting ≤5, fn-params ≤7

## Performance

- No unnecessary `clone()` in hot paths — use references or `Cow<'_, T>`
- `HashMap` for ≤10 keys → `Vec` with linear scan
- No allocation inside lock — clone after drop, or use `Arc`
- Capacity hints: `Vec::with_capacity`, `String::with_capacity` in hot paths
- `Duration` arithmetic: `.min(cap)` before `from_secs_f64` (prevents overflow panic)
- Seeded randomness: mix seed with iteration/attempt (not identical output every call)
- `RecordingSink::count()` — filter under lock, don't clone the Vec

## Concurrency

- Atomics: `Relaxed` only for counters; `Acquire`/`Release` for synchronization pairs
- `parking_lot::Mutex` OK in async if no `.await` under lock (short critical sections)
- `tokio::sync::RwLock` for async-aware read/write access
- Lock-free fast path + double-checked write lock for rate adjustment pattern
- `select!` cancel safety documented on every async method that uses it

## Dependency Hygiene

- `cargo deny check` must pass — licenses, advisories, bans, sources
- Allowed licenses: MIT, Apache-2.0, BSD-2/3, ISC, Zlib, MPL-2.0, Unlicense, CC0
- No `*` version requirements — pin to `"major.minor"` minimum
- Audit new deps: check download count, maintenance status, transitive tree size

## Security

- Credentials encrypted at rest (AES-256-GCM), `SecretString` zeroizes on drop
- No `unsafe` without a `// SAFETY:` comment explaining the invariant
- No `println!` / `eprintln!` in library code — use the `nebula-log` infrastructure
- Sanitize all external input at system boundaries (API handlers, plugin interfaces)
- Secrets not in `Debug` output — implement manual `Debug` with redaction

## MSRV

- rust-version 1.94 — CI runs `cargo check` with this exact version
- Don't use nightly features or unstable APIs
- If a dep bumps its MSRV above ours, pin the older version or find an alternative
