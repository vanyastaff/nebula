# CP1 §2 + §3 review — rust-senior idiomatic-correctness slice

**Date:** 2026-04-24
**Reviewer:** rust-senior (sub-agent, idiomatic Rust correctness only — no security threat-model, no DX usability, no devops)
**Target:** Rust 1.95.0, edition 2024 (workspace pin per `rust-toolchain.toml:17`)
**Inputs:** Tech Spec §0–§3 (this revision); `final_shape_v2.rs` (spike commit `c8aef6a0`); my own Phase 1 review `02c-idiomatic-review.md`; ADR-0035 §3 (per-capability inner sealed convention); ADR-0040 §1 (sealed DX tier).
**Severity legend:** 🔴 WRONG / 🟠 DATED / 🟡 PREFERENCE.

---

## Verdict

**RATIFY-WITH-NITS.** §2 + §3 hold idiomatically. Three small must-fixes (one 🔴, two 🟠) are the only corrections that should land before CP1 freezes. Everything else is preference and can defer to CP3 / CP2 without cost.

The spec correctly applies the modernization findings from my 02c review (single-`'a` + `BoxFut<'a, T>` alias, no `#[trait_variant::make]` adoption, RPITIT preserved on the typed family). The sealed-DX scaffold respects the ADR-0035 §3 per-capability-inner-sealed canonical form. The HRTB `ResolveFn` shape per credential Tech Spec §3.4 line 869 is correctly cited as load-bearing and not "modernizable."

---

## §2 RPITIT + Send-bound check

**🟢 RPITIT signature shape — correct verbatim against spike `final_shape_v2.rs:209-262`.** All four primary dispatch traits use `-> impl Future<Output = …> + Send + 'a` with the `+ Send` bound stated explicitly per method (matches my 02c §1 line 74: "All typed trait RPITIT methods say `+ Send` explicitly"). No silent single-threaded trait. Send-discipline is identical across `StatelessAction::execute`, `StatefulAction::execute`, `TriggerAction::handle`, `ResourceAction::execute`.

**🟢 Trait-level `Send + Sync + 'static` supertrait bound — correct.** Required for `Arc<dyn *Handler>` storage in the `ActionHandler` enum (§2.5). Spike Iter-1 §1.7 verified the bound chain. The `'static` is mandatory here because the handler is registry-stored across the process lifetime; this is **not** an "unnecessary `'static`" — it is load-bearing.

**🟢 Per-method `'a` lifetime — RPITIT-correct elision.** Each method names a single `'a` covariant over `&'a self`, `&'a ActionContext<'a>`, and (for `StatefulAction`) `&'a mut Self::State`. Rustc 1.95 elision rules accept this since RFC 2115 (1.51); spike `final_shape_v2.rs:209-262` confirms compile. The borrow chain ties the body's `+ 'a` future to the same lifetime — cancellation-safe by construction (matches §3.4 mechanism narrative line 530-532).

**🟡 PREFERENCE — `Output: Send + 'static` framing.** §2.2.1 narrative line 139 reads "`Output: Send + 'static` per spike final_shape_v2.rs:211 — both `Send` for handler erasure and `'static` for serialization through the engine's port projection." The spike actually has `Output: Send + 'static` (line 211 verbatim) but the **typed surface** doesn't need `'static` for a non-erased adapter — `'static` is a serialization invariant, not a type-system invariant of the trait. This is a non-defect because the spec is explicit about WHY the bound is there; just flagging that the rationale combines two reasons.

**🟢 Associated-type bound consistency vs spike.** Tech Spec adds `HasSchema + Send + 'static` to `Input` (line 127, 144, 195) — `final_shape_v2.rs:210, 221, 239` has only `Send + 'static`. The Tech Spec change is **deliberate and correct**: this lifts the adapter-side `HasSchema` bound (which 02c §3 finding line 213-217 flagged as "leaky adapter invariant") onto the trait itself. Error UX migrates from registration site → impl site, which is the right idiomatic call. Spec acknowledges this in §2.2.1 narrative ("documented per ADR-0039 §Context (Goal G2 — closes CR9 undocumented bound)"). Confirmed: this resolves my 02c §3 🟠 finding #5.

