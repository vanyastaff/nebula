# Rust Coding Rules — Nebula

## Edition & MSRV
- Edition 2024, `rust-version = "1.94"` — never use features beyond this
- Check CI runs MSRV gate: `cargo check` with Rust 1.94

## Error Handling
- Libraries (`crates/`): `thiserror` with typed error enums per crate
- Binaries / top-level: `anyhow::Result`
- Never `unwrap()` / `expect()` outside tests — use `?` or explicit error variants
- Error types are public API — changing them is a breaking change

## Type Design
- Newtypes for IDs: `NodeId(Uuid)`, `ActionKey(String)` — never raw primitives in public API
- Use `Validated<T>` or builder pattern for configs requiring validation
- Prefer `&str` over `String` in function parameters; `impl Into<String>` for constructors
- `serde_json::Value` is the universal data type — don't invent wrappers around it

## Traits
- Trait changes in `nebula-core` cascade to 25+ crates — treat as breaking
- Default impls should do something useful or not exist — no empty defaults
- `Send + Sync` bounds on async traits unless there's a documented reason not to

## Testing
- Unit tests in `mod tests` inside the source file
- Integration tests in `tests/` directory of the crate
- Test names: `snake_case` describing behavior, not the function: `rejects_negative_timeout` not `test_config`
- Use `#[should_panic(expected = "...")]` sparingly — prefer `assert!(matches!(result, Err(...)))`
- `MemoryStorage` for test-only storage — never mock the `Storage` trait directly

### When a test fails
**Never adjust test expectations to match broken logic.** A failing test is a signal — investigate:
1. **Is the logic wrong?** — fix the implementation, not the test
2. **Is the test wrong?** — only if the test's assumptions were incorrect from the start
3. **Never** change assertions just to get a green pass — that hides bugs

## Dependencies
- New deps require: MIT/Apache-2.0 compatible license, check `deny.toml`
- Prefer `parking_lot` over `std::sync::Mutex`
- `tokio` for async runtime — no mixing with `async-std`
- Check `cargo deny check` passes before adding new dependencies

## Clippy Discipline

Zero warnings policy — `cargo clippy --workspace -- -D warnings`.

### `#[allow(...)]` rules
When clippy fires, always try to **fix the code first**:
1. **Refactor** — restructure, extract, simplify to satisfy the lint
2. **Only if impossible** — add `#[allow(...)]` with a `// Reason:` comment explaining why the code cannot be improved

Never add `#[allow(...)]` just because "it works" or to make CI green faster. If clippy complains, the code is almost always improvable.

### Allowed exceptions (only after refactor attempt failed)
- `#[allow(clippy::excessive_nesting)]` — only for match arms on deep enums and async closures that can't be flattened
- `#[allow(clippy::too_many_arguments)]` — only for internal constructors where a builder would be overengineering
- `#[allow(clippy::type_complexity)]` — only for trait-associated types with unavoidable generics
- `#[allow(clippy::cognitive_complexity)]` — never. Refactor the function instead.

### Nesting
- Use early return (`let-else`, guard clauses) to keep nesting ≤ 3 levels
- Extract inner logic into named functions when nesting grows
- `clippy.toml` threshold is 5 — but aim for 3

### Complexity
- `cognitive-complexity-threshold = 25` in `clippy.toml` — if you hit it, split the function
- `too-many-lines-threshold = 100` — functions over 100 lines need splitting
- `too-many-arguments-threshold = 7` — use a config struct or builder above 4 params

## Performance
- `Arc<T>` for shared ownership — never `Rc<T>` (everything is `Send`)
- Avoid `clone()` in hot paths — use references or `Cow<'_, T>`
- `Box::pin` for async trait return types
