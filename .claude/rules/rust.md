# Rust Coding Rules — Nebula

## Edition & MSRV
- Edition 2024, `rust-version = "1.93"` — never use features beyond this
- Check CI runs MSRV gate: `cargo check` with Rust 1.93

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

## Dependencies
- New deps require: MIT/Apache-2.0 compatible license, check `deny.toml`
- Prefer `parking_lot` over `std::sync::Mutex`
- `tokio` for async runtime — no mixing with `async-std`
- Check `cargo deny check` passes before adding new dependencies

## Performance
- `Arc<T>` for shared ownership — never `Rc<T>` (everything is `Send`)
- Avoid `clone()` in hot paths — use references or `Cow<'_, T>`
- `Box::pin` for async trait return types
