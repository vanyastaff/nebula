# Structure Summary — rayclaw

## Crate count
Single crate (`rayclaw` v0.2.5). No workspace members. Feature flags: `telegram`, `discord`, `slack`, `feishu`, `weixin`, `web`, `sqlite-vec`, `openssl-vendored`.

## LOC
Approx 39,440 lines across src/ (via `wc -l src/*.rs src/tools/*.rs`). Full tokei not run (tool not in PATH on analysis machine). Major files: `src/llm.rs` (>2,500 lines incl. tests), `src/agent_engine.rs` (>2,600 lines incl. tests), `src/db.rs` (>3,000 lines), `src/acp.rs` (>3,000 lines), `src/tools/mod.rs` (~830 lines).

## Key dependencies
- tokio 1 (async runtime)
- reqwest 0.12 (HTTP / LLM calls)
- rusqlite 0.32 bundled (SQLite)
- serde/serde_json/serde_yaml (serialization)
- async-trait 0.1 (object-safe async traits)
- tracing/tracing-subscriber (structured logging)
- thiserror 2 / anyhow 1 (error handling)
- cron 0.13 / chrono-tz 0.10 (scheduling)
- axum 0.7 (web, optional)
- teloxide 0.17 (Telegram, optional)
- serenity 0.12 (Discord, optional)
- sqlite-vec 0.1.7-alpha.10 (vector memory, optional)
- aes/cipher/ecb/md-5 (WeChat message crypto)

## Test count
Extensive inline `#[cfg(test)]` suites. Estimated 400-600 unit tests across all modules based on test function density observed. Integration tests in `tests/` directory.

## Git activity
Tags: v0.2.1, v0.2.2, v0.2.3, v0.2.4, v0.2.5. Active development (Rust 1.95 clippy fix, WeChat adapter, Feishu Phase 2, skill evolution, error classifier all added within recent commits).
