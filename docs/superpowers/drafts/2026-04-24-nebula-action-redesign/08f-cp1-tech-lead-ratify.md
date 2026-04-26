# 08f — CP1 tech-lead ratification (post-iteration)

**Decider:** tech-lead (solo-decider mode)
**Date:** 2026-04-24
**Document ratified:** `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` DRAFT CP1 (post 5-reviewer iteration 2026-04-24)
**Inputs:** 08a (spec-auditor REVISE), 08b (rust-senior RATIFY-WITH-NITS), 08c (security-lead ACCEPT-WITH-CONDITIONS), 08d (dx-tester REVISE), 08e (devops RATIFY-WITH-NITS), CHANGELOG entries for the iteration pass
**Mode:** solo-decider on G6 + §2.7.1 ratification per architect's handoff request line 769

---

## Ratification verdict

**RATIFY. Commit-ready: yes.**

The architect's single-pass iteration absorbed every 🔴 BLOCKER and every blocking 🟠/REVISE finding from the 5-reviewer matrix. The CHANGELOG (lines 745-757) maps each closure to a verified reviewer finding. No round-2 needed; orchestrator can commit.

---

## §2.7.1 Terminate ratification

**RATIFIES my Phase 1 solo call** (`decision_terminate_gating.md`, 2026-04-24) verbatim.

§2.7.1 lines 370-381 picks **wire-end-to-end** for both `Retry` and `Terminate`. The 4-evidence list (Phase 0 S3 false-capability, my Phase 1 solo decision, Strategy §4.3.2 symmetric-gating, observability-as-completion) is exactly the rationale chain I stipulated in memory: "do NOT ship gate-only-without-wiring; finish partial work in the same pass since engine changes are already in cascade scope."

Strategy §6.9 (line 463-465) confirms scheduler infrastructure is in cascade scope per the chosen path, satisfying my memory's "revisit when" condition (engine scheduler change exceeds one-PR scope → fallback to gate-only). It does not exceed; ratify.

`feedback_active_dev_mode` discipline preserved. Section is freeze-grade.

---

## §2.9 Input/Output REJECT ratification

**RATIFIES.**

§2.9.5 picks Option C (status quo, no consolidation). Rationale chain (§2.9.6) is structurally sound:

1. **TriggerAction divergence is structural, not stylistic** — the projection `<Self::Source as TriggerSource>::Event` and unit Output reflect that triggers are event-driven (event → effect), not parameter-driven (input → output). Forcing `Action<I, O>` parameterization on triggers requires lying or redundant projection. Verified against §2.2.3 lines 197-211 — the actual TriggerAction shape has no `type Input` / `type Output`; "input" is the projected event. Symmetry argument fails honestly here.
2. **Sub-trait pattern (Option B) has no current consumer** — §2.5 ActionHandler enum + §3 runtime go through JSON erasure (`serde_json::Value`), not typed `Input`/`Output` reflection. Adding `ExecutableAction` is speculative DX surface per `feedback_active_dev_mode`. Confirmed by reading §2.5 (lines 305-316) and §3.2 dispatch (lines 642-650).
3. **Spike validated status quo at commit `c8aef6a0`** — `final_shape_v2.rs:209-262` is non-consolidated; Probes 1-6 + Iter-2 §2.2/§2.4 PASS. Consolidation invalidates the "this compiles end-to-end" property without payback.

**Re-open trigger** (§2.9.7 lines 565-569) is concrete and load-bearing: (a) fifth primary trait sharing Stateless/Stateful/Resource I/O shape, (b) concrete consumer for typed I/O reflection. Both conditions are honest and observable; not "we'll think about it later." Matches my memory discipline on `feedback_adr_revisable` (revisit conditions named, not vague).

REJECT decision is principled.

---

## R5 ser/de bound check (rust-senior 08b 🔴)

**RESOLVED.** Iteration applied bound lifting to `StatelessAction` / `StatefulAction` / `ResourceAction` (§2.2.1 line 159, §2.2.2 line 177-178, §2.2.4 line 227-228) — all three now read:

```rust
type Input: HasSchema + DeserializeOwned + Send + 'static;
type Output: Serialize + Send + 'static;
```

§2.0 deliberate-divergence overlay #1 (line 103) explicitly notes the lift: "closes CR9 (undocumented schema bound) and resolves the 'leaky adapter ser/de invariant' rust-senior 02c §3 finding."

This matches the `decision_terminate_gating` discipline of "finish partial work in cascade" — the redesign was supposed to close the 02c §3 finding, and the iteration delivered. Rust-senior 08b's 🔴 was the only severity-1 finding from that reviewer; it's now closed. Trigger is correctly excluded from the lift (no `type Input` / `type Output` per §2.9 REJECT).

---

## R8 feature flag granularity check (devops 08e NIT 1)

**RESOLVED, but not via the option I expected.**

Iteration committed **parallel flags** `unstable-retry-scheduler` + `unstable-terminate-scheduler` (§2.7.2 line 432-433 + line 438 commitment paragraph). Devops 08e NIT 1 noted §2.7.2 was freezing a name (`unstable-action-scheduler`) whose granularity §2.7-1 said was deferred — that contradiction is now gone. §2.7-1 is closed; §15 open items reflect this (line 728: "RESOLVED at CP1 iteration").

