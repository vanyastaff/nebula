# Tech Spec CP3 — Tech-Lead Ratification

**Date:** 2026-04-24
**Reviewer:** tech-lead (subagent dispatch, consensus mode — dx-tester reviewing §11 in parallel)
**Reviewing:** [`docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md`](../../specs/2026-04-24-nebula-resource-tech-spec.md) §9-§13 (CP3 draft, 2026-04-24)
**Inputs:** CP2 ratification (`phase-6-cp2-tech-lead-ratification.md`); CP1 ratification (`phase-6-cp1-tech-lead-ratification.md`); Strategy §4.3-§4.8 (frozen CP3); Phase 2 amendments

---

## Verdict

**RATIFY_WITH_EDITS** — three bounded edits, none directional. All four architect-flagged decisions confirm without amendment. CP3 round 1 lock viable; convergence with dx-tester expected on §10.2/§11 axis (both surfaces strain the same DX budget — co-decision risk concentrated there).

CP3 §9-§13 derives every cut from CP1/CP2 locks and Strategy §4 commitments without re-litigation. Function-level submodule assignments verified against current `manager.rs` (2101 LOC; spot-checked lines 269, 347, 561, 752, 1138, 1259, 1295, 1360, 1430, 1458, 1636 — all match the §9 tables exactly). Engine path verified (engine `src/` lacks both `daemon/` and `scheduler/`; `runtime/` exists but houses execution-runtime shapes per §12.1 rejection rationale).

---

## Architect-flagged decisions — four CONFIRMs

### 1. §10.2 dual-helper register_* (10 public methods) — **CONFIRM**

10-method API surface accepted. Three reasons in §10.2 are load-bearing and concrete:

- **Migration parity is dispositive.** All 5 in-tree consumers register with implicit `R::Auth = ()`. Atomic per-Strategy §4.8 means renaming the bound, not the call site. Forcing `RegisterOptions` everywhere desyncs ~60% of registrations (unauthenticated caches/local services) into mandatory builder boilerplate.
- **Compile-time enforcement beats runtime check.** `register_pooled<R: Pooled<Credential = NoCredential>>` rejects credential-bearing `R` at compile time. A unified `RegisterOptions::credential_id == None` runtime check catches mismatches at next register call, not at call-site type-resolution. Type-level is unfaultable per CP2 §5.2 discipline.
- **Mechanical thin wrappers.** Each `register_*_with` is ~30 LOC; current `register_pooled` at [`manager.rs:404`](../../../crates/resource/src/manager.rs) is 26 LOC. All 10 funnel through `register_inner` (§3.1) — single dispatch logic. Doc-comment cost amortizes after the first.

The 2am test: 10 methods do not wake anyone up. `Manager`'s public surface already has 35 methods; adding 5 (the `_with` variants — the `register_pooled` family already exists) brings it to 40. The new-hire test: pattern is mechanical and self-documenting after one example. The next-month test: a unified-only API would require boilerplate-suppressing in every consumer for 60% of registrations *forever*; the dual pattern is one-time write, zero-time read.

Phase 2 priority-call discipline (`feedback_hard_breaking_changes.md` + "split file, keep type") concurs: the redesign is allowed hard breaking changes; what it isn't allowed is *boilerplate-cost compounding*. 10 methods is the boilerplate-cost-zero option. **Locked.**

Trade-off accepted on the public-surface budget. Open item §10.2 (CP3 deferrals: "if DX feedback flags 10 as over-budget...") is the right escape hatch — dx-tester's parallel review is the gate; if dx-tester pushes back with concrete DX evidence (newcomer confusion in §11.2 walkthrough, surface clutter in rustdoc), we co-decide. Default: lock dual.

### 2. §10.5 `tainting_policy` deferred — **CONFIRM**

CP2 SL-1 deferral correctly reaffirmed. Both gates remain unmet:

