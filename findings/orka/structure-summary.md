# orka — Structure Summary

## Repository

- GitHub: https://github.com/excsn/orka
- Created: 2025-05-17 (first commit)
- Last updated: 2025-12-10
- Stars: 5 / Forks: 4
- License: MPL-2.0 (per core/Cargo.toml; missing from GitHub top-level license field)
- crates.io: published as `orka` v0.1.0 (665 total downloads, 96 recent)

## Workspace Members

| Crate path | Purpose |
|-----------|---------|
| `core` (published as `orka`) | Library — pipeline engine core |
| `examples/ecommerce_app` | Binary — integration example |

Total: 2 workspace members. Only 1 library crate.

## Source File Count

- Total `.rs` files: 62
- Total LOC: ~7,822 (tokei not available; counted with `wc -l`)

## Key source modules (core/src/)

```
src/
  lib.rs            (73 lines) — re-exports public API
  error.rs          (69 lines) — OrkaError enum, OrkaResult alias
  registry.rs       (178 lines) — Orka<E> type-keyed registry
  core/
    mod.rs           — re-exports
    context.rs       — Handler<TData,Err> type alias, AnyContextDataExtractor
    context_data.rs  — ContextData<T> = Arc<RwLock<T>> wrapper
    control.rs       — PipelineControl, PipelineResult enums
    pipeline_trait.rs— AnyPipeline<E> trait
    step.rs          — StepDef<T>, SkipCondition<T>
  pipeline/
    definition.rs    — Pipeline<TData,Err> struct + construction methods
    execution.rs     — Pipeline::run() implementation
    hooks.rs         — before_root, on_root, after_root, set_extractor, on<SData>
    mod.rs
  conditional/
    mod.rs
    builder.rs       — ConditionalScopeBuilder + ConditionalScopeConfigurator
    provider.rs      — PipelineProvider trait, StaticPipelineProvider, FunctionalPipelineProvider
    scope.rs         — ConditionalScope<T,S,E>, AnyConditionalScope trait
```

## Dependencies

Runtime: `anyhow` ^1.0, `thiserror` ^2.0, `async-trait` ^0.1.77, `tracing` ^0, `parking_lot` ^0

Dev: `criterion` 0.5 (benchmarks), `tokio` 1, `tracing-subscriber` 0.3, `once_cell` 1.10, `serial_test` 3.2.0

No database, network, or crypto dependencies. No serde in core (example app uses serde_json).

## Test Count

Test files: 5 (`pipeline_execution_tests.rs`, `conditional_scope_tests.rs`, `context_management_tests.rs`, `error_handling_tests.rs`, `registry_tests.rs`)
Example files: 6 in core/examples/ + full ecommerce_app example

## Commit History

Only 2 commits visible (depth 50 clone showed all). Very new project.

## Maintainers

Primary: `normano` (Excerion Sun, dev@excsn.com) — 1 contributor total.
