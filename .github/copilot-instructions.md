# Copilot Instructions for Nebula

## Project Context
Modular type-safe Rust workflow engine. Edition 2024, MSRV 1.94, alpha stage.
Architecture: Core → Business → Exec → API (one-way deps, no upward).
Universal data type: serde_json::Value.
Error handling: thiserror in libs, anyhow in binaries.

## What to Flag in Reviews

### Critical (always comment)

1. **Layer violations** — `crates/core/*` importing from `crates/engine/*` etc.
   Check Cargo.toml dependencies against the layer hierarchy:
   `core < business (credential/resource/action/plugin) < exec (engine/runtime/storage/sandbox/sdk/plugin-sdk) < api`.
2. **Panic in library code** — `unwrap()`, `expect()`, `panic!()`, indexing without bounds check, `unreachable!()` outside exhaustive match.
   Exception: `#[cfg(test)]` and binary crates allowed.
3. **Silent error suppression** — `let _ = result;` on `Result`, `.ok()` discarding meaningful errors, `.unwrap_or_default()` on fallible IO/parse.
4. **Direct state mutation in execution/engine** — `node_state.state = X` without going through `transition_node()`. Loses version bump. See past incident #255.
5. **Missing `Send + Sync`** on async types in runtime/engine paths.
6. **Untrusted Duration** — `Duration::from_secs_f64(user_input)` without clamping (NaN/inf/negative panics).

### Useful (comment if confident)

7. **Logical bugs in new code** — off-by-one, wrong comparison operator, swapped args.
8. **Missing edge case tests** — but only when adding new public API or branching logic. Be specific: name the case.
9. **Public API without doc comment** — only on `pub fn`/`pub struct`, only when fn name doesn't fully describe behavior or error contract.

## What NOT to Flag (stop-list)

DO NOT comment on:

- **Style / formatting** — rustfmt + nightly handles this. Never suggest reformatting.
- **Naming preferences** — no "consider renaming X to Y" unless name is actively misleading.
- **Generic suggestions** — no "consider adding logging", "consider error handling", "consider tests" without naming a specific case.
- **Missing comments on private code** — internal fns don't need doc comments.
- **README / CHANGELOG updates** — separate process.
- **Suggesting `#[derive(Debug)]`** etc — assume it's there for a reason if absent.
- **Test file nits** — naming, ordering, helper extraction — none of it.
- **Things CodeRabbit will catch** — secret handling in `crates/credential/**`, lock ordering in `crates/{engine,runtime,execution}/**`, sandbox escapes in `crates/sandbox/**`. Skip these — CodeRabbit owns them.
- **MSRV checks on syntax** — CI MSRV job catches it; don't comment on use of recent features unless there's actual breakage.

## Project-Specific Patterns

### Metrics

Single path: `nebula-telemetry::MetricsRegistry` → `nebula-metrics` (Prometheus export).
Domain crates consume via DI: `Option<Arc<MetricsRegistry>>`.
Flag PRs that introduce alternate metrics stacks.

### Errors

- Library crates: `thiserror` enums with `#[from]` for layer transitions.
- Binary crates: `anyhow::Result` only at the boundary.
- Errors must include actionable context — don't comment "add context" generically; only comment when error swallows the cause.

### No "Value" crate

Universal interchange = `serde_json::Value`. Flag PRs introducing `enum Value { ... }` or wrapper types.

## Test Conventions

- `cargo nextest run` for unit tests, `cargo test --doc` for doctests.
- Memory backends are real (concrete impls), not mocks. Don't suggest replacing them with mockall.
- Integration tests live in `tests/`, not `src/`.

## Avoid Suggesting

- `unsafe` blocks (require explicit `SAFETY:` comment + justification — only flag if missing on existing unsafe).
- `Rc<T>` in async paths (use `Arc<T>`).
- Heavy mocks instead of memory backends.
- Alternate metrics stacks.
- A separate "value" crate.
- Bringing back removed `.project/*` conventions.
