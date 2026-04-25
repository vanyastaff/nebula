---
reviewer: dx-tester
mode: parallel review (Phase 6 CP1)
date: 2026-04-24
target: docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md (DRAFT CP1, §0–§3)
slice: DX of §2 trait contract from plugin-author perspective
parallel-with: spec-auditor + tech-lead + rust-senior + security-lead (Phase 6 CP1 reviewer matrix)
budget: 25 min
---

## Verdict

**REVISE.** The §2 signatures are technically correct against `final_shape_v2.rs`, and the underlying redesign genuinely closes the Phase 1 friction (CR1, CR2, CR3, CR9, semver re-export gap, etc.). But the **§2 narrative as written cannot be absorbed by a newcomer in one read**. Three concrete defects gate this conclusion: (1) `ActionSlots` is the load-bearing supertrait of `Action` and is referenced 4× in §2.1, §3.1, §3.2, but is **never defined** in the spec — its location is forward-promised to "§2.6 with the sealed DX traits" and §2.6 does not contain the definition; (2) `Action::metadata(&self) -> &ActionMetadata` requires a non-trivial method body, so the §2.1 doc-comment claim that "User code does NOT implement `Action` directly... the `#[action]` macro emits the blanket impl" is misleading — the macro emits a *concrete* impl per action, not a blanket; and (3) the canonical authoring example a plugin author would search for ("how do I declare a credential field; what does my struct look like under `#[action]`; what does `ctx.credential::<S>(key)` actually look like in §2 / §3 land") is **absent from §2 and §3** — the spec freezes signatures but never assembles them into a single end-to-end author-facing snippet. Phase 1 measured 32 min to a credential-bearing action; §2 as written does not show me the path to the <5 min target structurally.

Severity is REVISE, not REJECT — fixes are localized (define `ActionSlots`; correct the §2.1 doc comment; add a §2.0 / §2.9 author-facing primer); the underlying decisions (sealed DX, RPITIT, BoxFut alias, ActionResult variant gating) are sound.

---

## §2.2 newcomer signature absorbability

### What works

- **RPITIT body shape** (`-> impl Future<Output = ...> + Send + 'a`) is the right call for 1.95 — newcomer sees `fn execute<'a>(&'a self, ctx, input) -> impl Future<...>` and reads it linearly. No `async-trait` / `Box<dyn Future>` opacity in the **primary** trait surface. This closes the Phase 1 §1.7 verbose-`ctx: &(impl ActionContext + ?Sized)` jar — `ctx: &'a ActionContext<'a>` is concrete and one read.
- **Bound chain is consistent across all four primaries.** `Input: HasSchema + Send + 'static`, `Output: Send + 'static`, `Error: std::error::Error + Send + Sync + 'static`. After reading one trait, the next three are predictable. This is a real improvement over today's surface where Phase 1 had to grep `has_schema.rs` to discover the bound (Action 1 finding 7).
- **Probe-cited associated types** (`type Resource: Resource`, `type Source: TriggerSource`) carry forward the spike's compile-fail evidence (Probes 1+2). A newcomer who omits one gets `E0046: missing: Resource` — actionable diagnostic, no hand-holding required.
- **`Output: 'static` justification is in the prose** (line 139 — "for serialization through the engine's port projection"). Phase 1 Action 1 lookup #2 was caused by exactly this absence in v2 spec; CP1 closes it.

### What's missing for newcomer absorbability

1. **`ActionSlots` is referenced 4× and never defined.** `Action: ActionSlots + Send + Sync + 'static` (line 112), "carries `credential_slots() -> &'static [SlotBinding]`" (line 117), `ActionSlots::credential_slots()` (line 422), "the macro emits... `ActionSlots` impl" (ADR-0036 line 50). The §2.1 prose forward-promises to §2.6: "`ActionSlots` (defined §2.6 with the sealed DX traits)." But **§2.6 contains no `pub trait ActionSlots {...}` block** — only `mod sealed_dx { ... }` and the 5 DX traits. A newcomer reading §2 in order will reach the end without ever seeing the trait shape they apparently must implement. The macro emits the impl, but per ADR-0037 §1 the emission shape is `impl ActionSlots for SlackBearerAction { fn credential_slots() -> &'static [SlotBinding] { ... } }` — meaning the trait must look something like `pub trait ActionSlots { fn credential_slots() -> &'static [SlotBinding]; fn resource_slots() -> &'static [ResourceBinding]; }`. That signature is nowhere in §2. **REQUIRED EDIT.** This is the biggest gap.