- **Gate 1.** No in-tree consumer with multi-tenant pool sharing one resource across N credentials. Phase 1 enumeration found none; spike found none; CP1-CP2 review found none. The §5.3 line 1252 trade-off ("zero in-tree consumers fit the multi-tenant exception") still accurate at CP3.
- **Gate 2.** No security-review hook in CP3 surface review wave. Security-lead's work closed at CP2 ratification per the convergent-review pattern. Adding `tainting_policy` to CP3 would necessitate threading a fresh security review, which CP3 cadence does not include.

Per `feedback_no_shims.md` + CP2 §5.6 deferral commitment: ship unconditional taint (§5.3 option (b)) only. `#[non_exhaustive]` on `RegisterOptions` (§10.4) preserves additive future field landing without breaking struct-literal patterns. **Locked.**

The next-month test: a knob added now would either be unused (zero in-tree consumers) or land without security-review (skipping the gate). Both paths are bad — defer.

### 3. §12.1 `crates/engine/src/daemon/` — **CONFIRM**

Verified against current engine layout:

- `crates/engine/src/runtime/` exists (`blob.rs`, `queue.rs`, `registry.rs`, `runtime.rs`, etc.) — execution-runtime shapes. Daemon as a subpath here would dilute the runtime module's purpose, as §12.1 argues.
- `crates/engine/src/scheduler/` does NOT exist. Pre-naming a module after a future cascade (Strategy §6.5) that may never fire is speculative. `daemon/` is honest about today's contents.
- No existing engine module owns long-running worker primitives. `daemon/` as new top-level cleanly carves the boundary.

If the future `nebula-scheduler` cascade fires (Strategy §6.5 trigger), rename `daemon/` → `scheduler/` is a single mechanical PR per `feedback_no_shims.md` (no shim, no compat alias, replace-the-wrong-thing-directly). The path choice today does not foreclose the rename. **Locked.**

The boundary-erosion test (`feedback_boundary_erosion.md`): engine takes ownership of Daemon/EventSource per ADR-0037 amended-in-place gate text. Co-locating in a NEW dedicated `daemon/` module — not folding into existing `runtime/` — preserves the boundary as a *decision*, not a *convenience cut*.

### 4. §9.7 `gate.rs` rename (was `execute.rs`) — **CONFIRM rename**

The dominant content of the file is the gate-admission state machine (`enum GateAdmission`, `fn admit_through_gate`, `fn settle_gate_admission`); `execute_with_resilience` is the consumer of `gate_admission`. CP3 §9.7's "reads more honestly" framing is correct. Per CP2 §0.3 freeze policy, file-structure was CP2 territory; *function placement within files* is CP3 territory — the rename is not re-opening a CP2 lock, it's a CP3 refinement. The CP2-to-CP3 file-name adjustment is permitted.

Pushback option rejected. `execute.rs` would prioritize one of three contents (the resilience wrapper) over the dominant shape (gate state machine + execute helpers). The new-hire test favors `gate.rs`: a contributor opening `gate.rs` immediately understands "this is the gate-admission seam"; opening `execute.rs` invites the question "execute what?" **Locked.**

Note: `wait_for_drain` placement also moved (§9.7 line 1702 → §9.6) — that's a separate CP3 refinement of the CP2 cut, also permitted, also correct (drain-tracker access is shutdown-side, not gate-side).

---

## §9-§13 priority items — all RATIFY

**§9 cuts (function-level).** Verified. `manager.rs` is 2101 LOC at HEAD; spot-checked spec line citations (12 of them) all match actual file. Submodule assignments trace existing internal seams per Strategy §4.5. `lookup` co-located with `register_inner` because both touch `registry` field directly — that reasoning is correct (moving `lookup` to `dispatch.rs` would require `pub(crate)` registry access and violate §5.4 discipline). `ResourceDispatcher` `pub` + `#[doc(hidden)]` re-export at `__internal::` is the right walk-back from CP2 §5.4's `pub(crate)` for production-test access without polluting the public surface.

