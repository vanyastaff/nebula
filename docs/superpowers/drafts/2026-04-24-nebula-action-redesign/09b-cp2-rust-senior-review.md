# CP2 Rust-Senior Review — §4 macro emission + §5 test harness + §7 lifecycle

Scope per parallel-review prompt: §4 (emission shape, perf bound, compile_error! discipline), §5 (harness pinning + layout, qualified-syntax probe), §7 (lifecycle adapter execute, SchemeGuard borrow chain). §6 is security-lead's. No redrafting.

## Verdict

**RATIFY-WITH-NITS.**

The emission shape, qualified-syntax probe form, and SchemeGuard borrow chain are sound and grounded in the spike artefact (commit `c8aef6a0`) plus ADR-0037. The HRTB fn-pointer choice over `Box<dyn Fn>` is correctly motivated and idiomatically the right call on Rust 1.95. Probe 7 addition is well-targeted at the real `with_parameters`-vs-`with_schema` bug (verified at `crates/action/macros/src/action_attrs.rs:131` + `crates/action/src/metadata.rs:292`). The dual enforcement layer (§4.4) cleanly separates structural ground (type system) from DX cleanup (proc-macro).

Three nits below; none block CP2 lock. One open item flagged in CHANGELOG (§5.3-1) gets a clean answer here.

## §4 macro emission idiom check

The emission contract at §4.3 is correct on the load-bearing points:

- **HRTB fn-pointer over `Box<dyn Fn>` is the right call.** Zero heap per slot, one-fn-pointer-sized field per `SlotBinding`, no lifetime erasure dance. The coercion `::nebula_engine::resolve_as_bearer::<SlackToken> as ::nebula_action::ResolveFn` works because Rust coerces a generic fn item with `for<'_>` elided lifetimes to the HRTB-quantified fn pointer at the cast site. Spike `bin/iter2_compose.rs` confirms the coercion compiles for all three canonical schemes (NOTES §2.2 + §2.3 PASS).
- **`SlotBinding: Copy + 'static`** is verified at the spike level (slot.rs static assert per NOTES §1.1) and is structurally enforceable because every field is `Copy + 'static` (`&'static str`, enum-of-`Copy`, fn pointer). `&'static [SlotBinding]` storage at `ActionSlots::credential_slots(&self) -> &'static [_]` is well-formed. No hidden allocations in the emission shape itself.
- **`#[non_exhaustive]` on `SlotType` and `Capability`** at §3.1 is the right call for engine-side pattern-match resilience to future variants.

Pattern-2 / Pattern-3 dispatch table at §4.3 (capability marker projected from phantom-shim trait at emission time, reading `const CAPABILITY: Capability` per credential Tech Spec §15.5) is sound — selection happens at macro-expansion time, monomorphizes per slot. No vtable indirection at runtime.

🟢 **Spec narrative claims spike Probe 6 verified the HRTB-coercion** at §4.3 line 818. Probe 6 actually verified the wrong-Scheme bound; the coercion shape is verified by the iter-2 compose binary (NOTES §2.2 + §2.3 — `resolve_as_oauth2::<GitHubOAuth2>` and `resolve_as_basic::<PostgresBasicCred>` both compile clean in const slot slices). Citation accuracy fix; evidence still holds via different spike artefact.

🟢 **Field-name inconsistency between ADR-0037 §1 and Tech Spec §3.1.** ADR-0037's example uses `key`/`capability`/`resolve_fn`; Tech Spec §3.1 uses `field_name`/`slot_type`/`resolve_fn` with capability folded into `SlotType` enum. Tech Spec is correct (matches `final_shape_v2.rs:48-55` and the three-variant SlotType pipeline from credential Tech Spec §9.4). ADR-0037 example should be flagged for amendment to match Tech Spec's authoritative shape — otherwise an implementer reading ADR-first will write the wrong struct.

## §4.5 perf bound assessment

1.6-1.8x adjusted ratio is **reasonable for a production macro** that absorbs ~20-25 LOC of user-written `impl StatelessAction + DeclaresDependencies` boilerplate per old action. The naive 3.2x ratio is the wrong frame; the adjusted ratio is what plugin authors feel.

Per-slot incremental of ~10 LOC (slot binding literal) is structurally minimal — one `SlotBinding { field_name, slot_type, resolve_fn }` literal per slot. Linear scaling at this magnitude is fine; even N=10 slots emit ~161 LOC total which is well within compile-time tolerance.

🟢 **The bound is verifiable via `cargo expand`** as ADR-0037 §5 Positive item 6 names. Macrotest snapshots (§5.5, three fixtures) lock the byte-budget at the snapshot level — drift triggers diff. CP3 §9 picks hard-fail vs warn CI policy; recommend hard-fail to prevent silent emission-bloat creep.

## §4.7 compile_error! discipline

Hard-error on string-form `credential = "key"` is the correct call. `compile_error!` form in §4.7.2 has the right shape (mention bad form, name correct alternative, name where the key actually comes from — `<C as Credential>::KEY`). Span attribution to the offending string literal gives clean fix-it.

