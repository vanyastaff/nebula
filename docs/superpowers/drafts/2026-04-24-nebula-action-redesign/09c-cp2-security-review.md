# CP2 security review — Tech Spec §5–§6 (nebula-action redesign)

**Reviewer:** security-lead
**Date:** 2026-04-24
**Document reviewed:** [`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`](../../specs/2026-04-24-nebula-action-tech-spec.md) — §5 (lines 932–1052) + §6 (lines 1054–1262) in scope; §6 is co-decision territory per Strategy §6.3
**Mode:** **PRIMARY REVIEWER** for §6 (co-decision: tech-lead + security-lead)
**Re-verified against current `crates/action/`:**
- `src/context.rs:635-668` — no-key `credential<S>()` heuristic still live (lines confirmed verbatim; deletion target intact).
- `src/stateful.rs:561-582` — `from_value(input.clone())` + `from_value::<A::State>(state.clone())` both depth-unbounded today (matches §6.1.1 site table).
- `src/webhook.rs:1378-1413` — `check_json_depth(&[u8], usize)` primitive exists as `fn`-private; usage at line 347 via `body_json_bounded`.
- `src/error.rs:55-71` — `ValidationReason` is `#[non_exhaustive]`; adding `DepthExceeded` is non-breaking per §6.1.3.

Cross-checked against [`08c-cp1-security-review.md`](08c-cp1-security-review.md) (5 prep gaps) and [`03c-security-lead-veto-check.md`](03c-security-lead-veto-check.md) §2 (must-have floor). Memory verified per `feedback_review_verify_claims.md`.

---

## Verdict

**ACCEPT-WITH-CONDITIONS.** §6 is freeze-grade for security with **two required edits** before CP2 ratification (none rises to VETO). All four must-have floor items (Strategy §2.12) are bound to concrete implementation forms; all five CP1 prep gaps close cleanly (Gap 5 forward-tracks к CP3 §9 as expected). §6.2 hard-removal language is unambiguous and quotes my 03c §1 VETO trigger verbatim — **VETO authority retained as written; no regression**. §5.4 qualified-syntax probe is correctly committed in qualified form per ADR-0039 §3 + spike finding #1.

The two required edits are mechanical clarifications (§6.1.2 primitive visibility + §6.3.1 stateless apply-site form), not security-substance defects.

---

## §6.1 JSON depth cap mechanism check

**Closes S-J1 + S-J2 — accepted.** Pre-scan via existing `check_json_depth` is the right call (avoids new `serde_stacker` dep surface per `feedback_boundary_erosion.md`; existing primitive already audited via webhook bounding). Three apply sites enumerated correctly:
- `stateless.rs:370` (input)
- `stateful.rs:561` (input) — verified `from_value(input.clone())` at this line today
- `stateful.rs:573` (state) — verified `from_value::<A::State>(state.clone())` at this line; **closes S-J2 simultaneously** as 03c §2 item 1 required.

Webhook boundary at `crates/api/src/services/webhook/transport.rs` confirmed pre-bounded via `body_json_bounded` per §6.1.1 note.

**Required edit §6.1-A (MEDIUM, mechanical).** `check_json_depth` is currently `fn`-private inside `webhook.rs` (verified at line 1378 — no `pub`). The §6.1.2 code sketch calls `crate::webhook::check_json_depth(...)` from `stateless.rs` / `stateful.rs`, which compiles since they share the same crate, but the function is undocumented and not part of the crate's public-surface contract. Add a one-line forward-pointer at §6.1.2: "*`check_json_depth` is promoted to `pub(crate)` visibility (was `fn`-private at `webhook.rs:1378`); CP3 §9 confirms the visibility-bump PR is part of the depth-cap landing.*" Without this, CP3 implementation drift is possible (e.g., re-implementing the primitive at the apply site rather than reusing it — defeating the "single audited primitive" rationale).