**§10 RegisterOptions final shape.** CP1 Q4 `credential_rotation_timeout` field present (§10.4 line 1875). CP3 §10.4 adds `credential_id: Option<CredentialId>` per §3.1 and the four builder methods (`with_credential_id`, `with_credential_rotation_timeout`, `with_scope`, `with_resilience`, `with_recovery_gate`) consistent with `ShutdownConfig` builder pattern. `#[non_exhaustive]` preserves additive evolution per §13.2. The §3.3 default 30s + per-resource override semantics line up.

**§11 adapter contract spec.** Defer DX-axis ratification to dx-tester. From the high-level shape: 8 subsections cover imports/minimum-impl/topology-guide/NoCredential-opt-out/credential-bearing-walkthrough/override-pattern/testing/pitfalls — that map covers the lifecycle a newcomer needs. Compile-clean acceptance gate (`cargo doc --all --no-deps` + `cargo test --doc`) is the right enforcement per `feedback_idiom_currency.md`. The `MockPostgresPool` walkthrough avoiding third-party driver deps is the right design choice for in-tree compile validation. **High-level shape RATIFY.**

**§12 per-consumer migration.** Five consumers covered. `crates/action/`, `crates/sdk/`, `crates/plugin/`, `crates/sandbox/` are all no-op (Phase 1 + ADR-0037 evidence). `crates/engine/` is the implementation site. Verification methodology (`rg "Daemon|EventSource" crates/{action,sdk,plugin,sandbox}/src/`) baked into the migration PR is correct re-verification discipline.

**§13 evolution policy.** Versioning posture (`frontier` → `core` post-soak) tracks `docs/MATURITY.md`. §13.2 `feedback_no_shims.md` discipline correctly cited. §13.5 freeze cadence (in-cascade → post-cascade soak FROZEN → post-soak `frontier` → post-`core` deprecation cycle) matches Strategy §6.3-§6.4. The `core` bump as separate PR per MATURITY.md review cadence is the right mechanism — not folded into migration-PR scope.

---

## Required edits (3, bounded)

**Edit E1 — §9.5 ResourceDispatcher visibility framing (line 1660).** The table cell says `pub` (with `#[doc(hidden)]` re-export) but the next-row `Notes` comment says "Internally `pub(crate)` semantically, but exposed via `crate::__internal::ResourceDispatcher` for test access." The framing is correct but reads two-faced. Fix: change the visibility column to `pub(crate)` with `#[doc(hidden)] pub use` and adjust the line 1671 paragraph to say "the trait body is `pub(crate)`; `#[doc(hidden)] pub use manager::ResourceDispatcher as __internal_ResourceDispatcher` in `lib.rs` exposes it for production test access without listing it on the public-API documentation." This matches the §10.1 line 1782-1783 export shape (`pub use manager::ResourceDispatcher as __internal_ResourceDispatcher;`) and avoids the "internally pub(crate)" oxymoron. Spec-hygiene only.

**Edit E2 — §9.1 `wait_for_drain` placement gloss (table line 1574 vs §9.6 line 1685).** Table at §9.1 line 1574 lists `cancel_token` (current line 1659) under `manager/mod.rs`. That's correct — but the §9.7 line 1702 explanation that `wait_for_drain` "moved per §9.6 because semantically it is a shutdown helper" needs cross-pointed from §9.6 line 1685 too (currently §9.6 says `wait_for_drain` is at `manager.rs:1592`, but doesn't mention this is a CP3 refinement of CP2 §5.4). Fix: add one sentence to §9.6 line 1685: "CP3 refinement: CP2 §5.4 placed `wait_for_drain` in `manager/execute.rs`; CP3 §9 moves to `shutdown.rs` because the drain-tracker access pattern co-locates here, not with gate-admission." Closes the cross-section consistency for spec-auditor's eventual structural pass.

