# CP4 Tech Spec freeze ratification (tech-lead)

**Reviewer:** tech-lead (solo decider)
**Document:** [`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`](../../specs/2026-04-24-nebula-action-tech-spec.md) — DRAFT CP4 (iterated 2026-04-25), 2700 lines
**Companion:** [`docs/adr/0039-action-macro-emission.md`](../../../adr/0039-action-macro-emission.md) — amendment-in-place verification
**Inputs:** spec-auditor [`11a-cp4-tech-spec-audit.md`](11a-cp4-tech-spec-audit.md) REVISE (3 🔴 + 3 🟠 + 3 actionable 🟡); security-lead [`11b-cp4-security-review.md`](11b-cp4-security-review.md) ACCEPT (no edits)

---

## Freeze ratification verdict (RATIFY-FREEZE / RE-ITERATE / ESCALATE)

**RATIFY-FREEZE.** All three spec-auditor 11a 🔴 BLOCKERs cleared with explicit CHANGELOG closure rows (lines 2682-2684); all three 🟠 HIGH cleared (lines 2685-2687); three actionable 🟡 (1 / 4 / 5) cleared (lines 2688-2690); three non-actionable 🟡 (2 / 3 / 6) confirmed by audit as "current form correct" (line 2691). security-lead 11b ACCEPT verbatim, no edits required. Freeze gate clean — no design-level rework, no contested calls surfaced.

---

## §15.5 ADR-0039 amendment enactment verification

ADR file edits landed verbatim on disk:

- **Status header** [`0039-action-macro-emission.md`](../../../adr/0039-action-macro-emission.md) line 4 → `proposed` and line 22 → `Proposed (amended-in-place 2026-04-25)` ✓
- **CHANGELOG entry** lines 24 (narrative naming Tech Spec CP4 §15.5 + ADR-0035 amended-in-place precedent + supersede-aware "credential Tech Spec §9.4 line 2452 authoritative ... (`Concrete { type_id }`, `ServiceCapability { capability, service }`, `CapabilityOnly { capability }`)") ✓
- **§1 SlotBinding shape rewritten** lines 49-95: `field_name` rename from `key`; capability folded into `SlotType` enum; `SlotBinding` shape three fields (`field_name`, `slot_type`, `resolve_fn`); three-variant `SlotType` enum (`Concrete { type_id }` / `ServiceCapability { capability, service }` / `CapabilityOnly { capability }`); §15.8 supersession cited at lines 86-90 with `RegistryEntry::capabilities` registry-computed authority replacing pre-CP5 `capabilities_enabled` plugin-metadata field ✓
- **§3 / §4 / §5 unchanged** per "not load-bearing" claim — verified intact at lines 108-148 ✓

Substantively correct: shape preservation across §9.4 → §15.8 supersession (matching axes verbatim per credential Tech Spec line 3522); only filter authority source shifts. The §15.5.1 enactment record names this orthogonality explicitly at line 2316 (CP4 supersession-acknowledgement paragraph).

---

## §16.1 (a/b/c) framing check

Tech Spec presents three paths; does NOT pre-pick. Verified:

- §16.1 line 2390 verbatim anchor: "**This Tech Spec presents the table; user picks at Phase 8 cascade summary.** Per §1.2 N5 — Tech Spec does NOT pre-pick."
- Path naming at lines 2384-2386 matches Strategy §4.2 row labels verbatim (`(a) Single coordinated PR` / `(b) Sibling cascades` / `(c) Phased B'+ surface commitment`)
- Path (c) viability gate: explicit silent-degradation guard at line 2386 ("VIABILITY GATE — per Strategy §6.6 (line 416-426) silent-degradation guard"); architect+tech-lead co-decision required per Strategy §6.8; orchestrator does NOT silently activate
- Cascade-final criteria column extends Strategy table additively, not contradictorily

Phase 8 user-pick mechanic preserved.

---

## §16.3 DoD must-have floor check

7-item DoD checklist at lines 2412-2422. Item 2 enumerates all four security floor items as **cascade-landing-PR obligations** (not follow-ups):

1. CR4 / S-J1 JSON depth bomb — depth cap 128 + typed `ValidationReason::DepthExceeded`
2. CR3 / S-C2 hard removal of `CredentialContextExt::credential<S>()` (NOT `#[deprecated]` shim — `feedback_no_shims.md` + 03c §1 VETO retained inline)
3. `ActionError` Display sanitization via `redacted_display(&e)` wrap
4. Cancellation-zeroize test — three sub-tests + per-test `Arc<AtomicUsize>` probe

security-lead 11b ACCEPT verbatim cross-checks 1:1 against my CP2 §6 sign-off (09c lines 125-133). VETO authority retained across three independent anchors (§0.2 invariant 3 / §6.2.3 / §16.3 item 2). No silent deferral. No double-counting hazard with item 1's umbrella.

---

## §16.4 rollback feature-flag correction

§16.4 Layer 1 line 2430 verbatim verified: "**Layer 1 — Feature-flag gate path (symmetric `unstable-retry-scheduler` + `unstable-terminate-scheduler`).**" Body text and parenthetical now agree. Aligns with §0.2 invariant 4 freeze on parallel-flag signature per CP1 §2.7.2 line 438.

CHANGELOG line 2684 explicitly records the parenthetical correction from the superseded `unstable-action-scheduler` form. The CP1-locked text at line 372 (the "OR keep separate" open-question framing inside §2.7.1) is preserved unchanged per CP1-locked discipline; the resolution is recorded immediately below at line 438. CP1 closed text not touched — correct procedure.

