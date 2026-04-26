# Renumber action cascade ADRs (collision-avoidance with resource cascade)

**Date:** 2026-04-25
**Reason:** Resource cascade merged to local main first; resource takes ADR-0036 (resource-credential-adoption) + ADR-0037 (daemon-eventsource). Action cascade must shift +2 to avoid collision.

## Renumber map (executed)

| Old | New |
|-----|-----|
| `0036-action-trait-shape.md` | `0038-action-trait-shape.md` |
| `0037-action-macro-emission.md` | `0039-action-macro-emission.md` |
| `0038-controlaction-seal-canon-revision.md` | `0040-controlaction-seal-canon-revision.md` |

Bare references shifted in lockstep:
- `ADR-0036` → `ADR-0038`
- `ADR-0037` → `ADR-0039`
- `ADR-0038` → `ADR-0040`

File renames done via `git mv` upstream of this task; substitution covers in-text refs (bare ADR-XXXX form, markdown links, filename strings).

## Renumber executed

- Files renamed: 3 (verified present at new paths)
- Refs updated: **428** (1:1 substitution; pre-renumber audit also 428, no losses)
- Files touched: **40**
- Verification grep results: **PASS**

### Per-step verification

| Step | Substitution | Stale-ref count post-step |
|------|--------------|---------------------------|
| 1 | `ADR-0038` → `ADR-0040`; `0038-controlaction-seal-canon-revision` → `0040-controlaction-seal-canon-revision` | 0 stale `ADR-0038` (old) — replaced |
| 2 | `ADR-0037` → `ADR-0039`; `0037-action-macro-emission` → `0039-action-macro-emission` | 0 stale `ADR-0037` |
| 3 | `ADR-0036` → `ADR-0038`; `0036-action-trait-shape` → `0038-action-trait-shape` | 0 stale `ADR-0036` |

Final greps within scope paths:
- `ADR-0036\b` → 0 matches (PASS)
- `ADR-0037\b` → 0 matches (PASS)
- `ADR-0038\b` (old controlaction-seal sense) → all matches now resolve to NEW ADR-0038 (action-trait-shape, formerly 0036) — semantically correct
- `0036-action-trait-shape` / `0037-action-macro-emission` / `0038-controlaction-seal-canon-revision` (filename forms) → 0 matches (PASS)

### Cross-cascade non-collision

- ADR-0035 references intact: 177 hits across action-cascade docs (untouched, as required)
- Resource ADR filenames (`0036-resource-credential-adoption`, `0037-daemon-eventsource`) absent from action-cascade scope (verified pre-renumber)
- No accidental rewrite of resource-cascade docs (different worktree, out of scope)

## Edge cases handled

1. **ADR self-references** — e.g., ADR-0040 (canon revision) text "this ADR-0038" → "this ADR-0040" (3 occurrences in 0040 ratification + cross-ref bottoms)
2. **Companion-ADR cross-refs** — ADR-0038 cites ADR-0039+ADR-0040; ADR-0039 cites ADR-0038+ADR-0040; ADR-0040 cites ADR-0038+ADR-0039 (all consistent post-renumber)
3. **Markdown link pairs** — `[ADR-0038 action trait shape](./0038-action-trait-shape.md)` form: BOTH the bare ref AND filename rewritten in same pass via two-statement sed (`s/ADR-XXXX/.../g; s/XXXX-name/.../g`)
4. **cascade-queue.md** — lines 11-13 (file path list) AND lines 28-31 (cite "ADR-0038 §2 Webhook/Poll precedent") all renumbered to ADR-0040
5. **Strict-order substitution** — descending order (0038→0040, 0037→0039, 0036→0038) chosen to prevent step-3 from undoing step-1's work. Verified: post-step-1 had 0 stale `ADR-0038`, but post-step-3 grep for `ADR-0038\b` re-populates with NEW correct refs (ADR-0036→ADR-0038 promotion), as designed.
6. **Final-shape spike files** — `final_shape_v2.rs` and `final_shape_v3.rs` contain ZERO embedded ADR refs (Rust comments are pattern annotations, not ADR citations); no rewrite needed
7. **ADR-0035 phantom-shim** — frozen, untouched (177 references retained as expected)
8. **Strategy + summary + Tech Spec** — three top-level specs renumbered; cross-references between them and to the 3 ADRs all consistent

## Outstanding issues

**None.** All scoped paths verified clean. Pre-renumber 428 stale refs → post-renumber 428 fresh refs (1:1 mapping confirmed). Cascade-queue.md prerequisite list updated to new filenames. Self-references and companion cross-references in the 3 renumbered ADRs all internally consistent.

**Out of scope (not touched, per task constraints):**
- ADR-0035 (frozen)
- ADR 0001-0034 (pre-existing, no action-cascade touch)
- Resource cascade docs / ADRs (different worktree)
- Credential cascade docs (frozen)
- Production code (`crates/`, `apps/`)

**Hand-off:** orchestrator commits the renumber as a single commit. ADR-README.md (taxonomy table) update — if it lists 0036/0037/0038 by old action names — is OUT of scope for this mechanical pass; flag for orchestrator if needed.
