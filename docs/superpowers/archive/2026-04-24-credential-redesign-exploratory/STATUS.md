---
name: credential redesign exploratory drafts — archived
status: not adopted
archived-on: 2026-04-24
archived-by: Path A decision per tech-lead + 3-agent convergence
related:
  - docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md (Spec H0 — finding #17 promoted)
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md (existing spec, remains active)
  - docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md (landed work)
---

# STATUS — not adopted

## TL;DR

Эти drafts — exploratory notes from a 3-round conversation + 37-finding user audit + 4-agent specialist review (security-lead, rust-senior, tech-lead, dx-tester). **Paper design failed three independent stress tests.** Archived as reference documentation; **not** a design that will be implemented.

One finding (#17) — multi-replica mid-refresh race с refresh_token rotation (n8n #13088 class) — promoted to standalone spec at `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`.

## Why archived

1. **Q1 compile test confirmed rust-senior's prediction** — `CredentialRef<dyn BitbucketCredential>` as Pattern 2 default не компилится usefully. E0191 fires; naming 4 assoc types defeats Pattern 2's purpose. Evidence: `scratch/q1-dyn-credential-test/` (test.rs + RESULT.md) showing rustc 1.95.0 output.

2. **Dx-tester hit 21 walls** trying to write 4 actions + 2 resources against proposed API. Ergonomics 1-2/5 на каждом component. **Pattern 2 (draft default) blocks on line 1 of first action written.**

3. **Security-lead identified 10 new structural findings** (N1-N10) — 3 HIGH severity requiring pre-spike trait shape changes: WebhookUrlScheme tracing side-channel, `sign()` default fallback = silent authN downgrade, plugin-declared FieldSensitivity self-attested. These shape trait signatures, cannot retrofit post-spec.

4. **Tech-lead priority call — Path C (defer redesign).** ROI негативный: 3-5 day spike estimate — 2-3x optimistic; realistic 6-10 weeks total (spike → spec → migration → tests) для trait-shape refinement which unblocks **zero consumers currently**. *"n8n doesn't lose customers over credential trait ergonomics."*

## What was material

The 37-finding audit materially moved things forward:

- **#17 promoted to spec** — finding was sharp and correct (real correctness gap, maps to concrete n8n pain #13088)
- **ADR-0031 "n8n parity" rationale identified as weak** — not standalone supersede target, но revisit if #17 work forces HTTP ownership question
- **Pattern 1 vs Pattern 2 default** — draft's original framing ошибочен; per user's data, most popular services multi-auth
- **Credential-oauth / oauth2-http feature gates** — shipped deletion as cleanup (PRs #1-2 в same change series as archive)
- **Three dead AuthScheme variants** — `FederatedAssertion` / `OtpSeed` / `ChallengeSecret` pruned (PR #3)

## What's valuable here as reference

Keep reading — these files have documentation value despite не being adopted:

- **`02-layer-map.md`** — accurate cross-crate responsibility map для credential domain. Usable как onboarding doc для newcomers asking "кто чем владеет".
- **`04-schemes-catalog.md`** — catalogue of 15 auth scheme types с injection mechanics. Reference document для future scheme additions regardless of trait shape.

## What's speculative (don't use as authority)

- **`01-type-system-draft.md`** — trait shape proposal that Q1 test invalidated. Useful only as "attempted shape that didn't work" for future redesign attempts.
- **`03-flows.md`** — flow diagrams some of which are still accurate (Flow 2 callback, Flow 7 revoke) but overall describes architecture we're not building.
- **`05-known-gaps.md`** — 37 findings triage. Finding #17 promoted; остальные not worked per tech-lead Path C. Useful only as "what we considered и chose not to do."
- **`06-prototype-plan.md`** — spike plan **not dispatched**. Superseded by decision not to redesign.
- **`00-overview.md` + `README.md`** — set frame для exploration которое не продолжилось.

## Explicit non-status

This archive **does not**:

- Claim paper design was wrong in all respects
- Claim audit was overkill (audit was materially good)
- Claim existing post-P6-P11 architecture is "done" forever — future consumer pain may force revisit
- Block future credential redesign — if concrete customer/integration-author hits wall, revisit

This archive **does**:

- Document что was explored, what found holes, why decided not to proceed
- Preserve valuable reference material (layer map, schemes catalog)
- Cite specific evidence (Q1 test, 4-agent review, dx-tester walls) для future decision-makers

## Canon / spec pointers

Active authority для credential architecture остаётся:

- `docs/adr/0028-cross-crate-credential-invariants.md`
- `docs/adr/0029-storage-owns-credential-persistence.md`
- `docs/adr/0030-engine-owns-credential-orchestration.md`
- `docs/adr/0031-api-owns-oauth-flow.md` (не superseded — revisit only if forced)
- `docs/adr/0032-credential-store-canonical-home.md`
- `docs/adr/0033-integration-credentials-plane-b.md`
- `docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md` (existing active spec)
- `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md` (new Spec H0 — finding #17)

## Finding-level outcome map

| Finding | Outcome |
|---|---|
| #17 multi-replica refresh race | **Promoted to Spec H0** — active development |
| #1, #2, #3, #32 type system | Archive — paper design issues, no consumer blocked |
| #5 Pattern 1 vs 2 default | Archive — moot given type system не ships |
| #8 sealed vs plugin | Archive — no plugin authors yet |
| #14 multi-credential resource | Archive — revisit when consumer need surfaces |
| #22, #23 multi-step flow | Archive — atomic-only sufficient today |
| #18-20 provider registry | Archive — defer to ADR if OAuth endpoint tenant templating needed |
| #34 WebSocket events | Archive — UX feature, defer until product demand |
| #35 trigger integration | Archive — это trigger-trait work, not credential |
| #36 runtime schema migration | Archive — revisit при first State v2 migration |
| All other findings | Archive — details, не blocking |

## Half-life

Re-read this archive в 6 months (2026-10-24). If new evidence emerges (real consumer hitting trait ambiguity, real plugin author blocked, real operator requesting multi-auth service), revisit. Otherwise, archive stays archive.