**🔴 WRONG — adapter-side `Serialize`/`DeserializeOwned` bounds are NOT lifted.** §2.2.1 / §2.2.2 / §2.2.4 adopt `HasSchema` on `Input` but say nothing about `Serialize`/`DeserializeOwned` on `Output` and `Input` — yet the adapter sites (per 02c §3 line 203-211) require `Input: DeserializeOwned`, `Output: Serialize`. This means the `with_parameters`-style "leaky adapter invariant" that 02c §3 flagged for Input/Output **persists** unless §2.2 lifts the bounds. Two paths:

  - Lift `Serialize` to `Output: Send + 'static + Serialize` and `DeserializeOwned` to `Input: HasSchema + Send + 'static + DeserializeOwned` on the trait itself (uniform across Stateless/Stateful/Resource).
  - Document explicitly in §2.2 that ser/de bounds remain on the adapter and the error UX shift is an accepted tradeoff (with cite to where the doc-comment lands).

CP1 picks neither. **Required fix:** §2.2.1, §2.2.2, §2.2.4 must either lift `Serialize`/`DeserializeOwned` onto the assoc-type bounds, OR add an explicit acknowledgement subsection naming the error-UX surface and pointing CP3 to resolve the adapter-trait bound symmetry. Without this, the same 02c finding survives the redesign.

**🟡 PREFERENCE — `State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static` framing.** §2.2.2 line 147 has the full bound; 02c §3 finding line 199 confirms this is "tight-but-correct." No issue. The `Clone` is for retry/redrive (per `crates/action/src/stateful.rs` adapter); narrative is correct.

**Send-bound on `<Self::Source as TriggerSource>::Event` projection.** §2.2.3 line 177 implicitly relies on `TriggerSource::Event: Send + 'static` (line 167) propagating through the projection. Verified compile in spike Iter-2 §2.2 (`GitHubListReposAction` round-trip). 🟢 correct.

---

## §2.3 BoxFut alias check

**🟢 Single-`'a` shape per Strategy §4.3.1 + 02c §6 line 358 — correct.**

```rust
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
```

This is the exact replacement I recommended in 02c §1 line 43-52 and §6 line 346. It compiles on 1.95 elision (RFC 2115 since 1.51); single `'a` is covariant over both `&self` and `ctx`. Replaces the legacy `for<'life0, 'life1, 'a> + where Self: 'a, 'life0: 'a, 'life1: 'a` quadruple-lifetime boilerplate from `async-trait` emission convention.

**🟢 Naming consistency.** Spec uses `BoxFut` (3 letters); spike final_shape_v2.rs:38 uses `BoxFuture` (8 letters); credential Tech Spec §3.4 line 869 also uses `BoxFuture`. Tech Spec §2.3 line 215 has `BoxFut`. Two-name divergence is a CP3 ergonomics call; not a 🔴 because the alias is local to nebula-action and doesn't conflict with `futures::future::BoxFuture` which has identical shape. **🟡 PREFERENCE:** rename to `BoxFuture` to match spike + credential spec, OR document the rename rationale at §2.3.

**🟢 Dyn-safety preservation.** §2.3 line 220 correctly states "BoxFut is **not** dyn-safe by itself; it is the return shape used by dyn-safe handler trait methods (§2.4)." This is the right framing — the alias is not a trait, it's a return type. The dyn-safety check applies to the `*Handler` traits using this return shape (§2.4).

**🟠 DATED — `BoxFut` as crate-private alias risks duplication with `BoxFuture` from credential / runtime crates.** Tech Spec doesn't say WHERE `BoxFut` lives in the module tree. Two scenarios:

  - If `BoxFut` lives in `nebula-action` crate-public, then engine adapters (`crates/runtime/src/...`) and credential code that need to invoke handler trait methods will import `nebula_action::BoxFut`. That's fine.
  - If credential Tech Spec exposes its own `BoxFuture` at `nebula_credential::BoxFuture` (per `final_shape_v2.rs:38`), and they have **identical** definitions, downstream callers will see a type-collision-by-name (not by shape — the two aliases ARE the same type). Confusing for IDE auto-import.

Tech Spec §2.3 should commit to one of: (a) re-export the credential crate's `BoxFuture` so there's a single `nebula_action::BoxFuture` aliased to it; (b) live with the duplicate; (c) hoist a single `BoxFuture` to a shared crate (e.g. `nebula-core`). Per `feedback_boundary_erosion.md` ("'one small helper in the wrong crate' compounds"), (c) is preferred but is a CP3 §7 placement decision. **🟡 PREFERENCE:** flag this as a CP3 §7 open item. Not a CP1 blocker.

---

## §2.4 *Handler dyn-safety check

