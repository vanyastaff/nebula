# Strategy CP2 — Tech-Lead Ratification

**Date:** 2026-04-24
**Reviewer:** tech-lead (subagent dispatch)
**Document:** `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md`
**Checkpoint:** CP2 (§4 decision record + §5 open items + CP1 ratification edits applied)

---

## Ratification verdict (overall)

**RATIFY_WITH_EDITS** — advance to CP3 with the bounded edits in the "Required edits" section. CP2 faithfully encodes the Phase 2 LOCKED scope (`03-scope-decision.md` §4) and the merged amendments. The two architect-flagged calls (§4.4 engine-fold, §4.2 separate revoke method) match my Phase 2 priority-call intent and the verbatim Phase 2 amendments — I **concur with both**. The edits are scope-tightening on §4.3 dispatch (one Strategy-level invariant missing), §4.5 file-split (one cut-point gap), and §5.5 (evidence-based downgrade from open-item to §4.2 footnote).

Confidence: high. CP2 is a load-bearing decision record; nothing in §4.1-§4.9 contradicts Phase 2 LOCKED, and §5 is the right shape for what Phase 6 must answer.

---

## §4 per-subsection verdicts

### §4.1 Resource trait reshape — ENDORSE

Match: faithful to `03-scope-decision.md:86-92` and Phase 2 tech-lead Q2 (`phase-2-tech-lead-review.md:26-38`). §3.6 verbatim adoption + `type Credential = NoCredential;` opt-out + NO sub-trait. Cross-refs to §1.1 / §2.3 / §3.2 land. The "blue-green pool swap is internalised by the resource" framing matches credential Tech Spec §3.6 lines 961-993 and security amendment B-2's blue-green pattern preservation.

No edits required.

### §4.2 Revocation semantics — ENDORSE (architect-flagged: separate revoke method)

**Concur with architect's choice.** Separate `on_credential_revoke` method, not dual-semantics-on-refresh. Three reasons:

1. **Semantic distinctness.** Revoke ≠ refresh. Refresh has a new `&Scheme` to swap in; revoke has nothing to swap, only a `&CredentialId` and an obligation to tear down. Forcing both through one method requires every implementer to branch on "is the new scheme nil?" — that's the kind of error-prone API that produces silent-drop bugs (the exact bug class this redesign is closing).
2. **Observability symmetry.** §4.9 already commits to two event variants (`CredentialRefreshed`, `CredentialRevoked`). Two events ↔ two methods is the consistent shape; one method emitting two events is harder to reason about and harder to verify in spec-auditor passes.
3. **Phase 2 security amendment B-2 (`phase-2-security-lead-review.md:67-74`) explicitly framed it as "either/or, but be explicit"** — security-lead deferred to tech-lead on which. Architect's choice resolves the deferral on the side that's less footgun-prone. I concur.

**One edit required (E2 below):** §4.2 default-body wording is too prescriptive. "Default body: destroy current pool instances, reject new acquires until a new credential is supplied" is **Phase 6 Tech Spec §5 territory**, not Strategy §4. Strategy should commit to the *invariant* (revocation must result in no further authenticated traffic on the revoked credential), not the *implementation* (destroy pool, block acquires). Phase 6 Tech Spec ratifies the default body. See E2.

### §4.3 Rotation dispatch mechanics — ENDORSE WITH EDIT

Match: faithful to `03-scope-decision.md §4.2:94-100` and Phase 2 tech-lead Q3 (`phase-2-tech-lead-review.md:40-60`). `join_all` initial + `FuturesUnordered` deferred + per-resource isolation + `&Scheme` borrow-not-clone all encoded.

**One edit required (E1 below):** §4.3 says "Per-resource isolation invariant: one resource's failing `on_credential_refresh` must NOT block sibling dispatches" but doesn't pin **timeout-isolation per resource** as part of the invariant. Per Phase 2 tech-lead Q3 amendment ("each per-resource future is bounded by its own timeout") — currently encoded as "Phase 6 Tech Spec §5 specifies the timeout configurable surface" but the *invariant* (per-resource timeout, not global timeout) needs Strategy-level commitment. A misread by Phase 6 author could land a global timeout that defeats isolation. See E1.

### §4.4 Daemon + EventSource extraction target — ENDORSE (architect-flagged: engine-fold)

**Concur with architect's choice.** Engine-fold over sibling crate. Architect's three pieces of evidence are all correct and load-bearing:

