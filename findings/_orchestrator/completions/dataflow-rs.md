# Completion — dataflow-rs — Tier 1

- timestamp: 2026-04-26T00:00:00Z
- word_count: 7353
- key_finding: dataflow-rs is a library-first, compile-time-extended JSONLogic rules engine, NOT an n8n-style visual workflow engine. It has no server, no persistence, no trigger system, no credentials, no plugin sandbox, and no AI/LLM integration — all by intentional design as an embeddable Rust crate. The IFTTT rebranding (v2.1.0, 2026-02-21) obscures this. z8run is the true n8n-style competitor; dataflow-rs competes with json-rules-engine / cel-go / Open Policy Agent style rule evaluators, not with Nebula or z8run.
- gaps:
  - A2 (DAG): No DAG exists — compared N/A for most graph sub-axes
  - A4 (Credentials): Fully absent — verified by grep, confirmed by DeepWiki
  - A5 (Resources): Fully absent — verified by grep
  - A11 (Plugin BUILD/EXEC): No plugin system — the WASM target is for browser deployment, not host-side plugin sandbox
  - A12 (Triggers): Fully absent — verified by grep and DeepWiki
  - A21 (AI/LLM): Fully absent — verified by grep
  - Issue citation: Only 8 total issues (all closed, 0 open). Quality gate of "≥3 cited issues for projects with >100 closed issues" does NOT apply. All 8 issues documented in issues-architectural.md.
- escalations: none
- artifacts:
  - architecture.md: findings/dataflow-rs/architecture.md (7353 words)
  - issues count: 8 (all closed; quality gate N/A — <100 closed issues)
  - deepwiki queries: 9/9 (all answered; repository indexed)
  - context7: not indexed (attempt made)
  - crates.io: 22.8K total downloads, 3.4K recent
