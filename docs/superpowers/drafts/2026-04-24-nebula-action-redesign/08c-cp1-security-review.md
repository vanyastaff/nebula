# CP1 security review — Tech Spec §0–§3 (nebula-action redesign)

**Reviewer:** security-lead
**Date:** 2026-04-24
**Document reviewed:** [`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`](../../specs/2026-04-24-nebula-action-tech-spec.md) (lines 1-573, §0–§3 in scope)
**Slice:** security-relevant claims in §3.4 cancellation safety + §2.7 Terminate wiring + §1 must-have-floor acknowledgment + readiness for CP2 §6 / §4 floor lock
**Re-verified against current code:**
- `crates/action/src/context.rs:635-668` — `CredentialContextExt::credential<S>()` no-key heuristic still live (S-C2 still exploitable today; CP2 §4 must hard-remove, not `#[deprecated]`).
- `crates/credential/src/` — `SchemeGuard` not yet on disk; CP1 cites it as forward-design from credential Tech Spec §15.7. Acceptable for CP1 scope (signature-locking only).

Cross-checked against Phase 1 [`02b-security-threat-model.md`](02b-security-threat-model.md) (S-C2 / S-C4 / S-C5 / S-J1 / S-W2) and Phase 2 [`03c-security-lead-veto-check.md`](03c-security-lead-veto-check.md) §2 (must-have floor). Memory verified against current code per `feedback_review_verify_claims.md`.

---

## Verdict

**ACCEPT-WITH-CONDITIONS.** Tech Spec §0–§3 is security-coherent: the cancellation-safety contract in §3.4 is structurally sound (closes S-C5 design-side and partially neutralizes S-C4), the §2.7.1 wire-end-to-end pick removes the §4.5 false-capability surface that `Terminate` currently embodies, and §1 G3 explicitly binds all four must-have floor items to freeze invariants per §0.2 item 3. The conditions below are forward-pointing — none gates CP1 ratification, but each must land verbatim in CP2 §4 / §6 to avoid a Phase-2-veto-check violation at implementation time.

The two CP2-readiness gaps (formalized below) are the only items that would harden CP1's contract; neither needs a CP1 edit.

---

## §3.4 cancellation safety contract check

The §3.4 invariant is the **strongest** it can be at signature-locking time and is the design-side fulfillment of must-have floor item 4. Specifically:

1. **Drop-ordering invariant is correctly grounded.** §3.4 mechanism item 2 cites the lifetime-gap-refinement form from credential Tech Spec §15.7 line 3503-3516 (iter-3): engine constructs `SchemeGuard<'a, C>` via `engine_construct(scheme, &'a credential_ctx)` so `'a` cannot outlive the borrow chain. This is the correct closure of the Phase 1 S-C4 attack scenario at the design level — a credential `'a`-tied to the action context cannot be moved into a `'static` future (`tokio::spawn` requires `'static`; `&'a` borrows reject the move at compile time). **This structurally eliminates S-C4 for the `SchemeGuard` path** — a stronger result than my Phase 2 `03c §3` "deferred-but-tracked" framing assumed. Worth calling out in CP2 §4.

2. **Cancellation-zeroize test contract is precisely stated.** §3.4 line 535-540 names the three sub-tests verbatim (`scheme_guard_zeroize_on_cancellation_via_select`, `scheme_guard_zeroize_on_normal_drop`, `scheme_guard_zeroize_on_future_drop_after_partial_progress`). This is the explicit closure of S-C5 — Phase 2 §2 item 4 ("add cancellation-zeroize test") is now signature-locked at design level.

3. **Auto-deref Clone shadow caught at design time.** §3.4 mechanism item 3 names the `<SchemeGuard<'_, C> as Clone>::clone(&guard)` qualified-form requirement to defeat the auto-deref `guard.clone()` resolving to `Scheme::clone` — this is the silent-green-pass risk from spike finding #1. **Excellent catch.** This is the kind of detail that survives sign-off precisely because someone wrote it down.

