---
name: Cross-cascade R1+R2 enactment report
status: enactment-complete
date: 2026-04-26
authors: [architect (cross-cascade enactment dispatch)]
scope: Cross-cascade consolidated review §7.1 path (a) routing — R1 (action Tech Spec §2.2.4 stub Resource trait removal) + R2 (resource Tech Spec §2.1 + ADR-0036 §Decision `on_credential_refresh` signature re-pin to credential CP5 §15.7 `SchemeGuard<'a, _>` shape)
inputs:
  - docs/superpowers/drafts/2026-04-24-cross-cascade-consolidated-review.md (cross-cascade review §7.1 amendment routing)
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.7 (canonical CP5 SchemeGuard shape source-of-truth)
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md §2.2.4 + §15.13 + status header (R1 enactment target)
  - docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md §2.1 + §2.1.1 + §2.3 + §11.6 + §15.7 + status header (R2 enactment target)
  - docs/adr/0036-resource-credential-adoption-auth-retirement.md §Status + §Decision + §"Amended in place on" + frontmatter (R2 ADR counterpart enactment)
posture: enactment — files modified; verification grep results recorded
---

# Cross-cascade R1+R2 enactment report

## R1 enactment (action §2.2.4)

**Files touched (1):**
- `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`

**Edits applied:**
- §2.2.4 callout box — third amended-in-place callout added at section top citing §15.13 enactment + cross-cascade consolidated review §2.2.1 + §7.1 path (a) routing
- §2.2.4 trait body — stub `pub trait Resource: Send + Sync + 'static { type Credential: Credential; }` hard-deleted
- §2.2.4 explanatory paragraph — replaced stub with reference to resource Tech Spec §2.1 line 157-299 as canonical authority
- §2.2.4 code block — `use nebula_resource::Resource;` import added immediately above `pub trait ResourceAction` declaration
- §0.1 frontmatter status — appended `+ cross-cascade R1+R2 per §15.13` qualifier
- §0.1 status table CP4 row — appended `+ cross-cascade R1 (action §2.2.4 stub Resource trait removal) per §15.13` qualifier
- §15.13 (NEW H3 subsection) — full cross-cascade R1 enactment record (5 subsubsections: §15.13.1 enactment table + per-ADR composition analysis; §15.13.2 amend-in-place vs supersede rationale; §15.13.3 cross-cascade and downstream impact; §15.13.4 §16.5 cascade-final precondition update; §15.13.5 §15.1 closure entries + counterpart R2 cross-reference)
- `### CHANGELOG — post-freeze amendment-in-place 2026-04-26 (cross-cascade R1 — action §2.2.4 stub Resource trait removal)` — full CHANGELOG entry appended to action Tech Spec end (paralleling Q1/Q6/Q7/Q8 CHANGELOG precedent)

**Verification grep results (PASS):**

```
$ rg "^pub trait Resource: Send" docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
(zero matches — stub trait body successfully removed)

$ rg "use nebula_resource::Resource" docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
453:use nebula_resource::Resource;
3476:| **R1 — Resource trait single-source-of-truth** — hard-delete stub `pub trait Resource: Send + Sync + 'static { type Credential: Credential; }`; replace with `use nebula_resource::Resource;` import-only ...
(import statement at the §2.2.4 code block + descriptive citations in §15.13)
```

R1 PASS.

## R2 enactment (resource §2.1 + ADR-0036)

**Files touched (2):**
- `docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md`
- `docs/adr/0036-resource-credential-adoption-auth-retirement.md`

**Edits applied to resource Tech Spec:**
- §2.1 trait method signature — `on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a` (re-pinned from pre-amendment `&<Self::Credential as Credential>::Scheme` shape); doc comment annotated with cross-cascade R2 marker + credential Tech Spec §15.7 line 3394-3429 + iter-3 lifetime-pin line 3503-3516 cross-refs
- §2.1.1 idiomatic impl form example — `async fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> Result<(), Self::Error>` walkthrough; `&*new_scheme` Deref pattern; zeroize-on-Drop comment per credential Tech Spec §15.7 line 3412 Drop ordering
- §2.3 first bullet (invariants) — borrow invariant replaced with owned-guard invariant; cite credential CP5 §15.7 line 3394-3429 + iter-3 lifetime-pin line 3503-3516 + probes #6 (SchemeGuard retention) and #7 (SchemeGuard clone attempt)
- §11.6 RealPostgresPool blue-green swap example — parameter shape changed to `SchemeGuard<'a, Self::Credential>` + `&'a CredentialContext<'a>`; `&*new_scheme` Deref pattern at `build_pool_from_scheme` call site; zeroize-on-Drop comment
- §15.7 (NEW H3 subsection) — full cross-cascade R2 enactment record (6 subsubsections: §15.7.1 enactment table + per-ADR composition analysis; §15.7.2 amend-in-place vs supersede rationale; §15.7.3 cross-cascade and downstream impact; §15.7.4 §16.4 + §16.5 cascade-final precondition update; §15.7.5 ADR-0036 amendment-in-place enactment; §15.7.6 §15.x closure entries + counterpart R1 cross-reference)
- Frontmatter status field — appended `+ amended-in-place 2026-04-26 — cross-cascade R2 per §15.7 — ... ; ADR-0036 §Decision counterpart amendment-in-place per §15.7.5` qualifier
- `## Changelog` section — new bullet entry appended at end describing R2 enactment (paralleling resource Tech Spec CP1/CP2/CP3 changelog precedent)

