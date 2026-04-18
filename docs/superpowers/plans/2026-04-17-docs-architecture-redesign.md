# Documentation Architecture Redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure the Nebula markdown corpus as LLM-memory architecture — layered canon, named-pattern vocabulary, priming-layer docs, crate-level normalization — rolled out in five sequential passes (one PR each).

**Architecture:** Spec in [2026-04-17-docs-architecture-redesign-design.md](../specs/2026-04-17-docs-architecture-redesign-design.md). Passes: (0) priming skeleton, (1) extract without rewriting, (2) canon surgery, (3) ADR + plans cleanup, (4) crate sweep. Each pass ends with a commit suitable for PR; user may pause between passes. Five discrete commits = five PRs in the default path; batching is allowed if the user prefers fewer PRs.

**Tech Stack:** Markdown, Rust rustdoc (for `lib.rs //!`), `cargo doc --no-deps` for rendering verification, `cargo +nightly fmt` for rust formatting, `grep`/`rg` for stale-reference detection. No new tooling added.

**Spec reference:** Every task cites the spec section it implements: `[spec §N]`.

---

## Pre-flight — State verification

Before any pass, confirm repo state. This is one task the operator runs once.

### Task 0: Verify baseline state

**Files:** none modified.

- [ ] **Step 1: Confirm worktree is clean**

Run: `git status --short`
Expected: empty output, or only the plan / spec files this session created.

- [ ] **Step 2: Inventory stale `nebula-parameter` references**

Run: `grep -rln "nebula-parameter\|ParameterCollection" docs/ README.md crates/*/README.md crates/*/src/lib.rs 2>/dev/null | sort`

Record the list — every hit must be updated in Pass 2 (canon) or Pass 4 (crate sweep). Current known hits (from spec §5.4 and pre-flight grep): `docs/PRODUCT_CANON.md`, `crates/action/README.md`, `crates/expression/README.md`, `crates/sdk/README.md`. Any additional hits must be tracked.

- [ ] **Step 3: Confirm crate count**

Run: `ls -d crates/*/ | wc -l`
Expected: `24`.

Record the full list for Pass 4 iteration:
`action api core credential engine error eventbus execution expression log metrics plugin plugin-sdk resilience resource runtime sandbox schema sdk storage system telemetry validator workflow`.

- [ ] **Step 4: Confirm spec is committed**

Run: `git log --oneline -5`
Expected: recent commit `docs: add design spec for documentation architecture redesign`.

No commit for this task — it is read-only verification.

---

## PASS 0 — Priming skeleton

**Goal:** Create empty infrastructure (templates, skeleton docs, CLAUDE.md updates) before filling any content. Low-risk, structural.

**PR shape:** one commit per task; merge as one PR titled `docs: pass 0 — priming layer skeleton`.

### Task 0.1: Create DOC_TEMPLATE.md (crate README template)

**Files:**
- Create: `docs/DOC_TEMPLATE.md`

[spec §6]

- [ ] **Step 1: Write the template**

Content:

````markdown
<!-- This template is normative for crates/*/README.md and, in reduced form, crates/*/src/lib.rs //!. See docs/PRODUCT_CANON.md §15 and docs/superpowers/specs/2026-04-17-docs-architecture-redesign-design.md §6. -->

---
name: nebula-<crate>
role: <named pattern from docs/GLOSSARY.md — e.g. "Transactional Outbox", "Idempotent Receiver", "Bulkhead Pool", "Stability Pipeline">
status: frontier | stable | partial
last-reviewed: YYYY-MM-DD
canon-invariants: [L2-11.1, L2-12.3, ...]   # optional; empty list if none
related: [nebula-core, nebula-error, ...]    # sibling crates and satellite docs
---

# nebula-<crate>

## Purpose

One paragraph. What this crate is for, framed as a problem it solves in the engine.

## Role

Named architectural pattern (see `docs/GLOSSARY.md` Architectural Patterns section) with a one-line book reference if applicable.

Example: *Transactional Outbox (DDIA ch 11; EIP "Guaranteed Delivery"). Persists control-plane signals atomically with state transitions.*

## Public API

Key types / traits / functions. Use rustdoc-style links where useful. Do NOT duplicate rustdoc in prose — keep this section a catalog, one line per item.

Example:

- [`ExecutionRepo`] — repository trait, seam for §11.1 CAS transitions.
- [`ExecutionControlQueue`] — durable outbox for cancel/dispatch signals (§12.2).
- [`ExecutionJournal`] — append-only event log.

## Contract

Invariants this crate enforces. Each invariant cites the canon layer (L1/L2/L3) and points to its seam test. Do NOT duplicate invariants' full text — reference canon section.

Example:

- **[L2-§11.1]** State transitions use CAS on `version`. Seam: `ExecutionRepo::transition`. Test: `crates/execution/tests/authority.rs::transition_cas`.
- **[L2-§12.2]** Outbox writes share the same transaction as state transitions. Seam: `ExecutionRepo::transition_with_signal`. Test: `crates/execution/tests/outbox_atomicity.rs`.

## Non-goals

What this crate deliberately does NOT do. Point to the crate that does if there is one.

Example:
- Not an expression evaluator — see `nebula-expression`.
- Not a retry pipeline — see `nebula-resilience`.

## Maturity

See `docs/MATURITY.md` row for this crate. Short summary here:

- API stability: stable | frontier | partial
- One sentence on what is still moving, if anything.

## Related

- Canon: `docs/PRODUCT_CANON.md` sections touched.
- Satellite docs: `docs/INTEGRATION_MODEL.md`, `docs/OBSERVABILITY.md`, …
- Siblings: list crates this one depends on or is depended on by.
````

Write the file exactly as above.

- [ ] **Step 2: Commit**

```bash
git add docs/DOC_TEMPLATE.md
git commit -m "docs(pass-0): add DOC_TEMPLATE.md for crate README normalization"
```

### Task 0.2: Create ADR_TEMPLATE.md

**Files:**
- Create: `docs/ADR_TEMPLATE.md`

[spec §4.2]

- [ ] **Step 1: Write the template**

Content:

````markdown
<!-- Template for docs/adr/NNNN-kebab-title.md. No central index — discovery via filename prefix (NNNN) and frontmatter. -->

---
id: NNNN
title: <short kebab title>
status: proposed | accepted | superseded | deprecated
date: YYYY-MM-DD
supersedes: []           # list of ADR ids this replaces
superseded_by: []        # filled in later if this ADR is replaced
tags: [schema, credential, execution, ...]
related: [other/docs.md]
---

# NNNN. <Title>

## Context

What situation prompts this decision. Include constraints, prior state, and what
forcing function triggered the decision.

## Decision

The decision itself, stated plainly. One or two paragraphs. If the decision is
"no, we will not do X" — say that clearly.

## Consequences

What changes because of this decision. Include:

- Positive consequences (what improves).
- Negative consequences (what we accept).
- Follow-up work this enables or requires.

## Alternatives considered

Alternatives we evaluated and rejected, each with the reason.

## Seam / verification

If this ADR locks in an L2 invariant, name the seam (code location) and test
that enforces it.
````

- [ ] **Step 2: Commit**

```bash
git add docs/ADR_TEMPLATE.md
git commit -m "docs(pass-0): add ADR_TEMPLATE.md"
```

### Task 0.3: Create MATURITY.md skeleton

**Files:**
- Create: `docs/MATURITY.md`

[spec §4.1, §7]

- [ ] **Step 1: Write skeleton with 24 empty rows**

Content:

````markdown
---
name: Nebula crate maturity dashboard
description: Manual per-crate state dashboard. Edited in PRs that change a crate's API stability, test coverage, doc state, engine integration, or SLI-readiness.
status: skeleton
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md, STYLE.md]
---

# Crate maturity dashboard

Updated manually in PRs that touch a crate's public surface, test bar, docs, engine integration, or observability. Reviewers check that this file stays truthful.

Legend:
- `stable` — end-to-end works, tested, safe to depend on.
- `frontier` — actively moving; breakage expected; do not add consumers without coordinating.
- `partial` — works for declared happy path; known gaps documented in the crate README.
- `n/a` — dimension does not apply to this crate.

| Crate | API stability | Test coverage | Doc completeness | Engine integration | SLI ready |
|---|---|---|---|---|---|
| nebula-action        |   |   |   |   |   |
| nebula-api           |   |   |   |   |   |
| nebula-core          |   |   |   |   |   |
| nebula-credential    |   |   |   |   |   |
| nebula-engine        |   |   |   |   |   |
| nebula-error         |   |   |   |   |   |
| nebula-eventbus      |   |   |   |   |   |
| nebula-execution     |   |   |   |   |   |
| nebula-expression    |   |   |   |   |   |
| nebula-log           |   |   |   |   |   |
| nebula-metrics       |   |   |   |   |   |
| nebula-plugin        |   |   |   |   |   |
| nebula-plugin-sdk    |   |   |   |   |   |
| nebula-resilience    |   |   |   |   |   |
| nebula-resource      |   |   |   |   |   |
| nebula-runtime       |   |   |   |   |   |
| nebula-sandbox       |   |   |   |   |   |
| nebula-schema        |   |   |   |   |   |
| nebula-sdk           |   |   |   |   |   |
| nebula-storage       |   |   |   |   |   |
| nebula-system        |   |   |   |   |   |
| nebula-telemetry     |   |   |   |   |   |
| nebula-validator     |   |   |   |   |   |
| nebula-workflow      |   |   |   |   |   |

Cell values populate in Pass 4 (crate sweep).
````

- [ ] **Step 2: Commit**

```bash
git add docs/MATURITY.md
git commit -m "docs(pass-0): add MATURITY.md skeleton (24 crate rows, empty cells)"
```

### Task 0.4: Create STYLE.md skeleton

**Files:**
- Create: `docs/STYLE.md`

[spec §4.1, §8]

- [ ] **Step 1: Write skeleton with section headers only**

Content:

````markdown
---
name: Nebula style guide
description: Consolidated house style — idioms, antipatterns, naming table, error taxonomy, type design bets. Read by every session before proposing changes.
status: skeleton
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md, GLOSSARY.md, CLAUDE.md]
---

# Nebula style guide

Read before proposing a new public type, API shape, or refactor. Cross-referenced
from `CLAUDE.md` read-order.

> **When to fight this guide:** see `docs/PRODUCT_CANON.md §0.2` — canon revision
> triggers apply to style as well. A style rule that blocks a measurable
> architectural improvement is a candidate for revision, not a blocker.

## 1. Idioms we use

*Filled in Pass 2.*

## 2. Antipatterns we reject

*Filled in Pass 2.*

## 3. Naming table

*Filled in Pass 2.*

## 4. Error taxonomy

*Filled in Pass 2.*

## 5. Type design bets

*Filled in Pass 2.*

## 6. When to fight canon

See `docs/PRODUCT_CANON.md §0.2 canon revision triggers`.
````

- [ ] **Step 2: Commit**

```bash
git add docs/STYLE.md
git commit -m "docs(pass-0): add STYLE.md skeleton (content fills in pass 2)"
```

### Task 0.5: Update CLAUDE.md — read-order, decision gate, trap catalog

**Files:**
- Modify: `CLAUDE.md`