4. **`tokio::select!` discipline is asserted, not just assumed.** §3.4 mechanism item 1 cites the action body's outermost `tokio::select!` as the cancellation point AND the Drop-fire point. Spike Iter-2 §2.4's three sub-tests already validated this empirically; CP1 inherits the spike result.

**Gap (forward-pointing only — for CP2 §4):** the §3.4 invariant covers the `SchemeGuard` path (the redesigned forward path). It does NOT cover the **legacy `CredentialGuard<S>`** still live at `context.rs:32-71` and reachable via `credential_typed` / `credential_by_id` / the no-key `credential<S>()`. Until CP2 §4 hard-removes the no-key method AND CP3 §9 fully migrates the typed path to `SchemeGuard<'a, C>`, S-C4 remains exploitable through `CredentialGuard::Clone` + `tokio::spawn` (`CredentialGuard: Clone` when `S: Clone` per `guard.rs:64-71`; no `'a` binding). This is acknowledged in the Tech Spec as the credential-CP6 transition boundary, but CP2 §4 floor lock should explicitly state: "S-C4 closure requires **both** §3.4 SchemeGuard wiring AND hard-removal of `CredentialContextExt::credential<S>()`; neither alone suffices."

**No required edit at CP1.** The §3.4 contract is freeze-grade; the gap is a CP2 floor-lock prompt, not a CP1 defect.

---

## §2.7 Terminate wire — new attack surface assessment

The §2.7.1 decision (wire-end-to-end for both `Retry` and `Terminate`) is **security-positive net.** Reasoning:

1. **Removes existing §4.5 false-capability violation.** Today `ActionResult::Terminate` is a public variant whose engine wiring is unimplemented (`crates/action/src/result.rs:217`). Per canon §4.5 ("public surface exists iff engine honors it end-to-end"), this is a false-capability today — a plugin author returning `Terminate` is silently ignored, and audit trails reflect a termination that did not happen. Wiring it end-to-end **closes** an active canon violation; gating-with-stub would preserve it.

2. **No new tenant-controlled input.** The new attack surface from wiring `Terminate` is the engine's scheduler consuming `ActionResult::{Retry, Terminate}` from the adapter (§2.7.2 line 386-389). The adapter's input is action-code-controlled (the action body author picks the variant and reason); tenant-controlled JSON input does NOT reach the scheduler dispatch path directly. Variant choice is plugin-author-trust, not tenant-attacker-trust. **Acceptable** under Nebula's threat model (malicious-plugin actor is already in scope; this surface does not expand the actor set).

3. **`TerminationReason` is a typed enum, not free-form string.** Per `crates/action/src/result.rs:212-218` (referenced by §2.7.2 line 388), termination reason carries a typed payload that propagates to audit log. **No injection vector** at the audit-log boundary, provided CP2 §4 floor item 3 (`ActionError` Display sanitization via `redacted_display()`) extends to `TerminationReason::Display` (or `TerminationReason` does NOT implement `Display` directly — implementer choice; this is a CP2 §6 prompt, not a CP1 defect).

4. **Feature-flag gating reduces false-capability surface.** The `unstable-action-scheduler` (or `unstable-terminate-scheduler` — granularity locked at CP3 §9 per §2.7-1) gates the variant from the public API surface during the implementation window. Plugin authors cannot return a variant that the engine does not honor end-to-end. **Symmetric** with `Retry`'s existing gate. The Phase 2 `03c §2` no-shim discipline is satisfied: this is feature-gate, not deprecated-shim.

**Gap (forward-pointing only — for CP3 §9):** §2.7-2 leaves the engine scheduler-integration hook (the dispatch path `Retry` + `Terminate` follow into the scheduler module) at "scheduler integration hook" line 397-398 with full detail deferred to CP3. The hook is named ("scheduler cancels sibling branches, propagates `TerminationReason` into audit log") but the trait surface is open. Security relevance: the hook is a **cross-action-cancellation primitive** — `Terminate` cancels sibling branches in the same execution. CP3 §9 must lock:
- Whether sibling cancellation propagates `CancellationToken` through the same channel as engine-wide cancellation (so SchemeGuard zeroize fires identically).
- Whether `TerminationReason` is sanitized at the audit-log boundary before serialization (preventing plugin-author leak via reason payload).
- Whether `Terminate` from action A in tenant T can cancel sibling branch action B in tenant T' (cross-tenant boundary check; my Phase 2 threat model did not pre-clear this because the wiring did not exist).

