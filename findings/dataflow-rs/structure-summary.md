# dataflow-rs — Structure Summary

## Crate inventory

**2 crates** (workspace members: `.` and `wasm/`):

| Crate | Type | Purpose |
|-------|------|---------|
| `dataflow-rs` | library (`lib.rs`) | Core rules engine — Workflow, Task, Engine, AsyncFunctionHandler, built-in functions |
| `dataflow-wasm` | cdylib + rlib (`wasm/src/lib.rs`) | wasm-bindgen wrapper for browser execution of the engine |

No binary crates. dataflow-rs is a **pure Rust library** — consumers embed it and call `Engine::process_message()`. No server, no CLI, no standalone binary.

## Layer separation

Single-crate monolith for the core engine:

```
src/engine/
├── mod.rs           Engine struct — process_message, with_new_workflows, channel routing
├── compiler.rs      LogicCompiler — JSONLogic pre-compilation at startup
├── executor.rs      InternalExecutor — built-in map/validate/filter/log execution
├── workflow_executor.rs WorkflowExecutor — condition evaluation, task orchestration, audit trail
├── task_executor.rs TaskExecutor — built-in vs custom function dispatch
├── workflow.rs      Workflow struct (id, name, priority, condition, tasks, status, channel, version, tags)
├── task.rs          Task struct (id, name, condition, function, continue_on_error)
├── message.rs       Message struct (payload, context{data/metadata/temp_data}, audit_trail, errors)
├── trace.rs         ExecutionTrace — per-step debug snapshots
├── error.rs         DataflowError enum (10 variants), ErrorInfo, retryable() method
├── utils.rs         get_nested_value / set_nested_value helpers
└── functions/
    ├── mod.rs        AsyncFunctionHandler trait, builtins registry
    ├── config.rs     FunctionConfig untagged enum (12 variants)
    ├── map.rs        MapConfig / MapMapping — JSONLogic data transformation
    ├── validation.rs ValidationConfig / ValidationRule — read-only rule checking
    ├── parse.rs      ParseConfig — JSON/XML payload parsing
    ├── publish.rs    PublishConfig — JSON/XML serialization to string
    ├── filter.rs     FilterConfig — pipeline halt/skip control flow
    ├── log.rs        LogConfig — structured logging with JSONLogic expressions
    └── integration.rs HttpCallConfig, EnrichConfig, PublishKafkaConfig
```

## Dependency highlights

Key direct dependencies:
- `datalogic-rs = "4.0"` — JSONLogic compiler/evaluator (first-party: GoPlasmatic)
- `tokio = "1"` with `rt` + `macros` (dev: `full`)
- `async-trait = "0.1"`
- `serde` / `serde_json = "1.0"`
- `thiserror = "2.0"`
- `chrono = "0.4"` — timestamps in audit trail
- `quick-xml = "0.37"` — XML parse/publish functions
- `uuid = "1.23"` — Message ID generation
- `log = "0.4"` — standard logging facade

Notable: no database dependency, no HTTP client dependency in core library (reqwest is only in dev-dependencies example). The `HttpCallConfig` typed config exists but the actual HTTP call is expected to be provided by the caller as an `AsyncFunctionHandler`.

## Lines of code

- Total Rust files: 30 (src + wasm + tests + examples)
- Total Rust LOC: ~9,822 (all files, including blanks/comments)
- Core engine src (excluding examples/tests): ~3,500 LOC estimated
- Test count: 91 `#[test]` / `#[tokio::test]` assertions

## Feature flags

- `default = []` — no features on by default
- `wasm-web` — enables `chrono/wasmbind`, `getrandom/wasm_js`, `uuid/js` for browser targets

## Related packages

- `@goplasmatic/dataflow-wasm` — npm WASM package (wasm-bindgen compiled)
- `@goplasmatic/dataflow-ui` — React component library for rule visualization (`ui/` directory)
- `datalogic-rs` — JSONLogic engine (same organization, GoPlasmatic)
