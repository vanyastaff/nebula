# Completion — runtara-core — Tier 2

- timestamp: 2026-04-26T05:30:00Z
- word_count: 6090
- key_finding: Runtara is a compile-to-WASM durable workflow engine (AOT: JSON DSL → rustc → WASM) with first-class AI agent step (tool-calling loop, conversation memory, structured output, MCP server) — the most complete AI-workflow integration found in this analysis set; its AiAgent step pattern, tool-call-as-edge labeling, conversation memory with compaction, and MCP-as-API-surface are all borrowable for Nebula.
- gaps:
  - A14 multi-tenancy: single-tenant per process only; no RLS/RBAC/SCIM evidence
  - A11 plugin: no external plugin format — static inventory linking only; no runtime plugin loading
  - A5 resource: no resource abstraction layer at all
  - A6 resilience: retry-only (no CB/bulkhead/hedging); no unified ErrorClass
  - DeepWiki: 0/7 queries answered (repo not indexed)
- escalations: none
- artifacts:
  - architecture.md: findings/runtara-core/architecture.md
  - issues count: 4 (3 closed, 1 open; repo has <100 total issues)
  - deepwiki queries: 0/7 (repo not indexed by DeepWiki)
  - structure-summary.md: findings/runtara-core/structure-summary.md
  - issues-architectural.md: findings/runtara-core/issues-architectural.md
  - deepwiki-findings.md: findings/runtara-core/deepwiki-findings.md