**No required edit at CP1.** §2.7 picks the principle correctly; the engine-side cross-tenant-cancellation question is correctly deferred to CP3 §9 where the trait shape is in scope.

---

## §1 must-have floor acknowledgment

§1 G3 line 64-71 binds all four must-have floor items by name with line-anchored citations:

| Floor item (Phase 2 03c §2) | Tech Spec §1 G3 line | Bind status |
|---|---|---|
| 1. JSON depth cap (S-J1 / CR4) | line 66 — "JSON depth bomb fix (CR4 / S-J1 — depth cap 128 at every adapter JSON boundary)" | ✅ bound |
| 2. Hard-remove no-key `credential<S>()` (S-C2 / CR3) | line 67 — "**hard removal** of `CredentialContextExt::credential<S>()` no-key heuristic, not `#[deprecated]` shim — `feedback_no_shims.md` + security-lead 03c §1 VETO" | ✅ bound — **explicitly cites my VETO trigger** |
| 3. `ActionError` Display sanitization (S-O4) | line 68 — "`ActionError` Display sanitization (route through `redacted_display()` helper)" | ✅ bound |
| 4. Cancellation-zeroize test (S-C5) | line 69 — "Cancellation-zeroize test (closes S-C5)" | ✅ bound |

§0.2 item 3 line 43 binds these as **freeze invariants**: "**Security floor change.** Any of the four invariant items in §4 (per Strategy §2.12 + §4.4) is relaxed, deferred, or has its enforcement form softened (e.g., 'hard removal' → 'deprecated shim' — `feedback_no_shims.md` violation)." This is the correct freeze-invariant binding — relaxing any item invalidates the freeze and requires an ADR-supersede before re-ratification.

**Net assessment:** ALL four must-have floor items are bound at G3, with item 2's "hard removal not `#[deprecated]`" condition cited verbatim from my Phase 2 veto check. The §0.2 freeze trigger correctly elevates "hard removal → deprecated shim" softening as a freeze-invalidator. **This is exactly the framing I asked for in 03c §4.** No required edit.

The fact that item 2's enforcement is deferred to CP2 §4 implementation is correct (CP1 is signature-locking; CP2 is floor-locking) — per the prompt's "do not VETO if 🔴 floor items are merely deferred to CP2 (they're CP2 scope by design)."

---

## CP2 §6 readiness gap

CP1 leaves the following items un-specified that CP2 §6 (or §4) will need to lock. None of these is a CP1 defect; all are correctly-deferred per §0.1 progression. Calling them out so CP2 drafting starts with the gap list visible:

### Gap 1 — Hard-removal mechanism for the no-key `credential<S>()` method (CP2 §4 must lock)

Currently live at `crates/action/src/context.rs:635-668`. The `#[action]` macro replacing `#[derive(Action)]` (G2) doesn't by itself remove the trait method. CP2 §4 must specify the **mechanism** of removal:
- **Option a (preferred):** delete the method from `CredentialContextExt`. Old call sites get `error[E0599]: no method named credential found` — compile error, not warning. Migration codemod (CP3 §9) rewrites to `ctx.resolved_scheme(&self.field)?` form.
- **Option b (rejected per `feedback_no_shims.md`):** `#[deprecated]` attribute. Compiles with warning. **Phase 2 03c §1 VETO.**
- **Option c (acceptable interim, requires sunset):** rename to `credential_unsafe<S>()` AND require `#[unsafe(action_unsafe_credential_lookup)]` opt-in flag at action-attribute time, with sunset target named in same PR. **Discouraged** but not VETO if explicit sunset.

CP2 §4 floor-lock prompt: state which option, cite the codemod path, name the migration window.

### Gap 2 — JSON depth cap implementation choice (CP2 §4 must lock)

