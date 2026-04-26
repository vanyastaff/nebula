# Structure Summary — orchestral

## Crate count: 6

| Crate | Path | Role |
|-------|------|------|
| orchestral-core | core/orchestral-core | Pure abstractions: traits, DAG, stores, types. No I/O. |
| orchestral-runtime | core/orchestral-runtime | LLM planners, actions, orchestrator, agent loop |
| orchestral | core/orchestral | Facade re-exporting core + runtime |
| orchestral-cli | apps/orchestral-cli | CLI + TUI (ratatui/clap), scenario runner |
| orchestral-telegram | apps/orchestral-telegram | Telegram bot adapter |
| examples | examples/ | Runnable SDK demos |

## LOC

tokei not run. From file inspection: orchestral-runtime/sdk.rs is 25,679 bytes; thread_runtime.rs is 25,679 bytes; mcp.rs is 39,520 bytes. Total estimated source: ~100-150 KLOC across all crates (rough).

## Key dependencies

- tokio 1 (full features) — async runtime
- async-trait 0.1 — trait object async support
- serde + serde_json — serialization (all I/O type-erased via Value)
- thiserror 1 — error types
- tracing 0.1 — logging/debug
- reqwest 0.12 — HTTP client (LLM API calls + HTTP MCP)
- llm_sdk (`graniet/llm`) — multi-provider LLM abstraction
- ratatui — TUI for CLI
- tokio-util — CancellationToken

## Test count

REFACTOR_PLAN.md reports 167 tests (cargo test passes 167 tests after Phase 1 refactor). Scenario smoke tests in `configs/scenarios/`.

## Git log (top 10)

```
4035de3 chore: bump workspace version to 0.2.0
754ae24 Merge pull request #18 from sizzlecar/feat/cli-ux
1515e52 style(cli): fix formatting in config.rs
0903054 fix(runtime): prevent empty AssistantOutput from stalling TUI
9d464e2 fix(tui): approval modal full-width to prevent background bleed
3881982 fix(cli): auto-detect planner backend when config's API key is missing
1f6dd5b feat(cli): CLI UX overhaul — welcome screen, --mcp-path/--skill-dir
3616baf docs(readme): add crates.io install instructions
a5455e9 chore: add crates.io metadata to orchestral-cli
30b7aed Merge pull request #16 from sizzlecar/feat/crates-publish
```

Tags: v0.2.0
