---
title: "refactor: Evict dead ADR weight from the working tree"
type: refactor
status: active
date: 2026-05-18
origin: docs/brainstorms/2026-05-18-001-docs-stack-contract-consolidation-requirements.md
note: executes the "Historical ADR export / 0001–0041 out of repo" item deferred by docs/plans/2026-05-18-001
---

# refactor: Evict dead ADR weight from the working tree

## Summary

git-rm the 18 Wave-B redirect stubs and the 41 frozen historical ADRs
(0001–0041), repoint every surviving inbound link with a cleanest-strategy
(stub refs → contract ADR; historical refs → `HISTORICAL.md`; no per-ADR anchor
scaffolding), and bring every normative/config doc that asserts these files
exist back into truth. `docs/adr/` drops 75 → 16 live files; full historical
text stays recoverable via git history + the external archive `ARCHIVE.md`
already names.

---

## Problem Frame

The doc-consolidation cascade (Waves A/B/C, PRs #691/#693/#694/#695) optimized
agent routing — thematic index, contract ADRs 0080–0082, behavior-first Rust
comments — but deliberately chose **stubs-not-delete** (origin KD-P6) and
**deferred** moving 0001–0041 out of the tree. Result: `docs/adr/` still holds
75 `.md` files (828K), of which **18 are 23-line redirect stubs** and **41 are
frozen historical records already deindexed** from agents
(`.claudeignore` + `.cursorignore`). The stated reason to keep them on disk —
deep links from Rust code — is now **void**: Wave C (#695) left **zero**
`ADR-00xx` / `canon §` citations in `crates/**/*.rs` (verified). The remaining
inbound links are ~9 integrator README/canon docs plus the `adr/README.md`
index itself. Frozen history is dead weight in the working tree with no live
code dependency.

---

## Requirements

**Eviction**
- R1. Delete the 18 redirect stubs: ADRs 0042, 0043, 0044, 0045, 0047, 0048, 0049, 0051, 0052 (schema-validator-condition-seam), 0058, 0059, 0060, 0061, 0062, 0063, 0064, 0066, 0067.
- R2. Delete the 41 historical ADRs 0001–0041 (full text recoverable via `git log -- <path>` and the external archive named in `docs/ARCHIVE.md`).
- R3. Do not touch live/contract ADRs (0046, 0050, 0053, 0054, 0055, 0056, 0057, 0065, 0068, 0069, 0072, 0080, 0081, 0082), `docs/plans/`, or `docs/brainstorms/`.

**Link integrity (cleanest strategy)**
- R4. Inbound links that referenced a deleted **stub** repoint to its **contract ADR** (0080/0081/0082 per the `adr/README.md` merge map).
- R5. Inbound links that referenced a deleted **historical** ADR repoint to `docs/adr/HISTORICAL.md` (single stable index target — no per-ADR anchor scaffolding).
- R6. The Tier-1 canon citation `PRODUCT_CANON.md` → 0020 repoints to `HISTORICAL.md` and its "and its pre-conditions" over-promise is trimmed to an honest git-history/index pointer (a normative doc must not over-claim a body that now lives only in git).
- R7. `docs/adr/README.md` drops the 18 stub rows and all dead historical/stub file-links from its Index and Supersession tables, while preserving the thematic index, contract table, and a text-only supersession map (audit trail survives without dead links).

**Truth-up**
- R8. `docs/ARCHIVE.md` line stating 0001–0041 full text is "on disk … indexed out" is corrected to git-history-only.
- R9. `docs/README.md` HISTORICAL/ADR-layout wording stops asserting `docs/adr/NNNN-*.md` full text is on disk for 0001–0041.
- R10. `.claudeignore` and `.cursorignore` drop the now-dead `docs/adr/000*` / `001*` / `002*` / `003*` / `0040*` / `0041*` glob block (deleted files → globs are noise).
- R11. `deny.toml` layer-wrapper comments pointing at `docs/adr/0044`, `docs/adr/0066` (deleted stubs) repoint their textual pointer to the contract `0081`.

**Gate**
- R12. After eviction, zero dead `docs/adr/00xx-*.md` markdown links remain repo-wide outside the allowlist (`docs/plans/**`, `docs/ARCHIVE.md`, `docs/brainstorms/**`, `crates/**/CHANGELOG.md` historical narrative); `docs/adr/` contains exactly 16 entries; a sample deleted file is recoverable via `git show`.

---

## Scope Boundaries

- Live/contract ADRs, `docs/plans/`, `docs/brainstorms/` — untouched.
- Agent-context optimization — already solved by prior waves; not re-litigated.
- `crates/schema/CHANGELOG.md` historical mentions of `0001-..0003-` / `0034-` — prose narrative of a past change, not resolvable nav links; left as-is.
- A3 "collapse 0001–0041 essence into one in-repo doc" — rejected during brainstorm in favor of git-history-only.

### Deferred to Follow-Up Work

- `.github/ISSUE_TEMPLATE/rfc.yml:106` stale ADR example (`docs/adr/0012-execution-context-design.md` — wrong filename, pre-existing) — tangential placeholder text, not caused by this work.
- CI automation of the dead-link `rg` gate (recommended follow-up by prior plans; not blocking).

---

## Key Technical Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| KD1 | Cleanest strategy: no per-ADR anchors added to `HISTORICAL.md` | User directive "я хочу чище" — anchor scaffolding is cruft; one stable index target is cleaner than 41 anchor IDs. |
| KD2 | Stub refs → contract ADR; historical refs → `HISTORICAL.md` | Two link classes, two correct targets: superseded decisions live in the contract ADR; frozen history lives in the index + git. |
| KD3 | Trim canon's 0020 over-promise, don't just repoint | A Tier-1 normative doc citing "the binding decision and its pre-conditions" must not point at an index row for a body that only exists in git — honesty over a dead precision claim. |
| KD4 | Repoint **before** delete (U1–U3 before U4) | No transient mid-sequence state where a live doc links a just-deleted file; each commit leaves the tree link-clean. |
| KD5 | Safe now, was not before #695 | Wave C left zero ADR cites in Rust — the on-disk rationale is genuinely void; verified, not assumed. |

---

## Implementation Units

### U1. Repoint stub-referencing inbound links → contract ADRs

**Goal:** Every live doc that links a to-be-deleted stub points at its contract ADR instead.

**Requirements:** R4, R11

**Dependencies:** none

**Files:**
- Modify: crate READMEs / docs referencing stub paths — `crates/action/README.md`, `crates/api/README.md`, `crates/api/src/openapi/audit.md`, `crates/credential-vault/README.md`, `crates/credential/README.md`, `crates/resource/README.md`, `crates/resource/docs/topology-reference.md`, `crates/resource/plans/06-action-integration.md`, `crates/validator/README.md`, `docs/PRODUCT_CANON.md` (stub refs only — historical 0020 is U2)
- Modify: `deny.toml` (comments → `docs/adr/0081`)

**Approach:** Per-file grep for `docs/adr/004[2-9]|0051|0052-schema|005[8-9]|006[0-4]|0066|0067` link targets; map each to its contract per `adr/README.md` Wave-B table (0052-schema/0058–0064 → 0080; 0042–0045/0051/0066/0067 → 0081; 0047–0049 → 0082). Rewrite the link target; keep surrounding prose.

**Patterns to follow:** Existing contract-ADR links already present in `docs/adr/README.md` Contract table.

**Test expectation:** none — documentation/config only; resolution proven by U6 gate.

**Verification:** No live doc links a stub path; stub refs now resolve to an existing contract ADR file.

---

### U2. Repoint historical-referencing links → HISTORICAL.md; trim canon over-promise

**Goal:** Every live doc that links a 0001–0041 file points at `HISTORICAL.md`; canon's 0020 citation is honest.

**Requirements:** R5, R6

**Dependencies:** none

**Files:**
- Modify: `docs/PRODUCT_CANON.md` (0020 → `HISTORICAL.md`; trim "and its pre-conditions" to a git-history/index pointer)
- Modify: `crates/credential/README.md` (0033 ×2, 0035, 0004), `crates/metadata/README.md` (0018 ×2), `crates/plugin-sdk/README.md` (0006), `crates/plugin/README.md` (0018, 0027), `crates/sandbox/README.md` (0006 ×2, 0025), `crates/resource/docs/README.md` (0037)

**Approach:** Replace each `docs/adr/00NN-*.md` link target (NN ≤ 41) with `docs/adr/HISTORICAL.md` (relative-path-correct per file depth); keep the ADR id in the visible text so the reference stays meaningful. For canon line 57, reword so it no longer promises full pre-conditions text at the link.

**Patterns to follow:** `docs/README.md:25` already models the HISTORICAL.md pointer phrasing.

**Test expectation:** none — documentation only; resolution proven by U6 gate.

**Verification:** No live doc links a 0001–0041 path; canon sentence makes no dead precision claim.

---

### U3. Rewrite docs/adr/README.md index

**Goal:** Index reflects the post-eviction tree; audit trail preserved without dead links.

**Requirements:** R7

**Dependencies:** U1, U2 (so README is the last doc still linking stubs/historical)

**Files:**
- Modify: `docs/adr/README.md`

**Approach:** Remove the 18 stub rows from the "Index (0042–0072)" table (keep accepted-standalone rows: 0046, 0050, 0053–0057, 0065, 0068, 0069, 0072). Replace the "Supersession" / "Supersession (Wave B)" file-link cells with text-only ids (e.g. `0044 → 0081`, no `./0044-*.md` link). Keep the thematic index, the Contract ADRs table, and the Historical-ADRs paragraph (now: index lives in `HISTORICAL.md`, full text in git history).

**Test expectation:** none — documentation only.

**Verification:** `adr/README.md` links only files that still exist; supersession map readable as text.

---

### U4. git-rm 18 stubs + 41 historical (59 files)

**Goal:** The dead weight leaves the working tree.

**Requirements:** R1, R2, R3

**Dependencies:** U1, U2, U3

**Files:**
- Delete: `docs/adr/0001-*.md` … `docs/adr/0041-*.md` (41)
- Delete: `docs/adr/0042-*.md`, `0043`, `0044`, `0045`, `0047`, `0048`, `0049`, `0051`, `0052-schema-validator-condition-seam.md`, `0058`–`0064`, `0066`, `0067` (18 stubs)

**Approach:** `git rm` the exact 59 paths. Do not glob-delete `004*`/`005*`/`006*` blindly — accepted standalones share those prefixes (0046, 0050, 0053–0057, 0065, 0068, 0069). Enumerate explicitly.

**Test expectation:** none — file deletion; recoverability proven by U6 gate (`git show`).

**Verification:** `docs/adr/` lists exactly 16 entries (README, HISTORICAL, 0046, 0050, 0053, 0054, 0055, 0056, 0057, 0065, 0068, 0069, 0072, 0080, 0081, 0082).

---

### U5. Truth-up normative/config docs

**Goal:** No surviving doc/config asserts the deleted files are on disk.

**Requirements:** R8, R9, R10

**Dependencies:** U4

**Files:**
- Modify: `docs/ARCHIVE.md` (0001–0041 full-text-on-disk line → git-history-only)
- Modify: `docs/README.md` (HISTORICAL row + "ADR layout" 0001–0041 wording)
- Modify: `.claudeignore`, `.cursorignore` (drop the dead `docs/adr/000*`…`0041-*` glob block)

**Approach:** Edit each statement to read git-history-only. The ignore-glob block is pure noise once files are gone — remove the whole `Pre-0042 ADRs` section in both ignore files.

**Test expectation:** none — documentation/config only.

**Verification:** `rg 'on disk|NNNN-\*\.md'` in ARCHIVE.md/README.md no longer claims 0001–0041 presence; ignore files have no `docs/adr/00*` globs.

---

### U6. Verification gate

**Goal:** Prove the eviction is link-clean and recoverable.

**Requirements:** R12

**Dependencies:** U5

**Files:** none (gate only)

**Approach:** Run the checks below; all must pass before shipping.

**Test scenarios:**
- Zero dead links: `rg -o 'docs/adr/00[0-9]{2}-[a-z0-9-]+' --glob '*.md' --glob '!docs/plans/**' --glob '!docs/ARCHIVE.md' --glob '!docs/brainstorms/**' --glob '!**/CHANGELOG.md'` → every hit resolves to an existing file (the 16 survivors only).
- Count: `docs/adr/` contains exactly 16 entries; the 59 paths are absent.
- Untouched: live/contract ADR files (0046/0050/0053–0057/0065/0068/0069/0072/0080–0082) byte-identical to pre-change.
- Recoverable: `git show HEAD~:docs/adr/0001-schema-consolidation.md` and `…0058-…` return full prior text.
- No Rust regression risk: docs-only diff — `rg 'docs/adr/00' crates --glob '*.rs'` still zero.

**Verification:** All five scenarios pass.

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Glob-delete removes an accepted standalone (0046/0050/0053–0057/0065/0068/0069) | U4 enumerates 59 explicit paths; never prefix-globs |
| A missed inbound link dangles after delete | U6 repo-wide grep gate before shipping; repoint (U1–U3) precedes delete (U4) |
| Canon left with a weakened normative claim | U2/KD3 rewrites the sentence, not just the link target |
| Reviewer wants history browsable without git | Accepted trade-off (brainstorm A2 over A3); `HISTORICAL.md` index + `ARCHIVE.md` external archive + `git log` documented |

---

## Sources & References

- Origin (deferred-item parent): `docs/brainstorms/2026-05-18-001-docs-stack-contract-consolidation-requirements.md` ("Deferred for later" → move 0001–0041 out)
- Prior cascade: PRs #691/#693/#694/#695; `docs/plans/2026-05-18-001-refactor-docs-stack-contract-consolidation-plan.md`
- `docs/ARCHIVE.md` (recovery policy already established for `superpowers/`)
