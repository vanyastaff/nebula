---
title: "refactor: Rust code — reduce ADR/canon citation noise in comments"
type: refactor
status: draft
date: 2026-05-18
origin: docs/brainstorms/2026-05-18-001-docs-stack-contract-consolidation-requirements.md
blocked-by: docs/plans/2026-05-18-001-refactor-docs-stack-contract-consolidation-plan.md
---

# refactor: Rust code — reduce ADR/canon citation noise in comments

## Summary

**Wave C** follow-up to doc stack plan `2026-05-18-001`: replace inline `ADR-00xx` and `canon §x.y` pins in `crates/**/*.rs` with behavior-first comments and at most one module-level link to crate README or `INTEGRATION_MODEL`. Normative traceability stays in docs and supersession tables—not in every hot path.

## Scope

- In scope: `//!` module docs, `//` comments, test module comments in `crates/`
- Out of scope: `deny.toml` policy comments, ADR stub files, `docs/**`

## Sequencing (draft)

1. Add code citation policy to `docs/README.md` (origin R16)
2. `storage-port` + `storage` + `engine` (highest density)
3. `api` + `resource` + `credential` + `credential-runtime`
4. Remaining crates grep sweep

## Status

Draft only — activate after Wave A+B gate in plan `2026-05-18-001` (U10).
