# Phase 1 — Pain enumeration consolidated

**Date:** 2026-04-24
**Orchestrator:** claude/upbeat-mendel-b30f89
**Inputs (4 parallel agents):**
- [`02a-dx-authoring-report.md`](./02a-dx-authoring-report.md) (dx-tester; 3 action types authored in scratch worktree)
- [`02b-security-threat-model.md`](./02b-security-threat-model.md) (security-lead)
- [`02c-idiomatic-review.md`](./02c-idiomatic-review.md) (rust-senior; post-Phase-0 second pass)
- [`02d-architectural-coherence.md`](./02d-architectural-coherence.md) (tech-lead)

---

## 0. Gate decision

**Gate status:** ✅ PROCEED to Phase 2 scope narrowing.

Phase 1 gate threshold (per cascade prompt): escalate if total 🔴 = 0 AND 🟠 < 3. Phase 1 produced **~11 deduplicated 🔴 findings** and **~30+ 🟠 findings** across 4 agents. Gate passes by a wide margin — redesign is **evidence-justified**, not speculative.

---

## 1. TL;DR

Four agents converged on the same core story with complementary angles:

- **The action crate is structurally sound at runtime.** Cancel safety is "reference-quality" (rust-senior). Error taxonomy is "reference-quality" (rust-senior). Engine dispatch is clean 4-variant (tech-lead verified via grep, not memory). Webhook crypto primitives are correct (security-lead: constant-time compare, timing-invariant decode, 1 MiB body + 256 header caps, fail-closed default).
- **The integration surface is broken or underspecified.** Three independent agents (dx-tester, security-lead, rust-senior) each hit the same `#[derive(Action)]` emission bug (C2 `parameters = Type`). Credential integration is unusable in both string form (silently drops to zero deps) and typed form (`CredentialLike` has **zero workspace implementors**). The type-name-lowercase credential key is a **cross-plugin shadow attack vector exploitable today**, not hypothetical.
- **The 10-trait surface is a governance issue, not a shape issue.** Dispatch is 4 clean variants; DX traits are compile-time helpers that erase via adapters. Canon §3.5 says 4 and code ships 10 with no canon revision — this is documentation drift, fixable by revision or sealing.
- **CRITICAL REFRAME** (tech-lead): Credential Tech Spec CP6 vocabulary (`CredentialRef<C>`, `SchemeGuard`, `SlotBinding`) has **zero `src/` matches in the credential crate itself**, not just action. **Action is not lagging a shipped credential API; both crates are out-of-sync with a still-unimplemented spec.** This changes Phase 2 scope framing fundamentally (see §5).

---

## 2. Convergence patterns

Four agents, four independent passes, four angles — but the findings cluster cleanly:

### P1 — Credential integration is comprehensively broken