2. **§2.1 doc-comment claim "the macro emits the blanket impl" is wrong.** The doc comment at line 106-111 says "the `#[action]` macro emits the blanket impl from one of the four primary dispatch trait impls below." But `Action` has a non-trivial method `fn metadata(&self) -> &ActionMetadata;` — a *blanket* impl `impl<T: StatelessAction> Action for T` cannot supply `metadata()` because `StatelessAction` doesn't expose it. The macro must emit a **concrete** `impl Action for SlackBearerAction` per action, threading metadata from the action's own `ActionMetadata` instance (probably built from `#[action(name=..., version=...)]` attribute fields per ADR-0037 §1). The "blanket" wording either (a) needs to change to "concrete impl per action" or (b) reflects an unreflected design choice (move `metadata` to `ActionSlots::metadata()` so `Action` itself becomes blanket-impl-able). Either way, the doc comment as written would mislead a plugin author trying to write the impl by hand.

3. **No §2.0 / §2.9 author-facing primer ties §2.1–§2.7 together.** §2 freezes 13 separate signature blocks across 8 subsections. A newcomer reading §2 has no equivalent of "here's what one full Stateless+Bearer action looks like end-to-end." Phase 1 measured 8 lookups to write Action 3; if §2 added a 30-line "complete authoring shape" snippet (struct + `#[action(...)]` attribute + `impl StatelessAction for X { ... }` + `ctx.credential::<BearerScheme>(...)` body line) the newcomer's mental model collapses from "13 signatures across 4 trait families" to "one canonical example." §7 / §9 likely fill this in CP3, but CP1 §2 is the freeze gate — the absence here is felt now. **RECOMMEND** adding §2.0 "What a complete action looks like (forward-pointer to §7)" — even just the struct + attribute + `impl ... { fn execute ... }` skeleton would absorb 90% of the discoverability surface that Phase 1 spent 32 minutes finding.

4. **`ActionMetadata` is referenced (line 113) but never defined or forward-pointed.** Phase 1 Action 1 finding 6 (`with_parameters` non-existent method) was rooted in `ActionMetadata` API drift. §2 freezes `Action::metadata()` returning `&ActionMetadata` without saying where `ActionMetadata` is defined or what fields it carries. Even a `// defined in crates/action/src/metadata.rs` comment or a forward-pointer to §7 (assuming CP3 will lock metadata shape) would prevent the silent assumption that "the macro builder is canonical." Per ADR-0036 line 49, the macro emits `Action` impl with metadata threaded through — but if metadata's shape isn't locked in §2, the macro emission is locked against an undefined-in-spec target.

### Severity tag

§2.2 newcomer absorbability — **REVISE**. The four primaries themselves are clean; the supertrait + macro narrative gap is the friction.

---

## §2.6 sealing impact on plugin DX

### What works

- **Sealed-DX architecture is the right call.** Per ADR-0038 §1, sealing closes the canon §3.5 governance drift cleanly. A community plugin author cannot accidentally extend the dispatch surface by impling `ControlAction for X` — they get a `private_in_public` error or `unimplemented sealed trait` diagnostic that points them at the primary trait.
- **`mod sealed_dx { pub trait XSealed {} ... }` pattern reads** as a recognized Rust idiom in 2026; experienced plugin authors will recognize the per-capability inner-sealed-trait shape.

### What blocks plugin DX

