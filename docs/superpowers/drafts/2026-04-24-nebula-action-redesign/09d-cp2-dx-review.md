---
reviewer: dx-tester
mode: parallel review (Phase 6 CP2)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (DRAFT CP2, §4 macro emission + §5 harness)
slice: DX of `#[action]` attribute macro from plugin-author perspective
parallel-with: spec-auditor + tech-lead + rust-senior + security-lead (Phase 6 CP2 reviewer matrix)
budget: 25 min
prior-reviews:
  - 08d-cp1-dx-review.md (CP1 §2 trait contract — flagged CP2 macro-syntax preview concerns)
  - 02a-dx-authoring-report.md (Phase 1 hit-list: silent string-drop, broken `parameters = Type`, etc.)
---

## Verdict

**RATIFY-WITH-NITS.** §4 closes the four CP1-flagged macro-syntax preview risks (zone-syntax single-form lock, dual-enforcement diagnostic span, `parameters = Type` C2 fix, string-form hard rejection) cleanly. Each Phase 1 BLOCKING finding (#1 silent-drop string form, #5 `semver` re-export gap fixed via `::semver::` paths preserved + `parameters` schema fix, #6 `parameters = Type` broken path, #7 `HasSchema` bound surfaced) gets a §4-locked closure with cited line evidence. The dual-enforcement layer (§4.4) is the strongest piece — proc-macro layer fires first with an actionable diagnostic *and* the type-system layer is the structural ground if a hand-roll bypasses the macro. §5 ports six spike probes plus the new Probe 7 (`parameters = Type` where `Type: !HasSchema`), which directly closes the C2 emission-bug regression-coverage hole that let three independent agents hit the same bug.

Five non-blocking nits surface from a plugin-author lens (§4.1 zone-syntax trailing-comma + empty-vs-omitted-zone equivalence; §4.4.2 hand-roll diagnostic asymmetry; §4.6.1 Probe 7 doesn't catch `description = "doc"` → `description: doc` typos in same class; §4.7.2 codemod lookup pointer; §5.4 unqualified-form regression test).

The three CP1 §2 blocking gaps (R1 `ActionSlots` undefined / R2 blanket-impl wording / R3 sealed-DX migration target) are **not** in CP2 scope — they sit in §2 which is locked at CP1 freeze. Flagging them here as outstanding *only* in case CP4 cross-section pass needs to reconcile them; CP2 §4 cannot fix them unilaterally.

---

## §4.1 zone syntax DX

### What works

**Single-form lock is achieved.** §4.1.3 invariants name three rejection rules with `compile_error!`: duplicate slot name (span at second occurrence), slot/non-zone field collision, unknown attribute key. This **closes CP1 R6 silent-drop concern dead** — there is exactly one form, and §4.7.2 hard-rejects the legacy `credential = "key"` shape. A plugin author who copy-pastes a stale README example gets a `compile_error!` redirect, not silent-drop.

**Three credential-pattern dispatch table in §4.1.1 is well-organized.** Pattern 1 (concrete: `slack: SlackToken`) → Pattern 2 (service-bound capability: `gh: dyn ServiceCapability<GitHub, Bearer>`) → Pattern 3 (capability-only: `bearer: dyn AnyBearer`) are exhaustively enumerated, each with the rewritten form and the engine-side `SlotType` variant cited. A plugin author reading §4.1.1 once sees all three forms in one table — not buried in three subsections of cross-references.

**Field ordering policy explicit.** §4.2 fourth paragraph states "Zone-injected fields appear before struct-body fields ... Plugin authors must not rely on field order semantically; Tech Spec does not commit to ordering stability across versions." This is the right call — explicit non-commitment is better than silent-stable-then-breaks-on-1.96.

**Empty zone is permitted.** §4.1.1 last paragraph: `credentials()` and the omitted `credentials(...)` zone are equivalent (both produce `credential_slots() -> &'static []`). This means a zero-credential action doesn't have to write `credentials()`. **Good** — newcomer who builds an `EchoAction` with no credentials doesn't trip the zone parser at all.

### Nits

1. **§4.1 doesn't surface trailing-comma policy.** The example at line 733-741 ends `resources(http: HttpClient),` with a trailing comma after the last zone. Is `#[action(key = "x", name = "X")]` (no trailing comma) accepted? Both shapes are common in Rust attribute macros. Phase 1 newcomer would not know. **NIT** — §4.1 should add one sentence: "Trailing commas are accepted both inside zones and at the top-level attribute list (matches Rust attribute idiom)." Cheap; eliminates one round-trip to `cargo check`.

2. **`credentials()` empty-zone vs omitted-zone equivalence isn't visible from the macro signature alone.** §4.1.1 line 756 says "Empty zone (`credentials()`) is permitted ... Omitting the zone entirely is permitted; equivalent to `credentials()` (still emits `ActionSlots` impl with `credential_slots() -> &'static []` empty slice — supertrait satisfaction per §2.1)." Good prose. But the example in §4.1 (line 733-741) shows `credentials(slack: SlackToken)` — there is no example of the **zero-credential action** which is the most common newcomer first-action shape (an `EchoAction`, a `MathAction`, a `LogAction`). **NIT** — add one canonical example inline at §4.1: `#[action(key = "ex.echo", name = "Echo")] pub struct EchoAction { pub message: String }` to show the **"no zones at all"** path; this is what Phase 1 newcomer would type first.

3. **§4.1.3 invariants are exhaustive for parser errors but not for parser ambiguity edge-cases.** What about: `credentials(slack: SlackToken, slack: GitHubToken)` (legitimate duplicate — §4.1.3 catches this) ✓. What about: `credentials(http_client: SlackToken)` plus `resources(http_client: HttpClient)` (cross-zone slot-name collision — same identifier in both zones)? §4.1.3 names "Duplicate `slot_name` within one zone" and "`slot_name` collides with non-zone field name" but is **silent on cross-zone collision**. The rewritten struct would have two `pub http_client: ...` fields → would fail at `E0428` from rustc. The proc-macro layer should catch this as a §4.1.3 fourth invariant rather than letting downstream rustc surface a less-helpful diagnostic. **NIT — REQUIRED EDIT** — §4.1.3 add bullet: "Cross-zone slot-name collision (same identifier in `credentials(...)` and `resources(...)`) is `compile_error!` with span at the second occurrence."

---

## §4.4 enforcement diagnostics DX

### What works

**Diagnostic span attribution is correct.** §4.4.2 line 832-845: "the macro emits `compile_error!(...)` with span pointing at the offending field." Plugin author hovering over the error in their IDE sees the span on the `pub slack: CredentialRef<SlackToken>` line, not on `#[action(...)]` macro-internals. This is the right design — Phase 1 finding 6 (parameters error pointing at `#[derive(Action)]` site, not at the attribute line) was caused by exactly this category of failure in the old derive macro. CP2's compile_error-with-field-span fixes that class for the credential surface.

**Diagnostic message is human-readable, not macro-jargon.** Line 844: `did you forget to declare this credential in `credentials(slot: Type)`?` — the message **names the fix**, not the symptom. Compare to `error: trait CredentialRef::__nebula_action_marker is not implemented for X` which is what an unenforced layer would emit. Phase 1 newcomer reading this knows immediately what to add.

**Dual-layer redundancy is intentional and load-bearing.** §4.4.1 (type-system) + §4.4.2 (proc-macro) are explicitly redundant. A community plugin author who hand-rolls `impl ActionSlots for X { ... }` to bypass the macro still hits the type-system layer at registration time (per §4.4.3). This means the seal is genuine — there's no "soft" bypass path. ADR-0036 §Negative item 2 names removal of either as weakening the contract; §4.4 surfaces this discipline cleanly.

**§4.4.3 invariant statement preempts the "DIY ActionSlots" antipattern.** Lines 851-855 explicitly note that hand-rolled `impl ActionSlots` compiles but produces a slot-less action — `ctx.credential::<S>(key)` calls fail at runtime with `ResolveError::NotFound`. This is the right warning — the macro is the recommended path, hand-rolls are technically permitted but functionally pointless. **And** Probe 6 (wrong-Scheme) catches the worse hand-roll case where someone fabricates a `SlotBinding` with a mismatched `resolve_fn` — the registration-site bound check fires.

### Nits

1. **§4.4.2 diagnostic detection is "any field whose type is `CredentialRef<_>` or its dyn-shaped equivalents."** The "dyn-shaped equivalents" phrase is hand-wavy — does the macro detect `CredentialRef<dyn AnyBearer>`? `CredentialRef<dyn ServiceCapability<X, Y>>`? `CredentialRef<dyn ServiceCapabilityPhantom<X, Y>>` (the post-rewrite shape — would the macro detect a *post-rewrite* phantom-shim form if the user pre-applied the rewrite manually)? **NIT** — §4.4.2 should enumerate the detection patterns exactly: `CredentialRef<C>` where `C` is `ident-path` OR `dyn TraitBound` OR `dyn TraitBound + Send` etc. A grep-equivalent rule is more useful than a hand-wave for an implementer trying to write the detection logic.

2. **§4.4.3 hand-roll asymmetry undocumented.** Per §4.4.3, hand-implementing `ActionSlots` is "technically possible (the trait is `pub`)." But §4.4.3-1 (open item) flags whether to seal the trait. **DX implication:** if §4.4.3 is committed to "leave `ActionSlots` `pub`", a plugin author *can* technically write `impl ActionSlots for X { fn credential_slots(&self) -> &'static [SlotBinding] { &[] } }` to bypass the macro for an action that doesn't need credentials but wants to skip macro emission for some reason (build perf, debugging). **What §4.4.3 doesn't say:** is this a supported escape hatch (bug-class entirely on the user's head), or an architectural smell that should be sealed? CP3 §9 picks; CP2 should at least state the *direction* of the picking. **NIT** — §4.4.3-1 add: "Recommended direction: seal `ActionSlots` at CP3 §9 (matches `feedback_no_shims.md` posture). Hand-rolls remain technically possible only for §5.3 Probe 3/4/5 negative-test fixtures (cfg-gated to `tests/`)."

3. **`compile_error!` diagnostic doesn't enumerate the fix space.** Line 844: `did you forget to declare this credential in `credentials(slot: Type)`?`. A newcomer who reads this is told *what* to add but not the *example syntax*. Compare to thiserror's `#[error("...")]` derive, which on missing-field surfaces the exact attribute syntax in the diagnostic. **NIT** — diagnostic message should name the slot suggestion: `` did you forget to declare this credential in `credentials(slack: SlackToken)`? Add it to the `#[action(...)]` attribute. `` — synthesizing the slot name from the field name and the type from the `CredentialRef<Type>` parameter. This is a 5-LOC change in the proc-macro emit and saves one round-trip for every confused author.

---

## §4.6 parameters/version DX

### What works

**Phase 0 C2 fix is direct and traceable.** §4.6.1 line 882-895: the broken `with_parameters` emission is replaced with `with_schema(<#ty as ::nebula_schema::HasSchema>::schema())`. The fix is **structurally equivalent** to `ActionMetadata::for_stateless::<A>()` at metadata.rs:176 — meaning the macro emission converges with the existing typed-builder API instead of forking. Phase 1 finding 6 closes here.

**Probe 7 closure of bound-error diagnostic.** §4.6.1 last paragraph + §5.3 Probe 7: a `parameters = Type` where `Type: !HasSchema` produces `error[E0277]: trait bound Type: HasSchema not satisfied`, **not** the misleading "no method named `with_parameters`" error from the old emission. This is exactly the diagnostic-improvement Phase 1 wanted (finding 6 root cause was the macro pointing at the derive site, not the attribute line). The diagnostic now points at the actual missing trait, which a newcomer can solve by adding `#[derive(HasSchema)]` (or `#[derive(Schema)]` per Phase 1 finding 7 nomenclature).

**`version = "X.Y[.Z]"` parsing preservation.** §4.6.2 reuses the existing `parse_version` helper from action_attrs.rs:51-54. **No regression risk** — the version-attribute surface that already works in production is preserved verbatim. No silent-drop introduced.

**`description` doc-fallback is preserved.** §4.6.3 — if `description` attribute is absent, struct's `///` doc-string fills in. Newcomer who writes a doc comment on the struct gets it surfaced as the action description automatically. Good ergonomic — matches `clap` derive's `///` → help-text precedent.

### Nits

1. **§4.6.1 Probe 7 catches `Type: !HasSchema` but not the typo `description = doc` (missing quotes).** Phase 1 finding 6 was rooted in `parameters = Type` mis-emission. But the same parser class — typed-vs-string confusion — affects `description`, `name`, `key`. What if author writes `#[action(name = MyAction)]` (forgot quotes)? §4.1.3 invariants name "Unknown attribute key is `compile_error!`" but **don't name "wrong-form value"**. The `name` attribute value is a string, and `MyAction` is a path → either silent-rejection or downstream rustc error. **NIT** — §4.6 should add one paragraph at §4.6.4 (new): "Attribute value type is enforced at parse time. `name`, `key`, `description`, `version` require string literals (quoted); `parameters` requires a type path; `credentials(...)` / `resources(...)` zone entries require `slot_name: Type` syntax. Mismatches are `compile_error!` with span at the offending value." This closes the same class of trap §4.7.2 closes for `credential = "key"` — symmetric coverage of value-form mistakes across the entire attribute surface.

2. **§4.6.1 doesn't say what happens for a *missing* `parameters` attribute.** Some action shapes don't take parameters (Phase 1 Action 1 `EchoAction` had `type Input = Self`). Is `parameters = Self` required? Or is `parameters` optional and defaults to "Input is the struct itself"? §4.1 example shows `parameters = SlackSendInput` but doesn't comment on omission. **NIT** — §4.6.1 should add: "If `parameters` is omitted, the macro emits `<Self as ::nebula_schema::HasSchema>::schema()` as a default — i.e., the action struct itself is the input type. This matches the `type Input = Self;` idiom from §2.2.1." Closes a discoverability gap that Phase 1 hit at Action 1.

3. **No diagnostic for `version = "1.0.0.0"` (4-part version) or `version = "1"` (1-part).** §4.6.2 says "Default `\"1.0\"` if absent" — but the parse rule doesn't say what *invalid* version strings produce. Per `parse_version` in action_attrs.rs:51-54 (existing code), this is presumably handled, but §4.6.2 doesn't surface the diagnostic shape. **NIT** — §4.6.2 add: "Invalid version strings (4+ parts, non-numeric, missing `.`) are `compile_error!` with span at the version-attribute literal, message naming the expected format `X.Y[.Z]`." Same symmetric-diagnostic discipline as §4.7.2.

---

## §4.7 string-form rejection DX

### What works

**Hard `compile_error!` is the right call.** §4.7.2 line 911-928. Per `feedback_no_shims.md`, silent-drop is the worst possible UX. Phase 1 finding 6 measured this concretely: an author writes `#[action(credential = "slack_token")]`, the macro silently drops the declaration, and the resulting action ships with **zero declared credential dependencies**. Hard-error is the correct migration signal.

**Diagnostic message is fully self-contained.** Line 921-923: `the `credential` attribute requires a type, not a string. Use `credential = SlackToken`, not `credential = "slack_token"`. The credential's key is provided by `<C as Credential>::KEY`.` — three pieces in one diagnostic: (a) what's wrong, (b) the correct syntax, (c) where the implicit value comes from. A newcomer reading this doesn't have to grep anything; the fix is named in the error.

**Span at the offending string literal.** Line 918-919 example: `#[action(credential = "slack_token", ...)]` with the diagnostic caret pointing at `"slack_token"`. IDE-friendly. Phase 1 finding 6 root cause was the diagnostic pointing at the derive site; §4.7.2 fixes that for this specific class.

**Codemod path named.** Line 925-928 mentions the codemod auto-rewrites `credential = "key"` to `credentials(<inferred slot name>: <inferred type>)` per CP3 §9. This means the migration story is **not** "blocking compile error and good luck" but "compile error + automated codemod + manual-review fallback." The author isn't stranded.

### Nits

1. **§4.7.2 diagnostic doesn't say "see codemod at <link>"** — just emits the fix. **NIT** — diagnostic message should add a third line: `Codemod available — see CHANGELOG / migration guide.` Cheap, gives the author a discoverable next step instead of forcing them to grep CHANGELOG. Compare to thiserror's diagnostic on attribute typos which links to the docs.

2. **Symmetric coverage.** §4.7.2 covers `credential`, `credentials`, `resource`, `resources` keys (line 913) — which is the load-bearing security-relevant surface. But what about the rare case where a plugin author writes `#[action(credential = "key", credential = SlackToken)]` (mixed form)? Does the macro reject the string-form first, or does the type-form win? **NIT** — §4.7.2 should add: "Mixed-form attributes (e.g., `credential = \"key\"` + `credential = SlackToken` in the same `#[action(...)]`) emit two `compile_error!` invocations: first rejecting the string form, second rejecting the duplicate-key. Author sees both errors at once." Avoids a second compile-recompile round-trip.

3. **Open item §4.7-1 (line 928) flags codemod-inference success rate.** Per Strategy §4.3.3 transform 3, codemod must error on remaining call sites with crisp diagnostic. CP3 §9 quantifies. **NIT** — for plugin authors, this is the **worst-case fallback**: the codemod can't infer their credential type from the string. CP2 §4.7.2 should commit a *manual-marker* contract for this case: "If codemod cannot infer the type, it inserts a `// CODEMOD-MANUAL-REVIEW: <reason>` marker at the attribute site and leaves the original `credential = \"key\"` for human resolution." Plugin author sees the marker, fixes by hand. Beats silent-rewrite-to-wrong-type 100% of the time.

---

## §5 harness DX impact on authors

### What works

**Zero impact on plugin authors writing actions.** §5 harness sits in `crates/action/macros/tests/` — it's a *macro-crate* test surface, not a plugin-author-facing API. A plugin author authoring `SlackSendAction` does **not** need to update trybuild fixtures to add a new credential type. They write `#[action(credentials(slack: SlackToken))]` and the macro emits the slot binding; their action ships without touching `crates/action/macros/tests/`. **Zero new author-facing toolchain surface.**

**Probe 7 is the killer feature for plugin authors.** §5.3 Probe 7 (`parameters = Type` where `Type: !HasSchema`) catches the **most common** Phase 1 mistake — author writes `parameters = MyInput`, forgets `#[derive(HasSchema)]`. The diagnostic now names the missing bound (`HasSchema not satisfied`), not the macro-internal method (`no method `with_parameters``). This is the Phase 1 finding 6 closure, regression-locked.

**Macrotest expansion snapshots (§5.5) are author-invisible.** Three snapshots (stateless_bearer, stateful_oauth2, resource_basic) lock per-slot emission stability. A plugin author writing a fourth action shape doesn't add a snapshot — that's macro-crate maintenance work. The snapshot mechanism is internal-only; CI catches drift.

**Spike-probe port is comprehensive.** Six probes from spike `c8aef6a0` cover the full failure-mode surface: missing assoc types (P1, P2), bare CredentialRef without zone (P3), SchemeGuard Clone bypass (P4), SchemeGuard lifetime escape (P5), wrong-Scheme resolve_fn (P6). A plugin author who *triggers* one of these gets a typed diagnostic — none of the failure modes is silent-drop.

### Nits

1. **§5.4 qualified-form Probe 4 is correct — but no test exists for the unqualified form.** Per §5.4 last paragraph: "the unqualified form `guard.clone()` is the user-trap shape (auto-deref to `Scheme::clone`)." A plugin author who writes `guard.clone()` will **silently green-pass** at compile time and leak credentials at runtime — the qualified-form probe doesn't catch user code. **The author trap is in the unqualified shape, not the qualified shape.** §5.4 acknowledges this trap exists but doesn't add a probe for it. **NIT — REQUIRED EDIT (potentially)** — §5.4 should add a Probe 4b: a runtime fixture that calls `guard.clone()` (auto-deref form), uses the resulting `Scheme` clone after `'a` lifetime, and asserts at runtime that the credential value is **already zeroized** (i.e., the auto-deref clone produces a zeroized scheme, not a usable copy). If the credential's `Drop`/`Zeroize` machinery is correctly wired, the auto-deref clone produces a "dead" `Scheme` and the author's misuse is detectable at test time. Without this probe, Phase 1 finding 6's "silent green pass" is regression-not-locked.

2. **§5.3 dev-deps include `nebula-engine` as a path dep on `nebula-action-macros`.** This is **flagged** as Open Item §5.3-1 — layering concern. From a plugin-author lens, this is invisible (the dev-dep is internal). But from a workspace boundary lens, this is `feedback_boundary_erosion.md` precedent — flagging here for completeness; the rust-senior CP2 review catches the load-bearing concern.

3. **§5.5 macrotest snapshots include three fixtures (stateless_bearer, stateful_oauth2, resource_basic) but no `BatchAction` / `WebhookAction` / `PollAction` fixtures.** §2.6 sealed-DX traits — `BatchAction`, `WebhookAction`, `PollAction` — would gain emission-snapshot coverage if they're macro-emitted via `#[action(batch(...))]` / `#[action(webhook(...))]` zones (CP3 §9 scope). For CP2 the three primaries cover the load-bearing surface; flagging that CP3 expansion needs symmetric snapshot coverage for the sealed-DX adapters once their attribute zones lock. **NIT** — §5.5 add forward-pointer: "CP3 §9 commits sealed-DX adapter macro emissions (`batch(...)`, `webhook(...)`, `poll(...)`); macrotest snapshots for those land at CP3 ratification." Avoids surprise when CP3 expands the snapshot count.

---

## Required edits (if any)

CP2 ratification gates on these edits before §4 / §5 freeze:

1. **R1 (REQUIRED — blocking) — §4.1.3 cross-zone slot-name collision invariant.** Add bullet to §4.1.3: "Cross-zone slot-name collision (same identifier in `credentials(...)` and `resources(...)`) is `compile_error!` with span at the second occurrence." Currently §4.1.3 catches within-zone duplication and zone/non-zone collision but is silent on cross-zone. The rewritten struct would fail at `E0428: duplicate field` from rustc — less helpful than a parser-level rejection with span attribution.

2. **R2 (REQUIRED — blocking) — §4.4.2 detection-pattern enumeration.** Replace "any field whose type is `CredentialRef<_>` (or its dyn-shaped equivalents)" (line 833) with explicit enumeration: `CredentialRef<C>` for `C ∈ {ident-path, dyn TraitBound, dyn TraitBound + Send}`. The current hand-wave leaves the implementer guessing which forms to detect, and a missing form means the proc-macro layer silently doesn't fire (only the type-system layer fires) — the diagnostic-quality contract breaks. Phase 1 finding 6 silent-drop precedent is exactly this class.

3. **R3 (RECOMMENDED — non-blocking) — §4.6 symmetric value-type enforcement.** Add §4.6.4 "Attribute value type discipline": all attribute keys enforce expected value type at parse time. Closes the same trap class §4.7.2 closes for `credential = "key"` — but for `name = MyAction` (forgot quotes), `version = 1.0` (forgot quotes), `parameters = "MyInput"` (added quotes). Symmetric coverage; cheap to add.

4. **R4 (RECOMMENDED — non-blocking) — §4.6.1 default for missing `parameters` attribute.** §4.6.1 should explicitly state: "If `parameters` is omitted, default is `parameters = Self` — the action struct is the input type. Matches §2.2.1 `type Input = Self;` idiom." Closes a Phase 1 Action 1 discoverability gap (newcomer wrote `parameters = HttpGetInput` then realized struct itself is input).

5. **R5 (RECOMMENDED — non-blocking) — §4.7.2 diagnostic links to codemod.** Augment the `compile_error!` message in §4.7.2 with a third line pointing at the codemod / migration guide. Avoids author having to grep CHANGELOG. 1-line change in the proc-macro emit.

6. **R6 (RECOMMENDED — non-blocking) — §5.4 unqualified-form runtime probe.** Add Probe 4b at §5.4 that exercises the **unqualified** `guard.clone()` shape and asserts zeroize discipline at runtime. The qualified-form probe (§5.4) catches the *intended-violation* shape; the unqualified-form probe would catch the *user-trap* shape that §5.4 explicitly acknowledges as the silent-pass risk. Without a probe, the silent-pass trap is not regression-locked.

7. **R7 (NIT) — §4.4.2 diagnostic message slot-suggestion.** The compile_error message should synthesize a slot suggestion from the field name + type. Current message names `credentials(slot: Type)` generically; better message names the specific slot suggestion (`credentials(slack: SlackToken)`) inferred from the `pub slack: CredentialRef<SlackToken>` field. 5-LOC change in the proc-macro emit.

8. **R8 (NIT) — §4.1 zero-credential canonical example.** Add a §4.1 inline example showing the **no-zones** action shape (`#[action(key = "ex.echo", name = "Echo")] pub struct EchoAction { pub message: String }`). Phase 1 newcomer first-action shape is zero-credential; the §4.1 example only shows the credential-bearing shape.

9. **R9 (NIT) — §4.1 trailing-comma policy.** Add one sentence to §4.1: "Trailing commas are accepted both inside zones and at the top-level attribute list." Avoids round-trip to `cargo check`.

10. **R10 (NIT) — §4.4.3 hand-roll direction commit.** §4.4.3-1 Open Item should commit the direction (`Recommended: seal at CP3 §9`) rather than leaving the choice fully open. CP3 picks the timing; CP2 commits the direction.

---

## Summary

**Verdict: RATIFY-WITH-NITS.** §4 closes the four CP1-flagged macro-syntax preview risks cleanly: (1) zone-syntax single-form lock with §4.1.3 invariants and `compile_error!` rejection; (2) dual-enforcement diagnostic with span attribution to user code (not macro internals); (3) Phase 0 C2 `parameters = Type` broken-emission fix using the existing `with_schema` builder API + Probe 7 regression-coverage; (4) Phase 1 finding 6 silent-string-drop replaced with hard `compile_error!` and codemod migration path. §5 harness sits in macro-crate-internal test surface; **zero impact on plugin authors writing actions** — they don't add fixtures, they don't update snapshots, the dev-deps are internal-only.

**Top 2 DX concerns:** (1) §4.1.3 cross-zone slot-name collision invariant missing — implementer needs to enumerate this fourth invariant or downstream rustc emits a less-helpful `E0428` (R1, blocking); (2) §5.4 has a probe for the *qualified*-form `<SchemeGuard<'_, C> as Clone>::clone(...)` violation but **no probe for the unqualified `guard.clone()` author-trap shape** — the silent-pass risk §5.4 explicitly names is not regression-locked (R6, non-blocking but high-impact).

Three CP1 §2 blocking gaps (R1 `ActionSlots` definition gap / R2 §2.1 doc-comment "blanket impl" wrong / R3 sealed-DX migration target gap) are **not** in CP2 scope — they live in §2 which is locked at CP1 freeze. Flagging here in case CP4 cross-section pass needs to reconcile, but CP2 §4 cannot fix them unilaterally.

---

*Reviewer: dx-tester (newcomer-persona DX of §4 macro emission + §5 harness from plugin-author angle). Parallel review with spec-auditor / tech-lead / rust-senior / security-lead per Strategy §6.3 reviewer matrix. No overlap with rust-senior idiom review (different lens — author-facing diagnostic discoverability vs idiomatic emission shape). Phase 1 finding handles cited inline (CC1–CC5 + finding numbers from `02a-dx-authoring-report.md`). CP1 review (`08d-cp1-dx-review.md`) flagged three CP2 macro-syntax preview concerns; §4 closes all three structurally with the §4.1.3 / §4.4.2 / §4.7.2 invariants.*
