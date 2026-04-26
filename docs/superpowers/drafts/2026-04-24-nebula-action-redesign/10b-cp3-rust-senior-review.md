# CP3 §9 + §10 + §11 + §12 review — rust-senior idiomatic-correctness slice

**Date:** 2026-04-24
**Reviewer:** rust-senior (sub-agent, idiomatic Rust correctness only — no security overlap with §9.5, no DX usability, no devops)
**Target:** Rust 1.95.0, edition 2024 (workspace pin per `rust-toolchain.toml:17`)
**Inputs:** Tech Spec §9–§12 (CP3 active); ADR-0038 / ADR-0039 / ADR-0040; spike `final_shape_v2.rs` (commit `c8aef6a0`); CP1 review `08b-cp1-rust-senior-review.md`.
**Severity legend:** 🔴 WRONG / 🟠 DATED / 🟡 PREFERENCE.

---

## Verdict (RATIFY / RATIFY-WITH-NITS / REVISE)

**RATIFY-WITH-NITS.** §9 / §10 / §11 / §12 are idiomatically sound. Two 🟠 must-fixes, two 🟡 preferences. No 🔴s.

§9 surface inventory respects the ADR-0038 §Neutral item 2 promise (4 primary trait identity preserved). §9.3.1 hard-cut table aligns with `feedback_no_shims.md`. §9.3.2 closes the CP1 §2.3 `BoxFut` single-home open item per CP1 08b 🟠 PREFERENCE (line 58-63) — the resolution shape is correct. §10 codemod tagging (T1/T3/T4 AUTO; T2/T5/T6 MANUAL) honestly reflects which transforms are mechanical vs semantic. §11 adapter contract correctly distinguishes macro-emitted (§11.1 community path) from hand-authored (§11.2 internal-Nebula). §12 sealed-DX pattern matches ADR-0040 §1 verbatim and composes correctly with the CP1 §2.6 `+ ActionSlots` refinement.

---

## §9 public API idiom check

**🟢 §9.1 four primary trait identity preserved.** Tech Spec correctly notes that the bound chain additions (`Input: HasSchema + DeserializeOwned`, `Output: Serialize`, `StatefulAction::State: ...`) are surface-additive, not surface-renaming. The "DX win, not surface change" framing at line 1527 is honest — bounds at impl site catch errors earlier than at registration site. This is the correct fix for the CP1 08b 🔴 finding (adapter ser/de bound asymmetry); §9.1 confirms CP1's lift landed.

**🟢 §9.2 sealed-DX surface — five trait identifiers stay `pub`, sealing is structural.** The `pub trait ControlAction: sealed_dx::ControlActionSealed + StatelessAction` shape (line 1536) is correct — community plugin code that names `ControlAction` in a bound (e.g., `fn requires_control<A: ControlAction>(...)`) compiles; only direct `impl ControlAction for X` is closed. This is the right "interface vs implementation" sealing posture per ADR-0040 §1.

**🟢 §9.3.1 hard-cut table — `feedback_no_shims.md` discipline.** All four removed items have a named replacement: `CredentialContextExt::credential<S>()` → `ctx.resolved_scheme(&self.<slot>)`; `CredentialGuard<S>` → `SchemeGuard<'a, C>`; `nebula_action_macros::Action` → `nebula_action_macros::action`. No `#[deprecated]` shim path — hard removal aligns with the cascade discipline.

**🟢 §9.3.1 `credential_typed::<S>(key)` decision lock — recommend remove.** The three-bullet rationale (achievable via explicit `CredentialRef::from_key`, two-parallel-API DX cost, zero `nebula-sdk::prelude` re-export) is structurally correct. Removing it unifies the authoring guidance on `resolved_scheme`. **Idiom-only opinion: agree with the recommendation.**

**🟢 §9.3.2 `BoxFut<'a, T>` single-home commit — closes CP1 08b 🟠.** §9.3.2 line 1567 explicitly resolves the CP1 forward-track: `nebula-action::BoxFut` is the single home; spike `final_shape_v2.rs:38` and credential Tech Spec §3.4 line 869 both re-pin to this. The note that future cascade may hoist to `nebula-core` is honest deferral. **Resolution shape is correct;** the CP1 §2.3 finding is closed.

