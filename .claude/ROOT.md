# Nebula
DAG-based workflow automation engine in Rust (n8n/Zapier).

## Crates (→ .claude/crates/{name}.md)

**Core layer**
core · validator · parameter · expression · memory · workflow · execution

**Cross-cutting** (importable at any layer)
log · system · eventbus · telemetry · metrics · config · resilience · error

**Business logic**
credential · resource · action · plugin · auth (RFC — not yet in workspace)

**Exec / API / Infra**
engine · runtime · storage · api · webhook · sdk

**Desktop app**
apps/desktop (Tauri — React + TypeScript; replaces former nebula-app/egui)

## Cross-cutting Docs
→ decisions.md — architecture decisions
→ pitfalls.md — read before changing anything
→ active-work.md — current focus areas

## Conventions
- Edition 2024, rust-version 1.94
- `serde_json::Value` as universal data type (no nebula-value crate — was removed)
- Errors: `thiserror` in libs, `anyhow` in binaries
- Layers enforced by `cargo deny`: Infra → Core → Business → Exec → API
- Cross-cutting crates are exempt from layer restrictions
- `auth` crate is in RFC phase — API not stable, do not depend on it