**Edit E3 — §10.2 line 1817 final-paragraph framing.** "`register<R: Resource>` (the type-erased low-level method at [`manager.rs:347`](...)) is preserved for callers that need to register a non-topology-specialised resource — it accepts `TopologyRuntime<R>` directly." This reads as if `register` is *additional* to the 10. It IS — but the framing reads ambiguous to a reviewer counting public methods (is the count 10 or 11?). Fix: clarify "the 10 helpers above are convenience over the lower-level `register<R: Resource>(TopologyRuntime<R>, ...)` (preserved). Total `register*` surface: 11 public methods (10 helpers + 1 low-level)." Closes the ambiguity preemptively before spec-auditor flags it.

---

## Convergence

**CP3 round 1 lock — high probability (~80%).**

The four architect-flagged decisions all confirm without amendment. The three required edits are spec-hygiene (visibility framing, cross-section gloss, method-count clarification) — all bounded, none directional. CP3 §9-§13 is materially correct.

**Round-2 trigger conditions** (narrow, single-axis):

1. **dx-tester pushes back on §10.2 dual-helpers.** Most plausible — DX axis is exactly where the 10-method surface strains. If dx-tester surfaces concrete newcomer-confusion evidence (e.g., the `MockPostgresPool` walkthrough's `register_pooled` vs `register_pooled_with` choice creates a documentation fork that confuses), we co-decide. My position holds: lock dual; mitigations exist (clearer docstring on each variant, table in `adapters.md` mapping bound → method). If dx-tester proposes unified-only, orchestrator escalates per consensus protocol.

2. **dx-tester surfaces §11 walkthrough gap.** §11.2 (`MockPostgresPool` minimum impl) might miss `HasSchema` derivation or `ResourceConfig::validate` shape — that would be a §11 amendment, not a §9-§13 directional change. CP3 lock unaffected; §11 amendment lands in same wave.

3. **Spec-auditor (post-convergence) catches forward-ref drift.** §13 forward-refs to CP4 §15 + CP4 §16; spec-auditor verifies these land. Not a tech-lead axis; flagged for the audit pass.

If round 2 happens it's most likely on §10.2 axis only. Other axes (engine path, file rename, `tainting_policy` deferral, function-level cuts) are settled.

**Handoffs.**

- **dx-tester** (parallel co-decision): your gate on §11 adapter contract specifically + §10.2 DX axis. Frame your output as your position with reasoning; if we disagree on §10.2 dual-vs-unified, orchestrator surfaces tie-break to user. My position: dual locks for migration parity + compile-time enforcement + zero-cost-zero-boilerplate; the trade-off is +5 public methods, mitigated by the `_with` family being mechanical thin wrappers.

- **spec-auditor** (post-convergence): structural audit of §9-§13. Verify cross-section consistency (every §9 method citation in actual `manager.rs` line range — 12 already verified); forward-reference integrity (every `CP4 §X` lands at a real future site, every Strategy/CP1/CP2 cite resolves); claim-vs-source (every `manager.rs:N` cite, every spike cite, every ADR-0036/0037 cite). Pay specific attention to the §9.7 file rename (`execute.rs` → `gate.rs`) — verify §0.3 freeze policy permits CP2-to-CP3 cut adjustments at this granularity. The three edits I called out (E1 visibility framing, E2 cross-section gloss, E3 method-count clarification) are spec-hygiene the auditor likely flags too — cleaner to fix in CP3 than at audit pass.

**ADR posture.** ADR-0036 + ADR-0037 already at `accepted` per CP1 ratification. CP3 ratification does not change ADR posture. CP4 (§14-§16 meta + open items + handoff) is next; the `core` maturity bump + post-soak schedule are CP4 + Phase 8 territory, not CP3 gate.

**Memory hygiene.** No new architect-initiative calls in CP3. The CP3 priority-call axes (10-method surface, file rename, engine path, SL-1 deferral) all derive from prior locks (CP1 surface design, CP2 file-split, ADR-0037 amended-in-place gate, CP2 SL-1). No new "we'll fix it later" debt incurred — the only deferral is `tainting_policy` and that has clean gating.

---

*End of ratification. Awaiting orchestrator synthesis with dx-tester.*
