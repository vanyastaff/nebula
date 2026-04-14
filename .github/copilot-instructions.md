# Copilot Instructions for Nebula

## Project Overview

Nebula is a modular, type-safe workflow automation engine in Rust (like n8n/Zapier). 25-crate workspace with strict one-way layer dependencies. Edition 2024, MSRV 1.94.

## Architecture Layers

```
API layer          api · webhook
Exec layer         engine · runtime · storage · sdk
Business layer     credential · resource · action · plugin
Core layer         core · validator · parameter · expression · memory · workflow · execution
Cross-cutting      log · system · eventbus · telemetry · metrics · config · resilience · error
```

No upward dependencies. Cross-cutting crates importable at any layer.

## Key Conventions

- **Universal data type:** `serde_json::Value` — no custom value crate
- **Errors:** `thiserror` in libraries, `anyhow` in binaries
- **No `unwrap()` / `expect()`** outside tests — use `?` or typed errors
- **Testing:** `cargo nextest run` (not `cargo test`), test names describe behavior
- **Naming:** no `get_` prefix on getters, `can_`* → bool, `try_*` → Result
- **Clippy:** zero warnings policy (`-D warnings`)
- **Doc comments** on all public items with `# Examples` and `# Errors` sections

## Code Review Focus

When reviewing PRs, prioritize:

1. **Layer violations** — no upward deps (Core → Business → Exec → API)
2. **Panic safety** — no `unwrap()`/`expect()` in lib code
3. **Error handling** — no silent `.ok()`, `let _ =` on Results that matter
4. **Naming consistency** — conventional commits in PR title, Rust API Guidelines
5. **Missing tests** for new behavior
6. `**Duration` overflow** — `.min(cap)` before `from_secs_f64`
7. **Credential/secret safety** — no secrets in Debug output or logs

## Metrics System

Single unified path: `nebula-telemetry` (MetricsRegistry) → `nebula-metrics` (naming + Prometheus export). Domain crates receive `Option<Arc<MetricsRegistry>>` via DI. No custom atomic counter structs.

## What NOT to Suggest

- Adding `unsafe` code (all lib crates have `#![forbid(unsafe_code)]`)
- Using `Rc<T>` (everything is `Send` — use `Arc<T>`)
- Mocking the Storage trait (use `MemoryStorage` in tests)
- Adding the `metrics` ecosystem crate (removed — use `nebula-telemetry`)
- Creating a `nebula-value` crate (removed — use `serde_json::Value`)