[spec §4.1, §5.2]

- [ ] **Step 1: Read current CLAUDE.md to understand structure**

Run: `cat CLAUDE.md`

Record current sections. The existing `## Product canon (mandatory)` section is at the top — we extend it.

- [ ] **Step 2: Insert read-order block after the existing "Product canon (mandatory)" section**

After the existing `## Product canon (mandatory)` block (which currently tells agents to read `docs/PRODUCT_CANON.md`), add a new sub-block:

````markdown
### Session read order (priming layer)

Every session, before proposing changes, load in this order:

1. `CLAUDE.md` (you are here) — commands, conventions, this read-order.
2. `docs/PRODUCT_CANON.md` — normative core. If you hit a rule that seems to block a good improvement, check §0.2 canon revision triggers before giving up.
3. `docs/MATURITY.md` — which crates are frontier vs stable; calibrates proposal ambition.
4. `docs/STYLE.md` — idioms, antipatterns, naming, error taxonomy. Gate on any new public type or API.
5. When working inside a specific crate: that crate's `README.md` and `lib.rs //!`.

Satellites loaded on demand:
- `docs/INTEGRATION_MODEL.md` — integration model details (Resource / Credential / Action / Plugin / Schema).
- `docs/COMPETITIVE.md` — positioning, peer analysis.
- `docs/OBSERVABILITY.md` — SLI / SLO / events / core analysis loop.
- `docs/GLOSSARY.md` — terms and architectural patterns.
- `docs/adr/` — past decisions; search by filename or frontmatter.

### Decision gate (before proposing an architectural change)

Answer these six questions to yourself. If any answer implies a canon violation,
stop and open an ADR — see `docs/PRODUCT_CANON.md §0.2`.

1. Does this strengthen the golden path (PRODUCT_CANON §10) or divert it?
2. Does this introduce a public surface the engine does not yet honor end-to-end (§4.5)?
3. Does this change an L2 invariant without an ADR?
4. Does this leak detail upward (cross-cutting crate depending on integration crate)?
5. Does this introduce an implicit durable backbone via in-memory channel (§12.2)?
6. Does this advertise a capability in docs that the code does not deliver (§11.6)?

### Quick Win trap catalog

Recognize these traps; prefer the deeper fix:

- **Rename / redefine to avoid a contract.** If a type conflicts with an invariant, do not rename the type — open an ADR about the invariant.
- **`Clone` to satisfy the borrow checker.** Consider `Cow<'_, T>`, lifetime redesign, or typestate first. Document the tradeoff if cloning is the right answer.
- **Suppress the error with `.unwrap_or_default()` / `.ok()`.** Surface the error with proper classification (`NebulaError`, `Classify`) unless the default is documented-correct.
- **Add a `_` prefix to an "unused" var to silence the lint.** The variable is either needed (use it) or not (delete it). Shim-naming is canon-level feedback (see memory `feedback_direct_state_mutation.md` equivalent).
- **Patch a symptom in a downstream crate.** Root cause may be upstream; propose the fix there even if the PR is bigger.
- **Log-and-discard on an outbox consumer.** Violates §12.2. Either wire a real consumer or mark the path `// DEMO ONLY`.
````

- [ ] **Step 3: Verify the insertion renders as markdown**

Run: `head -80 CLAUDE.md`
Expected: new sections present, existing sections intact.

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(pass-0): add session read-order, decision gate, and trap catalog to CLAUDE.md"
```

### Pass 0 — final verification

- [ ] **Step 1: Verify all pass-0 files exist**

Run: `ls docs/DOC_TEMPLATE.md docs/ADR_TEMPLATE.md docs/MATURITY.md docs/STYLE.md`
Expected: all four paths.

- [ ] **Step 2: Verify CLAUDE.md new sections**

Run: `grep -c "Session read order\|Decision gate\|Quick Win trap catalog" CLAUDE.md`
Expected: `3`.

- [ ] **Step 3: Create pass-0 PR (or mark stop-point)**

If submitting as PR:

```bash
git log --oneline -5
# verify 5 pass-0 commits
# push and open PR titled: "docs: pass 0 — priming layer skeleton"
```

Otherwise continue to Pass 1 in the same branch.

---

## PASS 1 — Extract without rewriting

**Goal:** Move content out of PRODUCT_CANON.md into satellite files. Do NOT rewrite yet; just copy verbatim and add draft frontmatter. Canon still has the duplicated content; Pass 2 will shrink it.

**PR shape:** one commit per task; merge as `docs: pass 1 — extract competitive / integration-model / observability satellites`.

### Task 1.1: Create docs/INTEGRATION_MODEL.md (extract §3.5 + §3.6–§3.9 + §7.1)

**Files:**
- Create: `docs/INTEGRATION_MODEL.md`
- Reference: `docs/PRODUCT_CANON.md:95-170` (§3.5 + §3.6–§3.9), `:277-393` (§7.1)

[spec §4.2]

- [ ] **Step 1: Extract sections verbatim**

Copy content of PRODUCT_CANON.md §3.5 (lines ~95–146), §3.6 (~148–152), §3.7 (~154–158), §3.8 (~160–164), §3.9 (~166–170), §3.10 (~172–189), and §7.1 (~277–393) into a new file, preserving heading levels but demoting: §3.5 → `## Integration model`, §3.6 → `## nebula-resource`, etc.

- [ ] **Step 2: Prepend frontmatter**

At the top of the new file:

````markdown
---
name: Nebula integration model
description: Draft extraction from PRODUCT_CANON.md §3.5 + §3.6–§3.9 + §3.10 + §7.1. Content unchanged in this pass. Pass 2 rewrites with nebula-schema replacing nebula-parameter and sealed-trait form.
status: draft — extracted from canon, surgery in Pass 2
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md, GLOSSARY.md, STYLE.md]
---

# Nebula integration model

> **Status:** this document was extracted verbatim from `PRODUCT_CANON.md` in Pass 1
> of the docs architecture redesign. Pass 2 will rewrite references to the deleted
> `nebula-parameter` crate, apply `nebula-schema` as the shared schema crate, and
> introduce sealed-trait form for the structural contract. Until Pass 2 lands,
> treat naming in this file as pre-surgery.

---
````

Then paste the extracted sections under this header.

- [ ] **Step 3: Verify the file is well-formed**

Run: `wc -l docs/INTEGRATION_MODEL.md`
Expected: 150–250 lines (rough — depends on exact extract).

Run: `head -30 docs/INTEGRATION_MODEL.md`
Expected: frontmatter + draft banner + first heading.

- [ ] **Step 4: Commit**

```bash
git add docs/INTEGRATION_MODEL.md
git commit -m "docs(pass-1): extract integration model sections from canon verbatim"
```

### Task 1.2: Create docs/COMPETITIVE.md (extract §2 + §2.5)

**Files:**
- Create: `docs/COMPETITIVE.md`
- Reference: `docs/PRODUCT_CANON.md:24-79`

[spec §4.2]

- [ ] **Step 1: Extract sections verbatim**

Copy §2 (lines ~24–45) and §2.5 (~46–79) of PRODUCT_CANON.md into the new file.

- [ ] **Step 2: Prepend frontmatter**

````markdown
---
name: Nebula competitive positioning
description: Position vs n8n / Temporal / Windmill / Make / Zapier and our bets against each. Extracted from PRODUCT_CANON.md §2 + §2.5 in Pass 1. Explicitly persuasive, not normative.
status: draft — extracted from canon, surgery in Pass 2
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md]
---

# Nebula competitive positioning

> **Status:** extracted verbatim from `PRODUCT_CANON.md`. This file is
> **persuasive** content — positioning and bets. Normative rules live in
> `PRODUCT_CANON.md`. If this file contradicts the canon, canon wins; open an
> issue to update this file.

---
````

- [ ] **Step 3: Commit**

```bash
git add docs/COMPETITIVE.md
git commit -m "docs(pass-1): extract competitive positioning (§2 + §2.5) from canon verbatim"
```

### Task 1.3: Create docs/OBSERVABILITY.md skeleton

**Files:**
- Create: `docs/OBSERVABILITY.md`

[spec §4.2, §9 Pass 3 fills content]

- [ ] **Step 1: Write skeleton**

````markdown
---
name: Nebula observability contract
description: SLI / SLO / error budget, structured event schema for execution_journal, core analysis loop for operators. Fills in Pass 3.
status: skeleton
last-reviewed: 2026-04-17
related: [PRODUCT_CANON.md, MATURITY.md]
---

# Nebula observability contract

> **Status:** skeleton; content fills in Pass 3 of the docs redesign.

## 1. Service level indicators (SLIs)

*Filled in Pass 3. Candidate SLIs from spec §10 (OBSERVABILITY rationale):*

- `execution_terminal_rate` — percent of started executions reaching terminal state.
- `cancel_honor_latency` — p95 time from outbox Cancel row to terminal Cancelled.
- `checkpoint_write_success_rate` — percent of checkpoint writes that succeed.
- `dispatch_lag` — p95 delay between outbox row insert and consumer acknowledgement.

## 2. Service level objectives (SLOs)

*Filled in Pass 3.*

## 3. Error budgets

*Filled in Pass 3.*

## 4. Structured event schema (execution_journal)

*Filled in Pass 3. Fields (proposed, subject to code verification):*
- `execution_id`, `node_id`, `attempt`, `correlation_id`
- `trace_id`, `span_id`, `event_type`, `payload`, `timestamp`

## 5. Core analysis loop

*Filled in Pass 3. Four-step operator procedure from Observability Engineering:*
1. What failed?
2. When?
3. What changed?
4. What to try?
````

- [ ] **Step 2: Commit**

```bash
git add docs/OBSERVABILITY.md
git commit -m "docs(pass-1): add OBSERVABILITY.md skeleton (content fills in pass 3)"
```

### Pass 1 — final verification

- [ ] **Step 1: Verify satellite files exist and have draft frontmatter**

Run: `grep -l "status: draft\|status: skeleton" docs/INTEGRATION_MODEL.md docs/COMPETITIVE.md docs/OBSERVABILITY.md`
Expected: all three paths.

- [ ] **Step 2: Canon is still the full 686 lines**

Run: `wc -l docs/PRODUCT_CANON.md`
Expected: `687` (unchanged from baseline; may differ ±1 due to line-ending).

- [ ] **Step 3: Create pass-1 PR (or continue)**

```bash
git log --oneline -10
# three pass-1 commits on top of five pass-0 commits
# optional: push and open PR "docs: pass 1 — extract satellites"
```

---

## PASS 2 — Canon surgery

**Goal:** Shrink PRODUCT_CANON.md to ≤250 lines with thin pointers to satellites; apply L1/L2/L3 tags; evict L4 content to crate READMEs; add §0.2 revision triggers and §17 graduated DoD; fix all P0 issues. Fill STYLE.md content. Expand GLOSSARY.md with architectural patterns.

**PR shape:** one PR titled `docs: pass 2 — canon surgery (layered rules, DMMF §4.5, revision triggers)`. Tasks below may commit individually for review readability.

### Task 2.1: Add §0.2 canon revision triggers to PRODUCT_CANON.md

**Files:**
- Modify: `docs/PRODUCT_CANON.md`

[spec §5.2]