**🟠 DATED — §9.3.2 `BoxFut` vs `BoxFuture` naming inconsistency persists.** The CP1 08b 🟡 PREFERENCE (line 54) flagged that spike + credential spec both name `BoxFuture`, while Tech Spec §2.3 has `BoxFut`. §9.3.2 line 1567 says "Spike `final_shape_v2.rs:38` and credential Tech Spec §3.4 line 869 both name `BoxFuture` — Tech Spec re-pins both to `nebula-action::BoxFut`." Re-pinning *across two source documents* to the shorter Tech Spec name is fine if intentional, but the rationale is not stated. Two paths:
  - **(a) Accept the rename:** add a one-line note at §9.3.2 explaining why `BoxFut` (3 letters) over `BoxFuture` (matches `futures::future::BoxFuture`, matches credential Tech Spec, matches spike). My idiom opinion: matching `futures::future::BoxFuture` is the higher-payoff convention since IDE auto-import will surface `futures` first; deviating risks confusion.
  - **(b) Reverse the call:** rename Tech Spec §2.3 to `BoxFuture`, no cross-doc re-pin needed.

**Required fix:** §9.3.2 line 1567 should either (i) state the rationale for the rename to `BoxFut`, OR (ii) re-pin to `BoxFuture` matching spike + credential. Currently the rename happens with no stated rationale. **Smallest fix:** add one sentence to §9.3.2's `BoxFut<'a, T>` row: "Renamed from spike's `BoxFuture` to avoid confusion with `futures::future::BoxFuture` import path / clarity preference per CP3 §X."

**🟠 DATED — §9.4 `ActionContext` field-vs-method shape deferred to CP4 §15 with hand-wave.** §9.4 line 1586 says `ActionContext<'a>` has `pub(crate) field: &'a CredentialContext<'a>` access; the community-author method is `resolved_scheme`. This is correct, but the line "CP4 §15 may revisit field-vs-method shape" defers a decision that affects the **public API contract surface**. CP4 §15 is positioned as housekeeping; deferring an API contract decision into housekeeping is a CP3 freeze invariant 4 risk per §0.2.

**Idiom-only observation:** if the field is `pub(crate)`, then community plugins literally cannot access it — only the method matters. The "may revisit" hand-wave is a no-op decision (the field is already locked by `pub(crate)`). **Required nit:** §9.4 should state explicitly: "Field is `pub(crate)`; community plugins use `resolved_scheme` only. CP4 §15 reviews the *internal* engine ↔ action ergonomic of accessing `creds`, NOT the public surface." The current wording reads like the public shape might change, which is misleading.

**🟢 §9.4 prelude cohesion — `nebula-sdk::prelude` is the entry point.** The split between "exposed in prelude" (community plugin authoring path: 4 primary traits + `ActionContext` + result types + `#[action]` macro + 5 DX trait identifiers + test harness) vs "lower-level access" (`*Handler`, `ActionHandler`, `*ActionAdapter`, `SlotBinding`) is the right partition. Community plugins should never need to name handler types — the macro emits the adapter, the engine consumes the handler.

**🟢 §9.4 ActionSlots `pub` (NOT sealed) decision — correct.** The three-bullet rationale at line 1599 (macro is recommended path; Probe 4/5/6 enforce in tests; advanced internal-Nebula crates may need hand-impl) is sound. Sealing `ActionSlots` would close a door without current upside, and the doc-comment + probe coverage already handle the discouraged-but-possible posture. **Tech-lead ratify call;** idiom-only opinion: agree.

**🟢 §9.5 cross-tenant Terminate boundary — security-lead scope; not reviewed here per task brief.**

---

## §10 codemod transform soundness

**🟢 §10.2 transform table tagging is honest.** The AUTO/MANUAL split correctly reflects which transforms are mechanical vs semantic:
  - **T1 (`#[derive(Action)]` → `#[action(...)]`)**: AUTO; attribute extraction from prior derive form is mechanical token-tree manipulation. Idiomatically sound — proc-macro authors do this routinely.
  - **T3 (`Box<dyn>` → `Arc<dyn>`)**: AUTO with verification note ("transform is a safety net for any pre-cascade `Box<dyn>` patterns the codemod surfaces"). Already-canonical `Arc<dyn>` per `crates/action/src/handler.rs:39-50` makes this nearly a no-op. Honest.
  - **T4 (HRTB → BoxFut)**: AUTO; token-form rewrite from `for<'life0, 'life1, 'a> + where Self: 'a, 'life0: 'a, 'life1: 'a` quadruple-lifetime form to single-`'a` + `BoxFut<'a, T>`. Mechanical because the source pattern is structurally rigid (the `async-trait` emission convention). The codemod's "validates dyn-safety preserved" check is correct — automated check via `cargo check` post-rewrite catches any regression.
  - **T2 (`ctx.credential::<S>()` → `ctx.resolved_scheme(&self.<slot>)`)**: MANUAL REVIEW. The note that "AUTO for cases with explicit type annotation `ctx.credential::<SlackToken>()` AND `SlackToken` registered in workflow manifest" is actually the **only mechanically-decidable case** — for the common case where the type-name heuristic is implicit, the codemod cannot determine the slot binding without parsing the workflow manifest. **Manual tagging is correct.**
  - **T5 (`tracing::error!(action_error = %e)` → `redacted_display!`)**: MANUAL REVIEW. The note "Cannot mechanically distinguish leak-prone from safe" is correct — the codemod cannot inspect a foreign `Display` impl to determine if it includes credential material. **Manual tagging is correct;** the alternative (auto-wrap everything) would be over-correction and clutter non-credential error sites.