**🟢 All four `*Handler` companion traits are dyn-safe per the standard rules.**

- `Send + Sync + 'static` supertrait — required for `Arc<dyn *Handler>` storage.
- No GAT in method signatures.
- No `Self: Sized` in method signatures.
- Each method has a single named lifetime `'a`.
- Methods return `BoxFut<'a, T>` — concrete type-aliased Box, dyn-compatible.
- No generic methods in the `*Handler` traits themselves (generics live on the typed `*Action` traits).

This matches my 02c §1 line 358 prescription and resolves the inconsistent-discipline finding (only `ResourceHandler` had the alias; now all four use `BoxFut`).

**🟢 Lifetime correctness.** For each handler method:
- `&'a self` borrows self for the future's lifetime.
- `&'a ActionContext<'a>` (and `&'a mut serde_json::Value` for stateful) bound to same `'a`.
- Returned future captures both — `BoxFut<'a, ...>` correctly.

The borrow chain forces the future to outlive neither self nor ctx — cancellation safety preserved per §3.4 line 530-532.

**🟢 No `Self: Sized` exclusion needed.** Because the trait carries no associated types and methods take only borrowed inputs (no by-value `Self`), dyn-compat passes without `Self: Sized` on any method. Cleaner than the legacy shape.

**🟢 ResourceHandler signature.** §2.4 line 256-263 has `resource_id: ResourceId` rather than `&'a Self::Resource` (compare typed `ResourceAction` line 199-204 with `resource: &'a Self::Resource`). This is correct — at the dyn handler boundary, the engine looks up the resource by ID; the typed trait operates on the resolved `&Resource`. **🟢 Right erasure point.** Note for CP3: the engine's resource-resolution shim between `ResourceHandler::execute(resource_id)` and the typed `ResourceAction::execute(&resource)` is currently un-spec'd; CP3 §9 should lock this.

**🟡 PREFERENCE — JSON-typed boundary.** §2.4 lines 232/241/251/261 have `serde_json::Value` for input/output/state at the handler trait boundary. This matches `crates/action/src/handler.rs:11-19` JSON-level contract, and §2.4 narrative (line 266) correctly notes "JSON-typed input/output at the handler boundary preserves the JSON-level contract." The `serde_json::from_value` call sites are where G3 floor item 1 (JSON depth cap 128) lands per §4 (CP2). Acknowledge: this implies a JSON allocator hop per dispatch. For Action the cost is on registration / dispatch boundary (not body hot path), which is fine per `feedback_idiom_currency.md`. No action.

---

## §2.6 sealed_dx pattern check

**🟢 ADR-0035 §3 per-capability inner sealed pattern correctly applied.** §2.6 line 291-301 declares:

```rust
mod sealed_dx {
    pub trait ControlActionSealed {}
    pub trait PaginatedActionSealed {}
    pub trait BatchActionSealed {}
    pub trait WebhookActionSealed {}
    pub trait PollActionSealed {}
}
```

This matches ADR-0035 §3 amendment 2026-04-24-B verbatim — outer `mod sealed_dx` is crate-private (no `pub` prefix); inner `Sealed` traits are `pub` so the public DX trait's supertrait reference does not trigger `private_in_public`. Per ADR-0035 §3 line 191-194, this is the **only supported shape** under Rust 2024 edition (a `pub(crate)` inner trait on a `pub` outer module triggers `private_in_public` as **hard error in Rust 2024**, not warning).

Tech Spec narrative line 287-289 cites this correctly: "the `mod sealed_dx` is crate-private; each inner `Sealed` trait is `pub` within that module (so the public DX trait's supertrait reference does not trigger `private_in_public`)."

**🟢 Blanket-impl shape — correct.** §2.6 line 318 has `impl<T: StatelessAction> sealed_dx::ControlActionSealed for T {}`. This grants membership only to types satisfying `StatelessAction` — community plugins cannot bypass because `mod sealed_dx` is unreachable from outside the crate. ADR-0040 §1 ratifies this exact shape.

**🟡 PREFERENCE — comment at §2.6 line 313-317 forward-references CP3 §7 audit.** The trait-by-trait audit ("which primary each DX trait wraps") is locked at CP3 §7. CP1 is correct to defer; the line "trait-by-trait audit at Tech Spec §7 design time" tracks per ADR-0040 §Implementation notes. No action.

