# CP3 §9.5 focused security review — cross-tenant Terminate boundary

**Reviewer:** security-lead
**Date:** 2026-04-24
**Document reviewed:** [`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`](../../specs/2026-04-24-nebula-action-tech-spec.md) — §9.5 only (lines 1601-1639)
**Cross-ref:** [08c CP1 §Gap 5](08c-cp1-security-review.md) line 109-111 (verbatim VETO trigger language source)
**Scope:** focused — §9.1-§9.4 + §10-§13 are out of scope per dispatch.

---

## Verdict

**ACCEPT. No required edits. No VETO.**

§9.5 closes 08c §Gap 5 cleanly. The verbatim invariant language is preserved at §9.5.1 line 1609; the silent-skip + `tracing::warn!` + counter telemetry shape (§9.5.2 step 3) is the right defense-in-depth posture (observable misbehavior, no DoS-as-feature); structural-error → Fatal discrimination (§9.5.2 step 4) preserves fail-closed for engine-internal bugs while keeping the policy boundary observable.

---

## §9.5 enforcement contract check

**§9.5.1 invariant language matches 08c §Gap 5 line 111 verbatim:**

> §9.5.1 line 1609: "`Terminate` from action A in tenant T cancels sibling branches **only within tenant T's execution scope**; engine MUST NOT propagate `Terminate` across tenant boundaries."

08c §Gap 5 line 111 reads: "`Terminate` from action A in tenant T cancels sibling branches **only within tenant T's execution scope**; engine MUST NOT propagate `Terminate` across tenant boundaries." **Identical.** No softening; "MUST NOT" preserved (not relaxed to "should not", "by default does not", or "SHOULD NOT" — RFC 2119 strength preserved).

**§9.5.5 implementation-time VETO retained verbatim** (line 1639): "any implementation-time deviation from §9.5.1's invariant language ('engine MUST NOT propagate `Terminate` across tenant boundaries') triggers security-lead VETO. The wording 'MUST NOT propagate' is normative — softening to 'should not' or 'by default does not' is a freeze invariant 3 violation per §0.2." This is exactly the trigger language I asked for; it elevates wording-softening to a §0.2 freeze-invariant breach (which requires ADR-supersede before re-ratification, not a unilateral PR).

**Architectural placement check (engine-side at scheduler dispatch):** §9.5.2 step 2 places the check at the **scheduler-integration hook**, before fanning Terminate to siblings (line 1613 "**before** fanning `Terminate` to siblings"). This is the correct placement — earlier rejected alternatives:
- **Action-side check:** would require leaking `tenant_id` into the action-author surface, violating §9.5.4 ("action authors see no tenant scope"). Rejected by §9.5.4.
- **Runtime-internal post-fan filter:** would mean the scheduler has *already enqueued* cross-tenant cancellations and is filtering them on dequeue. This creates a TOCTOU window where an in-flight cross-tenant cancellation could fire before the filter rejects it. **Engine-side pre-fan is the only TOCTOU-free placement.** §9.5.2 picks correctly.

**Sibling-cancellation containment:** the §9.5.2 step 3 enumeration loop iterates candidate siblings with `tenant_id_B` and only enqueues cancellation if `tenant_id_B == tenant_id_A`. Cross-tenant siblings are **un-reachable** through this mechanism (line 1620). The originating action's `Terminate` propagates within its own tenant scope normally; cross-tenant siblings observe no state change. **Sibling cancellation is contained.** No way for tenant T's `Terminate` to reach tenant T''s execution state.

---

## Telemetry sufficiency

**Audit trail without tenant identifier leak:** §9.5.2 step 3 second bullet (line 1619) emits:
- `tracing::warn!(tenant_id_termination_source = %tenant_id_A, tenant_id_sibling = %tenant_id_B, "cross-tenant Terminate ignored — sibling branch in different tenant scope")`
- Counter: `nebula_action_terminate_cross_tenant_blocked_total{tenant_origin, tenant_target}`

Both surfaces include `tenant_id` of source AND target. Security-relevant question: **does this leak tenant identity to a log aggregator with broader read access than the workflow engine?**

Assessment: **acceptable.** Tenant IDs in Nebula's threat model are operational identifiers (UUIDs / opaque scope tokens per the engine's existing tenant-isolation discipline) — they are NOT secrets. They're already in execution metadata, audit logs, and trace spans throughout the engine on the legitimate code path. A `tracing::warn!` at this site reveals that *tenant T attempted to terminate a sibling in tenant T'* — this is the **exact information the security ops surface needs** to detect attempted cross-tenant escalation; redacting it would defeat the audit purpose. The threat actor "log aggregator with broader read access" is concerned with **secrets** (API keys, credentials, session tokens) — not tenant identity in a misbehavior signal.