**🟢 §10.2 T6 (ControlAction → StatelessAction) — manual scope honestly tagged.** §10.2 line 1672 + §12.4 manual-review path enumeration distinguish:
  - **AUTO common-case path:** `impl ControlAction { fn execute(...) -> ControlOutcome }` → `impl StatelessAction { type Output = ControlOutcome; ... }` + `#[action(control_flow, ...)]` zone. Token-tree rewrite at the trait + signature level, body preserved.
  - **MANUAL edge cases:**
    - Custom `Continue` / `Skip` / `Retry` reason variants (semantic — codemod cannot determine user intent).
    - Interaction with `ActionResult::Terminate` (semantic — manual confirmation of wire-end-to-end discipline).
    - Test fixtures exercising `impl ControlAction` directly via mock dispatch (semantic — reviewer rewrites tests to use adapter dispatch).

**Honest scope.** T6 is non-trivial because `ControlAction::execute` returning `ControlOutcome` and `StatelessAction::execute` returning `Result<Output, Error>` have different shapes. The codemod can rewrite the trait + signature mechanically, but the user-typed body's `Continue { reason }` / `Skip { reason }` flavor handling is plugin-specific. **Tag is correct; auto-rewrite of the trivial pass-through case is sound; manual-review flagging for the semantic cases prevents silent incorrect rewrites.**

**🟢 §10.2.1 codemod execution model — AUTO with `--dry-run` + MANUAL marker insertion.** The unified diff via `--dry-run` flag for review is the standard codemod pattern (`rustfmt`, `cargo fix`, `clippy --fix` all work this way). MANUAL marker insertion via `// TODO(action-cascade-codemod): ...` strings is also standard. Idempotent re-running (no marker insertion if already migrated) is correct — codemods must be idempotent or they break CI re-run safety.

**🟢 §10.4 plugin-author migration guide — 7-step runbook.** Steps 1-7 are the right granularity. Step 5 (resolve manual markers) per-transform breakdown is correct; step 6 (run plugin tests) leverages the new compile-fail probes (§5.3 Probes 1-7) and §6.4 cancellation-zeroize tests for early breakage detection.

**🟡 PREFERENCE — §10.5 auto-vs-manual ratio framing.** §10.5 line 1724-1725 states "~70% automatable" / "~30% manual review." This is a useful summary, but the granularity is per-transform, not per-site. Engine has ~10 T4 sites + ~5 T2 sites + ~5-7 T5 sites — the manual share is actually higher in engine specifically (the heavy reverse-dep). For plugin authors with 1 stateless action, the manual share may be 0% (no T2/T5/T6 hits). **Idiom-only suggestion:** add a one-line note that the 70/30 ratio is workspace-aggregate and may differ per consumer (e.g., trivial plugin: ~100% auto; heavy engine consumer: closer to 50/50). Not blocking.

---

## §11 adapter contract boundaries

**🟢 §11.1 macro-emitted adapter — community plugin path.** The framing "for community plugins, `#[action]` is THE adapter" (line 1737) is the right boundary. The example at lines 1750-1770 (the `StatelessActionAdapter<A>` shape) correctly shows:
  - Generic struct over `A: StatelessAction`.
  - `StatelessHandler` impl with `BoxFut<'a, ...>` return.
  - The 6-step body responsibilities (depth pre-scan, deserialize, resolve creds, invoke, serialize, return).

The "Plugin code never names `StatelessActionAdapter` directly" line is the load-bearing API discipline — this is the surface boundary that the macro hides.

**🟢 §11.2 internal-Nebula adapter authoring — narrow exception list.** The three cases enumerated (engine-internal `MetaAction` test fixtures; sandbox out-of-process runner `*HandlerProxy<A>`; future custom DX shapes) are honest. Hand-authored adapters are rare and crate-internal — community plugins never see this path.

**🟢 §11.2 sealed_dx pattern composition — matches ADR-0040 §1 + ADR-0035 §3.** The example at lines 1786-1801 shows the per-capability inner sealed trait pattern correctly:

```rust
mod sealed_dx {
    pub trait MyCustomShapeSealed {}
}
pub struct MyCustomShapeAdapter<A: StatelessAction> { inner: A, metadata: ActionMetadata }
impl<T: StatelessAction + ActionSlots> sealed_dx::MyCustomShapeSealed for T {}
impl<A: StatelessAction + ActionSlots> StatelessHandler for MyCustomShapeAdapter<A> { /* ... */ }
```

The `+ ActionSlots` bound on the blanket impl correctly inherits the CP1 §2.6 refinement. The note "`pub use` from `crates/action/src/lib.rs` is restricted to the adapter type identifier, not the sealed inner trait" is the right export-discipline.

**🟢 §11.3.1 serialize/deserialize boundary — matches CP1 §2.4 + CP2 §6.1 + §7.1.** The 4-bullet checklist (input pre-scan + input deserialize + output serialize + stateful pre-scan) is correctly grounded in cited sections. The depth cap 128 via `crate::webhook::check_json_depth` is the security floor item 1 wiring.

**🟢 §11.3.2 error propagation — `redacted_display` wrap discipline.** The three-bullet path (every foreign Display goes through `redacted_display`; `From<A::Error>` provided by user-typed error; cancellation does NOT propagate as error) is correct. The cancellation-not-propagating-as-error invariant aligns with `tokio::JoinHandle::abort` semantics — adapter does not catch the abort signal.

**🟢 §11.3.3 cancellation safety — ZeroizeProbe contract surface.** The four-bullet contract (BoxFut cancellable at any `.await`; SchemeGuard Drop runs naturally; test contract via three sub-tests; ZeroizeProbe instrumentation) correctly grounds adapters in the §3.4 cancellation safety invariant + spike Iter-2 §2.4.

**🟡 PREFERENCE — §11.3-1 perf budget forward-track to CP4.** §11.3 line 1836 commits perf measurement to CP4 §15 housekeeping. This is acceptable (microbenchmark is implementation-time concern), but the deferred surface is real: per-dispatch overhead = depth pre-scan + slot resolution + JSON round-trip × 2. For a community plugin author building a high-throughput stateless action, this overhead matters. **Idiom-only suggestion:** §11.3-1 could name a target order-of-magnitude (e.g., "<10µs per dispatch on a representative `SlackSendAction` shape") to give CP4 measurement a concrete acceptance gate. Not blocking; CP4 picks.

---

## §12 sealed pattern Rust shape

