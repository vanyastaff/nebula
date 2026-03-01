# Nebula Docs Refactor Spec

## Purpose

This spec defines the ongoing documentation task for the Nebula workspace:

- convert legacy crate docs into production-grade documentation
- preserve all historical material in `_archive`
- use a single template and consistent structure across all crates
- document cross-crate contracts for a Rust workflow platform (n8n-class)

## Primary Task

For each crate docs folder `docs/crates/{crate}`:

1. Study inputs:
- existing docs in `docs/crates/{crate}`
- archived notes and drafts
- actual implementation in `crates/{crate}`

2. Rebuild docs using template:
- copy structure from `docs/crates/_template`
- adapt content to real crate behavior and target architecture
- add deep reasoning, trade-offs, and production constraints

3. Preserve history:
- move old docs into `docs/crates/{crate}/_archive/`
- do not delete historical material unless explicitly requested

4. Produce production-oriented outputs:
- current state vs target state
- interaction contracts with sibling crates
- security/reliability/test/migration plans
- phased roadmap with measurable exit criteria

## Required Output Files Per Crate

- `README.md`
- `ARCHITECTURE.md`
- `API.md`
- `INTERACTIONS.md`
- `DECISIONS.md`
- `ROADMAP.md`
- `PROPOSALS.md`
- `SECURITY.md`
- `RELIABILITY.md`
- `TEST_STRATEGY.md`
- `MIGRATION.md`
- `_archive/README.md`

## Template Source

Use:
- `docs/crates/_template/`

This template is mandatory unless the user explicitly asks to deviate.

## Interaction Documentation Rule

`INTERACTIONS.md` must include:

- ecosystem map (existing + planned crates)
- upstream dependencies and downstream consumers
- interaction matrix (`contract`, `sync/async`, `failure handling`)
- cross-crate ownership boundaries
- compatibility/breaking-change protocol

## Comparative Architecture Rule

When defining target architecture and proposals, evaluate patterns from:

- n8n
- Node-RED
- Activepieces/Activeflow
- Temporal/Prefect/Airflow (where relevant)

For each major idea, classify as:
- `Adopt`
- `Reject`
- `Defer`

with clear rationale.

## Definition of Done (Per Crate)

1. Legacy docs archived in `_archive` without loss.
2. Full template-based document set created.
3. Content aligned with real codebase, not aspirational-only.
4. Cross-crate contracts documented.
5. Security/reliability/test/migration sections completed.
6. Roadmap includes phases, risks, and exit criteria.

## Workflow Notes

- Prefer incremental crate-by-crate updates.
- Keep commits scoped to one crate or one infrastructure change (like template/spec).
- If needed, add `INTERACTIONS.md` links from crate README for navigation.
