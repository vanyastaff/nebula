---
name: Documentation Architecture Redesign
description: Restructure Nebula's markdown corpus as an LLM-memory architecture — layered canon, named-pattern vocabulary, priming-layer docs, crate-level normalization — to prevent context loss, Quick Win traps, and canon-induced architectural lock-in.
status: proposed
last-reviewed: 2026-04-17
owner: vanyastaff
related:
  - docs/PRODUCT_CANON.md
  - CLAUDE.md
  - docs/GLOSSARY.md
  - docs/plans/
---

# Documentation Architecture Redesign

## 1. Motivation

The docs corpus currently serves three audiences poorly:

1. **LLM sessions (primary)** — Nebula is a 25-crate Bevy-style workspace. Every vibe-coding session rediscovers crate responsibilities, product goals, and style conventions from scratch. Context window is spent on persuasion copy, not normative truth.
2. **Integration authors (secondary)** — they need per-crate scope statements, but current `crates/*/README.md` coverage is uneven (9 to 80+ lines of `//!`; `nebula-schema` has no README at all).
3. **Operators (tertiary)** — `docs/PRODUCT_CANON.md` mixes product positioning, invariants, and implementation details; operators have no clear place to read durability/observability contracts.

Concrete failures observed:

- **Context loss:** new sessions re-derive project purpose and crate topology; agents drift from house style.
- **Quick Win trap:** without a priming-layer decision gate, agents propose tactical patches (rename, local fix, add flag) where architectural moves are needed.
- **Canon-induced lock-in:** because PRODUCT_CANON mixes strategic principles with implementation details at equal authority, agents refuse legitimate architectural improvements citing canon rules that are themselves stale or wrong.
- **Uneven crate maturity invisible:** `nebula-schema` just landed Phase 1 Foundation; `nebula-credential` was recently renamed; `nebula-sandbox` has partial enforcement. Agents cannot see which crates are frontier vs stable, so they build on sand or refuse safe refactors.
- **Competitive bet unprotected:** pre-1.0 is the only window to lock in sealed traits, typestate patterns, `Zeroize` discipline, and error taxonomy. Without a single-source style document, each session re-litigates.

Pre-1.0 stage lets us choose the best design, not the minimum diff. That includes the documentation architecture.

## 2. Book-backed design principles

| Book | Principle applied |
|---|---|
| Ousterhout — Philosophy of Software Design | Deep modules with thin interfaces. Canon is a read-every-session artifact; it must fit a small context budget while being load-bearing. Strategic programming over tactical. |
| Wlaschin — Domain Modeling Made Functional | Make illegal states unrepresentable — applied to canon rules AND to agent workflow: bad moves become representationally illegal via decision gate and revision triggers. |
| Hohpe & Woolf — Enterprise Integration Patterns | Shared named-pattern vocabulary: Outbox, Idempotent Receiver, Guaranteed Delivery, Competing Consumers. |
| Kleppmann — DDIA | Durability vocabulary (WAL, CAS, optimistic concurrency), failure-mode tables, honest exactly-once framing. |
| Nygard — Release It! | Stability patterns (Circuit Breaker, Bulkhead, Timeout, Back Pressure) named; antipatterns catalog (Integration Points, Cascading Failures, Blocked Threads) extends §14. |
| Beyer et al. — SRE Book | SLI / SLO / error budget framing applied to §4.6 observability and to dev-maturity dashboard. |
| Majors et al. — Observability Engineering | Structured high-cardinality events; core analysis loop as operator procedure. |
| Feathers — Working Effectively with Legacy Code | Seams named on invariants; §13 knife as characterization test suite. |
| Fowler — Refactoring 2nd | Extract Subdocument (move §3.5/§2.5/§4.6 out of canon); Divergent Change + Inappropriate Intimacy smells diagnose canon dysfunction. |
| Matsakis et al. — Rust API Guidelines | Naming, predictability (C-CASE, C-COMMON-TRAITS), future-proofing (C-EVOLVE), docs (C-DOCS-EXAMPLES). |
| Drysdale — Effective Rust | Types as primary modeling tool (Item 1); Error taxonomy (ch Concepts); sealed-trait future-proofing. |
| Gjengset — Rust for Rustaceans | Designing Interfaces chapter: sealed traits, typestate, object safety. |
| Richards & Ford — Fundamentals of SW Architecture | Name the style; characteristic tradeoffs explicit per pillar. |
| Rust Design Patterns | Idioms (mem::take, Default, newtype, Builder, RAII guard) and antipatterns (clone-to-satisfy, Deref polymorphism) consolidated in STYLE.md. |