**Edits applied to ADR-0036:**
- Frontmatter status field — `status: accepted` → `status: accepted (amended-in-place 2026-04-26 — cross-cascade R2)`
- §Status section body — new amendment paragraph appended citing cross-cascade consolidated review §3.2.1 + §6.3 Pattern A supersession-propagation gap + §7.1 path (a) routing
- §Decision conceptual signature — `async fn on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme) -> Result<(), Self::Error>` re-pinned to `async fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> Result<(), Self::Error>`; doc comment annotated with cross-cascade R2 marker + credential Tech Spec §15.7 lifetime-pin cross-ref
- §"Amended in place on" — empty placeholder replaced with full 2026-04-26 cross-cascade R2 entry (paralleling ADR-0035 amended-in-place precedent)

**Verification grep results (PASS):**

```
$ rg "on_credential_refresh.*&Scheme|on_credential_refresh.*&<Self::Credential" docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md
2698:- `crates/resource/src/resource.rs:233` — trait method signature changes from `fn on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme) ...` to `fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) ...`
2721:2. **§Decision** — conceptual signature re-pinned from `async fn on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme) -> Result<(), Self::Error> { Ok(()) }` to `async fn on_credential_refresh<'a>(&self, new_scheme: SchemeGuard<'a, Self::Credential>, ctx: &'a CredentialContext<'a>) -> Result<(), Self::Error> { Ok(()) }`. ...
(matches are descriptive prose in §15.7 enactment record narrating what was changed; no actual signature uses pre-amendment shape)

$ rg "on_credential_refresh.*&<Self::Credential|on_credential_refresh.*new_scheme: &" docs/adr/0036-resource-credential-adoption-auth-retirement.md
(zero matches — pre-amendment signature successfully replaced in §Decision)

$ rg "SchemeGuard" docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md
(matches at trait declaration §2.1, §2.1.1 impl example, §2.3 invariants, §11.6 walkthrough, §15.7 enactment record — all 4 amendment sites + enactment narrative)
```

R2 PASS.

## Status qualifiers updated

- **Action Tech Spec frontmatter status** — appended `+ cross-cascade R1+R2 per §15.13` qualifier (R1 inline at §15.13; R2 cross-referenced to counterpart documents). §0.1 status table CP4 row appended `+ cross-cascade R1 (action §2.2.4 stub Resource trait removal)` qualifier.
- **Resource Tech Spec frontmatter status** — appended `+ amended-in-place 2026-04-26 — cross-cascade R2 per §15.7 — ... ; ADR-0036 §Decision counterpart amendment-in-place per §15.7.5` qualifier.
- **ADR-0036 frontmatter status** — `status: accepted` → `status: accepted (amended-in-place 2026-04-26 — cross-cascade R2)`. §Status body section gains amendment paragraph; §"Amended in place on" gains 2026-04-26 cross-cascade R2 entry.

## Cross-references added