**Required edit §6.1-B (LOW, observability).** §6.1.3 spec for `ValidationReason::DepthExceeded { observed: u32, cap: u32 }` is correct, but the observed-depth value in the §6.1.2 code sketch is left as `...` placeholder. Specify: "*`check_json_depth` MUST return the observed depth on rejection (currently returns `serde_json::Error`); CP3 §9 amends the primitive's signature to `Result<(), DepthCapError>` carrying `{observed, cap}` so the typed error variant has real values, not synthesized.*" Without this, the typed error ships with `observed: 0` placeholder — defeating `feedback_observability_as_completion.md`.

S-J1 + S-J2 closure: **complete on landing both edits.**

---

## §6.2 hard-removal language check (VETO retained?)

**VETO retained — unambiguous.** §6.2.2 commits to **Option (a): hard delete** verbatim — "delete the method from `CredentialContextExt`. Old call sites get `error[E0599]: no method named credential found for type X` at compile time (not warning)." §6.2.3 quotes my 03c §1.B VETO trigger language verbatim, including the second 03c §4 handoff quote ("If tech-lead and architect converge on B' and the implementation later attempts to ship a `#[deprecated]` instead of hard-removing... I will VETO the landing."). §6.2.3 closing sentence: "*Any implementation-time deviation toward `#[deprecated]` shim form invalidates the freeze per §0.2 item 3 AND triggers security-lead implementation VETO per 03c §1.*"

**No softening detected.** Option (b) (`#[deprecated]`) is explicitly rejected. Option (c) (`credential_unsafe<S>()` rename + sunset) — the CP1 §Gap 1 "acceptable interim" — is **not** offered as a fallback in §6.2; CP2 commits to Option (a) only. This is **stronger** than CP1 §Gap 1 framing and removes the interim-fallback escape hatch. Endorsed.

**§6.2.5 companion handling** — `credential_typed::<S>(key)` retention is **security-neutral** (explicit-key, no shadow attack vector). Open item §6.2-1 leaves CP3 §9 to pick removal vs retention; security has no veto position. **No required edit.**

S-C2 / CR3 closure: **complete on hard-delete landing.** Implementation-time VETO authority on shim-form regression: **retained verbatim.**

---

## §6.3 redacted_display() rule set check

**Accepted with one required edit.** §6.3.2 commits to dedicated `nebula-redact` crate per my 08c §Gap 3 position — single audit point, layering correct (logging facade should not own redaction policy), CODEOWNER alignment. **Endorsed.**

§6.3.3 typed observability rules cover the three classes I require: (1) module-path strip (S-C3 closure), (2) credential-type pattern strip, (3) `SecretString`-bearing field replacement. The property-test invariant (`!e.to_string().contains(<actual_secret_value>)`) is the right negative-assertion form.

**Rule set sufficiency for `ActionError` Display fields.** Per Tech Spec §2.8 + verified in `crates/action/src/error.rs`: `ActionError::Validation { reason: ValidationReason, .. }`, `ActionError::Retryable { hint: RetryHintCode, .. }`, `ActionError::Fatal { source }`, etc. None of `ValidationReason::as_str()` / `RetryHintCode::as_str()` / `Classify::code()` ship secret material directly — they are stable static-str codes. **The leak vector is the variant `details: Option<String>`** (free-form context message; can include credential material if action body author misuses it). §6.3.3 invariant test catches this. **Sufficient.**

**Required edit §6.3-A (MEDIUM, scope-clarification).** §6.3.1 stateless apply-site note (line 1158-1160) says "the leak vector is the `e: serde_json::Error`'s `Display` (which can include path / value information from the offending JSON). The sanitization wraps the *outgoing error string*, not just the `tracing::error!` call." This is correct but understated: `serde_json::Error::Display` can leak **the offending JSON value verbatim** (e.g., `invalid value: string "actual_secret_token", expected bool at line 5 column 12`). Add explicit sanitization-form commitment at §6.3.1: "*All `format!("...: {e}", e = serde_json_err)` patterns at error emit sites MUST route through `redacted_display()` BEFORE the `format!` interpolation, not after — the `Display` impl is the leak surface, not the outer string. CP3 §9 confirms exact wrap-form (likely `format!("...: {}", redacted_display(&e))`).*" Without this, the wrap-form decision drifts to CP3 implementer discretion and the `Display`-pre-wrap invariant is unclear.

