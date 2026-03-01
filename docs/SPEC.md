# Nebula Docs Refactor Spec

## Purpose

This spec defines the ongoing documentation task for the Nebula workspace:

- convert legacy crate docs into production-grade **vision and specification** documents
- write a `CONSTITUTION.md` for every crate that captures its role, principles, and production vision
- preserve all historical material in `_archive`
- document cross-crate contracts for a Rust workflow platform (n8n-class)

## Audience and Scope

These docs are **for agents and developers working on the Roadmap** — not for end users of the library.

- **Primary use:** navigation, planning, implementation guidance, Roadmap execution
- **Not the purpose:** exhaustive API reference (that belongs in rustdoc inline comments in `crates/`)

### What belongs here

- Crate role in the workflow automation platform
- User stories — who uses this crate and what they need from it
- Production vision — what this crate looks like in a real fleet deployment
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

It should **not** enumerate every method signature or every enum variant.

---

## CONSTITUTION.md — The Primary Vision Document

Every crate **must have** a `CONSTITUTION.md`. This is the most important document in
the crate's doc folder. It is written before any other doc is touched and drives everything else.

### Why a Constitution?

The template-based docs (ARCHITECTURE, API, INTERACTIONS, etc.) describe **what** the crate does.
The Constitution answers a different question: **why does this crate exist in a workflow automation platform,
and what are the invariants that must never be broken?**

A good Constitution makes it possible to review any PR for a crate and answer:
- Does this change violate a non-negotiable?
- Does this change fit the production vision?
- Does this change serve the user stories?

### What makes a good Constitution

A good Constitution:
1. **Grounds the crate in platform context** — not "this crate does X", but "when a workflow runs Y, this crate is what makes Z safe"
2. **Uses concrete user stories** — with acceptance criteria that could be tested, not vague goals
3. **Mines archives for production insights** — the `_archive/` folders contain historical designs; good ones become the production vision
4. **States principles with rationale** — each principle explains why it exists and what bad outcome it prevents
5. **Names non-negotiables explicitly** — a short numbered list of invariants that PRs must not violate
6. **Has a governance section** — how to amend the constitution (PATCH/MINOR/MAJOR)

A bad Constitution:
- lists features without explaining platform context
- has principles without rationale ("always use async" is not a principle — it's a rule)
- ignores the archives
- non-negotiables that are too broad to enforce ("be correct")

---

## Constitution Structure

Every `CONSTITUTION.md` must follow this structure, in order:

```markdown
# nebula-{crate} Constitution

> **Version**: 1.0.0 | **Created**: YYYY-MM-DD

---

## Platform Role

[2-3 paragraphs. Answer: when a workflow runs, what does this crate do?
Show the platform data/control flow as ASCII. End with one sentence that is
"This is the {crate} contract."]

---

## User Stories

### Story 1 — [Title] (P1/P2/P3)

[Who needs what and why. Real operational scenario.]

**Acceptance**:
[Concrete: API calls, code snippets, or observable behaviors. No vague goals.]

[3-5 stories total, at least 2 at P1]

---

## Core Principles

### I. [Principle Title]

**[One sentence bold statement of the rule.]**

**Rationale**: [Why does this exist? What bad outcome does it prevent?]

**Rules**:
- [Specific, enforceable rules derived from the principle]

[4-6 principles total]

---

## Production Vision

[What this crate looks like in a real n8n-class fleet deployment.
Include ASCII architecture diagrams. Mine archives for insights.]

### From the archives: [insight]

[Reference specific archive files and what they contribute to the production vision.]

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|---------|-------|
| ... | ... | ... |

---

## Key Decisions

### D-001: [Title]

**Decision**: [What was decided.]

**Rationale**: [Why.]

**Rejected**: [Alternatives considered and why they lost.]

[3-6 decisions]

---

## Open Proposals

### P-001: [Title]

**Problem**: [What problem this solves.]

**Proposal**: [What to build.]

**Impact**: [Breaking changes, dependencies, scope.]

[2-4 proposals]

---

## Non-Negotiables

1. **[Short title]** — [one sentence explanation]
[5-8 items, numbered, enforceable]

---

## Governance

[PATCH/MINOR/MAJOR amendment protocol]
[What every PR must verify]
```

---

## Archive Mining (Required Before Writing)

Before writing a `CONSTITUTION.md`, **always read the archives**:

```
docs/crates/{crate}/_archive/
```

Look for:
- **Production architecture designs** — earlier, more ambitious designs that show where the crate was meant to go
- **Layer interaction examples** — how this crate fits into the action→resource→credential chain
- **Proposals and ideas** — backlog items that reveal the production vision
- **Historical decisions** — what alternatives were considered and rejected

Good archives to look for specifically:
- `archive-overview.md` — system-wide architecture showing each crate's role
- `archive-layers-interaction.md` — cross-crate data flows with code examples
- `archive-ideas.md` — backlog designs and aspirational architecture
- `legacy-DECISIONS.md` / `legacy-PROPOSALS.md` — historical decisions and open proposals

Archive insights should appear in the "Production Vision" and "Key Decisions" sections
with explicit attribution: `From the archives: {filename}`.

---

## Primary Task

For each crate docs folder `docs/crates/{crate}`:

1. **Mine archives** — read `_archive/` for insights before writing anything
2. **Write CONSTITUTION.md** — vision, principles, production direction; commit separately
3. **Update/rebuild remaining docs** — using template and codebase reality
4. **Preserve history** — keep all archived material intact

### Study inputs

- existing docs in `docs/crates/{crate}`
- archived notes and drafts in `docs/crates/{crate}/_archive/`
- actual implementation in `crates/{crate}/src/`
- related specs in `specs/` for related features

---

## Required Output Files Per Crate

- `CONSTITUTION.md` ← **new, primary vision document**
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

---

## Template Source

Use:
- `docs/crates/_template/` for template-based docs
- This SPEC for the Constitution structure

The template is mandatory for the 11 standard docs. The Constitution follows the
structure defined in this file, not a separate template file.

---

## Interaction Documentation Rule

`INTERACTIONS.md` must include:

- ecosystem map (existing + planned crates)
- upstream dependencies and downstream consumers
- interaction matrix (`contract`, `sync/async`, `failure handling`)
- cross-crate ownership boundaries
- compatibility/breaking-change protocol

---

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

---

## Definition of Done (Per Crate)

1. `CONSTITUTION.md` written and committed — platform role, 3+ user stories, 4+ principles, production vision, non-negotiables
2. Archives mined — at least one archive insight appears in Constitution
3. Legacy docs archived in `_archive` without loss
4. Full template-based document set created
5. Content aligned with real codebase, not aspirational-only
6. Cross-crate contracts documented
7. Security/reliability/test/migration sections completed
8. Roadmap includes phases, risks, and measurable exit criteria

---

## Existing Constitutions (Reference)

These are written and can be used as style reference:

- `docs/crates/system/CONSTITUTION.md` — sensing layer, pressure types, feature-flag API
- `docs/crates/credential/CONSTITUTION.md` — security boundary, scope isolation, interactive flows
- `docs/crates/memory/CONSTITUTION.md` — scoped arenas, reuse-first, pressure signaling
- `docs/crates/resource/CONSTITUTION.md` — connection pooling, scope enforcement, RAII lifecycle

---

## Workflow Notes

- Commit `CONSTITUTION.md` separately from other doc changes (one commit per crate)
- Prefer incremental crate-by-crate updates
- Keep commits scoped to one crate or one infrastructure change (like template/spec)
- If needed, add `INTERACTIONS.md` links from crate README for navigation
