# ADRs 0001–0041 (index)

**Agents:** Do not bulk-read every `docs/adr/0*.md` file. Open a specific ADR
only when code or an in-repo ADR cites it. For new work, prefer **0042+**
(see [`README.md`](./README.md)).

Full text for each row lives at `docs/adr/<file>` in this repo (hidden from
Cursor index via `.cursorignore`). Alternate numbering `0042-tool-provider`
is archive-only.

| # | Title | Status |
|---|-------|--------|
| 0001 | Schema consolidation | accepted |
| 0002 | Proof-token pipeline | accepted |
| 0003 | Consolidated `Field` enum | accepted |
| 0004 | Credential Metadata→Record rename | accepted |
| 0005 | `TriggerHealth` trait | accepted |
| 0006 | Sandbox Phase 1 broker | accepted |
| 0007 | Prefixed ULID identifiers | accepted |
| 0008 | Execution control-queue consumer | accepted |
| 0009 | Resume persistence schema | accepted |
| 0010 | Rust 2024 edition | superseded → 0019 |
| 0011 | `serde_json::Value` interchange | accepted |
| 0012 | Checkpoint recovery | accepted |
| 0013 | Compile-time deployment modes | accepted |
| 0014 | `dynosaur` macro | superseded → 0024 |
| 0015 | Execution lease lifecycle | accepted |
| 0016 | Engine cancel registry | accepted |
| 0017 | Control-queue reclaim policy | accepted |
| 0018 | PluginMetadata→PluginManifest | accepted |
| 0019 | MSRV 1.95 | accepted |
| 0020 | Library-first GTM | accepted |
| 0021 | Crate publication policy | accepted |
| 0022 | Webhook signature policy | accepted |
| 0023 | `KeyProvider` trait | accepted (location → 0029) |
| 0024 | Defer dynosaur migration | accepted |
| 0025 | Sandbox broker RPC surface | accepted |
| 0026 | `nebula-sdk` dependency closure | proposed |
| 0027 | Plugin load-path stable | accepted |
| 0028 | Cross-crate credential invariants | accepted |
| 0029 | Storage owns credential persistence | accepted |
| 0030 | Engine owns credential orchestration | accepted |
| 0031 | API owns OAuth flow | accepted |
| 0032 | `CredentialStore` canonical home | accepted |
| 0033 | Integration credentials (Plane B) | accepted |
| 0034 | `SecretValue` / credential seam | accepted |
| 0035 | Phantom-shim capability pattern | proposed |
| 0036 | Resource credential adoption | accepted — **superseded by [0044](./0044-supersede-0036-resource-credential-singular.md)** |
| 0037 | Daemon + EventSource engine fold | accepted |
| 0038 | Action trait shape (`#[action]`) | accepted |
| 0039 | Action macro emission | accepted |
| 0040 | `ControlAction` seal + canon §3.5 | proposed |
| 0041 | Durable credential refresh claim repo | proposed |
