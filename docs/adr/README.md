# Architecture Decision Records (ADRs)

Short, immutable records of architectural decisions that shape Nebula. One
ADR = one decision. Once `accepted`, an ADR is **not edited** — subsequent
changes land as a new ADR that `supersedes` it.

## Index

| #    | Title                                                   | Status   | Date       |
| :--- | :------------------------------------------------------ | :------- | :--------- |
| [0001](./0001-schema-consolidation.md) | Schema consolidation — delete `nebula-parameter`, adopt `nebula-schema` | accepted | 2026-04-17 |
| [0002](./0002-proof-token-pipeline.md) | Proof-token pipeline — `ValidSchema` / `ValidValues` / `ResolvedValues` | accepted | 2026-04-17 |
| [0003](./0003-consolidated-field-enum.md) | Consolidated `Field` enum (13 variants; drop `Date`/`DateTime`/`Time`/`Color`/`Hidden`) | accepted | 2026-04-17 |
| [0004](./0004-credential-metadata-rename.md) | Credential `Metadata` → `Record`, `Description` → `Metadata` rename | accepted | 2026-04-17 |
| [0005](./0005-trigger-health-trait.md) | `TriggerHealth` — atomic lock-free health state on `TriggerContext` | accepted | 2026-04-17 |
| [0006](./0006-sandbox-phase1-broker.md) | Sandbox Phase 1 broker — duplex JSON-RPC over UDS / Named Pipe | accepted | 2026-04-17 |
| [0007](./0007-prefixed-ulid-identifiers.md) | Prefixed ULID identifiers (Stripe-style) | accepted | 2026-04-18 |
| [0008](./0008-execution-control-queue-consumer.md) | Execution control-queue consumer | accepted | 2026-04-18 |
| 0008 ⚠️ [lease-lifecycle](./0008-execution-lease-lifecycle.md) | Execution lease lifecycle | proposed | 2026-04-18 |
| [0009](./0009-resume-persistence-schema.md) | Resume persistence schema (persist full `ActionResult` per node) | accepted | 2026-04-18 |
| [0010](./0010-rust-2024-edition.md) | Rust 2024 edition + MSRV 1.94 | accepted | 2026-04-19 |
| [0011](./0011-serde-json-value-interchange.md) | `serde_json::Value` as the workflow data interchange type | accepted | 2026-04-19 |

> ⚠️ **Number collision on 0008.** Two files share `id: 0008`:
> `0008-execution-control-queue-consumer` (accepted) and
> `0008-execution-lease-lifecycle` (proposed). The lease-lifecycle ADR must
> be renumbered when it moves to `accepted` — tracked separately, out of
> scope for this index.

## Writing a new ADR

1. Copy the frontmatter block from any existing ADR (keep the keys: `id`,
   `title`, `status`, `date`, `supersedes`, `superseded_by`, `tags`,
   `related`, optional `linear`).
2. Pick the next free number (currently **0012**). Do not reuse.
3. File name: `NNNN-kebab-case-title.md` matching the `title:` field.
4. Start `status: proposed`. Move to `accepted` only after review and merge.
5. **Do not edit an accepted ADR.** Open a new one with
   `supersedes: [NNNN]` and set the old one's `superseded_by`.

### Frontmatter convention

```yaml
---
id: NNNN
title: kebab-case-title
status: proposed | accepted | superseded | rejected
date: YYYY-MM-DD
supersedes: []
superseded_by: []
tags: [topic, topic]
related:
  - path/to/file.rs
  - docs/PRODUCT_CANON.md#section
linear:
  - NEB-XXX
---
```

### Body sections (suggested, not mandatory)

- **Context** — why is this decision needed? What forces apply?
- **Decision** — the explicit choice, in enough detail to implement.
- **Consequences** — positive / negative / neutral impacts.
- **Alternatives considered** — paths we rejected and why.
- **Follow-ups** — tracked issues, future ADRs, supersede hooks.

## How ADRs fit the canon

ADRs are the **L2 invariant diff log**. When a Product Canon invariant moves,
the change lands here first — never silently in code. See
[`docs/PRODUCT_CANON.md §0.2`](../PRODUCT_CANON.md#02) *canon revision
triggers* for when an ADR is required.

The session read-order in [`CLAUDE.md`](../../CLAUDE.md) loads this index on
demand; any non-trivial architectural change should cite or open an ADR
before code review.
