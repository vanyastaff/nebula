# Nebula — Vision charter (human draft)

> **Agents: do not use this file.** Read [`STRATEGY.md`](../STRATEGY.md) for product
> direction and [`docs/PRODUCT_CANON.md`](./PRODUCT_CANON.md) for binding invariants
> and **North Star** (§9). Mechanics: [`docs/INTEGRATION_MODEL.md`](./INTEGRATION_MODEL.md).
> Decisions: [`docs/adr/README.md`](./adr/README.md).

Status: **Draft / non-normative** · Last updated: 2026-05-18

---

## Mission (summary)

Build a production-grade **Rust workflow engine** with Python-level author ergonomics
and Rust-level safety: typed boundaries between steps, durable execution, honest
operational claims, and a serious integration SDK (`nebula-sdk`).

## Positioning (summary)

Nebula targets three profiles on **one engine**: API integration, AI/agent
orchestration, and long-running factory automation. Differences are **execution policy**
(storage, isolation, observability), not separate products.

## Architectural principles (pointers)

- **Library-first, local-first** — SQLite/single-process dev path; Postgres for production.
- **Integration orthogonality** — Resource, Credential, Action, Schema, Plugin (see INTEGRATION_MODEL).
- **Operational honesty** — ship only what the engine owns end-to-end (`docs/MATURITY.md`).
- **LLM at the edge** — providers are plugins; durability and tool execution live in the engine.

## Where the full charter went

The May 2026 design-session charter (~900 lines) was **compressed** so agents stop
treating this file as a second spec. Recover from git history before 2026-05-18 if needed.

## Related normative docs

| Need | Read |
|------|------|
| North Star & invariants | `docs/PRODUCT_CANON.md` |
| Direction & 2026 bar | `STRATEGY.md` |
| Integration mechanics | `docs/INTEGRATION_MODEL.md` |
| Accepted decisions | `docs/adr/README.md` (0042+) |
