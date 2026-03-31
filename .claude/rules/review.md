# Code Review Checklist — Nebula

When reviewing code (own or others'), verify each category.
Use this for both PR reviews and periodic crate audits.

## Correctness
- [ ] Does the change do what it claims?
- [ ] Are edge cases handled (empty input, zero, overflow, None)?
- [ ] Are error paths tested, not just happy paths?
- [ ] No silent swallowing of errors (`.ok()`, `let _ =` on Results that matter)
- [ ] No integer overflow in release mode (`debug_assert` is OFF — use `saturating_*` or `checked_*`)
- [ ] No `f64` division by zero without guard (produces NaN/Inf silently)
- [ ] RAII guards use `defused` flag pattern, not `mem::forget` (panic-safe)

## Architecture
- [ ] Layer boundaries respected — no upward deps (Core → Business → Exec)
- [ ] Cross-crate communication via `EventBus`, not direct imports between peers
- [ ] New public types in `nebula-core` approved? (cascade risk)
- [ ] DI via `Context` — no global state or singletons

## Safety & Security
- [ ] No `unwrap()` / `expect()` outside tests — use `?`, `map_or_else`, or typed error
- [ ] No hardcoded secrets, credentials, or API keys
- [ ] `unsafe` blocks have `// SAFETY:` comment explaining the invariant
- [ ] External input validated at boundaries
- [ ] Secrets not leaked in `Debug` output — sensitive types redact
- [ ] Deserialization of untrusted input has size/depth limits

## API Surface (Rust API Guidelines)
- [ ] Public API has doc comments with `# Examples` and `# Errors`
- [ ] Breaking changes documented and intentional
- [ ] Enums that may grow are `#[non_exhaustive]`
- [ ] Error types are meaningful, not stringly-typed
- [ ] All public types implement `Debug` (C-DEBUG)
- [ ] Common traits derived where applicable: `Clone`, `PartialEq`, `Eq`, `Hash`, `Default` (C-COMMON-TRAITS)
- [ ] Config types implement `serde::Serialize` + `Deserialize` (C-SERDE)
- [ ] Constructors have `#[must_use]` (unless return type already has it)
- [ ] Getters without `get_` prefix (C-GETTER)
- [ ] Conversions follow `as_`/`to_`/`into_` conventions (C-CONV)
- [ ] Consistent method naming across similar types (e.g., all use `.call()` not mixed `.call()`/`.execute()`)
- [ ] No redundant re-exports of generic names at crate root (e.g., `Outcome` too vague)
- [ ] Extension traits documented: "new methods will have default impls" (C-SEALED alternative)

## Naming (Rust API Guidelines)
- [ ] Types: `UpperCamelCase`, functions/methods: `snake_case`, constants: `SCREAMING_SNAKE_CASE`
- [ ] No `get_` prefix on getters — use field name directly
- [ ] `can_*` returns `bool`, `try_*` returns `Result` — don't mix
- [ ] Consistent word order within the crate (verb-object-error, not mixed)
- [ ] No abbreviation inconsistency (e.g., `ops` vs `operations` in same struct)
- [ ] Feature flags without `use-`/`with-` prefix (C-FEATURE)
- [ ] No module name stuttering with crate name

## Async Correctness
- [ ] `std::sync::Mutex` not held across `.await` — use `tokio::sync::Mutex` or `parking_lot` (if no `.await` under lock)
- [ ] No blocking calls in async context (`std::thread::sleep`, `std::fs`, heavy CPU)
- [ ] `tokio::select!` branches are cancel-safe (or documented as not)
- [ ] Spawned tasks are joined or documented as fire-and-forget

## Tests
- [ ] New behavior has corresponding tests
- [ ] Test names describe behavior: `rejects_X`, `returns_Y_when_Z`
- [ ] No flaky tests (race conditions, timing deps, random data without seed)
- [ ] Integration tests use `MemoryStorage`, not mocks
- [ ] Seeded randomness varies per iteration (not same seed → same output every call)
- [ ] Edge cases tested: zero, empty, overflow, concurrent access

## Performance
- [ ] No unnecessary `clone()` in hot paths
- [ ] Async functions don't block the runtime (`spawn_blocking` for CPU work)
- [ ] No unbounded collections growing without limit
- [ ] `HashMap` for ≤10 keys → consider `Vec` with linear scan
- [ ] No allocation inside lock (clone after drop, or use `Arc`)
- [ ] Capacity hints on `Vec::with_capacity` / `String::with_capacity` in hot paths

## Concurrency
- [ ] Atomic operations use correct `Ordering` (`Relaxed` only for counters, `Acquire`/`Release` for synchronization)
- [ ] No lock ordering violations (potential deadlock between multiple mutexes)
- [ ] RAII guards release resources on drop (cancel-safe)
- [ ] `Duration` arithmetic checked for overflow (`.min(cap)` before `from_secs_f64`)

## Hygiene
- [ ] `cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace` passes
- [ ] `cargo bench --no-run -p <crate>` passes (bench contracts not broken)
- [ ] Commit messages follow conventional commits
- [ ] `.claude/crates/{name}.md` updated if invariants/decisions/traps changed
- [ ] Crate docs (`docs/`, `README.md`, `lib.rs`) reflect current API
