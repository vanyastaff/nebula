# Copilot Instructions for Nebula

## Project Overview

Nebula is a modular, type-safe workflow automation engine in Rust. The workspace is in active development (edition 2024, MSRV 1.94) with one-way layer dependencies enforced by CI policy and review.

## Architecture Layers

```
API layer          api (webhook is a module inside nebula-api)
Exec layer         engine · runtime · storage · sandbox · sdk · plugin-sdk
Business layer     credential · resource · action · plugin
Core layer         core · validator · parameter · expression · workflow · execution
Cross-cutting      log · system · eventbus · telemetry · metrics · config · resilience · error
```

No upward dependencies. Cross-cutting crates are importable at any layer.

## Key Conventions

- **Universal data type:** `serde_json::Value` (no custom value crate)
- **Errors:** `thiserror` in libraries, `anyhow` in binaries
- **No `unwrap()` / `expect()`** outside tests
- **Testing:** `cargo nextest run` for tests, `cargo test --workspace --doc` for doctests
- **Naming:** no `get_` prefix on getters; `can_*` returns bool; `try_*` returns `Result`
- **Quality gate:** `cargo +nightly fmt --all`, `cargo clippy --workspace -- -D warnings`
- **Doc comments:** public APIs should include behavior and error contract

## Code Review Focus

When reviewing PRs, prioritize:

1. **Layer violations** — no upward deps (Core -> Business -> Exec -> API)
2. **Panic safety** — no `unwrap()`/`expect()` in library code
3. **Error handling** — avoid silent `.ok()` and ignored meaningful `Result`s
4. **Coverage** — new behavior should include focused tests
5. **Duration safety** — clamp before `Duration::from_secs_f64` when values can be untrusted
6. **Credential/secret safety** — no secrets in logs, debug output, or error text

## Metrics System

Unified path: `nebula-telemetry` (`MetricsRegistry`) -> `nebula-metrics` (naming + Prometheus export). Domain crates should consume metrics via DI (`Option<Arc<MetricsRegistry>>`).

## What Not to Suggest

- Introducing `unsafe` unless explicitly justified and documented with `SAFETY:`
- Using `Rc<T>` in async/runtime paths that require `Send + Sync`
- Replacing in-memory test backends with heavy mocks when concrete memory backends exist
- Introducing alternate metrics stacks instead of the `nebula-telemetry` + `nebula-metrics` path
- Introducing a dedicated "value" crate instead of `serde_json::Value`

