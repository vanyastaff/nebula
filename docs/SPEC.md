# Nebula Docs Refactor Spec

## Purpose

This spec defines the ongoing documentation task for the Nebula workspace:

- convert legacy crate docs into production-grade **planning and specification** documents
- preserve all historical material in `_archive`
- use a single template and consistent structure across all crates
- document cross-crate contracts for a Rust workflow platform (n8n-class)

## Audience and Scope

These docs are **for agents and developers working on the Roadmap** — not for end users of the library.

- **Primary use:** navigation, planning, implementation guidance, Roadmap execution
- **Not the purpose:** exhaustive API reference (that belongs in rustdoc inline comments in `crates/`)

### What belongs here

- Architecture decisions and trade-offs
- Current state vs target state gaps
- Cross-crate contracts and interaction boundaries
- Security/reliability/test/migration plans
- Phased roadmap with measurable exit criteria
- Open questions and proposals

### What does NOT belong here

- Method-by-method signatures and parameter lists
- Exhaustive enum variant tables already visible in source
- Content that duplicates what rustdoc already covers
- Implementation details that belong in code comments

### `API.md` scope

`API.md` is a **contract document**, not a reference manual. It should cover:

- What the crate promises to callers (stable surface, experimental, deprecated)
- Key usage patterns and gotchas
- Error semantics and retry classification
- Breaking-change policy and compatibility rules
- Concrete examples for the two or three most important use cases

It should **not** enumerate every method signature or every enum variant. That level of detail goes in the source code as rustdoc.

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
