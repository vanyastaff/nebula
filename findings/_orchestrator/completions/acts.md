# Completion — acts — Tier 1

- timestamp: 2026-04-26T04:15:00Z
- word_count: 7082
- key_finding: Acts is a lightweight embeddable Rust workflow library using JavaScript (QuickJS/rquickjs) for expressions and an inventory-based compile-time package registry; it has zero credential/resource/resilience/multi-tenancy/observability infrastructure and trades type safety for simplicity — it competes with Nebula only on the basic workflow execution layer (Workflow→Step→Branch→Act linear tree), not on any of Nebula's platform differentiators
- gaps:
  - A21 AI: confirmed no integration (negative grep), roadmap only — no code to analyze
  - A12 Schedule/Webhook: confirmed not implemented (roadmap items), no code exists
  - A4/A5: confirmed no implementations via grep — gap is clear but there is nothing to cite beyond absence evidence
  - acts-server (gRPC) is a separate repo not cloned — gRPC API surface could not be fully decomposed
  - crates.io downloads (24.4K total, 531 recent) confirm low adoption — no production war stories to cite
- escalations: none
- artifacts:
  - architecture.md: findings/acts/architecture.md
  - issues count: 9 total (7 closed, 2 open) — all 9 cited (< 100 closed, so all available cited)
  - deepwiki queries: 9 / 9
  - context7: skipped — acts has 24.4K total crates.io downloads but only 531 recent; marginally below "mature" threshold; DeepWiki + direct code reading provided sufficient coverage