- [ ] **Step 1: Insert new §0.2 section after the existing Audit alignment note (around line 14)**

After the `> **Audit alignment:** ...` blockquote in PRODUCT_CANON.md, insert a new section before the `---` separator:

````markdown
---

## 0.1 Layer legend

Canon rules are tagged by **revision cost**:

- **[L1 Principle]** — strategic product intent. Changing means Nebula is a different product. Requires product-level rethink.
- **[L2 Invariant]** — testable contract with a named code seam. Material semantic change requires an ADR (`docs/adr/`) and an updated seam test in the same PR. Wording polish does not.
- **[L3 Convention]** — default style answer. Changing requires a PR with rationale and, if it touches behavior, a test.
- **[L4 Implementation detail]** — not a canon rule. Lives in the owning crate's README. If you find an L4 rule in this file, open a revision per §0.2 and move it.

## 0.2 When canon is wrong (revision triggers)

Canon rules can be stale, premature, or plain wrong. If any of these apply,
**stop, open an ADR, propose revision, then proceed** — do not blind-follow.

- **Dead reference.** Rule mentions a crate, type, or endpoint that no longer exists or has been renamed.
- **Intimacy violation.** Rule can only be changed by editing canon when a single crate refactors. L4 detail leaked into canon. Fix: move to crate README, revise canon to describe the invariant rather than the mechanism.
- **Capability lag.** Rule freezes an implementation that predates a better architectural move, and the improvement is measurable (perf / safety / DX).
- **False capability.** Rule names a type or variant the engine does not honor end-to-end. Per §4.5 the type must be hidden or the rule must drop.
- **Uncovered case.** New failure mode or integration shape the canon is silent on. Write an ADR before blind-applying the nearest rule.

Canon is an articulation, not a prison. Blind-obeying a wrong rule violates
operational honesty (§4.5) more than explicitly revising it.

---
````

- [ ] **Step 2: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): add canon layer legend (§0.1) and revision triggers (§0.2)"
```

### Task 2.2: Replace §1 one-liner (drop nebula-parameter, add nebula-schema)

**Files:**
- Modify: `docs/PRODUCT_CANON.md:18-21`

[spec §5.4]

- [ ] **Step 1: Replace §1 body**

Old text (around lines 18–21):
```
**Nebula is a high-throughput workflow orchestration engine with a first-class integration SDK** — the typed surface in §3.5 (`nebula-parameter`, `nebula-resource`, `nebula-credential`, `nebula-action`, plugin registry) — **Rust-native, self-hosted, owned by you.**
```

New text:
```
**[L1]** **Nebula is a high-throughput workflow orchestration engine with a first-class integration SDK** — the typed integration surface (`nebula-schema`, `nebula-resource`, `nebula-credential`, `nebula-action`, plus the plugin registry — see `docs/INTEGRATION_MODEL.md`) — **Rust-native, self-hosted, owned by you.**
```

Apply via Edit tool, targeting the existing text exactly.

- [ ] **Step 2: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): update §1 one-liner — drop nebula-parameter, cite nebula-schema"
```

### Task 2.3: Replace §2 + §2.5 with thin pointer to COMPETITIVE.md

**Files:**
- Modify: `docs/PRODUCT_CANON.md:24-79`

[spec §9 Pass 2]

- [ ] **Step 1: Delete §2 body (lines 24–45) and §2.5 body (lines 46–79), replace with a pointer section**

New content to replace the deleted range:

````markdown
## 2. Position

**[L1]** Nebula is a Rust-native workflow automation engine: DAG workflows, typed boundaries, durable execution state, explicit runtime orchestration, first-class credentials / resources / actions.

**[L1]** Primary audience: developers writing integrations. Secondary: operators deploying and composing workflows.

**[L1]** Competitive dimension: reliability and clarity of execution as a system, plus DX for integration authors — not feature parity with n8n/Make.

For peer analysis, our explicit bets against n8n / Temporal / Windmill / Make / Zapier, and what we borrow from each, see `docs/COMPETITIVE.md`. That document is persuasive; this canon stays normative.

---
````

- [ ] **Step 2: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): shrink §2 to thin pointer; §2.5 body moved to COMPETITIVE.md"
```

### Task 2.4: Replace §3.5 + §3.6–§3.9 with thin pointer to INTEGRATION_MODEL.md

**Files:**
- Modify: `docs/PRODUCT_CANON.md:95-170`

[spec §5.4]

- [ ] **Step 1: Delete §3.5 through §3.9 bodies, replace with a pointer**

New content for §3.5:

````markdown
### 3.5 Integration model (one pattern, five concepts)

**[L1]** Nebula's integration surface is a small set of orthogonal concepts, each with a single clear responsibility, all sharing the same structural contract:

- **Resource** — long-lived managed object (connection pool, SDK client). Engine owns lifecycle.
- **Credential** — who you are and how authentication is maintained. Engine owns rotation and the stored-state vs consumer-facing auth-material split.
- **Action** — what a step does. Dispatch via action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). Adding a trait requires canon revision (§0.2).
- **Plugin** — distribution and registration unit. Plugin is the unit of registration, not the unit of size — full plugins and micro-plugins use the same contract.
- **Schema** — the cross-cutting typed configuration system (`nebula-schema`: `Field`, `Schema`, `ValidValues`, `ResolvedValues` with proof-token pipeline). Shared across Actions, Credentials, Resources.

**[L1]** Structural contract: every integration concept is `*Metadata + Schema` — UI-facing identity plus typed, validated configuration.

For the full model — structural-contract types, wiring rules, plugin packaging (`Cargo.toml` / `plugin.toml` / `impl Plugin`), plugin signing (status: planned), cross-plugin dependency rules — see `docs/INTEGRATION_MODEL.md`. That document is the authoritative source for integration mechanics; this canon states the invariants.

Sections 3.6 through 3.10 (per-crate pointers) are consolidated in `docs/INTEGRATION_MODEL.md`.
````

Delete the existing §3.6, §3.7, §3.8, §3.9, §3.10 headings and bodies (they now live in INTEGRATION_MODEL.md).

- [ ] **Step 2: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): shrink §3.5–§3.10 to thin pointer; bodies moved to INTEGRATION_MODEL.md"
```

### Task 2.5: Rewrite §4.5 to DMMF-style "public ⇒ honored"

**Files:**
- Modify: `docs/PRODUCT_CANON.md` — §4.5 section (currently around lines 217–224)

[spec §5.4]

- [ ] **Step 1: Replace §4.5 body**

New content for §4.5:

````markdown
### 4.5 Operational honesty — no false capabilities

**[L1]** **Public surface exists iff the engine honors it end-to-end.** A type, variant, or endpoint that can be called but the engine rejects at runtime is a **false capability** — per canon, such types must not ship publicly. Options:

1. **Implement end-to-end** — wire the behavior through `ExecutionRepo`, resilience pipeline, persistence, observability.
2. **Make the surface private or feature-gated** — `pub(crate)` or gated under `unstable-*` feature so consumers cannot bind to what the engine does not yet deliver.
3. **Remove the surface entirely.**

**[L1]** Corollaries:

- **Misconfiguration moves left.** Validation / activation-time checks over runtime rejection, wherever feasible for workflow shape.
- **JSON at edges is fine; JSON instead of validated boundaries is not.** Schemas and compatibility rules at workflow / action boundaries win over unstructured blobs.
- **In-process channels decouple components but are not a durable backbone.** Anything requiring reliable delivery — including cancel and dispatch signals — must share the persistence transaction with the owning state transition, or live in an explicit durable outbox with documented at-least-once semantics (see §12.2). A channel whose consumer logs and discards is not a contract.

See also `docs/STYLE.md` §5 (type design bets) for the Rust patterns that make this invariant easy to uphold: sealed traits, typestate, `#[non_exhaustive]`, `#[unstable]` feature gates.
````

- [ ] **Step 2: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): strengthen §4.5 to DMMF-style 'public ⇒ honored' invariant"
```

### Task 2.6: Replace §4.6 body with thin pointer to OBSERVABILITY.md

**Files:**
- Modify: `docs/PRODUCT_CANON.md` — §4.6 section (currently around lines 226–228)

[spec §4.2, §9 Pass 2]

- [ ] **Step 1: Replace §4.6 body**

New content:

````markdown
### 4.6 Observability

**[L1]** Durable is not enough — runs must be explainable. Execution state, append-only journal, structured errors, and metrics let an operator answer what happened and why a run failed without reading Rust source.

**[L1]** Observability is a first-class contract, not polish. SLIs, SLOs, structured event schema for `execution_journal`, and the core analysis loop live in `docs/OBSERVABILITY.md`.

**[L2]** Where a feature is still thin (e.g. lease enforcement at §11.6), say so — do not imply full auditability from partial signals.
````

- [ ] **Step 2: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): shrink §4.6 to thin pointer; detail moved to OBSERVABILITY.md"
```

### Task 2.7: Apply L1/L2/L3 tags to retained sections (§4.1–§4.4, §5, §7.2, §8, §9, §10, §11, §12, §14, §15, §16)

**Files:**
- Modify: `docs/PRODUCT_CANON.md` — remaining sections

[spec §5.1]

- [ ] **Step 1: Go through each retained section and prefix each rule with the appropriate tag**

Guidelines for tagging (cross-reference spec §5.1):

- **[L1]** — product-level intent, pillars, non-goals, decision filter, golden path shape. Sections §4.1–§4.4, §5 "is / is not" table rows, §8, §9, §10 steps, §14 anti-pattern headers, §16 filter questions.
- **[L2]** — specific testable contracts with seams. §11.1–§11.5 rows, §12.2 outbox atomicity, §12.4 RFC 9457 rule, §12.5 secret handling, §13 knife steps.
- **[L3]** — conventions. §12.4 `thiserror` vs `anyhow` line, §12.3 SQLite default, §12.1 layer direction.

Example edits (not exhaustive — apply consistently across file):

Current §4.1 heading body:
```
**Throughput and latency regressions in benchmarked paths are treated as bugs** where benchmarks exist (e.g. CodSpeed in CI).
```

Tagged:
```
**[L1]** Throughput and latency regressions in benchmarked paths are treated as bugs where benchmarks exist (e.g. CodSpeed in CI).
```

Current §12.4:
```
- Library crates: `thiserror`, not `anyhow`, in public library surfaces.
```

Tagged:
```
- **[L3]** Library crates: `thiserror`, not `anyhow`, in public library surfaces.
```

Current §12.2 bullet 1:
```
- **Authoritative execution state** lives in `nebula-execution` + `ExecutionRepo`. Handlers and API DTOs do not invent a parallel lifecycle, do not mutate state without going through **`ExecutionRepo::transition`** (CAS on `version`), and do not return synthesized timestamps or fake defaults for missing fields.
```

Tagged:
```
- **[L2]** Authoritative execution state lives in `nebula-execution` + `ExecutionRepo`. Handlers and API DTOs do not invent a parallel lifecycle, do not mutate state without going through `ExecutionRepo::transition` (CAS on `version`), and do not return synthesized timestamps or fake defaults for missing fields. Seam: `crates/execution/src/repo.rs`.
```

- [ ] **Step 2: Verify tag coverage**