| Agent | Finding |
|---|---|
| dx-tester | 🔴 String-form `#[action(credential = "key")]` silently drops to 0 deps; typed form requires `CredentialLike` with zero implementors |
| dx-tester | 🔴 V2 spec's `ctx.credential::<S>(key)` + `credential_opt::<S>(key)` don't exist; 3 alternative methods, none match spec; no-key variant requires `Zeroize` that `SecretToken` doesn't satisfy |
| security-lead | 🔴 Type-name-lowercase key enables cross-plugin shadow attack (plugin B registering `struct OAuthToken` resolves plugin A's token) |
| rust-senior | 🔴 (confirmed Phase 0 C2) `.with_parameters()` emission against nonexistent method |
| tech-lead | 🔴 Credential CP6 vocabulary unimplemented in credential crate itself; Option A = co-landing two cascades |

### P2 — Macro emission bugs are load-bearing, not cosmetic

| Agent | Finding |
|---|---|
| dx-tester | 🔴 `#[derive(Action)]` emits `::semver::Version` unqualified; user must add `semver` dep themselves (undocumented) |
| dx-tester | 🔴 `Input: HasSchema` bound undocumented in README and v2 spec; spec's `type Input = Self` example fails without hidden `#[derive(Schema)]` |
| rust-senior | 🔴 `parameters = Type` emission path broken (reconfirmed via `cargo expand`) |
| rust-senior | 🟠 Adapter bound leak (`DeserializeOwned`/`Serialize` required at adapter, not trait; errors migrate to registration site) |

Root cause: **no macro test harness** (Phase 0 T1) → bugs accumulated silently.

### P3 — Security hardening gaps are concrete, not theoretical

| Agent | Finding |
|---|---|
| security-lead | 🔴 **S-J1** Depth-unbounded `serde_json::from_value` at `StatelessActionAdapter::execute` — attacker-deep JSON via workflow input stack-overflows worker |
| security-lead | 🔴 **S-C2** Cross-plugin shadow attack (same as P1 security-lead row, counted once) |
| security-lead | 🟠 **S-W2** `SignaturePolicy::Custom(Arc<dyn Fn>)` is unbounded trust delegation; pass-through closure indistinguishable from real verifier at audit |
| security-lead | 🟠 **S-C4** `CredentialGuard` Drop+zeroize correct in common case but defeasible via `tokio::spawn` + `Arc` clone |
| security-lead | 🟠 **S-C5** No cancellation-zeroize test in `testing.rs` |

Disproven: **Retry feature flag hypothesis NOT exploitable** — engine handles via always-on `is_retry()` predicate at `engine.rs:1889`, never silently drops (security-lead + Phase 0 rust-senior both verified).

### P4 — Canon §3.5 drift is documentation, not structural

| Agent | Finding |
|---|---|
| tech-lead | 🟠 10 traits — all except ControlAction carry weight; dispatch is 4-variant enum; canon §3.5 drift = governance drift, not shape drift |
| rust-senior | 🟠 Dyn-safe `*Handler` companion traits verbose but not defective (HRTB pattern inherited from `async-trait` convention, can be tightened without breaking dyn-safety) |
| rust-senior | 🟢 Typed trait family uses RPITIT correctly everywhere; no `#[async_trait]` anywhere |
| tech-lead | ControlAction is helper-masquerading-as-trait — dispatch-wise IS a StatelessAction; public-non-sealed framing is wrong |

---

## 3. Critical reframe (tech-lead §8 finding)

Phase 0 C1 described credential CP6 vocabulary as absent from `nebula-action`. Tech-lead's grep of `crates/credential/src/` confirms: `CredentialRef`, `SchemeGuard`, `SlotBinding`, `SchemeFactory`, `AnyCredential` — **zero matches in credential crate**.

**Consequence for Phase 2 scope:**

- **Option A (Phase 0 framing)**: "Action adopts CP6 vocabulary" is more accurately restated as **"Co-land credential impl + action redesign in the same cascade"**. This doubles the scope (credential crate needs full implementation of §§2.7/3.4/7.1/15.7 first or in parallel).
- **Option B**: Action stays with current idiom; fix immediate bugs (C2 macro, C3 key heuristic, C4 JSON cap); defer CP6 implementation to a later credential-crate cascade. Action cascade stays scoped.
- **Option C (original escalation rule 10 trigger)**: Request credential Tech Spec §§2.7/3.4/7.1/15.7 revision to accommodate derive-based implementation. Unfreezes a frozen spec.

Tech-lead position: "lean A, fallback C, not B" — rationale: Option B requires a permanent engine-level bridge emulating phantom-safety at runtime (`feedback_boundary_erosion` violation). **But**: position NOT pre-decided; Phase 2 co-decision with architect + security-lead.

**Orchestrator note:** Option A is now a LARGER scope than Phase 0 suggested. The 5-day budget is tighter. Phase 2 must produce a clear cost-per-option analysis.

---

## 4. Deduplicated findings summary

Severity buckets, cross-referenced to sub-reports:

### 🔴 CRITICAL (11 deduplicated)

| # | Finding | Primary source | Cross-refs |
|---|---|---|---|
| CR1 | Credential CP6 vocabulary absent (both crates) | tech-lead §8 | Phase 0 C1, dx-tester 1, security S-C2 |
| CR2 | `#[derive(Action)]` `parameters = Type` emits nonexistent method | rust-senior §2 | Phase 0 C2, dx-tester §1 |
| CR3 | Type-name-lowercase credential key — cross-plugin shadow attack | security S-C2 | Phase 0 C3, dx-tester §3 |
| CR4 | JSON depth bomb at adapter boundary (unlimited recursion) | security S-J1 | Phase 0 C4 |
| CR5 | `#[action(credential = "str")]` silently drops to 0 deps | dx-tester §3 | — |
| CR6 | Typed `#[action(credential = Type)]` requires `CredentialLike` — zero implementors workspace-wide | dx-tester §3 | — |
| CR7 | `ctx.credential::<S>(key)` / `credential_opt::<S>(key)` (v2 §3) don't exist; 3 non-matching alternatives | dx-tester §3 | Phase 0 S4 |
| CR8 | `#[derive(Action)]` emits unqualified `::semver::Version` (user must add dep, undocumented) | dx-tester §1 | — |
| CR9 | `Input: HasSchema` bound undocumented; v2 spec's `type Input = Self` example fails silently | dx-tester §1 | — |
| CR10 | No-key `credential<S>()` variant requires `Zeroize` that built-in `SecretToken` doesn't satisfy | dx-tester §3 | — |
| CR11 | No macro test harness → all macro bugs (CR2, CR8, CR9) masked | Phase 0 T1, rust-senior §2 | — |

### 🟠 MAJOR (30+; categorized)

**Architectural coherence (4):**
- Canon §3.5 / §0.2 drift via ControlAction (tech-lead §1, Phase 0 S1)
- `ActionResult::Terminate` not gated despite "Phase 3 not wired" (Phase 0 S3)
- DX trait layer (5 traits) added without canon sign-off (tech-lead §1, Phase 0 S2)
- Plugin ecosystem migration risk: 7 reverse-deps, 69 files, 40+ sdk::prelude re-exports (Phase 0 §8)

**Macro emission hygiene (5):**
- Adapter bound leak (rust-senior §3)
- Optional credential attribute syntax (`credential(optional)`) not supported (Phase 0 S4, dx-tester)
- `#[action(...)]` emits nothing for ports (README promise unmet; Phase 0 §8)
- `#[nebula]` attribute registered but no branch (Phase 0 §4)
- `TerminationCode` forward-reference to nonexistent Phase 10 roadmap (Phase 0 §9)

**Idiomatic Rust (2):**
- `*Handler` HRTB verbose boilerplate — `for<'life0, 'life1, 'a>` inherited from `async-trait` convention; single `'a` + type alias works (rust-senior §1, §6; Phase 0 S6)
- `trait_variant::make(Send)` is canonical 1.95 replacement for custom HRTB emission (rust-senior §6)

**Security hardening (6):**
- S-W2 unbounded `Custom(Arc<dyn Fn>)` signature policy (security §5)
- S-C4 `CredentialGuard` zeroize defeasible via spawn+clone (security §3)
- S-C5 no cancellation-zeroize test (security §3)
- Plus 3 webhook-adjacent hardening items (security §5 full report)

**Tooling / CI / workspace (9):**
- No macro test harness → bugs masked (Phase 0 T1, already above but also tooling)
- `unstable-retry-scheduler` dead empty feature + `--all-features` CI (Phase 0 T2)
- Dead `nebula-runtime` reference in `test-matrix.yml:66` + CODEOWNERS:52 (Phase 0 T3)
- `zeroize` pinned inline vs workspace (Phase 0 T4)
- Lefthook pre-push doesn't mirror doctests / msrv / doc (Phase 0 T5)
- No action benchmarks (Phase 0 T6)
- SDK prelude 40+ re-export surface is public contract (Phase 0 T7)
- Engine 27+ import sites tight coupling (Phase 0 T8)
- Missing layer-enforcement deny rule for `nebula-action` (Phase 0 T9)

**DX pattern (4 new):**
- Time-to-first-successful-StatelessAction: 12 min (target <5 for senior dev)
- Time-to-first-successful-ResourceAction+Credential: 32 min (target <5)
- README claims `ResourceAction lives in nebula-resource` — lives in nebula-action (Phase 0 §8)
- 3 credential-access method variants (one type-name heuristic) vs spec's 2-method pair — choice paralysis at authoring

### 🟡 MINOR
Tracked in sub-reports. Not cascade-shaping individually; may be rolled into Phase 6 Tech Spec cleanup section.

---

## 5. Updated Phase 2 scope options

Revised framing post-Phase 1 (was A/B/C in Phase 0; now A'/B'/C' with cost estimates):

### Option A' — Co-landed cascade (action + credential implementation)

**Scope:**
- Credential crate implements CP6 vocabulary (`CredentialRef<C>`, `AnyCredential`, `SlotBinding`, HRTB `resolve_fn`, `SchemeGuard<'a, C>`, `SchemeFactory<C>`, `RefreshDispatcher`)
- Action crate adopts CP6 vocabulary: new `#[action]` attribute macro (not derive) with silent field-type rewriting; `CredentialRef<C>` field support; `ActionSlots` impl emission; new `ActionContext` methods matching spec
- Engine wires `resolve_as_<capability><C>` helpers, registers slot bindings, honors HRTB fn-pointers
- Plugin ecosystem migration: all 7 reverse-deps + 40+ sdk::prelude surfaces updated

**Cost estimate:** ~4× single-crate cascade. **Exceeds 5-day autonomous budget.** Escalation likely.

### Option B' — Action-scoped bug-fix + hardening (defer CP6 to separate cascade)

**Scope:**
- Fix CR2/CR5/CR6/CR8/CR9/CR10 macro bugs
- Fix CR3 credential key heuristic (deprecate `credential<S>()` no-key variant; require explicit key)
- Fix CR4 JSON depth cap at all adapter boundaries
- Add macro test harness (trybuild) — CR11
- Seal or canonize ControlAction (tech-lead solo call — Phase 1 input)
- Feature-gate + wire `ActionResult::Terminate` (tech-lead solo call — Phase 1 input)
- Modernize `*Handler` HRTB boilerplate to single `'a` + type alias
- Clean up workspace hygiene (zeroize pin, dead nebula-runtime reference, lefthook gaps)

**Cost estimate:** Fits 5-day budget. Defers CP6 adoption to later credential cascade.

**Risk:** Leaves action with superseded credential idiom. Plugin authors continue to encounter P1 friction. Option B' is local-optimal, not global-optimal.

### Option C' — Escalate credential spec revision

**Scope:**
- Request credential Tech Spec CP6 §§2.7/3.4/7.1/15.7 revision to permit derive-based implementation (no attribute macro; no silent field-type rewriting; explicit user-visible types)
- Action adopts revised shape
- Credential crate implements revised shape

**Cost estimate:** Unknown; depends on user call. Unfreezes frozen spec — **triggers escalation rule 10 hard-stop**.

---

## 6. Stakeholder positions (Phase 1 inputs for Phase 2)

- **dx-tester**: no explicit option vote; evidence heavily supports "unusable credential idiom is unacceptable in any form" — implicit lean A' or C' (anything that fixes P1). Neutral on B' if P1 handled via deprecation + doc fix + bug fixes.
- **security-lead**: no explicit option vote; CRITICAL findings (S-C2 shadow attack, S-J1 JSON bomb) must be addressed in any scope. S-C2 fundamentally unsolvable without key explicitness (rules out keeping current heuristic). Implicit lean toward A' (full vocabulary eliminates shadow attack class) or B' with CR3 deprecation.
- **rust-senior**: no explicit option vote; idiomatic improvements (HRTB modernization) fit all three options. Macro bugs must be fixed regardless. Compatible with B'.
- **tech-lead**: explicit position — **lean A', fallback C', not B'**. Rationale: B' requires permanent bridge layer; `feedback_boundary_erosion` violation.

**Orchestrator synthesis:** Tech-lead's A'/C'/not-B' position is load-bearing because he owns priority calls. However, A' cost estimate exceeds autonomous budget. Phase 2 must converge architect + tech-lead + security-lead on whether:
- A' is split across two cascades (this one is action-scoped; credential impl is next)
- A' lands with an extended budget (user authorization required)
- B' is ratified with explicit sunset (action knowingly ships superseded idiom)
- C' is authorized (user ratifies spec revision)

---

## 7. Load-bearing decisions matrix (updated)

| Question | Phase 0 options | Phase 1 refinement | Decision owner |
|---|---|---|---|
| Credential integration scope | A/B/C | **A'/B'/C'** (cost-re-estimated; A' now = co-cascade, B' = action-scoped only, C' = spec unfreeze) | Phase 2 co-decision; **user authorization likely needed for A' or C'** |
| ControlAction canon status | 3 options | tech-lead solo: seal + canonize DX tier in §3.5 as "erases to primary" | tech-lead ratified (Phase 1 solo call) |
| DX trait layer status | 3 options | tech-lead: don't dissolve; ratify with canon revision OR seal DX + doc as adapter-patterns | tech-lead + architect (Phase 2) |
| `#[action]` macro vs `#[derive]` | 3 options | A' requires attribute macro; B'/C' compatible with derive | blocked on credential scope |
| `ActionResult::Terminate` gating | 3 options | tech-lead solo: feature-gate + wire in cascade (`feedback_active_dev_mode`) | tech-lead ratified (Phase 1 solo call) |
| Macro test harness | 3 options | Add trybuild + macrotest as cascade deliverable (all agents support) | devops (implementation phase) |
| Lefthook parity | 2 options | Fix divergence per user feedback memory | devops (implementation phase) |
| `zeroize` workspace pin | 2 options | Migrate to workspace=true | devops (implementation phase) |

Three solo-decided tech-lead calls (ControlAction seal, Terminate gate+wire, DX tier framing) are ratified as Phase 1 outputs — Phase 2 does not re-litigate these, but architect/security-lead may contest if they surface conflicts.

---

## 8. Adjacent concerns for separate filing

Not in action cascade scope but surfaced by audits:

1. **T3 dead `nebula-runtime` reference** in `test-matrix.yml:66` + `.github/CODEOWNERS:52` — likely causes failing shard on push/workflow_dispatch. Separate PR.
2. **Credential crate implementation of CP6 vocabulary** — zero `src/` matches for `CredentialRef`/`SchemeGuard`/`SlotBinding`/`SchemeFactory`. This is its own cascade (or its own Phase 1 of a wider effort). Action cascade can decide whether to absorb (Option A') or defer (Option B').
3. **`unstable-retry-scheduler` feature hygiene** — dead empty feature turned on every CI run. Minor hygiene fix.

---

## 9. Phase 2 dispatch readiness

Phase 2 is a **co-decision protocol**:
- architect (proposes scope options with trade-offs)
- tech-lead (priority call on scope)
- security-lead (blocks if scope drops security-critical findings CR3 / CR4)

Phase 2 gate: scope locked with explicit OUT-of-scope markers and pointer to future sub-specs (per cascade prompt §Phase 2).

**Orchestrator note on escalation posture**: Phase 1 reframe elevates probability of user escalation in Phase 2 (A' exceeds budget; C' unfreezes spec). If Phase 2 co-decision picks A' or C' and cost analysis confirms, orchestrator will write `ESCALATION.md` rather than unilaterally picking scope. Expect this.

---

*End of Phase 1 consolidation. Orchestrator proceeds to Phase 2 dispatch.*