1. **Discovery of "what does a community plugin author do instead" is implicit, not surfaced in §2.6.** §2.6 says "community plugin crates may NOT implement [the DX trait] directly; they go through the underlying primary trait + adapter." But the **adapter pattern is not shown** in §2.6 — only the sealed-DX traits and a hand-wave to "registration time" (line 322). A plugin author hitting `error: trait ControlAction is sealed` will read §2.6, see the seal, and have no idea what to do next. The spec must either:
   - **(a)** Show the adapter call site explicitly: "Community plugins implement `StatelessAction` and use `register_with_control_flow(action)` to gain `ControlAction` semantics" — concrete API surface; OR
   - **(b)** State that **community plugins do not need any of the DX traits** because the primary `StatelessAction` covers all use cases that `ControlAction` adds DX sugar on top of. If (b), say so explicitly: "DX traits are crate-internal authoring sugar; external plugin authors ship `StatelessAction` directly and the primary trait covers all dispatch paths." This kills the question dead.
   - Right now §2.6 leaves the plugin author in limbo. ADR-0038 §1 mentions an "adapter pattern" once and ADR-0038 NEGATIVE §4 says "code that today does `impl ControlAction for X` must move to `impl StatelessAction for X` + sealed adapter" — but §2 never shows this migration target. **REQUIRED EDIT** — clarify the (a) vs (b) story.

2. **`PaginatedAction` / `BatchAction` use cases are exactly the patterns that today's plugin ecosystem *would* want sealed access to.** Phase 1 Action 2 (GitHub list-issues paginator) was the canonical "I want to write a paginator" flow. Today's plugin author writes `impl PaginatedAction for ListGithubIssues` + activates with `impl_paginated_action!(ListGithubIssues)` (Phase 1 finding 2.2). Post-seal, the author must instead write `impl StatefulAction for ListGithubIssues` and **manually re-derive** the pagination state-machine logic that `PaginatedAction` automated. **Is the macro going to emit equivalent state-machine logic from `#[action(paginated(...))]`?** §2.6 doesn't say. If yes, this is fine — DX is preserved through the macro. If no, sealing `PaginatedAction` removes a real authoring shortcut for community plugins. **RECOMMEND** §2.6 explicitly state the macro-DX path: "Community plugins author paginated actions via `#[action(paginated(cursor=..., page_size=...))]`; the macro emits the `StatefulAction` impl with the iteration state machine; the sealed `PaginatedAction` trait is engine-internal." Without that promise, sealing `PaginatedAction` is a Phase 1 regression for community paginator authors.

3. **`#[non_exhaustive]` on `ActionResult` (line 348) is right; absence of similar non-exhaustive markers elsewhere is inconsistent.** `ActionHandler` enum (line 271) is `#[non_exhaustive]` — good. But `Capability` enum (line 441) is not — and per ADR-0038 sealing the DX tier is exactly the case where `Capability` will likely grow (e.g., adding `MutualTLS` capability in a future cascade). **NIT** — add `#[non_exhaustive]` to `Capability` and `SlotType`. This is a forward-compat hedge, not load-bearing for CP1.

### Severity tag

§2.6 sealing impact — **REVISE**. The seal itself is correct; the plugin-author migration target is not surfaced.

---

## §2.7 Retry/Terminate variant discoverability

### What works

- **§2.7.1 wire-end-to-end pick is principled.** The four-evidence list (S3 false-capability, tech-lead solo decision, Strategy §4.3.2 symmetric-gating, observability-as-completion) is the strongest decision rationale in §2 — a tech-lead reading this gets the symmetric-gating principle in one paragraph. Good.
- **Feature-flag gates are surfaced explicitly in the enum signature** (line 377-378, 390-391), with `cfg_attr(docsrs, doc(cfg(...)))` for nice rustdoc rendering. A plugin author reading the rustdoc will see `Retry` / `Terminate` annotated as gated. This is an improvement over today's `result.rs:217` "Phase 3 is not yet wired" inline note (Phase 0 finding S3).

### What's unclear for plugin-author choice

1. **Default selection rule between Continue / Skip / Drop / Break / Branch / Route / MultiOutput / Wait / Retry / Terminate is not stated anywhere in §2.7.** §2.7.2 lists 11 variants. A plugin author writing `fn execute(...) -> Result<ActionOutput<T>, Error>` historically returned `Ok(output)` and the engine wrapped it in `ActionResult::Success`. Now the action body returns `Self::Output` per §2.2.1, but `ActionResult` shows up as the "engine surface" — *when does a plugin author construct an `ActionResult` directly vs let the engine wrap?* The §2 spec doesn't say. Phase 1 Action 2 finding 2.1 was caused by `r#continue` vs `continue_with` naming drift, but the deeper question — "do I return `ActionResult` from my body or do I return `Self::Output`?" — is not surfaced. From §2.2.1 the answer is "you return `Result<Self::Output, Self::Error>`; engine handles the rest." If that's universal, why is `ActionResult` a public type at all? Plugin authors probably need it for `ControlAction` (skip / continue / break) — but §2.6 sealed `ControlAction`. **REQUIRED EDIT** — §2.7 must state explicitly: "Plugin authors of `StatelessAction` / `StatefulAction` / `TriggerAction` / `ResourceAction` return `Result<Self::Output, Self::Error>`; the adapter wraps in `ActionResult::Success` automatically. `ActionResult` other variants are produced by the macro / sealed DX adapters / the engine — not by plugin code." If that statement is wrong, the spec needs to clarify which primary signature variants surface to plugin authors directly.