S-O4 + S-C3 closure: **partial at CP2** (helper crate location locked + apply-site list locked); **complete on CP3 §9 rule-set landing**.

---

## §6.4 cancellation-zeroize test check

**Accepted — closes S-C5 + 08c §Gap 4 cleanly.** §6.4.2 commits to per-test `ZeroizeProbe: Arc<AtomicUsize>` per my 08c §Gap 4 position. Test surface real:
- Three sub-tests named per §3.4 (matches spike Iter-2 §2.4 contract)
- Test location at `crates/action/tests/cancellation_zeroize.rs` (integration-test layer, not embedded in `testing.rs` public surface) — correct per `feedback_boundary_erosion.md`
- `engine_construct_with_probe` constructor variant gated `#[cfg(any(test, feature = "test-helpers"))]` — production constructor unchanged; no public-API leak from the test instrumentation

**Cross-crate amendment к credential Tech Spec §15.7 flagged** but not enacted in §6.4.2 — same precedent as §5.4.1 (forward-track to CP4 cross-section pass + credential Tech Spec author lands inline). **Acceptable.**

**One open item §6.4-1 (`tokio::time::pause()` vs real-clock 10ms)** — CP3 §9 picks; security-neutral. Recommendation in spec is `tokio::time::pause()` (deterministic timing) — endorsed but not required.

S-C5 closure: **complete on CP3 §9 landing.** Test surface is real, not theatre. Per-test instrumentation eliminates flaky-CI cross-test contamination risk I flagged in 08c §Gap 4.

---

## CP1 5 prep gaps closure status

| CP1 §Gap | CP2 §6 closure | Status |
|---|---|---|
| **Gap 1** — hard-removal mechanism | §6.2.2 Option (a) hard-delete; §6.2.3 quotes 03c §1 VETO verbatim; Option (b) rejected; Option (c) interim **not offered** | **CLOSED — stronger than CP1 framing** |
| **Gap 2** — JSON depth-cap mechanism | §6.1.2 commits to pre-scan via existing `check_json_depth` primitive; rationale documented (avoids `serde_stacker` dep surface) | **CLOSED** (modulo §6.1-A visibility edit) |
| **Gap 3** — `redacted_display()` crate location | §6.3.2 commits to dedicated `nebula-redact` crate | **CLOSED — matches my 08c position** |
| **Gap 4** — `ZeroizeProbe` instrumentation | §6.4.2 commits to per-test `Arc<AtomicUsize>` | **CLOSED — matches my 08c position** |
| **Gap 5** — cross-tenant `Terminate` boundary | §6.5 forward-tracks к CP3 §9 with the engine-side enforcement form ("`if termination_reason.tenant_id != sibling_branch.tenant_id { ignore }`") | **FORWARD-TRACKED — acceptable** (CP3 §9 is the right scope; engine-side trait surface is open at CP2) |

All five gaps close as expected. Gap 5 deferral to CP3 §9 is **acceptable** because: (1) the engine scheduler trait surface is open at CP2 by design (per §2.7-2), (2) §6.5 commits the requirement-text the CP3 author MUST land, (3) the `unstable-action-scheduler` feature gate prevents the surface from shipping before CP3 lands.

---

## §5.4 qualified-syntax probe check

**Accepted as qualified form.** §5.4 (line 1002-1040) commits to:

```rust
let _g2 = <SchemeGuard<'_, SlackToken> as Clone>::clone(guard);  // E0277 fires here
```

