# Phase 2 — Security veto check on scope options

**Date:** 2026-04-24
**Agent:** security-lead
**Mode:** co-decision (parallel with architect's option proposal + tech-lead's priority call)
**Inputs:**
- [`02-pain-enumeration.md`](./02-pain-enumeration.md) §5 (A'/B'/C' framing — architect's `03a-architect-scope-options.md` not yet on disk at time of veto check; applying check to Phase 1 framing per orchestrator instruction)
- [`02b-security-threat-model.md`](./02b-security-threat-model.md) (Phase 1 self-report; used as findings checklist)

**Re-verified against current `crates/action/src/`:**
- S-J1 confirmed at `stateless.rs:370` — `serde_json::from_value(input)` still depth-unbounded.
- S-C2 confirmed at `context.rs:643-645` — `type_name::<S>()` → `rsplit("::")` → `to_lowercase()` heuristic still load-bearing for `credential<S>()` resolution.

---

## 0. Veto criteria (explicit thresholds)

The bar is fixed before reading options to keep this check honest:

- **VETO** — option does not structurally eliminate **S-C2** (type-name shadow attack) **OR** does not fix **S-J1** (JSON depth bomb at adapter boundary). Both are exploitable today against current `main`.
- **VETO** — option silently drops a 🔴 finding without explicit written sunset commitment (concrete next-cascade name + tracking issue).
- **ACCEPT-WITH-CONDITIONS** — 🔴 findings addressed in scope but ≥1 🟠 security finding (S-W2 / S-C4 / S-J2 / S-O1 / S-I2) deferred without a sunset plan.
- **ACCEPT** — all 🔴 findings structurally eliminated **AND** documented sunset commits exist for any deferred 🟠 finding **AND** must-have hardening §2 is in scope.

Frame: Nebula §4.2 (safety invariants — credential confidentiality, no plaintext leak path) + §12.5 (encryption-at-rest, secret-type wrapper, no `Debug`/`Display` leak) are the load-bearing canon clauses. An option that strands either is rejected regardless of cost.

---

## 1. Per-option veto verdict

Applied to A'/B'/C' from Phase 1 §5.

### Option A' — Co-landed cascade (action + credential CP6 implementation)

**🔴 findings addressed:**
- **S-C2** — eliminated structurally. CP6 vocabulary keys credentials via `CredentialRef<C>` with const `C::KEY` and `SchemeFactory<C>` dispatch — no `type_name::<S>()` heuristic anywhere. Cross-plugin shadow attack class is removed by construction (locale-independent, compiler-version-independent, namespace-explicit).
- **S-J1** — depth cap MUST be added at `StatelessActionAdapter::execute` regardless of option chosen (must-have §2 below); A' must not skip it.
- **CR1 / CR3 / CR5 / CR6 / CR7 / CR10** — credential integration entirely re-shaped; the broken paths cease to exist.

**🔴 findings deferred:** none. Macro-emission CRs (CR2 / CR8 / CR9) are not security-critical but are co-resolved.

**🟠 findings addressed:**
- **S-C6** (locale-dependent `to_lowercase`) — eliminated by removing the heuristic.
- **S-C3** (`type_name` module-path leak in error) — eliminated; explicit-key error surface does not embed `type_name`.
- **S-C1** (`CredentialGuard::Clone` extra zeroize point) — addressable via `CredentialRef<C>` non-`Clone` shape if architect picks that direction.

**🟠 findings deferred (acceptable with sunset):**
- **S-W2** (`SignaturePolicy::Custom(Arc<dyn Fn>)`) — supply-chain trust delegation; survives unchanged. Acceptable to defer if §3 sunset is recorded.
- **S-C4** (detached `tokio::spawn` defeats zeroize) — survives unchanged unless `CredentialRef<C>` is keyed to a context lifetime. Acceptable to defer.
- **S-J2** (stateful state-deserialization depth bomb) — must be co-fixed with S-J1 since same primitive (must-have §2).
- **S-O1** (output size cap) / **S-I2** (`CapabilityGated` documented-false-capability) / **S-O2 / S-O3 / S-O4** — defense-in-depth; deferral acceptable with sunset note.