Run: `grep -cE '\*\*\[L[123]\]\*\*' docs/PRODUCT_CANON.md`
Expected: at least 30 tags across the document. Exact count depends on granularity; err on the side of tagging every distinct rule.

- [ ] **Step 3: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): apply L1/L2/L3 tags to retained canon sections"
```

### Task 2.8: Evict L4 detail from canon to crate READMEs

**Files:**
- Modify: `docs/PRODUCT_CANON.md`
- Modify: `crates/execution/README.md` (§11.3 key format → here)
- Modify: `crates/action/README.md` (§3.8 CheckpointPolicy status → here; already was in INTEGRATION_MODEL after Task 2.4)
- Modify: `crates/credential/README.md` (§3.7 "twelve universal auth schemes" count → here)
- Modify: `crates/resource/README.md` (§11.4 DrainTimeoutPolicy / ReleaseQueue → here)
- Modify: `crates/core/README.md` (§3.10 AuthScheme location → here; already was in INTEGRATION_MODEL after Task 2.4)

[spec §5.1, §5.4]

- [ ] **Step 1: In PRODUCT_CANON.md §11.3, replace the specific key format with an invariant**

Old (around §11.3):
```
**One** idempotency story: deterministic key shape **`{execution_id}:{node_id}:{attempt}`**, persisted in `idempotency_keys`, checked and marked through `ExecutionRepo` before the side effect.
```

New:
```
**[L2]** One idempotency story: deterministic per-attempt key, persisted in `idempotency_keys`, checked and marked through `ExecutionRepo` before the side effect. Exact key format: see `crates/execution/README.md`. Seam: `crates/execution/src/idempotency.rs`.
```

- [ ] **Step 2: In `crates/execution/README.md`, add the evicted detail**

Read `crates/execution/README.md` first. In its Contract section (or create one if it does not exist), add:

````markdown
### Idempotency key format (evicted from PRODUCT_CANON.md §11.3)

The deterministic key shape is `{execution_id}:{node_id}:{attempt}`, persisted in `idempotency_keys`. The format string itself is an implementation detail (L4) — changing it requires only this README and the corresponding code; no canon revision. The invariant ("deterministic per-attempt, checked before side-effect") is canonical (L2).
````

- [ ] **Step 3: Evict §11.4 specific types to `crates/resource/README.md`**

In PRODUCT_CANON.md §11.4, change:
```
orphaned resources rely on the next process to drain via `DrainTimeoutPolicy` / `ReleaseQueue`
```

To:
```
orphaned resources rely on the next process to drain (mechanism types: see `crates/resource/README.md`)
```

Then, in `crates/resource/README.md`, add a section describing `DrainTimeoutPolicy` and `ReleaseQueue` as the drain mechanism.

- [ ] **Step 4: Evict §3.7 "twelve" count to `crates/credential/README.md`**

The "twelve universal auth schemes" claim moved to INTEGRATION_MODEL.md in Task 2.4. In INTEGRATION_MODEL.md, change:

```
**Twelve universal auth schemes** (plus extensibility via `AuthScheme`)
```

To:
```
A set of universal auth schemes (OAuth2, API key, mTLS, and others — full list in `crates/credential/README.md`) plus extensibility via the `AuthScheme` trait.
```

Then, in `crates/credential/README.md`, add (or update) the list of concrete schemes. Generate the list by running:

```bash
ls crates/credential/src/scheme/
```

And enumerate each scheme with a one-line description.

- [ ] **Step 5: Commit**

```bash
git add docs/PRODUCT_CANON.md docs/INTEGRATION_MODEL.md \
        crates/execution/README.md crates/resource/README.md crates/credential/README.md
git commit -m "docs(pass-2): evict L4 detail (idempotency format, drain types, auth scheme count) from canon to crate READMEs"
```

### Task 2.9: Add seam pointers to §11 invariants and §13 knife

**Files:**
- Modify: `docs/PRODUCT_CANON.md` — §11 and §13

[spec §5.4, §6]

- [ ] **Step 1: For each §11 invariant, add a `Seam:` line**

For §11.1:
```
**[L2]** Seam: `crates/execution/src/repo.rs` — `ExecutionRepo::transition`. Test: `crates/execution/tests/authority.rs`.
```

For §11.3:
```
Seam: `crates/execution/src/idempotency.rs` — `IdempotencyRepo::mark_and_execute`. Test: `crates/execution/tests/idempotency.rs`.
```

For §11.4:
```
Seam: `crates/resource/src/release_queue.rs` — `ReleaseQueue`. Test: `crates/resource/tests/basic_integration.rs`.
```

For §11.5:
```
Seam: `crates/execution/src/checkpoint.rs` + `crates/execution/src/journal.rs`. Test: `crates/execution/tests/persistence.rs`.
```

**Important:** before committing, verify each referenced test file exists. Run:

```bash
ls crates/execution/tests/authority.rs crates/execution/tests/idempotency.rs crates/execution/tests/persistence.rs crates/resource/tests/basic_integration.rs 2>&1
```

If a test file does not exist, **do not commit a seam pointer to it**. Instead:
- Option A: create a placeholder test file that compiles and is marked `#[ignore] // TODO(seam): pass 2 placeholder` — defer actual implementation to a separate plan.
- Option B: the seam pointer cites the module only, no test file. Add a note "test coverage: see `docs/MATURITY.md`".

Pick Option B (no placeholder tests in a docs PR; implementation work is separate).

- [ ] **Step 2: For §13 knife steps, add seam + test pointers per step**

For step 5 (cancellation), add:
```
Seam: `crates/api/src/handlers/cancel.rs` + `crates/execution/src/repo.rs::transition_with_signal`. Test: cite the integration test harness if it exists; otherwise cite the code seam only and mark the test-coverage gap in MATURITY.md.
```

Apply same approach to other steps as able.

- [ ] **Step 3: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): add seam + test pointers to §11 invariants and §13 knife steps"
```

### Task 2.10: Rewrite §17 DoD by layer

**Files:**
- Modify: `docs/PRODUCT_CANON.md` — §17

[spec §5.3]

- [ ] **Step 1: Replace §17 body**

New content:

````markdown
## 17. Definition of done — by canon layer

Incomplete if:

- **[L1]** violated without an explicit canon revision + product-level rationale in the PR description.
- **[L2]** violated without an ADR in `docs/adr/` + a seam test update in the same PR.
- **[L3]** violated without a PR rationale (one paragraph in the PR body) and, if behavior changed, a test.
- **[L4]** "violation" is not a violation — it is a move of detail into the owning crate's README. If you think an L4 rule is in canon, open an ADR per §0.2 and move it.

Additional DoD items (unchanged from prior canon):

- §13 knife scenario (execution and integration bar where those features exist) not broken or narrowed without replacement by a stronger scenario.
- No new public API surface that contradicts typed-error / layering rules.
- A new public behavioral contract requires §11-level honesty; docs must not mislabel implementation state.
- A new outbox / queue / worker lands with its consumer (or explicit §12.7 exemption) in the same PR.
- A removed backend, endpoint, or capability is not still advertised in `README.md` or `docs/`.
- Local path stays documented in `CLAUDE.md` or `README` where applicable.
````

- [ ] **Step 2: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): rewrite §17 DoD by canon layer (L1/L2/L3/L4)"
```

### Task 2.11: Fill STYLE.md with content

**Files:**
- Modify: `docs/STYLE.md`

[spec §8]

- [ ] **Step 1: Fill the five content sections**

Replace the `*Filled in Pass 2.*` placeholders with:

````markdown
## 1. Idioms we use

- **`mem::take` / `mem::replace`** — extract owned values from `&mut self` without cloning. Pairs with `Default`.
- **Newtype wrappers** — `pub struct CredentialKey(String)`, `pub struct ActionKey(String)` — strong types for identifiers, not `String` aliases.
- **Builder pattern** — for any type with more than three fields, especially when some are optional. Consumes `self` (not `&mut self`) to enable method chaining and prevent re-use after `build()`.
- **RAII guards** — release-on-drop for resource lifecycle. Companion to explicit `.release()` when an async release path exists; the guard handles the crash path.
- **Typestate** — phantom types on state transitions where the engine can enforce them at compile time. Example: `Execution<Running>` → `Execution<Terminal>` via a transition method.
- **`#[must_use]`** — on every `Result`, every builder, every function returning a cleanup or cancellation token.
- **`Cow<'_, T>`** — prefer over premature cloning for read-mostly borrows with occasional mutation.
- **Sealed traits** for extension points — define via private supertrait when Nebula owns all implementations; opens later if we decide to allow downstream impls.

## 2. Antipatterns we reject

- **`.clone()` to satisfy the borrow checker without a tradeoff note.** Consider `Cow<'_, T>`, lifetime redesign, typestate, or `Arc` first. If cloning is the right answer, leave a comment explaining why (rare — usually the signal is over-application of the clone).
- **`Deref` polymorphism.** Do not use `Deref` to simulate inheritance. Prefer explicit methods or trait delegation.
- **Stringly-typed public APIs.** `fn do(thing: &str)` where `thing` has a finite set of values — use an enum.
- **`anyhow` in library crates.** Use `thiserror` with typed errors. `anyhow` is for binaries only.
- **`unwrap` outside tests.** Use `expect` with a documented invariant at minimum, typed error propagation preferred.
- **Implicit panics in async state.** `assert!` inside an async fn on a path not guarded by a type-level invariant is a latent outage. Use typed errors.
- **Orphan modules.** A module that is never imported from the crate's `lib.rs` is either dead code or a test-only module in the wrong place.
- **Direct state mutation bypassing repository.** Any field like `ns.state = X` or `let _ = transition(...)` that skips version bumps is broken — see memory `feedback_direct_state_mutation.md`.

## 3. Naming table

| Suffix / pattern | Meaning | Example |
|---|---|---|
| `*Metadata` | UI-facing description: id, display name, icon, categories | `ActionMetadata`, `CredentialMetadata` |
| `*Schema` | Typed config schema (from `nebula-schema`) | `ActionSchema`, `CredentialSchema` |
| `*Key` | Stable identifier used across layers | `ExecutionKey`, `ActionKey`, `CredentialKey` |
| `*Id` | Runtime identifier, not stable across restarts | `ExecutionId` (durable), `SessionId` |
| `*Error` / `*ErrorKind` | Typed errors from `thiserror` | `ExecutionError`, `CredentialErrorKind` |
| `*Repo` | Storage port — trait abstracting persistence | `ExecutionRepo`, `CredentialRepo` |
| `*Handle` | Borrowed reference to a managed resource | `ResourceHandle<T>` |
| `*Guard` | RAII type enforcing cleanup on drop | `LeaseGuard`, `ScopeGuard` |
| `*Token` | Capability / continuation / dedup token | `CancellationToken`, `IdempotencyToken` |
| `*Policy` | Configuration type for a behavioral decision | `CheckpointPolicy`, `DrainTimeoutPolicy` |

## 4. Error taxonomy

Library crates use `thiserror`. Error types derive `Debug`, `thiserror::Error`, and do not implement `Clone` unless a specific consumer requires it.

All errors flow through `nebula-error::NebulaError` at module boundaries. The `Classify` trait decides transient vs permanent — classification is explicit, not inferred from error message strings.