2. **Retry vs Terminate vs Continue intent is implicit.** §2.7.2 doc-comments say:
   - `Continue` (line 353): "iteration progress" (implied — variant name only; no doc comment in the enum)
   - `Retry` (line 376): "Re-enqueue for engine-driven retry" — clear
   - `Terminate` (line 384): "End the whole execution explicitly" — clear
   But what about `Continue` semantics in a paginated body — does it mean "I'm not done; call me again with the same state" (which is what `PaginatedAction` would want) or "execute the next node in the workflow" (which is what graph-flow semantics would suggest)? `progress: Option<f64>` field hints at the former, but the variant docstring is missing. **NIT** — add 1-line doc comments to each variant inline. Phase 1 Action 2 lookup #2 was caused by `result.rs` being the only place to discover `continue_with`; signature-level docstrings would close that. Especially `Skip` / `Drop` / `Break` — these names are not self-explaining for a newcomer.

3. **Feature gate granularity (single `unstable-action-scheduler` vs separate `unstable-retry-scheduler` + `unstable-terminate-scheduler`) is open item §2.7-1.** That's CP3 §9 scope per the spec, fine. But **the open item should be flagged in §2.7.2's enum signature itself**, not just at the bottom of §2 in the open-items list. A plugin author reading the enum signature will see `unstable-retry-scheduler` on `Retry` and `unstable-action-scheduler` on `Terminate` and assume the latter subsumes the former — which is wrong; CP3 may unify them. **RECOMMEND** comment line "// open item §2.7-1: feature-flag granularity may unify into `unstable-action-scheduler`; CP3 §9 picks" inline at line 376 and line 390.

### Severity tag

§2.7 Retry/Terminate discoverability — **REVISE-WITH-NITS**. The decision rationale is strong; per-variant author-intent guidance is missing.

---

## CP2 §4 macro syntax DX preview (concerns)

§4 is CP2 scope, but CP1's §2 and ADR-0036 / 0037 already constrain enough of the macro surface that DX concerns are visible now. Flagging for CP2 readers:

