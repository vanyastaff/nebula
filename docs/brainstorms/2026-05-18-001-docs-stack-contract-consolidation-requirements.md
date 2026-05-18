---
date: 2026-05-18
topic: docs-stack-contract-consolidation
---

# Documentation stack consolidation and contract ADRs

## Summary

Aggressively clean Nebula’s in-repo documentation so **AI agents and humans** reach normative truth in minutes: one router (`docs/README.md`), North Star in `PRODUCT_CANON` only, direction in `STRATEGY`, mechanics in `INTEGRATION_MODEL`, and a **small set of thematic contract ADRs** instead of a long cascade plus execution leftovers. Deliver in **two waves**—agent router first, then ADR merges with explicit supersession stubs. A **follow-up track** (separate from wave B) reduces ADR/canon section citations in **code** so Rust stays readable without turning source into a doc index.

---

## Problem Frame

Documentation grew through feature cascades (M6, M11, storage, schema): many active ADRs (0042+), a ~950-line `VISION.md` claiming “single source of truth,” nine remaining `docs/superpowers/` files while `ARCHIVE.md` says removed, and a completed-but-unfinished consolidation plan (`docs/plans/2026-05-17-002`). Agents and contributors hit **competing authorities**, dead links, and “read ADR-00xx §n” noise in code comments—hurting trust and readability. The primary pain is **agent routing**: tools ingest the wrong layer before finding canon.

---

## Actors

- **A1. AI coding agent (Cursor / Compound Engineering):** Needs a short, reliable path to normative docs without bulk-reading 41+ historical ADRs or draft charter prose.
- **A2. Integration author (human):** Needs honest SDK/README/canon alignment and one North Star story.
- **A3. Maintainer / reviewer:** Needs merge maps, supersession stubs, and verifiable gates before deleting or merging ADR files.
- **A4. Operator (indirect):** Benefits when MATURITY and direction docs match engine truth (no false L3 claims).

---

## Key Flows

- **F1. Agent resolves “how do credentials bind to resources?”**
  - **Trigger:** Agent starts integration or review task.
  - **Actors:** A1
  - **Steps:** Read `docs/README.md` → `INTEGRATION_MODEL` relevant § → at most **one** cited contract ADR if pointer insufficient → crate README. Do not open `VISION.md`, `docs/superpowers/`, or bulk `docs/adr/000*`.
  - **Outcome:** Answer grounded in canon/IM/contract ADR without contradictory charter text.
  - **Covered by:** R1, R2, R3, R4

- **F2. Maintainer merges a thematic ADR cluster**
  - **Trigger:** Wave B approved merge map entry (e.g. schema 0058–0065).
  - **Actors:** A3
  - **Steps:** Write merged **contract ADR** → update `docs/adr/README.md` thematic index → add supersession row → replace deleted files with **stub redirects** (title + “superseded by ADR-NNNN”) → sync `INTEGRATION_MODEL` pointers (no ADR body paste) → run link verification.
  - **Outcome:** Fewer active files; old numbers still resolve via stub; audit trail preserved.
  - **Covered by:** R5, R6, R7

- **F3. Contributor reads Rust without doc archaeology (follow-up track)**
  - **Trigger:** Wave A/B complete; code-citation cleanup scheduled.
  - **Actors:** A2, A3
  - **Steps:** Replace `ADR-00xx` / `canon §x.y` comments with **behavior-first** prose (or link to crate README / one stable doc path); keep normative truth in docs, not scattered section pins in hot paths.
  - **Outcome:** Source reads as product code; deep traceability lives in docs/git history.
  - **Covered by:** R12, R13

---

## Requirements

**Agent router (Wave A — must ship first)**

- **R1.** `docs/README.md` remains the mandatory entry; conflict order unchanged: canon → INTEGRATION_MODEL → accepted ADR → STRATEGY → crate README.
- **R2.** **North Star** lives only in `PRODUCT_CANON.md` §9; `STRATEGY.md` links there (no duplicate north-star prose). Operators/authors/triggers stay the three stars.
- **R3.** `docs/VISION.md` is **demoted**: explicit status (draft / not for agents) **or** compressed to ≤1 screen of charter pointing at STRATEGY + canon—no competing “single SSOT” claim.
- **R4.** Remove execution tail from agent path: delete remaining `docs/superpowers/` (or move out per `ARCHIVE.md` policy); **zero** normative `docs/superpowers` links outside allowlist (`ARCHIVE.md`, historical `docs/plans/*` mentions); fix crate/ADR comments that still point at superpowers specs.
- **R5.** Thematic **ADR index** in `docs/adr/README.md` (groups: M6 resource/credential, schema/UI, storage/0072, action surface, observability, API/webhooks, AI deferred)—agents use groups before opening individual files.
- **R6.** `INTEGRATION_MODEL.md` accepted-decisions table includes **0072** and stays pointer-only (no ADR body duplication); aligns with README index through latest accepted number.
- **R7.** Reconcile `docs/plans/2026-05-17-002` status with reality (completed vs open units) or supersede with a new plan driven by this requirements doc.

**Contract consolidation (Wave B — after merge map approval)**

- **R8.** Publish a **merge map** (maintainer-reviewed) listing: source ADRs → target contract ADR, rationale, stub strategy. No file deletes without map row.
- **R9.** Merge **feature-era** and **fully superseded** active ADRs into fewer **contract ADRs** where canon + INTEGRATION_MODEL already carry mechanics—retain immutability via stubs, not silent deletion.
- **R10.** Resolve duplicate **0052** numbering (validator seam vs action-surface-hybrid) via renumber or documented suffix policy; fix phantom references (e.g. non-existent ADR-0069 in charter text).
- **R11.** **ADR-0057 (proposed):** Either remain **proposed** with clear deferred box in STRATEGY + IM, or fold into direction-only deferred text—no “accepted” index row until accepted.