**Verdict:** **ACCEPT.** A' is the security-optimal option. Both 🔴s structurally eliminated. Largest attack surface reduction.

**Conditions (none required for security veto)**: A' is otherwise security-acceptable. Cost is a tech-lead concern, not security's.

---

### Option B' — Action-scoped bug-fix + hardening (defer CP6)

**🔴 findings addressed:**
- **S-J1** — addressed by must-have §2 (depth cap at adapter). Same fix as in A'.
- **S-C2** — addressed *if and only if* CR3 fix is "deprecate `credential<S>()` no-key variant; require explicit key" (Phase 1 §5 wording). Critical: the deprecation must be **enforced at type level** (compile error or method removal), **NOT** a `#[deprecated]` attribute that lets old code keep compiling. A `#[deprecated]` warning is NOT structural elimination — the attack vector still ships.
- **CR2 / CR5 / CR6 / CR8 / CR9 / CR10 / CR11** — addressed.

**🔴 findings deferred:** none (assuming CR3 deprecation is hard removal, not soft warning).

**🟠 findings addressed:**
- **S-C6 / S-C3** — eliminated as collateral of removing the heuristic.
- Macro test harness (CR11 / `feedback_idiom_currency`) closes the macro-bug regression vector.

**🟠 findings deferred (acceptable with sunset):**
- **S-W2 / S-C4 / S-J2 / S-O1–O4 / S-I2** — same as A'.

**Verdict:** **ACCEPT-WITH-CONDITIONS.**

**Conditions (must be in cascade scope, not deferred):**
1. **CR3 fix MUST be hard removal of the no-key `credential<S>()` method**, not `#[deprecated]`. A soft-deprecated method that still compiles still ships the shadow attack — VETO if the conditions aren't met.
2. CR3 fix MUST land with the same release as the macro fixes (CR2/CR5/CR6) — partial landing leaves the heuristic alive while authors are migrating.
3. Sunset commit for CP6 adoption: explicit named follow-up cascade (e.g., "credential-CP6-cascade" with Phase 0 dispatch criteria) referenced in the cascade exit doc.

If conditions met: B' is security-equivalent to A' for the 🔴 layer. Tech-lead may pick on cost/scope grounds; security is neutral.

---

### Option C' — Escalate credential spec revision

**🔴 findings addressed:**
- **S-J1** — addressed by must-have §2.
- **S-C2** — addressed *only* if the revised spec preserves the explicit-key invariant. A revised spec that retains type-name heuristic for "ergonomic derive" would re-introduce the attack. **VETO TRIGGER** if the spec revision proposal removes explicit-key dispatch in favor of derive-magic.

**🔴 findings deferred:** depends on revision shape — unknown until revision lands.

**🟠 findings addressed:** depends on revision shape.

**Verdict:** **ACCEPT-WITH-CONDITIONS.**

**Conditions:**
1. Revised credential spec MUST retain explicit-key dispatch. If the revision proposes derive-magic that re-introduces a name-based key resolution, security VETOes the revision.
2. Same depth-cap and sunset commits as B'.
3. Spec revision review must include security-lead sign-off before adoption (escalation rule 10 territory).

C' is a wildcard — its security profile depends on what the revised spec says. Acceptable conditional on review at revision time.

---

## 2. Must-have hardening items (regardless of option picked)

These are **floor**: ANY option must include them. Dropping any is a VETO.