1. **`#[action(credentials(slot: Type), resources(slot: Type))]` zone syntax** (per ADR-0036 §Decision item 1) is novel — it doesn't follow Rust's existing attribute-macro idioms (compare `#[serde(rename = "...")]` flat-key style or `#[derive]`'s nested-list style). A plugin author seeing `#[action(credentials(slack: SlackToken), resources(db: PgPool))]` for the first time has to mentally parse it as **inner-attribute-as-zone-declaration**, not flat key=value. Phase 1 lookup count for Action 3 was 8; the credential-attribute parsing was ~3 of those lookups. The new syntax may regress this if the **parse rule is not stated upfront in CP2 §4**. **CP2 RISK**: if the macro permits `#[action(credentials(slot1: T1, slot2: T2))]` AND `#[action(credentials(slot1: T1), credentials(slot2: T2))]` AND `#[action(credentials = "slot1")]` (legacy compatibility?), the syntax surface explodes. CP2 §4 must lock exactly one form and reject the others with macro-time `compile_error!`. Phase 1 §6 finding 6 (silent string-form drop) is the precedent — the macro accepting an alternate form silently is the worst category of bug.

2. **Field-zone rewriting visibility (per ADR-0036 §Negative item 4 + Alternative B rejection).** The promise is "fields outside `credentials(...)`/`resources(...)` are not rewritten." Good — that preserves LSP/grep/rustdoc semantics. But the **field zone itself rewrites the type** (per ADR-0036 §Decision item 4 — `CredentialRef<dyn BitbucketBearer>` → `CredentialRef<dyn BitbucketBearerPhantom>`). A plugin author hovering over the rewritten field in their IDE will see `CredentialRef<dyn BitbucketBearerPhantom>` in tooltips, but their source code says `CredentialRef<dyn BitbucketBearer>`. This is acceptable per ADR-0036 (the rewrite is opt-in and zone-bounded) but **CP2 §4 must surface this in author-facing documentation**: "When you write `credentials(slack: dyn BitbucketBearer)`, the macro rewrites your field's generic argument to `dyn BitbucketBearerPhantom` for safety. Your hover / rustdoc will show the rewritten type. This is intentional per ADR-0035." Without that note, the IDE-divergence question hits at first hover.

3. **`#[derive(Action)]` removal codemod is named (Strategy §6.1, ADR-0036 §Negative item 1) — but a `compile_error!` redirect for the legacy attribute is not in any spec I can find.** A plugin author following an old README example will write `#[derive(Action)]` and get `cannot find derive macro Action` — uninformative. **CP2 §4 RECOMMEND**: emit a `proc_macro_attribute` shim named `derive_action` (or wrapper) that simply emits `compile_error!("`#[derive(Action)]` was replaced by `#[action]` in vX.Y; see codemod at <link>")`. One-line dx fix that reuses existing tokens.

4. **Macro emission probe gates feel right but trybuild output stability is fragile across compilers.** ADR-0037 §4 ports 6 probes from the spike, asserting `E0277` / `E0046` / etc. Phase 1 noted (Section 5) that no trybuild/macrotest harness existed — fixing this is good. But Rust 1.95+ diagnostic rendering is in flux (e.g., probe 6 already notes `E0277 subsumes E0271 per Rust 1.95 diagnostic rendering`). **CP2 RISK**: trybuild golden files are notoriously brittle across rustc patch versions. Recommend ADR-0037 §4 add: "Probes assert error code only (e.g., `E0277`), not full diagnostic text. Diagnostic message snapshots use `insta::assert_snapshot!` with explicit reviewer step on rustc upgrades."

---

## Required edits (if any)

CP1 ratification gates on these edits before §2 freeze:

1. **R1 (REQUIRED — blocking) — Add `pub trait ActionSlots { ... }` definition to §2.1 or §2.6.** Currently the trait is the load-bearing supertrait of `Action` and is referenced 4× without definition. Cite credential Tech Spec §3.4 line 851-863 for the body. Suggested location: end of §2.1 (just after `pub trait Action`) so the supertrait is co-located with its consumer. Required signature shape (inferred from ADR-0037 §1 line 49-60):
   ```rust
   /// Slot bindings emitted by the `#[action]` macro. User code does
   /// NOT implement this directly — the macro emits the impl from the
   /// `credentials(...)` and `resources(...)` zones.
   pub trait ActionSlots {
       fn credential_slots() -> &'static [SlotBinding];
       fn resource_slots() -> &'static [ResourceBinding];  // CP3-locked shape
   }
   ```

2. **R2 (REQUIRED — blocking) — Correct the §2.1 doc comment at line 106-111.** "the `#[action]` macro emits the blanket impl" is wrong because `Action::metadata` is a non-trivial method that requires per-action metadata. Replace with: "the `#[action]` macro emits a concrete `impl Action for X` per action, threading `ActionMetadata` from the attribute fields (`#[action(name=..., version=..., parameters=...)]`); user code does not write `impl Action for X` by hand." Or alternatively, move `metadata()` to `ActionSlots` so `Action` becomes truly blanket-impl-able from the four primaries — but that's an architectural call, not a doc fix.

3. **R3 (REQUIRED — blocking) — §2.6 must state the community-plugin-author migration target.** Currently §2.6 seals 5 DX traits and waves at "adapter pattern at registration." A plugin author has no way to know what to do with their existing `impl ControlAction for X`. Add 1 paragraph at end of §2.6:
   > **Community plugin authoring path.** External plugin crates do not implement any sealed DX trait directly. Pagination / batch / control-flow / webhook / poll patterns are authored via `StatelessAction` or `StatefulAction` primary + `#[action(paginated(...))]` / `#[action(control_flow=...)]` macro attribute zones (CP2 §4-locked). The macro emits the appropriate adapter; the engine erases to the primary at dispatch. See §7 (CP3) for end-to-end community-plugin example.

4. **R4 (RECOMMENDED — non-blocking) — Add §2.0 "Authoring shape at a glance" with one canonical example.** 30-line snippet showing struct + `#[action(name=..., version=..., credentials(slack: SlackToken))]` + `impl StatelessAction for X { type Input = …; … fn execute … }` + body line `let bearer: &BearerScheme = ctx.resolved_scheme(&self.slack)?;`. Anchor for the §2.1–§2.7 signatures. Explicit forward-pointer to §7 (CP3) for full author flow. Phase 1 measured 8 lookups for Action 3 — a §2.0 primer collapses 5–6 of those.

5. **R5 (RECOMMENDED — non-blocking) — §2.7.2 inline doc comments per variant.** Each enum variant gets a 1-line comment stating the author intent (`Skip` / `Drop` / `Break` / `Continue` are particularly opaque to newcomers). Inline comment is cheaper than a separate §2.7.x subsection.

6. **R6 (RECOMMENDED — non-blocking) — §2.7.1 add explicit "what plugin authors return" rule.** Sentence near top of §2.7: "Plugin authors of the four primary traits return `Result<Self::Output, Self::Error>` from `execute(...)`; `ActionResult` variants are constructed by the macro / sealed adapters / engine, not by plugin code directly." This kills the "do I return ActionResult or my Output" question dead.

7. **R7 (NIT) — Add `#[non_exhaustive]` to `Capability` enum (line 441) and `SlotType` enum (line 435).** Forward-compat hedge for sealed-DX growth (e.g., `MutualTLS` capability in future cascade). Cheap.

8. **R8 (NIT) — Annotate `unstable-action-scheduler` vs `unstable-retry-scheduler` open-item §2.7-1 inline at line 376 and line 390** with `// open item §2.7-1: granularity may unify; CP3 §9 picks`. Avoids a plugin author misreading the gates.

---

## Summary

**Verdict: REVISE.** The §2 trait contract is technically correct against the spike artefacts and closes the Phase 1 friction class — RPITIT bodies, single-`'a` `BoxFut` alias, sealed-DX governance fix, wired Retry/Terminate symmetry. Plugin-author absorbability is gated on three localized fixes:

1. **`ActionSlots` is undefined in §2** despite being the supertrait of `Action` and the slot-emission target of `#[action]`. R1 is blocking — without a `pub trait ActionSlots {...}` block somewhere in §2, the spec freezes a contract whose load-bearing trait is not in the freeze.
2. **Sealed-DX migration target is implicit.** R3 is blocking — community plugin authors hit `error: trait ControlAction is sealed` and §2.6 doesn't tell them what to do next. The macro-attribute path (`#[action(paginated(...))]` etc.) is mentioned in passing but never named in §2; CP2 §4 will lock it but CP1 §2 freezes the trait surface that strands the author.
3. **CP2 §4 macro syntax DX is the next concrete risk.** Three concerns surface from the §2 + ADR-0036/0037 framing alone: (a) zone-syntax parse rules must be exactly one form (not three legacy forms accepted silently — Phase 1 finding 6 precedent); (b) field-rewrite IDE-divergence at hover; (c) trybuild golden-file fragility on rustc upgrades.

**Top 2 DX concerns:** (1) `ActionSlots` definition gap (R1); (2) sealed-DX community-plugin migration target gap (R3). Both blocking; both fix in <30 LOC of spec edits.

**CP2 macro syntax preview concerns:** zone-syntax parse-rule ambiguity (single form must be locked, with `compile_error!` rejection of alternates per Phase 1 silent-drop precedent); field-zone rewrite IDE-divergence needs an author-facing note; trybuild probes should assert error codes only (not snapshot diagnostic text) for rustc-upgrade resilience.

---

*Reviewer: dx-tester (newcomer-persona DX of §2 trait contract). Parallel review with spec-auditor / tech-lead / rust-senior / security-lead per Strategy §6.3 reviewer matrix. No overlap with rust-senior idiomatic review (different lens — newcomer absorbability vs idiom currency). Phase 1 friction handles cited inline (Action 1 / Action 2 / Action 3 lookup counts and finding numbers from `02a-dx-authoring-report.md`).*