**Honesty and gates**

- **R12.** `MATURITY.md` and touched integrator crate READMEs match engine truth (no phantom catalog types; English operator-facing notes where required by prior consolidation plan).
- **R13.** Verification gate (documented commands + human checklist) passes before flagship implementation plan (`docs/plans/2026-05-17-001`) treats doc stack as green.

**Code documentation hygiene (Wave C — follow-up; after A/B)**

- **R14.** Reduce **ADR-number and canon-section pins** in library and test code where they harm readability; prefer short behavior-first module docs and **one** stable link (crate README or `INTEGRATION_MODEL` §) when traceability is needed.
- **R15.** Do **not** remove traceability from **normative docs** or from places where a specific ADR is the legal decision record (e.g. supersession tables, `deny.toml` comments if required)—scope is **source readability**, not erasing history.
- **R16.** Wave C produces a **style guideline** (short, in `docs/README.md` or pitfalls): when code may cite docs vs when it must not cite ADR § numbers inline.

---

## Acceptance Examples

- **AE1. Covers R1, R2, F1.** Given a new agent session on an integration task, when the agent follows `docs/README.md`, then it reaches credential/resource binding guidance without opening `VISION.md` or historical ADR bulk.
- **AE2. Covers R4.** Given `rg 'docs/superpowers'` on normative paths (excluding allowlist), when Wave A completes, then the command reports zero matches in `docs/`, `crates/**/*.md`, and active ADR bodies.
- **AE3. Covers R8, R9, F2.** Given merge map approves folding schema ADRs into one contract ADR, when Wave B lands, then old ADR URLs resolve to a stub naming the superseding ADR and README thematic index lists the new contract only.
- **AE4. Covers R14, F3.** Given a storage module previously citing `ADR-0008 §1` in five comments, when Wave C touches that module, then comments explain **behavior** (e.g. lease reclaim, SKIP LOCKED) without requiring the reader to open ADR section numbers—optional single README link at module level.

---

## Success Criteria

- An agent (or new contributor) can explain **North Star**, **direction**, and **integration mechanics** using ≤4 documents in ≤10 minutes without contradictory SSOT language.
- Active ADR **count perceived by agents** drops via thematic index + contract merges (target: materially fewer files to open for a typical integration task—not a numeric vanity goal without merge map).
- No normative dependency on `docs/superpowers/` inside the repo.
- Flagship integrator work can proceed with **trusted** MATURITY/README/canon alignment.
- **Wave C:** Hot-path Rust modules read cleanly; ADR archaeology is opt-in via docs, not mandatory in every comment block.

---

## Scope Boundaries

### In scope (this initiative)

- Waves A and B as above; requirements doc drives `/ce-plan` implementation plan.
- Stub/supersession discipline for merged ADRs.
- VISION demotion or compression; STRATEGY/canon North Star alignment.
- Finishing open hygiene from plan 002 where still failing (links, 0072 index, gate).

### Deferred for later

- **Wave C — code citation cleanup** across workspace (phased by crate or layer; may be its own plan after B).
- Moving full text of ADRs **0001–0041** out of the repository (index-only already; optional external archive).
- CI automation of `rg` doc gates (recommended follow-up, not blocking requirements).
- Implementing integrator flagship **code** (`docs/plans/2026-05-17-001`).
- Rewriting all historical ADR bodies or bulk AI summarization into one mega-doc.

### Outside this product's identity

- Using archived superpowers execution plans as implementation specs.
- Replacing ADRs with duplicate prose in canon (spec theater—pointers only in IM/canon).
- Deleting accepted decision history without stubs or supersession records.

---

## Key Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| KD-1 | **Two waves:** router (A) before contract merges (B) | User priority: agent routing; merges need reviewed map. |
| KD-2 | **Aggressive merge policy** with stubs, not silent deletes | User wants fewer files + clean contracts; audit trail preserved. |
| KD-3 | North Star authoritative in **canon §9** only | Existing conflict rule; removes VISION/STRATEGY duplication. |
| KD-4 | **Wave C separate** for code ADR/§ citations | User-requested; avoids mixing doc file merges with large Rust comment churn in one PR. |
| KD-5 | Execution artifacts (`superpowers`, feature plans) **not normative** | Align `ARCHIVE.md` with repo reality; stop agent leakage. |

---

## Dependencies / Assumptions

- **Depends on:** Maintainer approval of Wave B **merge map** before file deletions.
- **Assumes:** `docs/README.md` conflict hierarchy remains project policy (`CLAUDE.md`).
- **Assumes:** Accepted ADRs stay immutable in substance; consolidation = merge + supersession, not rewriting history in place without record.
- **Related plan:** `docs/plans/2026-05-17-002-refactor-doc-consolidation-plan.md` (partially landed; this doc supersedes open scope).

---

## Resolve Before Planning

| Item | Status | Notes |
|------|--------|-------|
| Wave B merge map contents | **Deferred to planning** | `ce-plan` should propose concrete source→target table for review before edits. |
| ADR-0057 handling | **Deferred to planning** | Proposed-only vs direction-only deferred—either acceptable if index honest. |
| Wave C crate order | **Deferred to planning** | Suggest exec/storage/api first (highest ADR comment density). |

---

## Outstanding Questions

- None blocking Wave A. Wave B/C details left to planning as above.
