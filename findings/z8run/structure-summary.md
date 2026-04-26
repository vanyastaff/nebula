# z8run — Structure Summary

## Crate count: 7 (5 library + 2 binaries)

### Library crates (under `crates/`)
1. `z8run-core` — Flow engine, DAG model, scheduler, 35+ built-in nodes, NodeExecutor trait
2. `z8run-protocol` — Binary WebSocket protocol (11-byte header, bincode payload)
3. `z8run-storage` — SQLite/PostgreSQL persistence, AES-256-GCM credential vault, migrations
4. `z8run-runtime` — WASM sandbox (wasmtime), plugin manifest, PluginRegistry
5. `z8run-api` — Axum REST/WebSocket server, JWT auth, rate limiting, vault routes

### Binary crates (under `bins/`)
6. `z8run-cli` — Main binary; serves HTTP, runs migrations, plugin CLI commands
7. `z8run-server` — Binary with `rust-embed` frontend serving

### Frontend (under `frontend/`)
- React + TypeScript + Tailwind CSS visual node editor
- ReactFlow for canvas rendering, Zustand for state management
- 20 `.ts` + 18 `.tsx` files
- Feature modules: editor, flows, auth, vault

## Rust source file count: 79 files
## Total Rust LOC: 22,339 lines (wc -l over .rs files)
## tokei: failed (not in PATH)

## Dependency graph (key)
```
z8run-cli → z8run-core + z8run-api + z8run-storage + z8run-runtime
z8run-api → z8run-core + z8run-protocol + z8run-storage + z8run-runtime
z8run-runtime → z8run-core
z8run-storage → z8run-core
z8run-protocol → z8run-core
```

## Top-10 external dependencies
1. `wasmtime` v42 — WASM sandbox (z8run-runtime)
2. `axum` v0.8 — HTTP/WebSocket server (z8run-api)
3. `sqlx` v0.8 — async database (sqlite + postgres + mysql)
4. `tokio` v1.50 — async runtime
5. `serde_json` v1.0 — JSON serialization throughout
6. `aes-gcm` v0.10 — AES-256-GCM encryption (z8run-storage)
7. `jsonwebtoken` v10 — JWT auth (z8run-api)
8. `argon2` v0.5 — password hashing
9. `reqwest` v0.12 — HTTP client (node HTTP calls, LLM API calls)
10. `rumqttc` v0.25 — MQTT client

## Test count: 270 `#[test]` / `#[tokio::test]` annotations across 79 files

## Git log summary (latest 20 commits)
- Most recent: v0.2.0 bump (docs, security fixes, CI improvements)
- Two main release tags: v0.1.0 (2026-03-06) and v0.2.0 (2026-04-01)
- Active CI/CD pipeline; dependabot for patch/minor updates
- Solo or very small team (evidence: one primary email hello@z8run.org, CODEOWNERS)