1. **Fix S-J1 / CR4 (JSON depth bomb).** Apply a depth cap (recommended: 128, matching `webhook.rs:1378-1413`'s `check_json_depth` guard) at all of:
   - `StatelessActionAdapter::execute` `from_value(input)` — `crates/action/src/stateless.rs:370`.
   - `StatefulActionAdapter::execute` `from_value(input)` AND `from_value(state)` — `crates/action/src/stateful.rs:561-582` (closes S-J2 simultaneously).
   - API webhook body deserialization boundary if it does not already pre-bound (verify `crates/api/src/services/webhook/transport.rs` reuses `body_json_bounded` not raw `from_slice`).
   Implementation options: `serde_stacker::Deserializer` wrap, or pre-scan via the existing `check_json_depth` primitive before `from_value`. Either is acceptable; choice is a rust-senior call.

2. **Replace S-C2 type-name heuristic with explicit keyed dispatch.** The credential resolution method signature must require the key to be supplied by the caller (either as `&str` literal, const associated constant `C::KEY`, or method-name parameter) — never derived from `std::any::type_name`. The fix is **method signature surgery**, not a runtime check. A version of `credential<S>()` that derives the key inside the method body is the attack surface; eliminate that method or change its signature so no key derivation happens.

3. **Sanitize `ActionError` Display path** in `stateful.rs:609-615` and `stateless.rs:382` — at minimum, route through a `redacted_display()` helper before `tracing::error!(action_error = %e)`. This closes S-O4 partially and supports §12.5 "no secrets in error / log strings". Low-cost; should ship in any option.

4. **Add cancellation-zeroize test** (closes S-C5). Test that a `CredentialGuard` held by a cancelled future drops + zeroizes. Pure test addition — no architectural cost.

These four are non-negotiable. If a final scope picks an option but skips any of items 1–4, security VETOes that scope.

---

## 3. Deferred-but-tracked items

Acceptable to ship in a follow-up cascade with named tracking (e.g., issue + sunset date in `02-pain-enumeration.md` outputs):

- **S-W2** (`SignaturePolicy::Custom(Arc<dyn Fn>)` audit-trail defeat) — Phase 2+ design note: constrain `Custom` to composed primitives, or attest closure-source via name/sha. **Sunset target**: webhook hardening cascade within 2 release cycles.
- **S-C4** (detached spawn defeats zeroize) — needs `!Send`/`!Sync` or context-keyed lifetime on `CredentialGuard`. **Sunset target**: credential-keyed-lifetime cascade (likely co-resident with credential CP6 work if A' lands; standalone if B' lands).
- **S-O1** (output size cap) — adapter-level ceiling on `BinaryStorage::Inline` / `BufferConfig::capacity`. **Sunset target**: output-pipeline hardening cascade.
- **S-O2 / S-O3** — `BufferConfig::capacity` ceiling, callback URL/token validation. Defense-in-depth; same cascade as S-O1.
- **S-I2** — `CapabilityGated` documented-false-capability. Either hide variant until enforcement lands, or emit WARN at engine dispatch. **Sunset target**: sandbox phase-1 cascade.
- **S-W1** — `FUTURE_SKEW_SECS = 60` configurability. Low priority.
- **S-W3 / S-F1 / S-I1 / S-U1 / S-C1** — minor; track in cascade exit notes.

---

## 4. Handoff posture

**Posture: ACCEPT all three options conditionally — co-decision can converge on any.**

- **A'**: ACCEPT (clean). Security-optimal.
- **B'**: ACCEPT-WITH-CONDITIONS — CR3 fix MUST be hard removal (not soft `#[deprecated]`). If tech-lead picks B' and the implementation phase tries to land a `#[deprecated]` shim instead, security VETOes that landing.
- **C'**: ACCEPT-WITH-CONDITIONS — revision must preserve explicit-key dispatch; security-lead sign-off required at revision time.

**No option is VETO-blocked at scope-decision time.** Tech-lead's priority call may converge on cost/budget grounds. Security's must-have §2 floor (4 items) applies to whatever option is picked.

If tech-lead and architect converge on B' and the implementation later attempts to ship a `#[deprecated]` instead of hard-removing the no-key `credential<S>()` method: I will VETO the landing. Flag this to orchestrator now so it's not a Phase 4 surprise.

**Handoff: tech-lead** — your priority call is unblocked from the security side. Any of A'/B'/C' is acceptable provided must-have §2 ships and (for B') CR3 is hard removal.

**Handoff: architect** — when you draft `03a-architect-scope-options.md` (if not already in flight), please cite must-have §2 explicitly as a non-deferrable floor in each option's scope description, so tech-lead's priority call sees them as in-scope by default.

---

*End of Phase 2 security veto check.*