The diagnostic message at line 913 is verbose but loadbearing — the three-sentence form is needed because the migration is non-obvious to plugin authors who copy-pasted the old shape. Approve as-written.

🟢 **§4.4.2 `compile_error!`** for bare `CredentialRef<_>` outside zone is similarly correct. Hint message ("did you forget to declare this credential in `credentials(slot: Type)`?") is actionable. Both layers (proc-macro + type-system) firing for the same violation produce two diagnostics; spike confirmed both stay readable (ADR-0037 §Negative item 4).

🟡 **Probe 7 (`parameters = Type` no-`HasSchema` rejection)** is the right addition — bug verified at `crates/action/macros/src/action_attrs.rs:131` (`with_parameters` emission) vs `crates/action/src/metadata.rs:292` (only `with_schema` exists). The diagnostic surfacing the actual missing `HasSchema` bound (not "no method `with_parameters`") is a real DX win. Recommend: the macro-emitted form should ALSO check whether the parameters-type's `Schema` projection is fallible (i.e., does `<T as HasSchema>::schema()` panic on bad schemas, or return `Result`?). If `HasSchema::schema() -> Result<Schema, _>`, the macro needs to thread the error path; if infallible, the `<T as HasSchema>::schema()` call shape in §4.6.1 is correct as-is. Worth a one-line clarification at §4.6.1.

## §5 harness pinning + layout

**Pinning sensible.** `trybuild = "1.0.99"` and `macrotest = "1.0.13"` are current latest-stable as the spec claims; both are stable APIs (1.0.x line). Localizing the pin to `crates/action/macros/Cargo.toml` rather than workspace inheritance is the right call — only consumer is the macro test harness, no surface gain from workspace.

**Layout matches spike at commit `c8aef6a0` `tests/compile_fail/`**, plus the new macrotest expansion side. Tests directory at `crates/action/macros/tests/` is the canonical Rust integration-test location. The `[lib] test = false` line in `crates/action/macros/Cargo.toml:16` (verified) means the macro crate's `src/` does not get unit tests — but `tests/` integration tests are unaffected. `trybuild` and `macrotest` both run from `tests/` correctly; `cargo nextest run -p nebula-action-macros --profile ci` will pick them up.

🔴 **§5.3-1 layering question — `nebula-engine` as dev-dep on `nebula-action-macros`.** This is the open item flagged for rust-senior CP2. Verified against `deny.toml:59-66`:

```toml
{ crate = "nebula-engine", wrappers = [
    "nebula-cli",
    "nebula-api",
], reason = "Engine is exec-layer orchestration; business/core crates must not depend on it" },
```

By default, `cargo-deny ban` enforces wrappers for **all** dependency kinds (normal + dev + build) unless `dev = "allow"` is set globally — the `[bans]` block in `deny.toml:41-81` does not relax `dev`. Adding `nebula-engine` as `[dev-dependencies]` to `crates/action/macros/Cargo.toml` will trip `cargo deny check bans` with `nebula-engine: dev-dep wrapper not in allowed list`.

Two clean paths:

1. **Add `nebula-action-macros` to the wrappers whitelist** for `nebula-engine` with a clear reason ("dev-dep only, for trybuild Probe 5 / Probe 6 fixtures requiring real `resolve_as_*` helpers; no production code path"). This is the path I recommend — it preserves the real bound-mismatch verification spike NOTES §1.5 establishes, and dev-dep is a structurally narrow surface.
2. **Mirror the `resolve_as_*` helpers** in a test-fixture stub crate. Loses Probe 6's real bound verification (the stub helpers wouldn't carry the `where C: Credential<Scheme = X>` constraint as engine ships it; you'd be testing the stub, not the engine).

Path 1 is the senior-engineer call. The wrappers whitelist is the correct mechanism for "this exception is intentional, this is why" — that's exactly what it's designed for. No cycle is introduced because dev-dep dependencies are not part of the production graph; cargo-deny's wrapper check is policy-enforcement, not graph-soundness. Recommend Tech Spec §5.3-1 commits to **path 1 with deny.toml amendment**, listing `nebula-action-macros` in the `nebula-engine` wrappers list with the reason inline.

`feedback_boundary_erosion.md` does NOT apply here — that pattern is about helper code creeping cross-crate; this is about a regression-test verifying a real engine bound at the boundary. The dev-dep is the canonical Rust pattern for "test-only depends on a higher-layer crate to verify type-shape contract."

## §5.4 qualified-syntax probe

**Form is correct per ADR-0037 §3 + spike finding #1.**

```rust
let _g2 = <SchemeGuard<'_, SlackToken> as Clone>::clone(guard);  // E0277 fires
```

This is the only probe form that catches the `SchemeGuard: !Clone` invariant violation. The unqualified `guard.clone()` resolves via auto-deref to `Scheme::clone` (silent green-pass) per spike NOTES §3 finding #1 line 179.