**🟡 PREFERENCE — declaration order vs implementation order.** The DX traits in §2.6 are declared but the corresponding adapter types are deferred to CP3 §9. This is fine for a signature-locking section but CP3 implementers must check: each DX trait + its adapter + the `compile_fail` probe verifying community plugins cannot impl directly should land together (so the sealing is verified, not just claimed). **Track for CP3 §9.** Not a CP1 blocker.

**🟢 Naming distinguishes from credential `sealed_caps`.** ADR-0035's example uses `mod sealed_caps` for credentials; ADR-0040 / Tech Spec §2.6 uses `mod sealed_dx` for action DX tier. Two separate modules, two separate sealed conventions, no name collision. 🟢 right.

---

## §2.7 Terminate feature flag granularity

**🟢 Wire-end-to-end decision is the correct idiomatic call** per `feedback_active_dev_mode.md` + Strategy §4.3.2. CP1 §2.7.1 picks the principled symmetric path: both `Retry` and `Terminate` graduate from gated-with-stub to wired-end-to-end at scheduler landing. Tech-lead's solo decision per Phase 1 pain enumeration §7 ratifies this.

**🟠 DATED — feature flag granularity question is not a Rust idiom decision; defer to CP3 §9 is correct.** §2.7-1 explicitly defers `unstable-action-scheduler` (unified) vs `unstable-retry-scheduler` + `unstable-terminate-scheduler` (parallel) to CP3 §9. That is the right call — feature-flag granularity is an API-stabilization decision (do consumers want to opt in to one variant without the other?), not a type-system decision.

**Idiom-level observation for CP3:** Cargo features must be **additive only** — turning on `unstable-action-scheduler` cannot remove items that are present without it. Both proposals in §2.7-1 satisfy additivity. The choice between unified vs parallel is purely about: "do we expect users to opt in to retry without terminate, or vice versa?" If the answer is "no — they always want both or neither," unified is simpler. If the answer is "yes — some users want retry, some terminate," parallel.

My idiom-only opinion: **unified `unstable-action-scheduler`** is preferable for symmetric semantics under feature-discovery DX. Two related variants behind two flags forces consumers to remember which flag controls which variant; one flag for the scheduler subsystem makes the boundary cleaner. But this is a CP3 design call, not a CP1 idiom check.

**🟢 `#[non_exhaustive]` on `ActionResult<T>` line 348.** Forward-compatible — adding a future variant (e.g., a new gated variant) doesn't break consumers. Correct for a public enum at the engine boundary.

**🟢 `#[cfg_attr(docsrs, doc(cfg(feature = "...")))]` line 378, 391.** Renders feature-gating in docs.rs output — best-practice for any feature-gated public surface. Confirms the gate is documented, not just compiled-out.

**🔴 WRONG — `#[derive(Clone, Debug, Serialize, Deserialize)]` on a feature-gated variant.** §2.7.2 line 346 has these derives at the enum level. When `unstable-retry-scheduler` is OFF, the `Retry` variant is `cfg`'d out. The derive macros see the conditionally-compiled enum — `serde::Serialize` derives must skip the absent variant correctly. `#[serde(tag = "type")]` (line 347) on a `#[non_exhaustive]` enum with cfg'd-out variants is a classic interaction risk. The serde derive **does** handle `#[cfg]` on variants correctly (verified via serde docs), so this is actually 🟢, but the spec narrative does not call out the interaction. **Required for CP3 §9:** add a one-line note at §2.7.2 confirming serde-with-cfg variants is verified compile-clean — `cargo check --no-default-features` and `cargo check --all-features` both must pass. (Not a 🔴, downgrading: 🟡 PREFERENCE — add a CHANGELOG note.)