- **Action Tech Spec §15.13** → resource Tech Spec §15.7 (R1 ↔ R2 counterpart cross-reference per §15.13.5 closure)
- **Action Tech Spec §15.13** → ADR-0036 §Decision + §"Amended in place on" (R1 ↔ R2 ADR counterpart per §15.13.5)
- **Action Tech Spec §15.13** → cross-cascade consolidated review §2.2.1 + §7.1 path (a) routing (R1 trigger + amendment routing)
- **Action Tech Spec §15.13** → resource Tech Spec §2.1 line 157-299 (canonical Resource trait authority)
- **Resource Tech Spec §15.7** → action Tech Spec §15.13 (R2 ↔ R1 counterpart cross-reference per §15.7.6 closure)
- **Resource Tech Spec §15.7** → ADR-0036 §Decision + §Status + §"Amended in place on" (R2 ADR counterpart per §15.7.5)
- **Resource Tech Spec §15.7** → cross-cascade consolidated review §3.2.1 + §6.3 Pattern A + §7.1 path (a) routing (R2 trigger + amendment routing)
- **Resource Tech Spec §15.7** → credential Tech Spec §15.7 line 3394-3429 (canonical CP5 SchemeGuard shape) + iter-3 lifetime-pin line 3503-3516 + probes #6, #7 line 3755-3756
- **Resource Tech Spec §2.1 / §2.1.1 / §2.3 / §11.6** → credential Tech Spec §15.7 line 3394-3429 + line 3412 (Drop ordering) + line 3503-3516 (lifetime-pin)
- **ADR-0036 §Status** → cross-cascade consolidated review §3.2.1 + §6.3 Pattern A + §7.1 path (a) routing
- **ADR-0036 §Decision** → credential Tech Spec §15.7 line 3394-3429 + iter-3 lifetime-pin line 3503-3516
- **ADR-0036 §"Amended in place on"** → resource Tech Spec §15.7 (counterpart enactment) + action Tech Spec §15.13 (R1 counterpart)

## Outstanding issues (if any)

None at the cross-cascade R1+R2 enactment scope.

**Per Strategy §4.8 atomicity invariant** + cross-cascade review §7.1: implementation kickoff for the credential CP6 cascade (slot 1) and engine cluster-mode cascade (slot 2) per `docs/tracking/cascade-queue.md` was previously gated on R1+R2 closure. With R1+R2 enacted, slot 1 unblocked; remaining slot-1-shape detail (per cross-cascade review §5.2.1 INCOMPLETE finding I3 — "slot 1 architect-recommended shape column updates to include Resource trait surface from resource Tech Spec §2.1") is a separate cascade-queue.md edit dispatch (not amendment-text), per the consolidated review §7.2 amendment routing.

**Per cross-cascade review §7.2** four 🟠 INCOMPLETE gaps (I1-I4) remain outstanding — these are SHOULD-close-pre-implementation items per §7.2, not 🔴 STRUCTURAL blockers. R1+R2 enactment closes the two 🔴 STRUCTURAL gaps that blocked implementation (§2.2.1 + §3.2.1 in cross-cascade review). I1-I4 routing is tech-lead's call per cross-cascade review §8 §8.2 audit hand-off — not in this enactment dispatch's scope.

**Per cross-cascade review §6.3 Pattern A** supersession-propagation discipline: future cascade governance MIGHT include "supersession-propagation pass" at every credential / action / resource Tech Spec super-amendment. This is a process-level recommendation in cross-cascade review §8.1 architect's reflection; not in this enactment dispatch's scope.

**No production code modified.** All amendments are spec-side documentation per cascade prompt constraints. Production code changes per R2 (5 in-tree consumer migration to `SchemeGuard<'a, _>` parameter shape; Manager dispatcher path constructs `SchemeGuard` from `SchemeFactory<C>`) land at implementation time per Strategy §4.8 atomic single-PR wave.

**No new ADRs created.** Both R1 + R2 fit ADR-0035 amended-in-place precedent per cross-cascade review §7.1 routing.

**ADR-0040 status preserved** at `proposed pending user ratification` per cascade prompt constraint ("Do not flip ADR-0040").

**Credential Tech Spec unmodified** per cascade prompt constraint ("Do not modify credential Tech Spec — it's source-of-truth; just cite").

**Strategy documents unmodified** per cascade prompt constraint ("Do not modify Strategy documents").