## 3. Key insight: canon is load-bearing LLM context

The canon is read by every session. Two consequences:

1. **Every line has a context-budget cost.** Persuasion (§2.5 competitive pitch), history (§18 change notes), and redundant detail (§3.6–§3.9 crate pointers duplicating crate READMEs) compete with invariants for token budget. Canon must earn its bytes.
2. **Every rule is potentially interpreted as a gate.** If rules are mixed strategic / tactical at equal authority, any improvement that conflicts with a tactical rule is blocked by agents who cannot tell the levels apart.

The fix is structural: canon becomes a **layered normative core with a defined revision path**, supported by satellite docs that carry detail.

## 4. Doc topology

### 4.1 Priming layer — loaded every session

| File | Role | Target size |
|---|---|---|
| `CLAUDE.md` (update existing) | Read-order, 6-question decision gate, Quick Win trap catalog, cross-ref to canon §0.2 revision triggers | ~120 lines (+30 from current) |
| `docs/PRODUCT_CANON.md` (shrink + restructure) | Normative core only: L1 principles, L2 invariants, L3 conventions, knife, DoD, revision triggers. Thin pointers to satellites. | ≤250 lines (from 686) |
| `docs/MATURITY.md` (new, manual) | 25-row dashboard: API stability × test coverage × doc completeness × engine integration × SLI-ready. Manually edited in PRs. | ~60 lines |
| `docs/STYLE.md` (new, consolidated scope B) | Idioms we use / antipatterns we reject / naming table / error taxonomy | ~400 lines |

Total priming budget ≈ 800 lines. Fits typical context windows with room to spare.

### 4.2 Supporting layer — loaded on demand

| File | Role |
|---|---|
| `docs/INTEGRATION_MODEL.md` (new, extracted) | Current §3.5 + §3.6–§3.9 + §7.1 rewritten with `nebula-schema` as the shared schema crate, sealed-trait form for the structural contract, plugin dependency rules. |
| `docs/COMPETITIVE.md` (new, extracted) | Current §2 + §2.5 — positioning, peer analysis, bets. Explicitly persuasive, not normative. |
| `docs/OBSERVABILITY.md` (new) | SLI / SLO / error budget; structured event schema for `execution_journal`; core analysis loop as operator procedure. |
| `docs/GLOSSARY.md` (expanded) | Existing terms + new Architectural Patterns section mapping Outbox / WAL / Idempotent Receiver / Bulkhead / Circuit Breaker / Optimistic CC to their Nebula implementations and book sources. |
| `docs/adr/NNNN-kebab-title.md` (new dir) | Per-decision ADRs. No central index — discovery via filename, frontmatter (`status`, `supersedes`, `tags`), `ls docs/adr/`. Rationale: parallel-worktree-agent workflow causes merge conflicts on central index. |

### 4.3 Surface layer — per-crate

| File | Role |
|---|---|
| `crates/*/README.md` (normalize all 25 + create `crates/schema/README.md`) | Uniform template (see §6). Role (named pattern) + Contract + Invariants (with seam tests) + Public API + Non-goals + Maturity row. |
| `crates/*/src/lib.rs //!` | Synced with README header. Rustdoc entry-point shape. Avoid bracketed intra-doc links at top of file (rustdoc `-D warnings` cannot resolve out-of-scope paths from `//!`). |

### 4.4 What gets deleted

- Task-plans in `docs/plans/` whose work has landed (verify per git log + code):
  `2026-04-14-batch3-resource-lifecycle.md`, `2026-04-14-batch4-api-sandbox-security.md`, `2026-04-14-batch5-misc-lifecycle.md` — if completed.
  Verify each before deletion; rationale logged in the PR body, not a changelog.
- Duplicated content after extraction (canon §3.5 / §3.6–§3.9 / §7.1 / §2 / §2.5).
- Any references to `nebula-parameter` (crate deleted in `ed3a0ce0`).

## 5. Layered Canon

### 5.1 Layer legend

| Layer | Meaning | Revision cost | Example |
|---|---|---|---|
| **L1 Principle** | Strategic product intent. If it falls, Nebula is a different product. | Product-level rethink. | §4.5 Operational honesty · §12.1 Layer direction · §12.3 Local-first default |
| **L2 Invariant** | Testable contract with a named code seam. | Material semantic change requires ADR + seam test update in same PR. (Wording polish does not.) | §11.1 CAS on `version` · §12.2 Outbox atomicity · §12.5 Secret zeroize/redaction |
| **L3 Convention** | Default answer to a style question. | PR with test / diff justification. | §12.4 RFC 9457 for API errors · `thiserror` in libs / `anyhow` in bins · SQLite default local |
| **L4 Implementation pointer** | Current fact about one crate. | **Does not live in canon.** Lives in `crates/*/README.md`. | Idempotency key format · specific type names · auth scheme count |