API boundary: every `ApiError` variant maps to an RFC 9457 `problem+json` response. New failure modes get a new typed variant with an explicit HTTP status — no ad-hoc `500` for business-logic mistakes.

Secret-bearing errors: never include the secret in the error string. Use a redacted indicator (e.g. `CredentialError::TokenRefreshFailed { credential_id: .., reason: RedactedReason }`).

## 5. Type design bets

Defended pre-1.0 — changing any of these is an ADR-level decision:

- **Sealed traits for integration extension points.** `Action`, `Credential`, `Resource` traits seal via `crate` supertrait; downstream crates implement via derive macros or helper traits rather than by naming the sealed bound.
- **Typestate for lifecycle enforcement.** `Execution<Planned>` → `<Running>` → `<Terminal>` — transitions are methods consuming `self`; invalid state transitions fail to compile.
- **`Zeroize` + `ZeroizeOnDrop` on secret material.** Every type containing `SecretString`, `SecretToken`, or raw key bytes implements zeroization. `Debug` is redacted — leaking is a PR-level blocker.
- **`#[non_exhaustive]` on public enums and structs we intend to grow.** Consumers must use `_` or `..` in matches / destructuring, leaving us room to add variants / fields without SemVer breakage.
- **`#[unstable(feature = "...")]`-gated public API for aspirational surface.** Anything not yet engine-honored hides behind an unstable feature flag with an issue tracker link — never ships on a stable release path.

## 6. When to fight canon

See `docs/PRODUCT_CANON.md §0.2`. Style rules here are subject to the same revision triggers.
````

- [ ] **Step 2: Verify STYLE.md reads coherently**

Run: `wc -l docs/STYLE.md`
Expected: 120–200 lines.

- [ ] **Step 3: Commit**

```bash
git add docs/STYLE.md
git commit -m "docs(pass-2): fill STYLE.md content (idioms, antipatterns, naming, errors, type design bets)"
```

### Task 2.12: Expand GLOSSARY.md with Architectural Patterns section

**Files:**
- Modify: `docs/GLOSSARY.md`

[spec §4.2]

- [ ] **Step 1: Read current GLOSSARY.md**

Run: `cat docs/GLOSSARY.md`

Understand current structure. Then append a new top-level section after existing content:

- [ ] **Step 2: Append Architectural Patterns section**

````markdown

---

## Architectural patterns

Named patterns Nebula uses. Shared vocabulary with the industry corpus (EIP, DDIA, Release It!). Canon rules refer to these by name — this section is the authoritative source for each pattern's Nebula implementation.

| Pattern | Book reference | Nebula implementation |
|---|---|---|
| **Transactional Outbox** | DDIA ch 11; EIP "Guaranteed Delivery" | `ExecutionControlQueue` (`crates/execution/src/control_queue.rs`). Signals written in the same tx as state transitions. |
| **Write-Ahead Log** | DDIA ch 3, 11 | `execution_journal` append-only table; replayable event history. |
| **Idempotent Receiver** | EIP | `crates/execution/src/idempotency.rs` — deterministic per-attempt key checked before side effect. |
| **Optimistic Concurrency Control** | DDIA ch 7 | CAS on `version` column via `ExecutionRepo::transition`. |
| **Bulkhead** | Release It! | `crates/resource/src/release_queue.rs` — scope-bounded resource release; failure in one scope does not cascade. |
| **Circuit Breaker + Timeout + Retry-with-Backoff** | Release It! | `nebula-resilience` composable pipelines — applied at outbound call sites inside actions. |
| **Layered Architecture with cross-cutting infrastructure** | Fundamentals of SW Architecture | `CLAUDE.md` layer direction: API → Exec → Business → Core, cross-cutting below. |
| **Sealed trait + typestate** | Rust for Rustaceans, ch Designing Interfaces | Integration extension points (`Action`, `Credential`, `Resource`) and execution lifecycle (`Execution<State>`). |
| **Make illegal states unrepresentable** | Domain Modeling Made Functional | Applied to public surfaces (§4.5): a type exists ⇔ engine honors it. |
````

- [ ] **Step 3: Commit**

```bash
git add docs/GLOSSARY.md
git commit -m "docs(pass-2): add Architectural Patterns section to GLOSSARY.md"
```

### Task 2.13: Shrink INTEGRATION_MODEL.md — fix nebula-parameter references, apply sealed-trait form, add status tags

**Files:**
- Modify: `docs/INTEGRATION_MODEL.md`

[spec §5.4]

- [ ] **Step 1: Remove the draft banner and update the frontmatter**

Change the frontmatter `status:` from `draft — extracted from canon, surgery in Pass 2` to `accepted`.

Delete the "Status" banner paragraph at the top.

- [ ] **Step 2: Replace every `nebula-parameter` reference with `nebula-schema` context**

Run: `grep -n "nebula-parameter\|ParameterCollection\|Parameter" docs/INTEGRATION_MODEL.md`

For each hit, rewrite per the new schema model:

- `nebula-parameter` → `nebula-schema`
- `ParameterCollection` → `Schema`
- `Parameter` (in the config-schema sense) → `Field` (matching `nebula-schema`'s `Field` enum)

Structural contract table:

````markdown
| Piece | Role |
|---|---|
| `*Metadata` | UI-facing description — id, display name, icon, version, concept-specific fields (categories, isolation, checkpoint policy for Actions). |
| `Schema` | Typed configuration schema (`nebula-schema`: `Field`, `Schema`, `ValidValues`, `ResolvedValues`). One schema system used across Resource config, Credential setup, and Action inputs. |
````

The proof-token pipeline paragraph (new — describes nebula-schema's `ValidSchema::validate` → `ValidValues` → `resolve` → `ResolvedValues` flow) should replace the old "parameter subsystem" paragraph.

- [ ] **Step 3: Apply status tags to crate pointers**

For each crate pointer (`nebula-resource`, `nebula-credential`, `nebula-action`, `nebula-schema`), add a status line:

```
Status: stable | frontier | partial (see docs/MATURITY.md).
```

Do not invent statuses — use placeholder `(status: see docs/MATURITY.md)` if uncertain; Pass 4 fills MATURITY and this ambiguity disappears.

- [ ] **Step 4: Tag §7.1 `[signing]` block as planned**

In the plugin packaging section (former canon §7.1), at the start of the Signing subsection, add:

```markdown
> **Status: planned.** The `[signing]` block fields and canonical serialization are tooling-defined and not frozen. Verification logic is on the isolation roadmap (see canon §12.6). Do not rely on signing as an active trust boundary until this status changes.
```

- [ ] **Step 5: Tag `CheckpointPolicy` reference**

In the Action section of INTEGRATION_MODEL.md, where `CheckpointPolicy` is mentioned, add:

```markdown
> **Status of `CheckpointPolicy`:** see `crates/action/README.md` for implementation state; current engine honoring of this policy is tracked in `docs/MATURITY.md` row for `nebula-action`.
```

- [ ] **Step 6: Remove "twelve universal auth schemes" specific count**

Replace:
```
Twelve universal auth schemes (plus extensibility via `AuthScheme`)
```

With:
```
A set of universal auth schemes (OAuth2, API key, mTLS, and others — full list in `crates/credential/README.md`) plus extensibility via the `AuthScheme` trait defined in `crates/core/src/auth.rs`.
```

- [ ] **Step 7: Commit**

```bash
git add docs/INTEGRATION_MODEL.md
git commit -m "docs(pass-2): rewrite INTEGRATION_MODEL.md for nebula-schema; add status tags to signing / CheckpointPolicy"
```

### Task 2.14: Update COMPETITIVE.md frontmatter

**Files:**
- Modify: `docs/COMPETITIVE.md`

[spec §4.2]

- [ ] **Step 1: Change status to accepted**

In the frontmatter, change `status: draft — extracted from canon, surgery in Pass 2` to `status: accepted`. Delete the "Status" banner at the top.

Optionally add a short note at the bottom on today-vs-aspiration framing:

```markdown

## Today vs aspiration

The bets above describe where Nebula intends to win. Current engine maturity — which bets are delivered end-to-end, which are in-progress — is tracked per-crate in `docs/MATURITY.md` and per-invariant in `docs/PRODUCT_CANON.md §11`. Do not read this document as a capabilities matrix; read it as an articulation of direction.
```

- [ ] **Step 2: Commit**

```bash
git add docs/COMPETITIVE.md
git commit -m "docs(pass-2): promote COMPETITIVE.md from draft to accepted; add today-vs-aspiration note"
```

### Task 2.15: Fix §15 satellites table in canon

**Files:**
- Modify: `docs/PRODUCT_CANON.md` — §15 table

[spec §5.4]

- [ ] **Step 1: Replace the §15 table**

New §15 table content:

````markdown
| Document | Role |
|---|---|
| `CLAUDE.md` | Commands, formatting, session read-order, decision gate, trap catalog. |
| `docs/PRODUCT_CANON.md` (this file) | Normative core — pillars (§4), golden path (§10), contracts (§11), invariants (§12), knife (§13), anti-patterns (§14), decision filter (§16), DoD (§17). Layer-tagged. |
| `docs/INTEGRATION_MODEL.md` | Integration model mechanics — Resource / Credential / Action / Schema / Plugin contract, wiring rules, plugin packaging, status of aspirational surfaces. |
| `docs/COMPETITIVE.md` | Peer analysis and our bets against n8n / Temporal / Windmill / Make / Zapier. Explicitly persuasive. |
| `docs/OBSERVABILITY.md` | SLI / SLO / error budgets, structured event schema, operator core-analysis loop. |
| `docs/STYLE.md` | Idioms / antipatterns / naming / error taxonomy / type-design bets. |
| `docs/GLOSSARY.md` | Terms and architectural patterns (Outbox, WAL, Idempotent Receiver, Bulkhead, Circuit Breaker, OCC). |
| `docs/MATURITY.md` | Per-crate state dashboard (API stability, test coverage, doc completeness, engine integration, SLI-ready). |
| `docs/dev-setup.md` | Local dev environment setup. |
| `docs/ENGINE_GUARANTEES.md` | Operator-facing guarantees narrative (satellite of §11). |
| `docs/UPGRADE_COMPAT.md` | Engine upgrade and workflow compatibility rules (satellite of §7.2). |
| `docs/PLUGIN_MODEL.md` | Plugin packaging mechanics (overlaps INTEGRATION_MODEL — consider merging in a later pass). |
| `docs/adr/` | Architecture Decision Records — `NNNN-kebab-title.md`, no central index. |
| `README.md` | Operator-facing summary. Must not contradict §5 / §11.5 / §12.3. |
| `crates/*/README.md` + `lib.rs //!` | Per-crate: Role (named pattern), Contract (invariants + seam tests), Public API, Non-goals, Maturity. |
| `crates/storage/migrations/{sqlite,postgres}/README.md` | Schema parity between dialects. |
````

- [ ] **Step 2: Commit**

```bash
git add docs/PRODUCT_CANON.md
git commit -m "docs(pass-2): rewrite §15 satellites table; add STYLE / MATURITY / dev-setup / INTEGRATION_MODEL / COMPETITIVE / OBSERVABILITY"
```

### Pass 2 — final verification

- [ ] **Step 1: Verify canon size**

Run: `wc -l docs/PRODUCT_CANON.md`
Expected: ≤ 280 lines (target ≤250, tolerance +30).

If over, identify sections still carrying persuasive text or per-crate detail and move to satellites. Do not over-compress L1 principles.

- [ ] **Step 2: Verify no stale nebula-parameter references remain in priming / satellite docs**

Run: `grep -rln "nebula-parameter\|ParameterCollection" docs/ CLAUDE.md README.md 2>/dev/null`
Expected: empty output (crate READMEs in `crates/*/README.md` are addressed in Pass 4).

- [ ] **Step 3: Verify all L-tags present**

Run: `grep -cE '\*\*\[L[123]\]\*\*' docs/PRODUCT_CANON.md`
Expected: ≥ 30.

- [ ] **Step 4: Verify canon has §0.1, §0.2, updated §17**

Run: `grep -c "^## 0\.\|^## 17\." docs/PRODUCT_CANON.md`
Expected: ≥ 2 (for §0.1, §0.2, §17 — match counts).

Run: `grep "revision triggers" docs/PRODUCT_CANON.md`
Expected: at least one match in §0.2.

- [ ] **Step 5: Create pass-2 PR**

```bash
git log --oneline -20
# pass-2 commits on top of pass-1 and pass-0
# push and open PR: "docs: pass 2 — canon surgery (layered rules, DMMF §4.5, revision triggers)"
```

---

## PASS 3 — ADR extraction and plans cleanup

**Goal:** Seed `docs/adr/` with ADRs extracted from completed plans; delete pure task-plans whose work has landed; fill `docs/OBSERVABILITY.md` with SLI/SLO content.

**PR shape:** one PR titled `docs: pass 3 — ADR extraction, plans cleanup, observability fill`.

### Task 3.1: Create docs/adr/ directory and extract 0001-schema-consolidation.md

**Files:**
- Create: `docs/adr/0001-schema-consolidation.md`
- Reference: `docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md`, commit `ed3a0ce0`

[spec §9 Pass 3]

- [ ] **Step 1: Create the ADR directory**

Run: `mkdir -p docs/adr`

- [ ] **Step 2: Read the source spec to extract the decision**

Run: `cat docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md | head -100`

Identify the decision and consequences — the ADR should capture: *we delete `nebula-parameter`, consolidate field types into `nebula-schema::Field`, add proof-token pipeline (`ValidSchema::validate` → `ValidValues::resolve` → `ResolvedValues`).*

- [ ] **Step 3: Write the ADR**

Create `docs/adr/0001-schema-consolidation.md`:

````markdown
---
id: 0001
title: schema-consolidation
status: accepted
date: 2026-04-17
supersedes: []
superseded_by: []
tags: [schema, parameter, integration-model]
related: [docs/INTEGRATION_MODEL.md, crates/schema/src/lib.rs]
---

# 0001. Schema consolidation — delete `nebula-parameter`, adopt `nebula-schema`

## Context

The `nebula-parameter` crate provided the typed configuration schema for Actions,
Credentials, and Resources: `Parameter`, `ParameterCollection`, validation rules,
conditions. Over time it accumulated scope creep (transformer pipelines, dynamic
fields, display modes) and duplicated functionality already present in
`nebula-validator`. The Field type hierarchy was an enum plus many per-kind
structs, leading to pattern-match fragility and a public API surface that
outpaced engine honoring.

Phase 1 spec (`docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md`)
proposed a consolidation.

## Decision

Delete `nebula-parameter`. Create `nebula-schema` with:

- A single consolidated `Field` enum covering all field kinds.
- `Schema` builder with structural lint (`Schema::lint`).
- Proof-token pipeline: schema-time validation via `ValidSchema::validate` returns `ValidValues`; runtime expression resolution via `ValidValues::resolve` returns `ResolvedValues`.
- Strongly typed error and path types.

Each integration concept (Action / Credential / Resource) composes `*Metadata + Schema`.

## Consequences

Positive:

- One schema system instead of two (nebula-parameter validation + nebula-validator rules).
- Proof-tokens make "this schema has been validated" and "these values resolve" compile-time-evident — caller cannot forget the check.
- Public API surface shrinks; pattern-match fragility eliminated for common field kinds.

Negative:

- Breaking change for any consumer of `nebula-parameter` (none outside the workspace at time of consolidation).
- All docs referencing `nebula-parameter` must be updated (see Pass 2 canon surgery, Pass 4 crate sweep).

Follow-up:

- Phase 2 (DX layer), Phase 3 (security), Phase 4 (advanced) per the source specs.
- Crate README for `nebula-schema` created in Pass 4 of the docs redesign.

## Alternatives considered

- **Keep `nebula-parameter`, incrementally adopt proof-tokens inside it.** Rejected: the surface area was already too large; proof-token adoption without a crate boundary change would not reduce complexity.
- **Split into `nebula-schema` + `nebula-fields` + `nebula-validation`.** Rejected: premature decomposition. One crate with clear module boundaries beats three crates with unclear edges at this stage.

## Seam / verification

Seam: `crates/schema/src/lib.rs`. Tests: `crates/schema/tests/` (populated during Phase 1 landing).

Canon reference: `docs/INTEGRATION_MODEL.md` (structural contract); `docs/PRODUCT_CANON.md §1` (one-liner names `nebula-schema`).
````

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0001-schema-consolidation.md
git commit -m "docs(pass-3): ADR 0001 — schema consolidation (delete nebula-parameter, adopt nebula-schema)"
```

### Task 3.2: Extract ADRs 0002–0006 from plans

**Files:**
- Create: `docs/adr/0002-proof-token-pipeline.md`
- Create: `docs/adr/0003-consolidated-field-enum.md`
- Create: `docs/adr/0004-credential-metadata-rename.md`
- Create: `docs/adr/0005-trigger-health-trait.md`
- Create: `docs/adr/0006-sandbox-phase1-broker.md`

[spec §9 Pass 3]

- [ ] **Step 1: Identify source plans for each ADR**

For each ADR, read the corresponding plan to extract the decision. Source mapping:

| ADR | Source plan | Related commit |
|---|---|---|
| 0002 | `docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md` (proof-token subsection) | `ed3a0ce0` |
| 0003 | `docs/superpowers/specs/2026-04-16-nebula-schema-phase1-foundation-design.md` (Field enum subsection) | `ed3a0ce0` |
| 0004 | `docs/superpowers/specs/2026-04-17-rename-credential-metadata-description.md` | `51baa36f` |
| 0005 | `crates/trigger/*` or relevant plan (verify with `git log --all --grep trigger`) | latest trigger-health commit |
| 0006 | `docs/plans/2026-04-13-sandbox-phase1-broker.md` + `docs/plans/2026-04-13-sandbox-roadmap.md` | latest sandbox commit |

- [ ] **Step 2: For each ADR, follow the same template as Task 3.1**

Template:
```
---
id: NNNN
title: <kebab>
status: accepted
date: <date when the decision landed per git log>
supersedes: []
superseded_by: []
tags: [...]
related: [...]
---

# NNNN. <Title>

## Context
## Decision
## Consequences
## Alternatives considered
## Seam / verification
```

Keep each ADR short and focused — one decision per ADR. Prefer referencing the source spec for detail rather than duplicating.

**If an ADR's source material is unclear or the work has not landed yet**, mark `status: proposed` rather than `accepted`, and add a note in the Context section.

- [ ] **Step 3: Commit per ADR**

Commit each ADR as a separate commit for review readability:

```bash
git add docs/adr/0002-proof-token-pipeline.md
git commit -m "docs(pass-3): ADR 0002 — proof-token pipeline (ValidSchema / ValidValues / ResolvedValues)"

git add docs/adr/0003-consolidated-field-enum.md
git commit -m "docs(pass-3): ADR 0003 — consolidated Field enum"

git add docs/adr/0004-credential-metadata-rename.md
git commit -m "docs(pass-3): ADR 0004 — credential Metadata→Record, Description→Metadata rename"

git add docs/adr/0005-trigger-health-trait.md
git commit -m "docs(pass-3): ADR 0005 — TriggerHealth trait (future: generalize to Health trait)"

git add docs/adr/0006-sandbox-phase1-broker.md
git commit -m "docs(pass-3): ADR 0006 — sandbox Phase 1 broker (ProcessSandbox foundation)"
```

### Task 3.3: Verify and delete completed task-plans

**Files:**
- Delete: various files in `docs/plans/`

[spec §4.4]

- [ ] **Step 1: Enumerate `docs/plans/` and classify each**

Run: `ls docs/plans/`

For each plan file, verify whether its tracked work has landed. Workflow per plan:

```bash
# Read the plan
cat docs/plans/<plan>.md

# Check git log for related commits
git log --oneline --all --grep "<keyword from plan title>" | head -10

# Check if referenced types / modules exist in code
grep -rln "<key type from plan>" crates/ 2>/dev/null
```

Classify each:
- **delete** — work landed, no unique rationale worth keeping (raw task checklists).
- **convert to ADR** — architectural decision worth preserving; if not already covered by 0001–0006, create a new ADR.
- **archive** — ambiguous state; move to `docs/plans/archive/`.
- **keep** — work still in progress; update frontmatter with `status: active`.

- [ ] **Step 2: Known deletions (from spec §4.4)**

For each of these, run the verification above. If confirmed landed:

```bash
git rm docs/plans/2026-04-14-batch3-resource-lifecycle.md
git rm docs/plans/2026-04-14-batch4-api-sandbox-security.md
git rm docs/plans/2026-04-14-batch5-misc-lifecycle.md
# Others as identified
```

Do not delete `docs/plans/2026-04-15-arch-specs/` (directory with subcontent — review separately) or active plans without verification.

- [ ] **Step 3: Create archive directory if needed**

If any plan is ambiguous:

```bash
mkdir -p docs/plans/archive
git mv docs/plans/<ambiguous-plan>.md docs/plans/archive/
```

- [ ] **Step 4: Commit the cleanup**

```bash
git status
git commit -m "docs(pass-3): delete completed task-plans (batch3/4/5 lifecycle); archive ambiguous plans"
```

### Task 3.4: Fill OBSERVABILITY.md content

**Files:**
- Modify: `docs/OBSERVABILITY.md`

[spec §10]

- [ ] **Step 1: Fill each placeholder section**

Replace `*Filled in Pass 3.*` placeholders with:

````markdown
## 1. Service level indicators (SLIs)

Nebula's SLIs describe observable, measurable engine behavior that matters to operators. Each SLI has a measurement method (where the number comes from), a rolling window (typically 28 days), and a canonical name used in dashboards and alerts.

| SLI | Measurement | Window |
|---|---|---|
| `execution_terminal_rate` | `SELECT count(*) FILTER (WHERE status IN ('succeeded','failed','cancelled')) / count(*) FROM executions WHERE started_at >= now() - interval '28 days'` | 28d |
| `cancel_honor_latency_p95` | Histogram of `cancelled_at - cancel_requested_at` over the same window | 28d, p95 |
| `checkpoint_write_success_rate` | Ratio of successful checkpoint writes to attempted checkpoint writes (emitted from `nebula-execution` metrics) | 28d |
| `dispatch_lag_p95` | Histogram of `control_queue_drained_at - control_queue_inserted_at` | 28d, p95 |

## 2. Service level objectives (SLOs)

SLOs are operator commitments. Numbers below are targets; actuals live in the maturity dashboard per crate (`docs/MATURITY.md` `SLI ready` column) and in the runtime dashboard outside this repo.

| SLI | SLO target | Rationale |
|---|---|---|
| `execution_terminal_rate` | ≥ 99.0% | 1% budget absorbs legitimate long-running / externally-blocked runs and genuine engine failures. |
| `cancel_honor_latency_p95` | ≤ 5 seconds under default dispatch interval | Operators expect "cancel" to mean "stop within a few seconds"; slower violates §10 knife step 5. |
| `checkpoint_write_success_rate` | ≥ 99.9% | Checkpoint loss degrades recovery fidelity; §11.5 best-effort framing assumes rare failure. |
| `dispatch_lag_p95` | ≤ 1 second | Control-plane signals (cancel, trigger) must feel immediate to operators. |

## 3. Error budgets

Error budget = `1 - SLO`. Budgeting policy:

- Budget burn > 10% in a rolling 7-day window triggers an investigation (not paging).
- Budget burn > 50% in 24 hours pages the on-call.
- Budget reset is rolling, not calendar — no "fresh budget on the 1st" effect.

## 4. Structured event schema (`execution_journal`)

Every durable event appended to `execution_journal` follows this shape:

```jsonc
{
  "execution_id": "exec_...",
  "node_id": "node_...",
  "attempt": 1,
  "correlation_id": "trace_...",
  "trace_id": "...",       // OpenTelemetry
  "span_id": "...",        // OpenTelemetry
  "event_type": "started" | "checkpoint" | "retry" | "cancel_requested" | "cancelled" | "failed" | "succeeded" | ...,
  "payload": { ... },      // event-type specific
  "timestamp": "2026-04-17T..."
}
```

High-cardinality fields (`execution_id`, `node_id`, `correlation_id`, `trace_id`) are required; enums (`event_type`) are documented with a closed set in `crates/execution/src/journal.rs`.

Principle (from Observability Engineering): append rich structured events first, aggregate to metrics second. Never add a metric without the underlying event being available for drill-down.

## 5. Core analysis loop

Operator procedure for any failed or stuck run:

1. **What failed?** Query `execution_journal` by `execution_id` for the last event before the failure. `event_type` + `payload.error` pins the failing step.
2. **When?** Compare `event_type='started'` timestamp to the failure event timestamp; cross-reference with `trace_id` in the observability stack.
3. **What changed?** Check recent deploys, config changes, dependency upgrades — `MATURITY.md` `frontier` crates are likely culprits if the run touched them.
4. **What to try?** For transient classifications (per `nebula-error::Classify`): wait and retry. For permanent: open an issue with the journal excerpt. For "unknown": ask in #observability with the trace_id; do not retry blindly.

This loop is the operational half of PRODUCT_CANON §2 success sentence: *you can explain what happened in a run without reading Rust source.*
````

- [ ] **Step 2: Update frontmatter status**

Change `status: skeleton` to `status: accepted` (or `draft` if some sections still need per-crate verification).

- [ ] **Step 3: Commit**

```bash
git add docs/OBSERVABILITY.md
git commit -m "docs(pass-3): fill OBSERVABILITY.md — SLIs, SLOs, error budgets, event schema, core analysis loop"
```

### Pass 3 — final verification

- [ ] **Step 1: Verify ADR directory**

Run: `ls docs/adr/`
Expected: `0001-schema-consolidation.md` through `0006-sandbox-phase1-broker.md` (or a subset if some ADRs were marked `proposed` without a source).

- [ ] **Step 2: Verify plans cleanup**

Run: `ls docs/plans/`
Expected: reduced from 10 to fewer. No `batch3`/`batch4`/`batch5` files remain.

- [ ] **Step 3: Verify OBSERVABILITY.md has content**

Run: `grep -c "Filled in Pass" docs/OBSERVABILITY.md`
Expected: `0`.

Run: `grep -c "^## " docs/OBSERVABILITY.md`
Expected: ≥ 5 (five content sections).

- [ ] **Step 4: Create pass-3 PR**

```bash
git log --oneline -30
# pass-3 commits on top of pass-2
# push and open PR: "docs: pass 3 — ADR extraction, plans cleanup, observability fill"
```

---

## PASS 4 — Crate sweep

**Goal:** Normalize all 24 `crates/*/README.md` to the template from `docs/DOC_TEMPLATE.md`; sync `crates/*/src/lib.rs //!` headers; create `crates/schema/README.md` from scratch; fill `docs/MATURITY.md` dashboard; naming audit across crate docs.

**PR shape:** one PR titled `docs: pass 4 — crate README + lib.rs sweep`. Given the scale (24 crates × ~100 lines each ≈ 2400 lines of doc), the PR may be split into subgroups if reviewers prefer — see batching note below.

**Batching option:** if a single PR is too large, split by layer (per `CLAUDE.md` boundary diagram):
- Batch A: cross-cutting (`core`, `error`, `resilience`, `log`, `metrics`, `telemetry`, `eventbus`, `system`, `config` if present).
- Batch B: core domain (`validator`, `expression`, `workflow`, `execution`).
- Batch C: business (`credential`, `resource`, `action`, `plugin`, `schema`).
- Batch D: exec (`engine`, `runtime`, `storage`, `sandbox`, `sdk`, `plugin-sdk`).
- Batch E: API (`api`).

Each batch ends with `MATURITY.md` rows filled for the included crates.

### Task 4.1: Create crates/schema/README.md from scratch

**Files:**
- Create: `crates/schema/README.md`

[spec §4.4, §5.4]

- [ ] **Step 1: Read `crates/schema/src/lib.rs` for accurate content**

Run: `cat crates/schema/src/lib.rs | head -80`

Record: public types (`Field`, `Schema`, `ValidValues`, `ResolvedValues`), public modules, key methods, proof-token semantics.

- [ ] **Step 2: Write the README per DOC_TEMPLATE.md shape**

Create `crates/schema/README.md`:

````markdown
---
name: nebula-schema
role: Typed Configuration Schema with Proof-Token Pipeline (bespoke; informed by Domain Modeling Made Functional "make illegal states unrepresentable")
status: frontier
last-reviewed: 2026-04-17
canon-invariants: [L1-3.5, L1-4.5]
related: [nebula-validator, nebula-expression, nebula-action, nebula-resource, nebula-credential]
---

# nebula-schema

## Purpose

Typed configuration schema used by every integration concept (Actions, Credentials, Resources). Replaces the deleted `nebula-parameter` crate. Provides schema-time validation and runtime resolution as compile-time-evident steps through a proof-token pipeline.

## Role

**Typed Configuration Schema with Proof-Token Pipeline.** The shared schema system across all integration concepts. A caller cannot skip validation or resolution because the types enforce the sequence: you hold a `Schema`, you call `validate` to get `ValidValues`, you call `resolve` to get `ResolvedValues`. Each step is a type transition; the next step is only callable when the previous has completed.

Pattern inspiration: DMMF proof-tokens (ch "Modeling with Types") and Rust typestate (Rust for Rustaceans, ch Designing Interfaces).

## Public API

- `Field` — unified enum over all field kinds (string, number, bool, enum, nested, …).
- `Schema` — builder-constructed, lint-checked schema definition.
- `Schema::builder() -> SchemaBuilder` — entry point.
- `Schema::lint() -> Result<ValidSchema, LintError>` — structural check.
- `ValidSchema::validate(&FieldValues) -> Result<ValidValues, ValidationError>` — schema-time validation; returns the first proof-token.
- `ValidValues::resolve(&impl ExpressionContext) -> Result<ResolvedValues, ResolveError>` — runtime resolution; returns the second proof-token.
- `FieldValues`, `ResolvedValues` — value containers.

See `src/lib.rs` rustdoc for the quick-start example.

## Contract

- **[L1-3.5]** Schema is the typed-configuration surface for all integration concepts. See `docs/INTEGRATION_MODEL.md`.
- **[L1-4.5]** `ValidValues` and `ResolvedValues` are compile-time-evident proof-tokens: a caller cannot invoke `resolve` without first holding `ValidValues`, cannot access resolved fields without `ResolvedValues`. No runtime flags.
- **Structural lint** — `Schema::lint` enforces constraints that cannot be expressed in the builder type alone (duplicate keys, invariant violations across fields). Seam: `crates/schema/src/schema/lint.rs`. Tests: `crates/schema/tests/`.

## Non-goals

- Not a validation rules engine — see `nebula-validator` for programmatic validators and declarative `Rule`.
- Not an expression evaluator — resolution delegates to a caller-supplied `ExpressionContext` (implemented by `nebula-expression`).
- Not a UI form renderer — schema carries UI hints as data, rendering lives elsewhere.

## Maturity

See `docs/MATURITY.md` row for `nebula-schema`.

- API stability: `frontier` — Phase 1 Foundation just landed (commit `ed3a0ce0`); Phases 2–4 (DX, security, advanced) in progress.
- Core pipeline (lint → validate → resolve) is stable; peripheral APIs (UI hints, expression context adapters) may move.

## Related

- Canon: `docs/PRODUCT_CANON.md §1`, §3.5 (via `docs/INTEGRATION_MODEL.md`).
- ADRs: `docs/adr/0001-schema-consolidation.md`, `docs/adr/0002-proof-token-pipeline.md`, `docs/adr/0003-consolidated-field-enum.md`.
- Siblings: `nebula-validator` (rules), `nebula-expression` (resolution context).
````

- [ ] **Step 3: Commit**

```bash
git add crates/schema/README.md
git commit -m "docs(pass-4): create crates/schema/README.md (replaces deleted nebula-parameter docs)"
```

### Task 4.2: Normalize existing crate READMEs (per-crate process)

**Files:**
- Modify: `crates/action/README.md`
- Modify: `crates/api/README.md`
- Modify: `crates/core/README.md`
- … (24 total; schema done in 4.1)

[spec §4.3, §6]

**Process per crate** — apply this loop for each of the 23 remaining crates:

- [ ] **Step 1: Read current README and lib.rs for context**

```bash
cat crates/<crate>/README.md
head -40 crates/<crate>/src/lib.rs
```

Record: current `//!` content, key public types, purpose one-liner.

- [ ] **Step 2: Identify the named pattern for this crate's Role**

Reference `docs/GLOSSARY.md` Architectural Patterns section. Typical mapping:

| Crate | Likely Role |
|---|---|
| `nebula-core` | Shared Vocabulary (identifiers, keys, auth primitives) |
| `nebula-error` | Error Taxonomy + Boundary |
| `nebula-resilience` | Stability Patterns Pipeline (Release It!) |
| `nebula-log` | Structured Tracing Initialization |
| `nebula-telemetry` | Metric Primitives (histograms, label interning) |
| `nebula-metrics` | Metric Export + Label-Safety (Prometheus-style) |
| `nebula-eventbus` | Publish-Subscribe Channel with Back-Pressure |
| `nebula-expression` | Expression Evaluator |
| `nebula-system` | Host Probes (CPU / memory / network / disk pressure) |
| `nebula-validator` | Validation Rules Engine + Declarative `Rule` |
| `nebula-workflow` | Workflow Definition + Validation |
| `nebula-execution` | State Machine + Transactional Outbox + WAL |
| `nebula-storage` | Storage Port (SQLite/Postgres abstraction) |
| `nebula-credential` | Credential Contract (stored state vs projected auth material) |
| `nebula-resource` | Resource Lifecycle (acquire / health / release; Bulkhead) |
| `nebula-action` | Action Trait Family + Execution Policy Metadata |
| `nebula-plugin` | Plugin Distribution Unit (registry + metadata) |
| `nebula-plugin-sdk` | Plugin Author SDK |
| `nebula-sandbox` | Process Sandboxing (correctness boundary, not adversary-grade — see §12.6) |
| `nebula-runtime` | Execution Runtime (scheduler, control plane) |
| `nebula-engine` | Engine Composition Root |
| `nebula-sdk` | Integration Author SDK |
| `nebula-api` | HTTP API + Webhook module |

If a crate's role is unclear, read its `lib.rs //!` and recent commits before assigning. Do not invent a role.

- [ ] **Step 3: Identify canon invariants (L2) this crate enforces**

Cross-reference `docs/PRODUCT_CANON.md` for L2 tags that cite this crate's path or seams. List in frontmatter `canon-invariants`.

- [ ] **Step 4: Write the new README per DOC_TEMPLATE.md**

Use the template from `docs/DOC_TEMPLATE.md`. Fill each section with crate-specific content:

- **Purpose** — one paragraph, framed as a problem this crate solves.
- **Role** — named pattern + book reference.
- **Public API** — catalog of top-level types / traits / functions, rustdoc-link style.
- **Contract** — L-tagged invariants with seam + test pointers.
- **Non-goals** — what this crate does NOT do, point to the crate that does.
- **Maturity** — one-sentence summary; cross-ref MATURITY.md row.
- **Related** — siblings, satellites, canon sections.

**If the existing README contains unique content not covered by the template** (e.g., lengthy examples, migration notes): preserve that content in an `## Appendix` or `## Examples` section at the end. Do not delete unique content.

- [ ] **Step 5: Sync `lib.rs //!` header**

The `lib.rs //!` header should mirror sections 1–3 (Purpose, Role, Public API) of the README, in rustdoc-friendly form.

- Do not use bracketed intra-doc links at the top of `lib.rs //!` (rustdoc `-D warnings` cannot resolve out-of-scope paths). Use backtick-only for type names: `` `ExecutionRepo` ``, not `[ExecutionRepo]`.
- Keep the `//!` header to ~30–50 lines. Detail lives in the README.
- Preserve existing rustdoc examples if they compile — do not break doctests.

- [ ] **Step 6: Verify the rustdoc still builds**

Run: `cargo doc --no-deps -p nebula-<crate>`
Expected: no warnings or errors. If `-D warnings` is enforced by CI, resolve any broken intra-doc links before committing.

- [ ] **Step 7: Verify doctests still pass (if any)**

Run: `cargo test --doc -p nebula-<crate>`
Expected: all doctests pass.

- [ ] **Step 8: Commit per crate (or per batch)**

Single crate:
```bash
git add crates/<crate>/README.md crates/<crate>/src/lib.rs
git commit -m "docs(pass-4): normalize crates/<crate>/README.md + lib.rs //! to template"
```

Batched (5–6 crates at once):
```bash
git add crates/core/README.md crates/core/src/lib.rs \
        crates/error/README.md crates/error/src/lib.rs \
        crates/resilience/README.md crates/resilience/src/lib.rs
git commit -m "docs(pass-4): normalize cross-cutting crate READMEs — core, error, resilience"
```

Use batching for similar crates in the same architectural layer; individual commits for crates with substantial unique content.

**Checklist of 23 remaining crates** (mark each as normalized):

- [ ] `nebula-action`
- [ ] `nebula-api`
- [ ] `nebula-core`
- [ ] `nebula-credential`
- [ ] `nebula-engine`
- [ ] `nebula-error`
- [ ] `nebula-eventbus`
- [ ] `nebula-execution`
- [ ] `nebula-expression`
- [ ] `nebula-log`
- [ ] `nebula-metrics`
- [ ] `nebula-plugin`
- [ ] `nebula-plugin-sdk`
- [ ] `nebula-resilience`
- [ ] `nebula-resource`
- [ ] `nebula-runtime`
- [ ] `nebula-sandbox`
- [ ] `nebula-sdk`
- [ ] `nebula-storage`
- [ ] `nebula-system`
- [ ] `nebula-telemetry`
- [ ] `nebula-validator`
- [ ] `nebula-workflow`

### Task 4.3: Fill MATURITY.md dashboard

**Files:**
- Modify: `docs/MATURITY.md`

[spec §7]

- [ ] **Step 1: For each crate, evaluate each column**

Go through the 24 crates (including `nebula-schema`). For each, assess:

- **API stability:** `frontier` if recent breaking changes or imminent changes expected; `stable` if public API has been stable for months; `partial` if parts are stable, parts not.
- **Test coverage:** `stable` if `cargo nextest run -p <crate>` covers the public API and integration seams; `partial` if core paths covered but edges not; `frontier` if test suite is sparse.
- **Doc completeness:** `stable` if README follows template and lib.rs is synced (post-Pass 4); `partial` if README exists but lib.rs is thin; `frontier` if new docs.
- **Engine integration:** `stable` if exercised end-to-end by knife (`PRODUCT_CANON §13`); `partial` if wired but not in knife; `frontier` if standalone.
- **SLI ready:** `n/a` for most crates; `stable` if the crate emits metrics honoring `docs/OBSERVABILITY.md` schema; `frontier` if instrumented but not honored.

Sources for assessment:
- `cargo nextest list -p <crate>` for test coverage feel.
- `git log --oneline -20 crates/<crate>/` for recent churn (frontier signal).
- README / lib.rs post-Pass 4 for doc completeness.

- [ ] **Step 2: Update each row in the table**

Example filled rows:

```
| nebula-action        | partial  | partial  | stable   | partial  | n/a      |
| nebula-api           | partial  | partial  | stable   | stable   | partial  |
| nebula-core          | stable   | stable   | stable   | stable   | n/a      |
| nebula-credential    | stable   | stable   | stable   | stable   | n/a      |
| nebula-execution     | partial  | partial  | stable   | stable   | partial  |
| nebula-schema        | frontier | partial  | stable   | frontier | n/a      |
| nebula-sandbox       | frontier | partial  | stable   | partial  | n/a      |
```

Do not invent assessments — if uncertain, use `partial` and add a short inline note. Update frontmatter `status:` from `skeleton` to `accepted`.

- [ ] **Step 3: Add a "Changelog" footer for future reviewers**

Append:

```markdown

---

## Review cadence

This file is a living dashboard. Reviewers check truthfulness on every PR that touches a crate's public surface, test suite, or docs. Canon §17 DoD includes "MATURITY.md row updated if the PR changes crate state."

Last full sweep: 2026-04-17 (Pass 4 of docs architecture redesign).
```

- [ ] **Step 4: Commit**

```bash
git add docs/MATURITY.md
git commit -m "docs(pass-4): fill MATURITY.md dashboard (all 24 crates)"
```

### Task 4.4: Naming audit across crate docs

**Files:**
- Modify: any `crates/*/README.md` / `crates/*/src/lib.rs` still referencing `nebula-parameter` or `ParameterCollection`

[spec §4.4, §9 Pass 4]

- [ ] **Step 1: Re-run the stale-reference grep**

Run: `grep -rln "nebula-parameter\|ParameterCollection" crates/*/README.md crates/*/src/lib.rs 2>/dev/null`
Expected: ideally empty after Pass 4 normalization. If any hits remain, they are residual from rewrites that did not apply the naming change.

- [ ] **Step 2: For each hit, rewrite per `docs/STYLE.md §3 naming table`**

- `nebula-parameter` → `nebula-schema`
- `ParameterCollection` → `Schema`
- `Parameter` (in schema-field sense) → `Field`

Apply via `Edit` tool per file.

- [ ] **Step 3: Verify no hits remain**

Run: `grep -rln "nebula-parameter\|ParameterCollection" crates/ docs/ CLAUDE.md README.md 2>/dev/null`
Expected: empty.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "docs(pass-4): naming audit — scrub remaining nebula-parameter / ParameterCollection refs"
```

### Pass 4 — final verification

- [ ] **Step 1: Every crate has a README and lib.rs matches template**

Run: `for d in crates/*/; do n=$(basename "$d"); [ -f "$d/README.md" ] && echo "OK $n" || echo "MISSING $n"; done`
Expected: 24 `OK` lines, zero `MISSING`.

- [ ] **Step 2: README frontmatter present and valid**

Run: `for d in crates/*/; do grep -l "^name: nebula-\|^role: " "$d/README.md" >/dev/null && echo "OK $(basename $d)" || echo "MISSING $(basename $d)"; done`
Expected: all 24 `OK`.

- [ ] **Step 3: rustdoc builds clean**

Run: `cargo doc --workspace --no-deps`
Expected: no warnings.

- [ ] **Step 4: Doctests pass**

Run: `cargo test --workspace --doc`
Expected: all pass.

- [ ] **Step 5: MATURITY.md dashboard filled**

Run: `grep -c '^| nebula-' docs/MATURITY.md`
Expected: `24`.

Run: `grep -c '|  |' docs/MATURITY.md`
Expected: `0` (no empty cells).

- [ ] **Step 6: Create pass-4 PR**

```bash
git log --oneline -40
# pass-4 commits on top of pass-3
# push and open PR: "docs: pass 4 — crate README + lib.rs sweep"
```

---

## Final cross-pass verification

After all five passes land:

- [ ] **Step 1: Grand grep for stale references**

```bash
grep -rln "nebula-parameter\|ParameterCollection" . 2>/dev/null \
  | grep -v "^\./target\|^\./\.git\|docs/superpowers/specs\|docs/superpowers/plans\|docs/adr"
```

Expected: empty. (Historical specs, plans, and ADRs are allowed to mention the deleted crate — that is how the story is told.)

- [ ] **Step 2: Canon size**

Run: `wc -l docs/PRODUCT_CANON.md`
Expected: ≤ 280.

- [ ] **Step 3: Priming layer budget**

Run: `wc -l CLAUDE.md docs/PRODUCT_CANON.md docs/MATURITY.md docs/STYLE.md | tail -1`
Expected: total ≤ 1000 lines.

- [ ] **Step 4: Full canonical build passes**

Run: `cargo +nightly fmt --all && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace && cargo test --workspace --doc && cargo deny check`
Expected: all green.

---

## Notes for the implementer

- **Do not skip verification steps.** `grep` and `wc` checks are cheap and catch drift.
- **If a verification step fails**, fix the underlying cause — do not adjust the expected value or weaken the check.
- **Between passes, pause for review.** Each pass is a natural stop-point. The user may want to review before Pass 2 (canon surgery) in particular.
- **If you cannot assign a pattern, status, or invariant confidently**, ask the user or mark the cell `TBD — verify with maintainer`. Do not invent.
- **Commit messages follow the convention** `docs(pass-N): <subject>` — this lets reviewers filter by pass.