Per ADR-0039 §3 + spike finding #1. Mechanism: explicit trait-projection `<T as Trait>::method` skips method resolution to `Scheme::clone` (which auto-deref would resolve to). `error[E0277]: trait bound SchemeGuard<'_, SlackToken>: Clone not satisfied` fires.

**Why this matters for security.** The unqualified naive form `let g2 = guard.clone()` silently green-passes (auto-deref resolves to `Scheme::clone`, producing a `Scheme` clone — itself a secret leak since `Scheme` carries `SecretString`). The compile-fail probe **silently green-passes** on the wrong shape. Qualified syntax forces the compiler to look only at `SchemeGuard`'s `Clone` impl (which doesn't exist).

§5.4.1 soft-amendment к credential Tech Spec §16.1.1 probe #7 correctly flagged (not enacted in this Tech Spec; coordination via credential Tech Spec author per ADR-0035 amended-in-place precedent). **Endorsed.** Without this amendment, the credential-side probe at `tests/compile_fail_scheme_guard_clone.rs` would still ship the silent-pass shape — action-side §5.4 catches it independently, but the credential-side probe should match.

No required edit.

---

## Required edits (if any) + co-decision sign-off

**Two required edits before CP2 ratification (both MEDIUM/LOW; mechanical):**

1. **§6.1-A (MEDIUM)** — visibility commit for `check_json_depth`: add forward-pointer at §6.1.2 stating `pub(crate)` promotion is part of the depth-cap landing PR.
2. **§6.1-B (LOW)** — observed-depth value commit at §6.1.3: amend `check_json_depth` signature to return `{observed, cap}` (not just bool/error) so `ValidationReason::DepthExceeded { observed, cap }` ships real values per `feedback_observability_as_completion.md`.
3. **§6.3-A (MEDIUM)** — wrap-form clarification at §6.3.1: explicit pre-`format!` sanitization for `serde_json::Error` Display surfaces; CP3 §9 commits exact wrap-form.

**No required edits to §6.2** — hard-removal language is unambiguous; **VETO retained verbatim, no regression**.

**Co-decision sign-off (§6 4 items):**

| Item | Sign-off |
|---|---|
| §6.1 JSON depth cap (S-J1 / S-J2) | **YES** — pending §6.1-A + §6.1-B edits |
| §6.2 Hard-remove no-key `credential<S>()` (S-C2) | **YES** — VETO retained; no regression |
| §6.3 `ActionError` Display sanitization (S-O4 / S-C3) | **YES** — pending §6.3-A edit |
| §6.4 Cancellation-zeroize test (S-C5) | **YES** — closes 08c §Gap 4 cleanly |

**No VETO trigger fired.** §6.2 hard-removal stance is preserved verbatim from 03c §1; §0.2 item 3 freeze invariant prevents post-CP2 softening to `#[deprecated]`.

If tech-lead concurs on the three required edits: ratify CP2 with edits applied. If tech-lead disagrees on any edit, surface for orchestrator escalation per co-decider mode.

---

## Summary

ACCEPT-WITH-CONDITIONS for CP2 §5–§6. All four must-have floor items (S-J1/J2, S-C2, S-O4/C3, S-C5) bound to concrete implementation forms; all five CP1 prep gaps close (Gap 5 forward-tracks к CP3 §9 as expected); §5.4 qualified-syntax probe correctly committed per ADR-0039 §3. §6.2 hard-removal language unambiguous and quotes my 03c §1 VETO trigger verbatim — **VETO authority retained, no regression**. Three required edits (mechanical/observability): §6.1-A `pub(crate)` visibility commit for `check_json_depth`, §6.1-B observed-depth value in `DepthExceeded`, §6.3-A pre-`format!` sanitization wrap-form for `serde_json::Error` Display. None rises to VETO. Implementation-time VETO authority on §6.2 shim-form drift retained per 03c §1 + §1 G3 + §0.2 item 3.

*End of CP2 security review.*