Every rule in canon prefixed with `**[L1]**`, `**[L2]**`, or `**[L3]**`. L4 content is **evicted** to the owning crate's README.

### 5.2 Canon revision triggers (new §0.2)

```markdown
## 0.2 When canon is wrong (revision triggers)

Canon rules can be stale, premature, or plain wrong. If any of these apply,
stop, open an ADR, propose revision, then proceed — do NOT blind-follow.

- Dead reference: rule mentions a crate / type / endpoint that no longer
  exists or has been renamed.
- Intimacy violation: rule can only be changed by editing canon when a
  single crate refactors. L4 detail leaked into canon. Fix: move to crate README.
- Capability lag: rule freezes an implementation that predates a better
  architectural move, and the improvement is measurable (perf / safety / DX).
- False-capability: rule names a type / variant the engine does not honor
  end-to-end. By §4.5 this type must be hidden or the rule must drop.
- Uncovered case: new failure mode or integration shape canon is silent on.
  Write ADR before blind-applying the nearest rule.

Canon is an articulation, not a prison. Blind-obeying a wrong rule violates
operational honesty (§4.5) more than explicitly revising it.
```

### 5.3 Graduated Definition of Done (§17 rewrite)

Current §17 treats any canon divergence as incomplete. New shape:

- Violation of **L1** without canon revision + product-level rationale → incomplete.
- Violation of **L2** without ADR + seam test update → incomplete.
- Violation of **L3** without PR rationale → incomplete.
- "Violation" of **L4** is not a violation — it is a move of detail into the correct layer.

### 5.4 P0 canon fixes during surgery

In addition to structural layering:

| Issue | Current location | Fix |
|---|---|---|
| Dead `nebula-parameter` references | §1, §3.5, §3.9, §3.10 | Rewrite §1; move §3.5 / §3.6–§3.9 to `INTEGRATION_MODEL.md` with `nebula-schema` as the schema crate (Field enum, proof-token pipeline). |
| §3.5 "five concepts" structure broken | §3.5 | In `INTEGRATION_MODEL.md`: structural contract becomes `*Metadata + Schema` where `Schema = nebula-schema::Schema`. |
| `CheckpointPolicy` without status | §3.8 | Tag `[L4]` and move to `crates/action/README.md` with §11.6 status. |
| `[signing]` block without status | §7.1 | In `INTEGRATION_MODEL.md`: tag `status: planned`. |
| §4.5 strengthening | §4.5 | Rewrite: "public surface exists ⇔ engine honors it. Sealed / feature-gated / absent otherwise." |
| §11.2 honesty table | §11.2 | Keep current table; add seam pointer for `implemented` row. |
| §11.3 idempotency key format | §11.3 | Keep invariant ("deterministic per-attempt key, persisted before side effect"); move format string to `crates/execution/README.md`. |
| "Twelve universal auth schemes" | §3.7 | Remove specific count; move to `crates/credential/README.md`. |
| §3.10 untagged cross-cutting claims | §3.10 | Each crate line gets `[L4]` tag and a status row moves to that crate's README. §3.10 itself becomes a responsibility graph only. |
| §13 knife unsourced | §13 | Each step gets seam pointer + test file name. |
| §15 missing `dev-setup.md` | §15 | Add row. |
| `AuthScheme` location | §3.10 | Move to `crates/core/README.md`. |

## 6. Crate README template

```yaml
---
name: nebula-<crate>
role: <named pattern from GLOSSARY>
status: frontier | stable | partial
last-reviewed: YYYY-MM-DD
canon-invariants: [L2-11.1, L2-12.3, ...]
related: [nebula-core, nebula-error, ...]
---
```

Fixed sections:

1. **Purpose** — one paragraph.
2. **Role** — named pattern with book reference (e.g. "Transactional Outbox — DDIA ch 11, EIP Messaging").
3. **Public API** — types, traits, key functions with rustdoc links.
4. **Contract** — invariants this crate enforces, each with its test file.
5. **Non-goals** — what this crate deliberately does not do.
6. **Maturity** — link to `docs/MATURITY.md` row.
7. **Related** — sibling crates, satellite docs.

`lib.rs //!` mirrors sections 1–3 in rustdoc-friendly form (no bracketed intra-doc links at top of file per past lesson with rustdoc `-D warnings`).

## 7. MATURITY.md shape (manual)

Columns, one row per crate:

| Crate | API stability | Test coverage | Doc completeness | Engine integration | SLI ready |
|---|---|---|---|---|---|

Cell values: `frontier` | `partial` | `stable` | `n/a` with optional short note. Example row:

| `nebula-schema` | `frontier` | `partial` — proof-token pipeline tested, expression resolution not | `partial` — README missing | `frontier` — replacing `nebula-parameter` | `n/a` |

Edited in PRs that land substantive work. `CLAUDE.md` DoD checklist adds a line: "If this PR changes a crate's API stability or engine integration state, update its `MATURITY.md` row."

## 8. STYLE.md shape (scope B, consolidated)

Sections:

1. **Idioms we use** — `mem::take` / `mem::replace`, `Default`, newtype wrappers, Builder for non-trivial constructors, RAII guards for lifecycle, typestate for state machines, `#[must_use]` on result-bearing types, `Cow<'_, T>` over premature cloning.
2. **Antipatterns we reject** — `.clone()` to satisfy the borrow checker without tradeoff note, `Deref` polymorphism, stringly-typed public APIs, `anyhow` in library crates, `unwrap` outside tests, implicit panics in async state.
3. **Naming table** — `*Metadata` (UI-facing), `*Schema` (typed config), `*Key` (stable identifiers), `*Error` / `*Kind` (error types), `*Repo` (storage port), `*Handle` (borrowed resource), `*Guard` (RAII release).
4. **Error taxonomy** — `NebulaError` + categories from `nebula-error`; `thiserror` in libraries; `Classify` trait for transient vs permanent decisions; RFC 9457 mapping at API boundary.
5. **Type design bets** — sealed traits for extension points we own; typestate for lifecycle enforcement; `Zeroize` + `ZeroizeOnDrop` on secret material; redacted `Debug` on credential wrappers; `#[non_exhaustive]` on public enums/structs we intend to grow.
6. **When to fight canon** — cross-ref to `PRODUCT_CANON.md §0.2` revision triggers.

## 9. Pass ordering

Each pass is one PR. Low-risk structural passes go first so templates and layers are proven before wide application.

### Pass 0 — Priming skeleton (low-risk, structural)

- Update `CLAUDE.md`: add read-order block, 6-question decision gate, Quick Win trap catalog, pointer to `§0.2` revision triggers.
- Create `docs/MATURITY.md` skeleton — 25 rows, empty cells plus header row.
- Create `docs/STYLE.md` skeleton — section headers only.
- Create `docs/DOC_TEMPLATE.md` (crate README template).
- Create `docs/ADR_TEMPLATE.md` (Context / Decision / Consequences / Status shape).

No canon changes. No content fills. Just infrastructure.

### Pass 1 — Extract without rewriting (structural)

- Create `docs/INTEGRATION_MODEL.md` — copy current §3.5 + §3.6–§3.9 + §7.1 verbatim, add a `status: draft — extracted from canon, surgery in Pass 2` note.
- Create `docs/COMPETITIVE.md` — copy current §2 + §2.5 verbatim, same draft note.
- Create `docs/OBSERVABILITY.md` — skeleton with SLI / SLO / Events / Core analysis loop section headers.

Canon still has duplicated content. Pass 2 shrinks canon and fixes extracted docs.

### Pass 2 — Canon surgery (high-impact, semantic)

- Shrink `PRODUCT_CANON.md` to ≤250 lines:
  - Remove §2 / §2.5 body, replace with pointer to `COMPETITIVE.md`.
  - Remove §3.5 / §3.6–§3.9 / §7.1 body, replace with pointer to `INTEGRATION_MODEL.md`.
  - Replace §4.6 body with pointer to `OBSERVABILITY.md`.
  - Apply L1/L2/L3 tags to every retained rule.
  - Evict L4 content to owning crate READMEs (format strings, scheme counts, specific type names).
- Fix P0 issues from §5.4 above.
- Add §0.2 canon revision triggers.
- Rewrite §17 DoD by layer.
- Rewrite §4.5 to DMMF-style "public ⇒ honored".
- Fill `INTEGRATION_MODEL.md`: `nebula-schema`-based structural contract, sealed-trait form, status tags on all referenced types.
- Fill `docs/STYLE.md` with content.
- Expand `docs/GLOSSARY.md` with Architectural Patterns section.

Largest semantic diff. Requires careful review.

### Pass 3 — ADR extraction + plans cleanup

