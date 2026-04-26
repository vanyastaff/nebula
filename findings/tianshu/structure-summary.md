# Tianshu-rs — Structure Summary

## Crate count: 6

| Crate | Lines of Rust (approx) | Role |
|-------|------------------------|------|
| `tianshu` (workflow_engine) | ~6,800 | Core engine |
| `tianshu-postgres` (workflow_engine_postgres) | ~700 | DB adapters |
| `tianshu-llm-openai` (workflow_engine_llm_openai) | ~800 | LLM adapter |
| `tianshu-observe` (workflow_engine_observe) | ~600 | Observability |
| `tianshu-dashboard` (workflow_engine_dashboard) | ~800 | HTTP dashboard |
| `approval_workflow` (example) | ~900 | Full example |
| **Total** | **~11,623** | 64 `.rs` files |

## LOC breakdown

Source: `find . -name "*.rs" | xargs wc -l` — 11,623 total across 64 files.

## Test count

75 `#[test]` functions across 15 test files. `#[tokio::test]` used for async tests. PostgreSQL integration tests tagged with `#[ignore]`.

## Key source files

- `crates/workflow_engine/src/workflow.rs` — `BaseWorkflow` trait, `WorkflowResult`, `PollPredicate`
- `crates/workflow_engine/src/context.rs` — `WorkflowContext` (600+ lines, the workhorse)
- `crates/workflow_engine/src/engine.rs` — `SchedulerV2`, 4-phase tick loop
- `crates/workflow_engine/src/llm.rs` — `LlmProvider`, `StreamingLlmProvider`, message types
- `crates/workflow_engine/src/tool.rs` — `Tool`, `ToolRegistry`, `ToolSafety`
- `crates/workflow_engine/src/retry.rs` — `RetryPolicy`, `ErrorClass`, `with_retry()`
- `crates/workflow_engine/src/compact.rs` — `ManagedConversation`, compaction strategies
- `crates/workflow_engine/src/poll.rs` — `PollEvaluator`, `ResourceFetcher`, `IntentRouterV2`
- `crates/workflow_engine/src/stage.rs` — `StageBase<S>`, `run_stages()` helper

## Dependencies (top-10 from workspace)

1. `tokio` — async runtime
2. `async-trait` — dyn trait async methods
3. `serde` / `serde_json` — serialization (JsonValue as universal I/O)
4. `anyhow` — error handling throughout
5. `tracing` — structured logging
6. `deadpool-postgres` — Postgres connection pool
7. `tokio-postgres` — raw Postgres driver
8. `reqwest` — HTTP client for LLM API calls
9. `axum` — HTTP server for dashboard
10. `chrono` / `uuid` — timestamps and IDs

## Git log (recent 20 commits)

```
b5c21c8 Merge pull request #9 from Desicool/feat/dashboard
e1b8215 feat(example): keep dashboard alive after demo + add E2E test script
46bcaeb chore: update Cargo.lock for tianshu-dashboard dependencies
e789747 feat(example): add --dashboard flag to approval_workflow
564de65 feat(dashboard): add CaseRow and StatsResponse re-exports
22a433f feat: add tianshu-dashboard crate with HTTP API and web UI
e5cd21f Merge pull request #7 from Desicool/docker
abfed3b docker: rewrite Dockerfile and docker-compose
f9f4e34 Merge pull request #6 from Desicool/use
095c803 style: run cargo fmt across workspace
d5c4ab8 docs: update READMEs for crates.io — tianshu-* names
fa53852 chore: rename crates to tianshu-* for crates.io publishing
5621b9f chore: switch to Apache-2.0 license
1b9b246 Merge pull request #5 from Desicool/feature/agent-definitions
16e26b0 docs: add pipeline routing to CLAUDE.md and AGENTS.md
f6050dc Merge pull request #4 from Desicool/feature/agent-definitions
0be8618 feat: add Claude Code agent definitions and concept docs
10d7cbd Merge pull request #3 from Desicool/plan_and_execute
4f374ba fix: per-tool timing in ToolCallRecord and fallback deduplication
14203bb Merge pull request #1 from Desicool/plan_and_execute
```
