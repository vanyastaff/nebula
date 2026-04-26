# kotoba-workflow — Structure Summary

## Repository metadata

- **Name:** `eaf-ipg-runtime` (root package name); project name: Kotoba
- **Repo:** https://github.com/com-junkawasaki/kotoba
- **Version:** 0.2.0 (root), 0.1.22 (workspace crates)
- **Rust edition:** 2021
- **License:** Apache-2.0

## Crate inventory

**Active crates (have Cargo.toml + Rust source, ~60 .rs files, ~12,821 LOC):**

| Crate | Path | Role |
|-------|------|------|
| `kotoba-os` | crates/010-logic/020-kotoba-os | Kernel + Actor + Mediator process orchestration |
| `kotoba-jsonld` | crates/010-logic/019-kotoba-jsonld | JSON-LD processing utilities |
| `kotoba-phonosemantic` | crates/010-logic/021-kotoba-phonosemantic | Phoneme/semantic mapping |
| `kotoba-owl-reasoner` | crates/010-logic/022-kotoba-owl-reasoner | OWL RDFS/Lite/DL inference (fukurow bindings) |
| `kotoba-storage-fcdb` | crates/030-storage/039-kotoba-storage-fcdb | FCDB content-addressable storage adapter |
| `engidb` | crates/engidb | sled Merkle DAG with CID/IPLD |
| `kotoba-types` | crates/kotoba-types | Core graph IR types |
| `kotobas-tamaki-holochain` | crates/kotobas-tamaki-holochain | Holochain DHT integration |
| Root binary | src/ | Demo server (axum) + ExecDag runtime |

**Ghost crates** (in `[workspace.dependencies]` with paths but no `Cargo.toml`): `kotoba-workflow-core`, `kotoba-workflow`, `kotoba-workflow-activities`, `kotoba-workflow-operator`, deployment crates, language crates. >30 planned crates are commented out.

## LOC

- **Total .rs files (non-archive, non-git):** 60
- **Total LOC estimate:** ~12,821 (from `wc -l` on all active .rs files)
- **tokei:** not available in environment

## Key dependencies

Top workspace-level deps: `tokio 1.0`, `serde/serde_json 1.0`, `async-trait 0.1`, `thiserror 2.0`, `anyhow 1.0`, `petgraph 0.6`, `sled 0.34`, `wasmtime 0.35` (unused), `axum 0.7`, `tracing 0.1`, `uuid 1.18`, `blake3 1.5`

External git deps: `fukurow-*` (OWL reasoning, from `github.com/com-junkawasaki/fukurow`), `fcdb-*` (content-addressable DB, from `github.com/com-junkawasaki/fcdb`)

## Test count

Approximately 15-20 tests across active crates (4 in kotoba-os/lib.rs, 3 in error.rs, 2 in storage-fcdb, plus holochain test files).

## Notable directories

- `_archive/251006/` — archived prior codebase (larger than active code; contains workflow, auth, AI, deployment, and language crates in various states)
- `examples/` — `generated_wasm.rs` (HTMX UI WASM transpiler example)
- `bench_db_cold/`, `bench_db_*` — benchmark JSON result files (no bench code active)
