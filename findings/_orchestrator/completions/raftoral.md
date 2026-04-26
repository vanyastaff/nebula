# Completion — raftoral — Tier 2

- timestamp: 2026-04-26T05:00:00Z
- word_count: 4455
- key_finding: Raftoral is an embedded Raft-consensus workflow engine — the only peer-to-peer distributed workflow library in Rust. Its architectural bet is that the Raft log IS the database (no external infrastructure), with a two-tier management+execution cluster topology and an owner/wait checkpoint pattern that reduces proposal traffic by 50-75%. It has no credential layer, no resource abstraction, no trigger system, no plugin system, no expression engine, and no AI integration — it is a focused distributed coordination primitive, not a full-stack orchestration platform. The `checkpoint_compute!` macro (exactly-once side-effect semantics) is a potentially borrowable primitive for Nebula's distributed mode.
- gaps:
  - A10: `!Send` handling not verifiable — closures must be `Send + Sync + 'static`; no `!Send` isolation confirmed absent
  - A13: No formal deployment mode taxonomy (only embedded vs sidecar from code; no "3 modes" like Nebula)
  - A15: No observability instrumentation at all (slog only); no metrics numbers available
  - Issues: 0 GitHub issues — pain points sourced from internal docs only; no community validation
- escalations: none
- artifacts:
  - architecture.md: findings/raftoral/architecture.md
  - structure-summary.md: findings/raftoral/structure-summary.md
  - issues-architectural.md: findings/raftoral/issues-architectural.md (0 GitHub issues; 5 internal pain points cited)
  - deepwiki-findings.md: findings/raftoral/deepwiki-findings.md (3/3 failures — repo not indexed)
  - deepwiki queries: 0 / 7 (3-fail-stop triggered on first 3 attempts)
  - issues count: 0 GitHub issues (below >100 closed threshold — no ≥3 citation requirement applies)