---

## File path / supersession citation pin-fixes

**File path (spec-auditor 11a 🔴 #1):** Verified `01b-workspace-audit.md` references at §14.5 lines 2185, 2187, 2189-2192; §13.4.1 line 2018; §13.4.2 line 2032; §13.4.3 line 2040; §16.2 line 2405. The single retained `01-current-state.md` reference at §14.5 line 2188 is the source-tier C1 finding ("derives structurally cannot do field-type rewriting") which actually grounds in the source-tier audit, not the workspace-tier — correct retention.

**Supersession citation (spec-auditor 11a 🔴 #2):**
- §14.3 row 7 + row 8 (lines 2152-2153) re-pinned to "§15.8 (CP5 supersession of §9.4)" with shape-preservation note + capability authority shift naming (`capabilities_enabled` → `RegistryEntry::capabilities`) ✓
- §15.5 supersession-acknowledgement paragraph at line 2316 names shape-preservation orthogonality + §15.5.1 enactment-bullet citation updated ✓
- §3.1 SlotType `ServiceCapability` doc comment (lines 624-631) re-pinned: cites §15.8 + `RegistryEntry::capabilities`; explicitly names pre-CP5 `capabilities_enabled` as REMOVED ✓
- ADR-0039 line 86-90 doc comment carries the same correction (cited credential Tech Spec §15.8 `RegistryEntry::capabilities` + supersession-of-§9.4 narrative; pre-CP5 `capabilities_enabled` flagged REMOVED) ✓

All three justification axes (Tech Spec §14.3 + §15.5 + ADR-0039 doc comment) re-pinned in coordinated edit. Substantive amendment correctness preserved (shape orthogonal to authority source).

---

## Cross-CP integrity sweep

Full §0-§16 sweep (sampled at section boundaries + risk surfaces):

- **Status table (§0.1)** + frontmatter agree: DRAFT CP4 (iterated 2026-04-25), CP1/CP2/CP3 marked locked CPN ✓
- **§0.2 invariants** 1-4 unchanged from CP1 lock; §15.5 ENACTED amendment-in-place fits invariant 2 mechanism (recorded in §15.5.1 enactment record) ✓
- **§2 signature-locking** unchanged from CP3 lock (deliberate-divergence overlays preserved); §3.1 SlotType doc comment edits do not touch signatures ✓
- **§6 security floor** unchanged from CP2 lock; §16.3 item 2 surfaces same four items at cascade-landing-PR obligation tier (not duplicated; not contradictorily relaxed) ✓
- **§9.5 cross-tenant Terminate** unchanged from CP3 lock; §15.1 closure row line 2253 + security-lead 11b cross-section consistency check ✓
- **§13.4 four hygiene Ts** consistent across §15.7 ratification table + §16.3 item 6/7 + §14.5 disposition table ✓
- **§14.3 row 7/8 supersession** pin-fix coordinated across §15.5 + §3.1 + ADR-0039 doc comment ✓
- **§16 phase numbering anchor** added at line 2376 per spec-auditor 🟡 #5 ✓
- **CHANGELOG completeness** CP1 + CP2 + CP3 + CP4 all present with explicit Open-items / Forward-track / Handoff sections; CP4 iteration append (lines 2680-2692) records all 3 🔴 + 3 🟠 + 3 actionable 🟡 closures ✓

No residual contradictions across §0-§16 detected.

---

## Freeze decision

**RATIFY-FREEZE.** Tech Spec internally consistent. All three 🔴 BLOCKERs cleared mechanically (no design rework). security-lead 11b ACCEPT verbatim. No contested calls. No co-decision routing required. ADR-0039 amendment-in-place ENACTED on disk. §16.3 DoD must-have floor binding as cascade-landing-PR obligation. §16.1 (a/b/c) framing presents-not-prepicks (Phase 8 user pick preserved). §16.4 rollback feature-flag names corrected to parallel form per §0.2 invariant 4 freeze.

After this ratification, status flips DRAFT CP4 (iterated 2026-04-25) → **FROZEN CP4 2026-04-25**; ADR-0038 / 0037 / 0038 status moves `proposed` → `accepted`; ADR-0039 retains `proposed (amended-in-place 2026-04-25)` qualifier per §15.5.1 record. Phase 6 closes.

**Escalation flag: NONE.**

---

## Summary

RATIFY-FREEZE — freeze yes. Tech Spec post-iteration is internally consistent and freeze-ready. (1) §15.5 ADR-0039 §1 SlotBinding amendment-in-place enacted on disk verbatim with shape-preserving supersession acknowledgement (§9.4 → §15.8 orthogonal to capability authority source); (2) §16.1 paths a/b/c presented per Strategy §4.2 framing without pre-picking — Phase 8 user pick preserved; (3) §16.3 DoD item 2 enumerates all four security must-have floor items as cascade-landing-PR obligations with security-lead 11b ACCEPT; (4) §16.4 Layer 1 parallel feature-flag names corrected per §0.2 invariant 4 freeze on parallel-flag signature; (5) file path + supersession citation pin-fixes coordinated across §14.5 / §13.4.x / §14.3 / §15.5 / §3.1 / ADR-0039 doc comment. No design-level rework. No contested calls. **Freeze gate: GREEN.** Escalation: NO.

*End of CP4 freeze ratification (tech-lead).*