**🟢 `Duration` serialization with `duration_ms` / `duration_opt_ms`** (line 380, 382) — correct custom serde adapter pattern. Standard idiom for `Duration` over wire (avoids `Duration`'s default seconds+nanos tuple, which is fragile for cross-language interop).

---

## §3 runtime path soundness

**🟢 §3.1 SlotBinding registry registration shape — correct.** `#[derive(Clone, Copy, Debug)]` on `SlotBinding` (line 427-432) is mandatory for `&'static [SlotBinding]` storage; spike `slot.rs` static-assert verified `Copy + 'static` per [NOTES §1.1]. Tech Spec preserves the Copy bound as load-bearing.

**🟢 `Map<ActionKey, &'static [SlotBinding]>` storage shape (§3.1 line 444).** Correct — slots are static slices; one-time clone-into-registry-index at construction; O(1) lookup via `(action_key, field_name)`. CP3 §9 locks the exact registry trait surface; CP1 only the input shape. Right granularity for §0.1 CP1 scope.

**🟢 §3.2 HRTB `ResolveFn` shape — correctly cited as load-bearing.**

```rust
pub type ResolveFn = for<'ctx> fn(
    ctx: &'ctx CredentialContext<'ctx>,
    key: &'ctx SlotKey,
) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>;
```

Per credential Tech Spec §3.4 line 869 verbatim. Tech Spec §2.3 narrative line 220 correctly notes "The HRTB used at the credential-resolution layer (§2.5 ActionHandler and §3.2 dispatch) uses the same fn-pointer shape per credential Tech Spec §3.4 line 869."

**🟢 HRTB monomorphization at registration (§3.2 dispatch step 3, line 467-468).** Correct — the macro emits one `ResolveFn` per slot (via `resolve_as_<capability>::<C>`); the HRTB monomorphization happens at the macro-emission site. Engine stores the `fn`-pointer in `SlotBinding::resolve_fn`; runtime invocation `(binding.resolve_fn)(&ctx, &key)` is a fn-pointer call (zero-cost beyond the indirect call). Spike `final_shape_v2.rs:43-46` validates the shape; 02c §5 line 293-302 confirms this is the load-bearing zero-cost erasure point.

**🟢 §3.2 dispatch sequence (lines 464-472) — soundness verified end-to-end.** Steps 1-7 trace the realistic dispatch path:

  1. Action body invokes `ctx.resolved_scheme(&self.bb)` → engine looks up SlotBinding.
  2. Binding carries macro-emitted `resolve_fn`.
  3. Engine invokes `(binding.resolve_fn)(&credential_ctx, &slot_key)`.
  4. `resolve_as_<capability>::<C>` enforces `where C: Credential<Scheme = X>` (compile-time gate per spike Probe 6).
  5. `ResolvedSlot` returns; engine wraps in `SchemeGuard<'a, C>` via `engine_construct(scheme, &'a ctx)`.
  6. `&'a SchemeGuard<'a, C>` exposed to action body via Deref → `&BearerScheme` directly.
  7. On scope exit / cancellation, `SchemeGuard::Drop` zeroizes deterministically.

Per spike Iter-2 §2.4, this path compiles cleanly under realistic dispatch. The borrow chain is sound — `'a` from `engine_construct(_, &'a ctx)` ties the guard to the request's lifetime; `PhantomData<&'a ()>` alone is **not** sufficient (spike `scheme_guard.rs` comment line 122-125 makes this explicit) — the construction signature is the load-bearing pin.

**🟢 §3.3 `resolve_as_<capability><C>` placement — correct.** Strategy §3.1 component 3 names `nebula-engine` as the home (not `nebula-action`); the helper signature has the `where C: Credential<Scheme = BearerScheme>` clause as the **second compile-time gate** per credential Tech Spec §3.4 step 3 (line 893-903). 🟢 right.

**🟢 §3.4 Cancellation safety invariant — correctly grounded in spike artifact.** The 4 sub-tests from spike `cancel_drop_zeroize.rs` + `cancel_in_action.rs` (commit `c8aef6a0`) port forward; Drop ordering (§15.7 line 3412, spike `scheme_guard.rs:144-151`) is correct; auto-deref Clone shadow (per 02c §3 line 222 + spike NOTES finding #1) is acknowledged at §3.4 line 532-534.

**🟠 DATED — Spike NOTES finding #1 (auto-deref Clone shadow on `SchemeGuard`) is mentioned in §3.4 narrative but not in §2 trait contract.** §3.4 line 533 says: "the qualified-form probe `<SchemeGuard<'_, C> as Clone>::clone(&guard)` is mandated at test time to catch the violation, since unqualified `guard.clone()` resolves to `Scheme::clone` via auto-deref (silent green-pass risk)."

This is correct, but **§2 (trait contract) is silent on `SchemeGuard`**. The `SchemeGuard<'a, C>` type itself is not declared in §2; it's a credential-crate type referenced through §2.7's resolve flow. So §2 cannot directly add the Clone-shadow note.

However, §2.4 narrative could add a one-line note: "Note: `SchemeGuard<'a, C>` exposed through `&'a ActionContext<'a>` is `!Clone` by design — see credential Tech Spec §15.7 + spike NOTES finding #1 for the qualified-form Clone test discipline." This grounds CP2 §4 (security floor item 4 detail) in the trait contract, not just the runtime model. **Required nit:** add this single line to §2.4 narrative or to §2.5 (where `ActionHandler` is declared) so that §2 closes the loop on the spike finding.

(Alternatively, §3.4 narrative itself could be promoted into §2 if the editorial decision is to keep Trait Contract self-contained. CP1 author's call.)

**🟢 §3.4 mechanism — sound.** Drop ordering invariant (`SchemeGuard::Drop` runs before scope unwind, per spike `scheme_guard.rs:144-151`); zeroize bound (`C::Scheme: Zeroize` at the type-level bound, spike line 111-113); engine-only `engine_construct` constructor (spike line 122-131) — all preserved verbatim from credential Tech Spec §15.7 iter-3.

**Open item §3.2-1 — `ResolvedSlot` wrap point.** §3.2-1 (line 474) acknowledges the spike NOTES §4 question 5 ambiguity ("engine-side wrapper, not inside `resolve_fn`") and defers to CP3 §9 per spike's recommendation. This is the right deferral. CP1 inheriting the spike's interpretation is sound — the spike explicitly tested the engine-side wrapper shape and validated it. **🟢 correct deferral.**

---

## Required edits (if any)

In order of severity, smallest possible diffs to make CP1 correct:

1. **🔴 §2.2.1 / §2.2.2 / §2.2.4 — adapter ser/de bounds.** Either lift `Serialize`/`DeserializeOwned` to assoc-type bounds on the trait itself, OR add a one-paragraph subsection naming the adapter-trait ser/de asymmetry as an accepted-tradeoff and pointing to where it's documented. The current state preserves the same 02c §3 finding the redesign was supposed to resolve. **Smallest fix:** add a "🟠 retained adapter invariant" subsection at §2.2 with explicit forward-pointer to CP3 §9 for resolution.

2. **🟠 §2.4 (or §2.5) — auto-deref Clone shadow note.** Add one line in §2.4 / §2.5 narrative noting that `SchemeGuard<'a, C>` exposed through the resolve flow is `!Clone` by design, with cross-reference to §3.4 line 532-534 + credential Tech Spec §15.7 / spike NOTES finding #1. This closes the §2 ↔ §3.4 loop and ensures CP2 §4 implementer doesn't write the unqualified `guard.clone()` probe.

3. **🟡 §2.7.2 — serde-cfg-variant interaction note.** Add one-line CHANGELOG entry confirming `#[derive(Serialize, Deserialize)]` on `ActionResult<T>` with `#[cfg(feature = "...")]` variants is verified to compile-pass under both `--no-default-features` and `--all-features`. Not a 🔴 (serde does handle this), but explicit verification is cheap insurance.

Optional preferences (not blocking):

- **🟡 §2.3 — name choice `BoxFut` vs `BoxFuture`.** Either align with spike (`BoxFuture`) or document the rationale for the shorter name.
- **🟡 §2.3 — placement of `BoxFut` alias.** CP3 §7 should commit to either re-exporting credential's `BoxFuture` or hoisting to `nebula-core`, to avoid duplicate-by-name across crates.
- **🟡 §2.7 — feature-flag granularity.** My idiom-only preference is unified `unstable-action-scheduler`; CP3 §9 picks.

---

## Summary

**Verdict: RATIFY-WITH-NITS.** §2 trait contract and §3 runtime model are idiomatically sound. RPITIT/Send-bound discipline correct, `BoxFut<'a, T>` modernization correctly applied, sealed-DX scaffold respects ADR-0035 §3 per-capability-inner-sealed canonical form, HRTB `ResolveFn` shape correctly cited as load-bearing.

**Top 3 findings:**

1. 🔴 Adapter ser/de bound asymmetry (`Serialize`/`DeserializeOwned` on Output/Input not lifted to trait assoc types) — same 02c §3 finding the redesign should have resolved; persists in §2.2.1/§2.2.2/§2.2.4.
2. 🟠 §2 silent on `SchemeGuard: !Clone` invariant — §3.4 acknowledges auto-deref Clone shadow per spike NOTES finding #1, but §2 trait contract doesn't reference it; one-line cross-reference closes the loop.
3. 🟡 `BoxFut` vs `BoxFuture` naming + placement — CP3 §7 must commit to a single shared definition (likely re-export from credential or hoist to `nebula-core`) to prevent duplicate-by-name across crates.

No re-drafted signatures, no security overlap, no commit. Findings forwarded to architect / orchestrator for CP1 ratification gate.
