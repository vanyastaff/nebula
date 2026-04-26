# Completion — orchestral — Tier 3 (AI-first full A21)

- timestamp: 2026-04-26T05:00:00Z
- word_count: 5006
- key_finding: orchestral is an LLM-as-planner agent runtime (6-iter replan loop, 9 LLM providers including Ollama, MCP bridge as plugin model, open Action trait with type-erased Value I/O) — inverse of Nebula's developer-defined DAG model; the MCP bridge pattern and action-selector pre-filter are worth examining for Nebula's Surge integration
- gaps:
  - Production store backend (PostgreSQL/SQLite impls) not visible in published crates (noted as living in plugins/ per CLAUDE.md)
  - Stars/forks/download count not retrieved
  - tokei LOC not run (tokei not installed in environment); rough estimates used
  - A12 trigger/event answered at summary depth only (Tier 3 does not require deep A12)
- escalations: none
- artifacts:
  - architecture.md: findings/orchestral/architecture.md
  - issues count: 0 (gh issue list returned empty)
  - deepwiki queries: 0 / 4 (all 4 failed — repo not indexed; 3-fail-stop rule satisfied after first 3 failures; 4th confirmed same error)