1. **No `nebula-scheduler` precedent.** Workspace grep (verifiable today via `Cargo.toml` workspace members) confirms no scheduler-shaped crate exists. Creating one with zero existing adopters is exactly the boundary-erosion anti-pattern `feedback_boundary_erosion.md` warns against — "extract to a new crate without precedent" is worse than "fold into the layer that already orchestrates the right thing."
2. **TriggerAction precedent in canon §3.5.** Engine already dispatches event-driven trigger lifecycles (`PRODUCT_CANON.md §3.5` line 82: `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). EventSource is a thin extension of TriggerAction substrate. Daemon as a parallel primitive is a small surface addition, not a new crate.
3. **Bundling reduces consumer migration churn.** 5 in-tree consumers migrating Daemon/EventSource references in the same atomic PR wave is half the work of two crates' migrations. `feedback_active_dev_mode.md` aligns: ship the bundled change now, don't split for hypothetical future flexibility.

**Phase 2 tech-lead Q6 (`phase-2-tech-lead-review.md:72-74`)** verified zero `crates/engine/` consumption of `DaemonRuntime|EventSourceRuntime|TopologyTag::Daemon|TopologyTag::EventSource` — extraction is safe regardless of landing site, but engine-fold is the correct landing site given the precedent above. Architect's §4.4 trade-off acknowledgement ("engine surface grows … if Daemon-specific lifecycle proves heavyweight, future cascade can spin out `nebula-scheduler` from the engine side") plus §5.1 revisit triggers (Daemon-specific code >500 LOC OR non-trigger workers >2) are the correct guardrails.

No edits required. **Strong endorse.**

### §4.5 Manager file-split — ENDORSE WITH EDIT

Match: faithful to `03-scope-decision.md:39` ("Split the file, NOT the type") and `02-pain-enumeration.md:76`. Proposed cuts (`mod.rs`, `options.rs`, `gate.rs`, `execute.rs`) trace existing internal seams.

**One edit required (E3 below):** the proposed cuts miss the **`on_credential_*` rotation dispatcher**. `manager.rs:1360-1401` (the rotation dispatcher) is its own internal seam — distinct from `register`/`acquire` (in `mod.rs`) and `execute_with_resilience` (in `execute.rs`). Phase 6 Tech Spec §5 will need a 5th submodule (`manager/rotation.rs` or similar) for the dispatcher + observability scaffolding (§4.9 trace span + counter + event emission). Strategy §4.5 should at minimum acknowledge that the file-split must accommodate the new dispatcher's location, even if the exact cut deferred to Phase 6. See E3.

### §4.6 Drain-abort fix — ENDORSE

Match: faithful to `03-scope-decision.md:80` (SF-2 absorbed into Option B) and `02-pain-enumeration.md:69-74`. Bundling with Manager file-split PR is correct — both touch `manager.rs:1493-1510` and `runtime/managed.rs:93`, splitting them costs more in review than it saves.

No edits required.

### §4.7 Documentation rewrite — ENDORSE

Match: faithful to `03-scope-decision.md:35` ("Rewrite docs AFTER trait shape locks. Docs are a Phase 6 deliverable, not Phase 3") and dx-tester Phase 2 input (`02-pain-enumeration.md:239-242`). The 6-deliverable enumeration (api-reference, adapters, dx-eval-real-world, Architecture, README, events) covers the doc rot from §1.3.

**Editorial nit (not required):** `crates/resource/docs/Architecture.md` deletion-or-rewrite question is left to Phase 6. I'd lean **delete + collapse into `README.md`** (one canonical file beats two drifting files), but architect's "OR delete" framing leaves it open which is fine — Phase 6 dx-tester ratification picks.

### §4.8 Migration wave — ENDORSE

Match: faithful to `03-scope-decision.md §4.7:135-137`. Atomic 5-consumer PR, no shims, no deprecation windows, MATURITY = `frontier`. The `MATURITY.md` row transition note ("may transition from `frontier` to `core` post-redesign … Phase 6 ratifies the maturity move; not assumed here") is the correct deferral — observability-partial gap closure is a Phase 6 verifiable claim, not a Strategy-level assumption.

No edits required.

### §4.9 Observability discipline — ENDORSE

Match: faithful to `03-scope-decision.md §4.4:109-116` (tech-lead amendment 2) + security amendment B-3 + `feedback_observability_as_completion.md`. Three-artefact DoD (trace span, counter, event variant) + Phase 6 CP-review gate + `Scheme::default()` ban all encoded. Trace span field redaction per `PRODUCT_CANON.md §12.5` is correctly cited.

The `CredentialRefreshed` + `CredentialRevoked` event symmetry with §4.2 is the right shape. The metric name `nebula_resource.credential_rotation_attempts` is a candidate (Phase 6 finalizes), and the companion latency histogram is a good addition I didn't explicitly amend in but endorse.

No edits required.

---

## §5 open items feedback

**The 6 items are the right set.** §5.1 (Daemon revisit triggers), §5.2 (`AcquireOptions::intent/.tags` interim), §5.3 (`Runtime`/`Lease` future cascade), §5.4 (convenience method symmetry), §5.5 (credential §3.6 revoke extension), §5.6 (Phase 4 spike confirmed) all match `03-scope-decision.md §8` open items 1-6 (with §5.6 being the spike-trigger confirmation that wasn't a Phase 2 open question but is correctly captured).

### §5.5 — recommend downgrade to §4.2 footnote (E4 below)

**Architect flagged §5.5 as "could be §4 decision if evidence exists in credential §3.7+."** I checked. Evidence:

- **Credential Tech Spec §3 ends at §3.6** (verified — no §3.7 exists; §3.6 is the last subsection of §3).
- **But credential `Credential` trait already has a `revoke()` method** at `2026-04-24-credential-tech-spec.md:227-228` ("`async fn revoke(...)` … revoke endpoint; implementations return Ok(()) with no-op").
- **§4 of the credential Tech Spec** documents soft/hard/scheduled revocation modes (lines 1062-1068) and the `revoked` `state_kind`.

So the credential side already has revoke primitives at the trait level (`Credential::revoke`) and at the engine-orchestration level (§4 lifecycle). What `on_credential_revoke` on the resource side does is **dispatch the engine's revocation event to per-resource teardown** — that's a **resource-side extension that consumes the credential primitives, not an extension of credential §3.6**.

**Recommendation:** §5.5 is mis-framed as a credential-side spec dependency. It's actually a one-way consumption of an existing credential primitive. Downgrade from open-item to §4.2 footnote: "*The credential side already provides `Credential::revoke` (Tech Spec line 227) and revocation lifecycle (§4 lines 1062-1068); this resource-side hook consumes those primitives. No credential-side spec extension required.*"

This is a **non-trivial scope clarification** — if §5.5 stays as an open item, Phase 6 Tech Spec author may waste a coordination round with credential-side spec-auditor that isn't needed. See E4.

### Missing open items: none

I went through `03-scope-decision.md §8` (6 open items) and `02-pain-enumeration.md` deferred-with-pointer findings (6 items) line-by-line. Everything load-bearing is encoded somewhere in §4 (in-scope decisions), §5 (open items), or §0 footer (deferred-with-pointer). No silent drops.

### Items that should NOT become §4 decisions

§5.1 (Daemon revisit), §5.3 (Runtime/Lease), §5.4 (NoCredential symmetry), §5.6 (spike trigger) all correctly stay as open items — they have explicit triggers or Phase 6 owners. §5.2 (`AcquireOptions::intent/.tags`) is an interim treatment with the (a)/(b) candidate already named — that's fine to leave to Phase 6 §5 (architect's note "(b) is more honest" is the right lean and matches my Phase 2 Q4 position `phase-2-tech-lead-review.md:64-66`).

---

## CP1 edits verification

All four CP1 edits landed correctly per the CHANGELOG (lines 374-381):

| ID | Description | Verification | Status |
|----|-------------|--------------|--------|
| **E1** | §2.4 Phase 4 spike-exit-criteria constraint | Line 196: "Spike exit criteria (Phase 4): do NOT include sub-trait fallback. The `Resource::Credential` shape per credential Tech Spec §3.6 is the locked target. If §3.6 ergonomics or perf fail the spike, escalate to Phase 2 round 2…" | ✅ landed |
| **E2** | §2.3 "CP2 §4 must extend §3.6" | Line 179: "**CP2 §4 must extend §3.6 with revoke semantics**" | ✅ landed |
| **E4** | §2.4 `feedback_active_dev_mode.md` reference | Line 195: "**`feedback_active_dev_mode.md`** — `frontier` maturity + active-dev posture means breaking changes ship now…" | ✅ landed |
| **E3** | §3.2 parenthetical on amendments | Line 208: "co-decision body … unanimously picked Option B in round 1 of the max-3 protocol (with 2 tech-lead amendments and 3 security-lead amendments tightening the in-scope envelope; all endorsed in the single round, per the per-review pointers below)" | ✅ landed |

E3 specifically: I instructed "keep 'unanimous' since that's the protocol-level fact" — line 208 keeps "unanimously" as the headline word and adds the parenthetical for amendment count visibility. **Correct application of my instruction.**

---

## Required edits before CP3 lock

| ID | Edit | Location | Priority |
|----|------|----------|----------|
| **E1** | §4.3: pin per-resource timeout isolation as Strategy-level invariant. Add to "Per-resource isolation invariant" sentence: "…must NOT block sibling dispatches **AND each per-resource future is bounded by its own timeout, not a global dispatch timeout** (timeout-isolation as part of the isolation invariant; Phase 6 Tech Spec §5 specifies the configurable surface)." | §4.3 paragraph 1 | **HIGH** — without this, Phase 6 could land global timeout that defeats isolation |
| **E2** | §4.2: relax default-body wording from implementation-prescriptive to invariant-prescriptive. Change "Default body: destroy current pool instances, reject new acquires until a new credential is supplied" → "Default body invariant: post-revocation, the resource emits no further authenticated traffic on the revoked credential. Phase 6 Tech Spec §5 specifies the default-body implementation (likely: drop pool + reject acquires, but the invariant — not the mechanism — is what Strategy commits to)." | §4.2 paragraph 1 | **MEDIUM** — keeps Strategy at the right abstraction layer; avoids preemptively over-specifying |
| **E3** | §4.5: acknowledge file-split must accommodate rotation dispatcher submodule. Add 5th proposed cut: "`manager/rotation.rs` — `on_credential_refresh` / `on_credential_revoke` dispatchers + observability scaffolding (trace span, counter emission, event broadcast per §4.9). Phase 6 Tech Spec §5 finalizes naming and scope." | §4.5 cut-points list | **MEDIUM** — closes a structural gap; rotation dispatcher is a real internal seam |
| **E4** | §5.5: downgrade from open item to §4.2 footnote. Move the credential-side coordination question into §4.2 with the resolution: "**Credential-side coordination.** The credential Tech Spec already provides `Credential::revoke` (line 227-228) and revocation lifecycle modes (§4, lines 1062-1068). This resource-side hook consumes those primitives — no credential-side spec extension required. CP3 §6 records this as a closed item, not an open dependency." Renumber §5.5 → drop, §5.6 → §5.5 (Phase 4 spike confirmed). | §4.2 + §5 | **MEDIUM** — saves a Phase 6 coordination round; evidence is in the credential Tech Spec today |

E1 is the only edit with substantive forward-binding effect (timeout isolation is hot-path correctness). E2/E3/E4 are scope-tightening + structural-gap close. None require a co-decision cycle — architect can apply directly.

---

## Convergence estimate

**Lock CP2 in iteration 1 after E1+E2+E3+E4 land. CP3 (§6 post-validation roadmap) can dispatch immediately after.**

Reasoning:
- Verdict is RATIFY_WITH_EDITS, not ITERATE — no Phase 2 re-litigation, no scope drift.
- Both architect-flagged calls (§4.4 engine-fold, §4.2 separate revoke) match Phase 2 priority-call intent and verbatim amendments. **Strong concur on both.**
- All 4 edits are bounded text additions/changes (no new decisions, no new dispatches). Estimated architect effort: 10-15 minutes.
- spec-auditor parallel structural-consistency review may surface additional citation-level issues; if so, those are a separate iteration trigger, not me.
- security-lead handoff is conditional (architect noted "if tech-lead flags" in §"Handoffs requested"). My ratification does NOT flag a security re-review — §4.2 / §4.3 / §4.9 all match Phase 2 security amendments verbatim. No security-lead round needed unless spec-auditor surfaces something.

**CP2 should advance to CP3 once E1+E2+E3+E4 land.** Estimated architect effort: 10-15 minutes of editing. Confidence: high.

---

## Artefact references

| Artefact | Path |
|---|---|
| This ratification | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp2-tech-lead-ratification.md` |
| Strategy CP2 | `docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md` |
| CP1 ratification | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp1-tech-lead-ratification.md` |
| Phase 2 tech-lead review | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md` |
| Phase 2 security-lead review | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md` |
| Phase 2 scope decision (LOCKED) | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md` |
| Phase 1 pain enumeration | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md` |
| Credential Tech Spec §3.6 (verbatim) | `docs/superpowers/specs/2026-04-24-credential-tech-spec.md:928-996` |
| Credential `Credential::revoke` primitive | `docs/superpowers/specs/2026-04-24-credential-tech-spec.md:227-228` |
| Credential revocation lifecycle modes | `docs/superpowers/specs/2026-04-24-credential-tech-spec.md:1062-1068` |

*End of ratification. Awaiting architect edit application + CP3 §6 dispatch.*