- Create `docs/adr/` directory.
- Extract ADRs from plans where architectural decisions exist:
  - `0001-schema-consolidation.md` (from `2026-04-16-nebula-schema-phase1-foundation-design.md`)
  - `0002-proof-token-pipeline.md`
  - `0003-consolidated-field-enum.md`
  - `0004-credential-metadata-description-rename.md`
  - `0005-trigger-health-trait.md`
  - `0006-sandbox-phase1-broker.md`
  - Others as identified from `docs/plans/` review.
- Delete pure task-plans after verifying completion via git log + code presence.
- Fill `docs/OBSERVABILITY.md` content: SLI definitions, SLO targets, event schema, core analysis loop procedure.

### Pass 4 — Crate sweep (scale)

- Normalize all 25 `crates/*/README.md` to template from §6.
- Create `crates/schema/README.md` from scratch.
- Sync all `crates/*/src/lib.rs //!` to mirror README header (sections 1–3 only).
- Fill `docs/MATURITY.md` dashboard with actual row values per crate.
- Naming audit across crate READMEs and `lib.rs //!` per §3.5 rewrite (`ParameterCollection` → `Schema` where applicable in docs). This pass does **not** rename Rust types in code — code renames are separate implementation work with their own plans.

Largest by file count. Risk mitigated because template and L1/L2 content are already proven in Passes 0–2.

## 10. Success criteria

- Any new agent session can orient on the project by reading ≤1000 lines of priming-layer docs.
- No canon rule blocks an architectural improvement without a documented revision-trigger path.
- Each crate's Role (named pattern) is readable from README in under 30 seconds.
- Frontier vs stable state of each crate is visible in `MATURITY.md` without reading src.
- All retained canon rules are L1/L2/L3 tagged; no L4 content leaked.
- Named architectural patterns (Outbox, WAL, Idempotent Receiver, Bulkhead, Circuit Breaker, Optimistic CC) appear in `GLOSSARY.md` with book references and Nebula implementation pointers.
- `PRODUCT_CANON.md` ≤ 250 lines post-surgery.
- No dangling references to `nebula-parameter` anywhere in `docs/` or `crates/*/README.md`.

## 11. Out of scope

- Behavioral fixes for false capabilities identified during surgery (implementation work — separate plans).
- CI markdown lint / link check workflows (possible follow-up; not required for Pass 4 completion).
- mdBook or published docs site (not a goal — per-crate rustdoc + markdown is the deliverable).
- Translation / localization.
- Automation for `MATURITY.md` updates (explicitly rejected — manual edits in PRs).

## 12. Risks and open questions

- **Pass 2 scope creep.** Canon surgery touches many sections simultaneously. Mitigation: explicit section-by-section checklist in the implementation plan; if a section balloons, defer it to a follow-up PR with canon-revision note.
- **`nebula-schema` semantic accuracy.** The §3.5 rewrite in `INTEGRATION_MODEL.md` must correctly describe proof-token pipeline, Field enum, `ValidSchema::validate` / `ValidValues::resolve`. Implementation plan will read `crates/schema/src/lib.rs` and current Phase 1 spec before writing.
- **MATURITY.md drift.** Manual dashboard can go stale. Mitigation: `CLAUDE.md` DoD checklist line; reviewers catch missing updates in PR review.
- **STYLE.md size.** Consolidated scope may exceed ~500 lines. Acceptable; if it exceeds ~800, split into `STYLE.md` + `NAMING.md` + `ERRORS.md` as a Pass-4 follow-up.
- **§13 knife seams verification.** Must confirm each named test file actually exists or create it in the same PR as the seam reference. Do not land a knife pointer to a non-existent test.
- **ADR extraction faithfulness.** Some plans mix completed work with aspirational sections. ADR must capture the decision that actually landed, not a planner's wishlist.
- **`docs/plans/` deletion.** Before deleting a plan, verify via git log that its tracked work is represented in code. If verification is ambiguous, archive rather than delete (`docs/plans/archive/`).
- **Concurrent sessions during Pass 2.** Canon surgery PR is long-lived; agents working on other branches during that window will read the pre-surgery canon and may be confused by stale references. Mitigation: land Pass 0 + Pass 1 first (which add `CLAUDE.md` read-order block pointing to in-progress surgery), and keep Pass 2 PR description up to date with which sections are already migrated.

## 13. Migration strategy for in-flight branches

Docs changes do not break code. Parallel worktree branches merge cleanly because:

- No central ADR index file (merge conflicts avoided by design).
- `MATURITY.md` edits land in the PR that changes a crate's state; no separate tracker.
- Canon shrinkage is one PR; subsequent work references new section numbers.

If a work-in-progress branch references a canon section that moved or was deleted, the branch rebases and updates the reference — cheap because sections are named (§11.2), not numbered to drift.