Parallel-flag pick aligns with §2.7.2's commitment rationale (lines 438): "the two variants share gating discipline but the *names* are independently meaningful: `Retry` and `Terminate` consume distinct scheduler subsystems (re-enqueue vs sibling-branch-cancel + termination-audit), so a downstream that wants only one path can compile-time-disable the other."

This is a tighter freeze surface than rust-senior 08b's idiom-only preference for unified `unstable-action-scheduler`, but the rationale (independently-meaningful subsystems = independent compile-time-disable axes) is principled and survives the freeze test. CP3 §9 may amend internal scheduler implementation but not flag names without ADR amendment — correct §0.2 binding.

**Net:** the freeze surface is now honest. NIT 1 closed.

---

## CP2 forward-track check (§15 open items)

§15 (lines 722-741) cleanly partitions:

**Closed at this iteration:**
- §2.7-1 feature-flag granularity (parallel flags committed).

**CP3 §7 scope (signature locks deferred):**
- §2.2.3 TriggerAction cluster-mode hooks shape; §2.6 DX trait blanket-impl trait-by-trait audit; §3.1 engine `ActionRegistry::register*` host-crate path; §3.2 ActionContext API location.

**CP3 §9 scope (engine integration):**
- §2.7-2 scheduler-integration hook trait surface; §3.2-1 `ResolvedSlot` wrap point; cross-tenant `Terminate` boundary (security-lead 08c §2 forward-track).

**CP2 §4 scope (security floor lock):**
- §2.8 `redacted_display()` helper crate location; G3 floor item 2 hard-removal mechanism (security-lead 08c §1 — VETO already binding via G3); JSON depth-cap mechanism choice.

**CP2 §8 scope (testing):**
- `SchemeGuard<'a, C>` non-Clone qualified-syntax probe; cancellation-zeroize test instrumentation.

**Verdict:** dependencies are clear. CP2 unblocked on `redacted_display()` location + hard-removal mechanism + depth-cap choice; CP3 unblocked on host-crate paths + scheduler-integration hook + cross-tenant boundary. No tangled cross-references. Architect can start CP2 drafting immediately on §4–§8.

---

## Required edits (none)

No required edits before CP1 freeze. Architect's iteration pass closed every blocking finding:

| Severity | Reviewer | Finding | Iteration closure (CHANGELOG line) |
|---|---|---|---|
| 🔴 BLOCKER | spec-auditor 08a | `credential_slots()` 3-way signature divergence | §2.1.1 new subsection with `&self` (line 748) |
| 🔴 BLOCKER | spec-auditor 08a | `SlotType::ServiceCapability` payload drop | §3.1 enum re-shaped per credential Tech Spec §9.4 (line 753) |
| 🔴 BLOCKER | spec-auditor 08a | §2.6 sealed-trait blanket bound mismatch | `+ ActionSlots` added per spike (line 751) |
| 🔴 | rust-senior 08b | adapter ser/de bound asymmetry | bounds lifted onto traits (line 749) |
| 🔴 (REVISE) | dx-tester 08d R1 | `ActionSlots` undefined in §2 | §2.1.1 defined (line 748) |
| 🔴 (REVISE) | dx-tester 08d R2 | "blanket impl" doc-comment wrong | §2.1 corrected to "concrete impl per action" (line 747) |
| 🔴 (REVISE) | dx-tester 08d R3 | Community plugin migration target absent | §2.6 "Community plugin authoring path" paragraph (line 752) |
| 🟠 HIGH | spec-auditor 08a | §2.0 compile-check warrant false | three deliberate-divergence overlays (line 746) |
| 🟠 | rust-senior 08b | §2 silent on `SchemeGuard: !Clone` | §2.8 cross-ref + qualified-syntax probe (line 756) |
| NIT 1 | devops 08e | §2.7.2 frozen-name vs deferred granularity | parallel flags committed, §2.7-1 closed (line 755) |
| NIT 3 | devops 08e | §3.1 cites non-existent `crates/runtime/` | re-pinned to `nebula-engine` (line 754) |
| R7 | dx-tester 08d | `Capability` / `SlotType` missing `#[non_exhaustive]` | added per R7 (line 754) |

The remaining open items are forward-tracked correctly (CP2 §4 / CP2 §8 / CP3 §7 / CP3 §9), matching `feedback_active_dev_mode` "honest deferral with cascade-home" discipline. Security-lead 08c's ACCEPT-WITH-CONDITIONS gaps (Gap 1-5) are all CP2 / CP3 forward-pointing, not CP1 defects — confirmed at line 738-741 forward-track block.

---

## Summary

**RATIFY. Commit-ready: yes. No round-2.**

The architect's single-pass iteration delivered every 🔴 BLOCKER closure and every blocking REVISE closure. §2.7.1 wire-end-to-end ratifies my Phase 1 solo call verbatim. §2.9 Input/Output REJECT is principled — TriggerAction divergence is structural; sub-trait pattern is speculative surface; spike validates status quo. R5 ser/de bound lifting closes rust-senior's only 🔴. R8 parallel-flags commitment closes devops NIT 1 and tightens the freeze surface beyond what unified-flag would have done. §15 open items have clear CP2 / CP3 cascade-homes. CHANGELOG line 745-757 maps each closure to its source finding. Orchestrator: commit.

Handoff: orchestrator (commit + CP2 §4 architect dispatch).