**🟢 §12.1 sealed adapter pattern — verbatim from ADR-0040 §1 with CP1 §2.6 refinement.** The blanket impl `impl<T: StatelessAction + ActionSlots> sealed_dx::ControlActionSealed for T {}` (line 1864) correctly inherits the CP1 §2.6 refinement requiring `+ ActionSlots`. This is structurally sound:
  - `mod sealed_dx` is crate-private (no `pub` prefix on the module itself).
  - `pub trait ControlActionSealed {}` inside is `pub` (so `pub trait ControlAction: sealed_dx::ControlActionSealed` doesn't trigger `private_in_public` per ADR-0035 §3 amendment 2026-04-24-B in Rust 2024 edition).
  - Blanket impl grants membership only to types satisfying `StatelessAction + ActionSlots`.
  - Community plugins cannot reach `mod sealed_dx` from outside the crate; cannot impl `ControlActionSealed` directly; cannot impl `ControlAction` directly.

**🟢 §12.2 community plugin DX flow — `impl StatelessAction` + `#[action(control_flow, ...)]`.** The example at lines 1875-1902 correctly shows:
  - `#[action(... control_flow, credentials(api: ApiToken))]` attribute zone signals control-flow shape.
  - `impl StatelessAction for ControlExampleAction` with `type Output = ControlOutcome` (typed control-flow output).
  - Plugin author never names `ControlAction` trait or `ControlActionAdapter` type.
  - Macro emits the `ControlActionAdapter<ControlExampleAction>` wrapper per §11.

This is the right two-step migration pattern per ADR-0040 §Negative item 4.

**🟡 PREFERENCE — §12.2 `control_flow` flag vs `control_flow = SomeStrategy` config deferred to CP4 §15.** Line 1904 says "exact attribute zone syntax — `control_flow` flag vs `control_flow = SomeStrategy` config — is CP4 §15 scope." This deferral is acceptable (the choice doesn't affect the public trait surface), but the CP1 §2.7 lesson applies: "feature flag granularity is an API-stabilization decision, not a type-system decision." Same applies here — the `control_flow` zone-syntax is a DX decision, not a Rust idiom decision. CP4 picks; my idiom-only opinion is irrelevant.

**🟢 §12.3 internal Nebula crate migration — small surface.** The three bullets (control.rs sealed; lib.rs:14 docstring truthful; canon §3.5 wording revision) are honest scope. Per ADR-0040 §Negative item 1 ("Likely small surface (no tracked external implementors per Strategy §1(c))"), the internal migration cost is bounded.

**🟢 §12.4 codemod T6 coverage — common-case automation.** The auto-rewrite path example (lines 1923-1940) is mechanically sound:
  - Source: `impl ControlAction for MyAction { fn execute(...) -> ControlOutcome { ... } }`
  - Target: `impl StatelessAction for MyAction { type Output = ControlOutcome; type Error = MyPluginError; fn execute<'a>(...) -> impl Future<...> + Send + 'a { async move { /* original body */ } } }` + `#[action(control_flow, ...)]` zone.

The body-preserving `async move { /* original body */ }` rewrite is correct — the `ControlOutcome` return type lifts directly to `StatelessAction::Output` without value-shape change. Idiomatically sound.

The manual-review enumeration (custom reasons, Terminate interaction, test fixtures) correctly identifies semantic cases that the codemod cannot decide.

---

## Required edits (if any)

In order of severity, smallest possible diffs to make CP3 §9-§12 correct:

1. **🟠 §9.3.2 `BoxFut` rename rationale missing.** Add one sentence to the `BoxFut<'a, T>` row of §9.3.2 stating WHY the rename from spike/credential `BoxFuture` (8 letters, matches `futures::future::BoxFuture`) to Tech Spec `BoxFut` (3 letters). Two acceptable resolutions: (a) state rationale (e.g., "shorter alias preferred for action-internal use; cross-crate consumers re-export via `pub use nebula_action::BoxFut`"); (b) reverse the call and rename Tech Spec §2.3 to `BoxFuture` matching spike + credential. **Smallest fix: option (a) — one sentence in §9.3.2.**

2. **🟠 §9.4 `ActionContext` field-vs-method hand-wave.** §9.4 line 1586 says "CP4 §15 may revisit field-vs-method shape" — but `creds: &'a CredentialContext<'a>` is `pub(crate)`, so community plugins never see it. The "may revisit" wording reads like the public shape might change. **Required nit:** clarify that CP4 §15 reviews internal engine ↔ action ergonomic only; public surface is locked at `resolved_scheme` method. **Smallest fix: replace "CP4 §15 may revisit field-vs-method shape" with "CP4 §15 reviews internal engine ↔ action ergonomic of `creds` field access; public community-plugin surface is locked at `resolved_scheme`."**

Optional preferences (not blocking):

- **🟡 §10.5 auto-vs-manual ratio framing.** The 70/30 aggregate is a useful summary; add a one-line note that the ratio is workspace-aggregate and may differ per consumer (trivial plugin: ~100% auto; heavy engine consumer: closer to 50/50).

- **🟡 §11.3-1 perf budget concrete target.** CP4 microbenchmark could name an order-of-magnitude target (e.g., "<10µs per dispatch on a representative shape") to give CP4 measurement a concrete acceptance gate.

---

## Summary

**Verdict: RATIFY-WITH-NITS.** §9 / §10 / §11 / §12 are idiomatically sound. The redesign correctly closes CP1 08b 🟠 PREFERENCE on `BoxFut` placement (single-home commit at §9.3.2). §10 codemod tagging honestly distinguishes mechanical (T1/T3/T4) from semantic (T2/T5/T6) transforms. §11 adapter contract correctly partitions community-plugin macro-emission (§11.1) from internal-Nebula hand-authoring (§11.2). §12 sealed-DX pattern composes correctly with ADR-0035 §3 per-capability inner sealed convention + ADR-0040 §1 + CP1 §2.6 `+ ActionSlots` refinement.

**Top 3 findings:**

1. 🟠 §9.3.2 `BoxFut` rename from spike/credential `BoxFuture` lacks stated rationale — one sentence in the table row closes the loop.
2. 🟠 §9.4 `ActionContext` field-vs-method "CP4 §15 may revisit" hand-wave reads like public surface uncertainty; field is `pub(crate)` so public surface is locked at `resolved_scheme`. Clarify wording.
3. 🟡 §10.5 70/30 aggregate ratio is workspace-aggregate; per-consumer share differs. Optional clarification.

No 🔴s. No re-drafted sections. No security overlap with §9.5. Findings forwarded to architect / orchestrator for CP3 ratification gate.