Phase 2 03c §2 item 1 names two acceptable mechanisms: `serde_stacker::Deserializer` wrap, OR pre-scan via the existing `check_json_depth` primitive before `from_value`. CP1 §1 G3 binds the cap at "128 at every adapter JSON boundary" but does not pick the mechanism. CP2 §4 must:
- Name the mechanism (rust-senior call per 03c).
- Enumerate the adapter sites: `stateless.rs:370` (input), `stateful.rs:561-582` (input + state), and verify API webhook deserialization boundary (`crates/api/src/services/webhook/transport.rs` — confirm `body_json_bounded` is used, not raw `from_slice`).
- Add a typed error variant (`ValidationReason::DepthExceeded { observed: u32, cap: u32 }` or equivalent) per `feedback_observability_as_completion.md`.

### Gap 3 — `redacted_display()` helper crate location (CP2 §4 must lock)

§2.8 line 412 explicitly names this as a Strategy §5.1.2 open item: "Helper crate location is CP2 §4 scope (Strategy §5.1.2 open item — likely `nebula-log` or new `nebula-redact`)." CP2 §4 must pick. Security-lead position: prefer `nebula-redact` as a dedicated, reviewable surface (single audit point); `nebula-log` as a co-resident is acceptable but mixes redaction policy with logging policy. Either is acceptable; deciding **now** at CP2 §4 (not pushing to CP3) closes G3 floor item 3.

### Gap 4 — `ZeroizeProbe` instrumentation choice (CP2 §8 must lock)

§3.4 line 540 names this: "per-test `ZeroizeProbe: Arc<AtomicUsize>` (test-only constructor variant on `Scheme`) OR `serial_test::serial`." Spike used global `AtomicUsize`. Security-lead position: prefer `ZeroizeProbe` per-test instrumentation — global counters create test-coupling antipatterns and cross-test contamination on flaky CI runs. `serial_test::serial` is acceptable but slows test parallelism. Decision impacts the regression-harness for the entire cancellation-zeroize family (all three sub-tests in §3.4 line 535-538). CP2 §8 must lock.

### Gap 5 — Sibling-branch cancellation cross-tenant boundary (CP3 §9 — flagged here so CP2 §6 can pre-prompt)

§2.7-2 line 397-398 leaves the engine scheduler-integration hook open: "scheduler cancels sibling branches, propagates `TerminationReason` into audit log." Security-relevant: cross-tenant cancellation is a new attack surface introduced by the wire-end-to-end pick. CP3 §9 must explicitly state: "`Terminate` from action A in tenant T cancels sibling branches **only within tenant T's execution scope**; engine MUST NOT propagate `Terminate` across tenant boundaries." This is a tenant-isolation invariant, not a Strategy decision — CP3 §9 must lock the engine-side check (likely `if termination_reason.tenant_id != sibling_branch.tenant_id { ignore }`).

---

## Required edits (none for CP1)

CP1 §0–§3 is **clean for security**. No required edits before CP1 ratification.

The gaps in "CP2 §6 readiness" are forward-pointing prompts for CP2 / CP3 drafting, not CP1 defects.

---

## Summary

ACCEPT-WITH-CONDITIONS. Top findings: (1) §3.4 cancellation contract structurally eliminates S-C4 for the `SchemeGuard` path (stronger than Phase 2 deferred-but-tracked stance — flag for CP2 §4 to record), and (2) §2.7.1 wire-end-to-end is security-positive net but introduces sibling-branch cancellation surface CP3 §9 must scope to tenant boundaries. §1 G3 binds all four must-have floor items with §0.2 freeze-invariant elevation; "hard removal not `#[deprecated]`" is cited verbatim from my 03c VETO position. CP2 prep notes: lock no-key removal mechanism (Gap 1), depth-cap implementation choice (Gap 2), `redacted_display()` crate location (Gap 3), `ZeroizeProbe` instrumentation (Gap 4) at §4 / §8; CP3 §9 must lock cross-tenant cancellation boundary (Gap 5). Co-decider posture: parallel with spec-auditor / rust-senior / dx-tester / devops; if any peer raises a 🔴 in §0–§3, surface for orchestrator escalation.

*End of CP1 security review.*