**Cardinality check on counter labels:** `tenant_origin` × `tenant_target` is O(N²) in tenant count. For a multi-tenant SaaS deployment with N=10000 tenants, the worst case is 10^8 label combinations — but this only materializes IF every tenant pair attempts a cross-tenant Terminate, which would itself be a system-wide attack signal. In practice, this counter should be near-empty (cross-tenant Terminate is by definition rare misbehavior). **Acceptable as-is** for the security audit purpose; if cardinality becomes a runtime problem post-cascade, the implementer can demote one label to a hash bucket without changing the security contract. Worth noting as a CP4 implementation note, NOT a CP3 §9.5 defect.

**Counter discoverability:** `nebula_action_terminate_cross_tenant_blocked_total` follows the project's existing metric naming convention (verified against `crates/observability` patterns in prior CP reviews). A non-zero value on this counter is a **direct security-ops alert signal**; this is exactly the per `feedback_observability_as_completion.md` "typed error + trace span + invariant check" DoD. **Sufficient.**

---

## Silent-skip vs Fatal discrimination

**The split is correct.** §9.5.2 step 4 (line 1620): "**Cross-tenant ignore is silent (not Fatal); structural errors are Fatal.**"

Discrimination logic:
- **Silent skip** is appropriate ONLY for the **legitimate cross-tenant Terminate case** — i.e., the policy boundary is operating as designed, the action body returned a valid `Terminate { reason }`, the scheduler enumerated siblings, and one of those siblings happens to be in a different tenant scope. This is **expected behavior** under the cross-tenant isolation invariant, not an error. Failing the originating action would punish the action author for the engine's policy boundary, which is unsound.
- **Fatal** is correctly retained for **structural errors** in the dispatch path: malformed `TerminationReason`, scheduler unavailable, persistence backend failure for audit log (line 1620). These are engine-internal bugs or ops-layer failures — they MUST NOT be silently skipped because that would mask scheduler malfunction or audit-log failure (which itself defeats the §9.5.3 "must NOT silently no-op without telemetry" reject reasoning).

**No bypass surface created by this split.** A malicious plugin attempting to use `Terminate` to attack tenant T' would hit the silent-skip path (legitimate cross-tenant case) — but the silent skip is **observable via `tracing::warn!` + counter**, so the attack is detectable. A malicious plugin attempting to use `Terminate` to crash the scheduler (e.g., by crafting malformed `TerminationReason`) would hit the Fatal path — and `Fatal` correctly fails their action, not silently masking the issue. **The two paths cover the two failure modes correctly; neither bypasses the other.**

**§9.5.3 reject paths verified:**
- "REJECT — silent cross-tenant cancel" (line 1626) closes the DoS-as-feature path my 08c §Gap 5 raised.
- "REJECT — silent cross-tenant no-op without telemetry" (line 1627) closes the structural-invisibility path that would defeat the security ops surface.

Both reject reasonings are accurately stated. **No fourth path I can identify** that softens the security posture without invalidating the §9.5.1 invariant.

**Action-author surface (§9.5.4):** action authors see `Ok(ActionResult::Terminate { reason })` regardless of whether sibling cancellation crossed tenant boundaries. This is correct — exposing tenant-scope to action authors would (a) leak engine-internal state to the plugin sandbox boundary, and (b) allow plugin authors to *probe* the engine's tenant scope by observing differential behavior of `Terminate`. The "action authors see no tenant scope" framing keeps the tenant-isolation invariant **engine-internal**, where the threat model places it. **Correct posture.**

---

## Required edits

**None.**

§9.5 is freeze-grade for security. The verbatim invariant language matches my 08c §Gap 5 word-for-word; the mechanism is TOCTOU-free; the telemetry covers the attempted-misbehavior detection use case without leaking secrets; the silent-skip vs Fatal split is correctly drawn between policy-boundary cases and structural-error cases.

The engine-side trait shape (`SchedulerIntegrationHook::on_terminate(&self, dispatch_ctx: &DispatchContext, reason: TerminationReason) -> Result<(), SchedulerError>` per §9.5.5) is correctly out of scope for this Tech Spec — that's the engine cascade's job. The §9.5 contract gives the engine cascade exactly the constraints it needs (cross-tenant skip MUST be silent, cross-tenant skip MUST emit telemetry, structural errors MUST be Fatal) and no more.

**Implementation-time VETO retained per §9.5.5 line 1639** — confirming the trigger language matches my 08c §Gap 5 wording. Any future PR that softens "MUST NOT" to "should not" or "by default does not" triggers VETO and forces ADR-supersede.

---

## Summary

ACCEPT. §9.5 closes 08c §Gap 5 verbatim with correct architectural placement (engine-side pre-fan, TOCTOU-free), sufficient telemetry (tracing::warn + counter at the scheduler dispatch path), and correct silent-skip vs Fatal discrimination (policy boundary silent + observable; structural errors fail closed). Implementation-time VETO retained on §9.5.1 invariant language strength per §9.5.5 + §0.2 freeze invariant 3 binding. No required edits.

VETO: **NO.**

*End of CP3 §9.5 focused security review.*