🟢 **§5.4 example code at line 1022** has a minor pedant nit: the Rust convention is `<SchemeGuard<'_, SlackToken> as Clone>::clone(&guard)` (with `&`) since `Clone::clone` takes `&self`. The spec's `<SchemeGuard<'_, SlackToken> as Clone>::clone(guard)` (no `&`) would only work if `guard` itself is already `&SchemeGuard<...>`. The probe at §5.4 line 1020 correctly binds `guard: &SchemeGuard<'_, SlackToken>` so the call as-written compiles, but the form is non-idiomatic — more readable as `clone(&*guard)` or `clone(guard)` with explicit type comment. Either works; current form is correct, just noting for snapshot reviewers.

🟢 **§5.4.1 soft amendment к credential Tech Spec §16.1.1 probe #7** is the right protocol — flag, don't enact. ADR-0035 amended-in-place precedent applies cleanly. Action-side probe (§5.4) catches the violation independently in the meantime, so there's no coverage gap during the cross-crate coordination window.

## §7 lifecycle path soundness

**Six-step adapter execute path at §7.1 compiles end-to-end** under realistic dispatch. The `SchemeGuard<'a, C>` borrow chain at step 5-6 is grounded in credential Tech Spec §15.7 line 3503-3516 iter-3 refinement (engine constructs guard with `&'a CredentialContext<'a>` pinning `'a`) and verified by spike Iter-2 (Stateless / Stateful / Resource all compile with the borrow chain).

**Step 4 invokes typed action body** with `&'a typed_ctx, typed_input` returning `impl Future<Output = Result<A::Output, A::Error>> + Send + 'a`. The `'a` here is the dispatch's borrow chain; `SchemeGuard<'a, C>` lives in the same `'a`; cancellation drops the future and Drop runs through the borrow chain — the §3.4 invariant.

🟢 **§7.3 error propagation table** is comprehensive. The `ResolveError::NotFound` → `Fatal` mapping (§7.3 line 1311) is provisional — Open item §7.3-1 leaves this for CP3 §9. Recommend: add a typed `Resolve { reason }` variant to `ActionError` rather than collapsing into `Fatal`. The collapse loses provenance (was this a missing key, a wrong type, a state-load failure?), and the rust-senior 02c §7 finding "ActionError taxonomy is the cleanest part of the crate idiomatically" applies here — preserving the error chain via a dedicated variant is the idiomatic Rust pattern. Punt to CP3 as flagged.

🟢 **§7.4 ActionResult variants handling.** The wire-end-to-end commitment for `Retry` + `Terminate` per §2.7.1 is correctly gated behind `unstable-retry-scheduler` + `unstable-terminate-scheduler` parallel flags. The handling table maps cleanly. No idiomatic concerns.

🟢 **§7.2 SchemeGuard RAII flow** is correctly cited-not-restated from credential Tech Spec §15.7. No action-side amendment surfaced beyond the §5.4.1 cross-crate flag.

## Required edits (if any)

None blocking. Nits worth folding into CP2 lock pass:

1. **§4.3 line 818 citation correction.** Change "spike Probe 6 verified at commit `c8aef6a0`" to "spike Iter-2 §2.2 + §2.3 verified the HRTB coercion" — Probe 6 is the wrong-Scheme bound, not coercion-shape.
2. **§5.3-1 commit to path 1.** Tech Spec should commit to "add `nebula-action-macros` to `nebula-engine` wrappers list in `deny.toml` with dev-dep-only reason" rather than leaving the path open. Path 2 (stub helpers) loses Probe 6 verification and isn't worth the boundary purity it claims.
3. **ADR-0037 §1 example shape mismatch.** ADR-0037 should be amended to use `field_name`/`slot_type` field names matching Tech Spec §3.1 (the authoritative implementer shape). Currently ADR has `key`/`capability` as separate fields; Tech Spec correctly folds capability into `SlotType` enum. This is an ADR amendment, not a Tech Spec change.

## Summary

§4 macro emission is idiomatically the right shape for Rust 1.95 — HRTB fn-pointer over `Box<dyn Fn>`, narrow zone rewriting, dual enforcement, hard-error on string-form. §5 harness pinning + layout is sound; the dev-dep layering question (§5.3-1) has a clean answer in `deny.toml` wrappers amendment. §7 lifecycle path compiles cleanly under the SchemeGuard borrow chain. Three nits flagged; none block CP2 lock.

**Top three findings:**

1. (🔴-rated for visibility, 🟢 in severity) §5.3-1 dev-dep on `nebula-engine` requires `deny.toml` wrappers amendment, not stub helpers. Path is straightforward; recommend committing now.
2. §4.3 citation for HRTB coercion verification is wrong probe — Iter-2 compose, not Probe 6. Correctness: holds.
3. ADR-0037 §1 example field-shape diverges from Tech Spec §3.1 authoritative; ADR needs amendment to match.
