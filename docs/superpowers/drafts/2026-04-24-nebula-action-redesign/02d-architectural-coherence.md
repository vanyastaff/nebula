# Phase 1 — Architectural coherence (tech-lead)

**Date:** 2026-04-24
**Author:** tech-lead (sub-agent, priority calls + big-picture shape)
**Mode:** Solo decider on priority calls; **co-decision participant** on Action × Credential integration (architect + security-lead parallel authority — positions below are *inputs* to Phase 2, not verdicts).
**Inputs:** Phase 0 consolidated (`01-current-state.md`), grounding grep on `crates/action/src/` + `crates/action/macros/src/` + `crates/credential/` (see §8).

Severity legend: 🔴 STRUCTURAL (wrong shape at arch level) / 🟠 COHERENCE (reasonable but not clean) / 🟡 PREFERENCE.

---

## 1. Trait hierarchy weight assessment (10 surfaces — justified or bloat?)

### Ground truth (from grep, not memory)

The engine's dispatch enum `ActionHandler` has **4 variants**: `Stateless`, `Stateful`, `Trigger`, `Resource` (`handler.rs:41-50`). The **public trait surface** is 10:

| Surface | Kind | Erases to (dispatch) | Erasure mechanism |
|---|---|---|---|
| `Action` | base trait (identity/metadata) | — | supertrait |
| `StatelessAction` | primary | `StatelessHandler` (Arc<dyn>) | `StatelessActionAdapter` |
| `StatefulAction` | primary | `StatefulHandler` (Arc<dyn>) | `StatefulActionAdapter` |
| `TriggerAction` | primary | `TriggerHandler` (Arc<dyn>) | `TriggerActionAdapter` |
| `ResourceAction` | primary | `ResourceHandler` (Arc<dyn>) | `ResourceActionAdapter` |
| `ControlAction` | DX | `StatelessHandler` | `ControlActionAdapter` (struct-level adapter, `control.rs:484-510`) |
| `PaginatedAction` | DX | `StatefulHandler` | `impl_paginated_action!` macro emits `StatefulAction` impl (`stateful.rs:170-227`) |
| `BatchAction` | DX | `StatefulHandler` | `impl_batch_action!` macro (same family) |
| `WebhookAction` | DX | `TriggerHandler` | `WebhookTriggerAdapter` (`webhook.rs:1074`) |
| `PollAction` | DX | `TriggerHandler` | `PollTriggerAdapter` (`poll.rs:1290`) |

**Every DX trait erases to one of the four dispatch-time handler dyn traits.** The engine never sees a `ControlAction`, `PaginatedAction`, `WebhookAction`, etc. — it sees `Arc<dyn StatelessHandler>` / `Arc<dyn StatefulHandler>` / `Arc<dyn TriggerHandler>`. The 4 primary + 5 DX layering is a **compile-time DX convenience**, not a dispatch-time extension.

### Weight call per trait

**🟢 Carry weight (keep):**
- `Action` — required supertrait, identity + metadata
- `StatelessAction` / `StatefulAction` / `TriggerAction` / `ResourceAction` — the four canon-blessed dispatch primaries; each genuinely covers a distinct execution shape (one-shot vs iterate vs external-start vs scoped-DI)

**🟠 Carry weight but poorly framed (reconsider framing, keep semantics):**
- `ControlAction` — **carries DX weight** (the `evaluate → ControlOutcome` shape compiles "If / Switch / Router / Stop / Fail" down to a clean trait body instead of a hand-written `execute` that re-implements outcome mapping for every node). But the **public-non-sealed-trait framing is wrong**: dispatch-wise it IS a `StatelessAction`. ControlAction is a **helper-masquerading-as-trait** for authoring convenience. The right framing is "seal + demote to DX helper" OR "promote to 5th dispatch primary with engine `ActionCategory::Control` first-class handling." Current middle state (public + non-sealed + adapter-erasure) is the worst of both — it exposes a 5th dispatch-time trait to consumers while not giving them 5th-dispatch-time semantics. **Canon §3.5 violation is real.**
- `PaginatedAction` / `BatchAction` — **carry DX weight** for specific iteration patterns (cursor-driven, chunked-items). But they are **pure `macro_rules!` sugar** over `StatefulAction` — the "trait" is just a shape contract for the macro's input. A user can get 100% of the behavior by writing `StatefulAction` directly; the DX win is maybe 20 LOC per action. Collapsing into `StatefulAction` helper functions + patterns doc would land the same ergonomics without the extra trait surfaces.

**🔴 DX weight marginal at current form (but not dismissable):**
- `WebhookAction` / `PollAction` — these DO carry structural weight beyond `TriggerAction`:
  - `WebhookAction` threads **signature-policy enforcement at the trait surface** (`WebhookConfig::signature_policy` = Required-by-default, `webhook.rs:35-42`) and **typed `WebhookRequest` with body-size/header-count limits at boundary** — that's a security-critical invariant the adapter enforces before the action body runs. Can't cleanly reduce to `TriggerAction + helpers` without losing the trait-surface enforcement point.
  - `PollAction` threads **interval-floor enforcement + warn-throttle + dedup cursor** — invariants you want checked once in the adapter, not re-invented per poll action.

  **However**, the surface overlap with `TriggerAction` is substantial (both have lifecycle register/handle/unregister) and the shape is "TriggerAction + transport-specific invariant enforcement." A `TriggerAction` + `TriggerTransport<T>` parameterization could push the transport-specific invariants into a transport trait and keep one dispatch + one authoring trait. Non-trivial refactor; not obviously a win.

### Verdict

**🟠 COHERENCE issue, not 🔴 STRUCTURAL.** 10 surfaces is on the high end but justified *for every trait except `ControlAction`*. The real structural issue is **canon §3.5 says 4 and code ships 10** without the canon revision or ADR §0.2 requires. That's a documentation/governance drift, not a code shape drift.

**Recommendation (Phase 2 input, not decision):** Do not dissolve the DX layer. Do ratify it with either (a) canon §3.5 revision to enumerate the DX tier as "erases to primary" OR (b) seal DX traits + document them as adapter-patterns-with-trait-shape. `ControlAction` specifically needs the seal-or-canonize call — current state is indefensible under `feedback_adr_revisable` (we are patching around a canon rule with a docstring that re-asserts it).

---

## 2. `#[action]` macro surface scope (too many / too few / right-sized)

### Ground truth

Current macro (`action_attrs.rs:10-33`) accepts 8 attribute keys: `key`, `name`, `description`, `version`, `parameters`, `credential`, `credentials`, `resource`, `resources`. It's a **`#[derive(Action)]`** (line 130 emits `.with_parameters(<T>::parameters())` — a method that doesn't exist on `ActionMetadata`, per Phase 0 C2). Emits:
- `Action` trait impl (identity + metadata)
- `DeclaresDependencies` impl (via `dependencies_impl_expr`, `action_attrs.rs:148-197`) listing credentials + resources by `TypeId::of::<T>()` + `CredentialLike::KEY_STR`

**Does NOT emit:** port declarations, `optional` credential variant, field-type rewriting, HRTB fn-pointers, schema-from-input-type bindings.

### The derive-vs-attribute question

A `#[derive(Action)]` **cannot rewrite field types**. This is a hard Rust constraint — derives see the struct as-is; they can emit trait impls next to it, nothing more. A `#[action(...)]` **attribute macro** can rewrite the struct body itself, but is more invasive (hides what the compiler sees from grep/LSP/goto-def).

CP6 Tech Spec vocabulary (`CredentialRef<C>` phantom with field-type rewriting) **requires an attribute macro.** A derive cannot produce it. This is not a stylistic choice — it's a language-level constraint.

### Call on "too many / too few / right-sized"

**🟠 Surface scope is right-sized *for current idiom*; wrong-sized *for CP6 idiom*.**

The 8-attribute-keys derive is an acceptable size for a derive. The problem is not width (8 keys), it's:

1. **🔴 Emission correctness** — C2 (broken `parameters = Type` path — emits a method that doesn't exist) is a latent bug the macro harness gap (T1) has been masking. Any redesign pass fixes this by contract.
2. **🟠 Attribute/derive mismatch for CP6** — if Option A (adopt CP6 vocabulary) is picked, the derive shape structurally cannot emit what the spec asks. You would need to **add** an attribute macro (or convert).

### The DX-discoverability question

User feedback (`feedback_hard_breaking_changes` — hard breaks OK for spec-correct outcomes) explicitly allows shape-breaking moves, but there's an independent DX concern:

- **Derive:** struct body is what the user wrote. Field types in the source = field types the compiler sees. Grep/LSP/goto-def works transparently. Cost: can't express CP6 phantom shape.
- **Attribute macro:** struct body can be rewritten. A user who writes `slack_token: CredentialRef<SlackToken>` might have the macro turn it into `slack_token: SlotBinding<SlackToken>` internally. Cost: silent rewriting hurts discoverability — goto-def on a field may land inside macro expansion, and "why does this field have a different type than I wrote" becomes a debugging class of bug. Pairs with the new-hire-test: "can a new contributor understand this in 30 minutes?" — not if the macro silently rewrites types.

**Recommendation (Phase 2 input):** If Option A is picked, choose attribute macro **but constrain the rewriting to a narrow, documented contract** — do not let the macro rewrite arbitrary fields; restrict to a specific attribute-tagged zone. Something like `#[action(credentials(slack: SlackToken))]` emitting phantom fields, rather than `slack: CredentialRef<SlackToken>` getting rewritten. Makes the rewriting explicit at the declaration site. Keeps LSP/grep honest.

### Verdict

**🟠 COHERENCE.** Current derive is serviceable but has one bug (C2) and cannot accommodate Option A. Derive-vs-attribute call is **downstream of Option A/B/C** — don't pick macro shape before the integration shape is picked.

---

## 3. Action × Credential × Resource coupling (Option A/B/C framed, not decided)

### Re-ground against current code (per `feedback_adr_revisable` — verify before recommending from spec)

**Grep on current `crates/credential/src/`:** `CredentialRef`, `SchemeGuard`, `SlotBinding`, `resolve_as_bearer`, `AnyCredential` — **zero matches**. The CP6 Tech Spec vocabulary exists entirely in `docs/superpowers/specs/2026-04-24-credential-tech-spec.md` and derivatives. Neither side of the boundary implements it yet.

This is load-bearing: Phase 0 C1 frames "action lacks CP6 vocabulary" — the reality is **neither crate has it.** The spec specifies shapes that are not yet realized. Action hasn't drifted from a shipped credential implementation; it's out-of-sync with a still-unimplemented spec.

This changes the Option A framing significantly.

### Option A — Action adopts CP6 vocabulary wholesale

**What it means in practice:** Action redesign lands **simultaneously** with credential crate's CP6 implementation (they have to, because the vocabulary doesn't exist anywhere yet). Action's new `#[action]` attribute macro emits `CredentialRef<C>` phantoms → credential crate owns the type → action's `SlotBinding` binding holds HRTB resolve_fn pointing into credential's `RefreshDispatcher`. Handler-bodies see `&C::Scheme` via `SchemeGuard<'a, C>` RAII.

**Cost:** One very large coordinated PR / one very large phased cascade. Migration order: leaf first (credential crate lands the types) → action adopts → engine bridges → plugins migrate. 7 direct reverse-deps × 69 files × 63 public items × ~40 through `sdk::prelude`. Plugin-ecosystem-wide breaking change.

**Win:** Single source of truth for credential shapes. Typed handles. Phantom-safety (can't access a credential you didn't declare; compile-time enforcement). Zeroization RAII. Spec-correct outcome.

**Risk:** We're coordinating two in-flight specs (credential Tech Spec CP6 + action redesign). Any slippage in credential cascades into action. The "critical path has two uncertain phases" risk is real.

### Option B — Action stays simpler; engine bridges

**What it means:** Action retains typed access but through a simpler interface (`ctx.credential::<C>(key) -> Result<CredentialGuard<C>, _>` — the C3 bug fixed with mandatory-key). No phantom-rewriting macro; no `SlotBinding` HRTB contract. Engine (or a new `credential-bridge` layer) owns the translation between plugin declarations and credential-crate's internal types.

**Cost:** Lower today. Macro stays as `#[derive(Action)]`. Plugin migration is "rename `credential_by_id(id)` and type-name-key usage to explicit-key `credential::<C>(key)`." Much smaller plugin blast radius.

**Win:** Fixes C3 (type-name-as-key heuristic) immediately. Unblocks action crate from waiting on credential CP6 landing. Smaller coordinated change.

**Risk:** Two vocabularies long-term — action speaks `CredentialGuard`, credential internals speak CP6. The "engine bridges" layer becomes a permanent translation point. Per `feedback_boundary_erosion`: a bridge layer that lives inside engine is one more place where responsibilities smear. **And: per `feedback_adr_revisable`, if following Option B forces workarounds later** (when engine has to emulate phantom-safety), that's the supersede signal.

### Option C — Escalate for partial spec deferral

**What it means:** Propose to credential spec owners: defer §§2.7/3.4/7.1/15.7 (phantom / SlotBinding / HRTB / SchemeGuard) to a future CP7; CP6 ships with stable `AuthScheme` + typed-key access but without phantom-rewriting. Action crate adopts a **CP6-minus** vocabulary that maps 1:1 to what credential ships.

**Cost:** Escalation-rule-10 territory — amends a frozen Tech Spec. Requires architect + security-lead + whoever owns credential CP6 to agree. Time cost: at least one co-decision cycle.

**Win:** Unblocks both cascades. Lets action ship a spec-compatible-subset shape now. Phantom-safety + RAII can land in CP7 when both crates are ready.

**Risk:** "Defer to CP7" is the exact "we'll fix it later" pattern that `feedback_incomplete_work` flags. Only valid if CP7 is scheduled, owned, and scoped, not just "eventually." Otherwise it's a polite Option B with a promise attached.

### tech-lead framing for Phase 2 (not a pick)

Three considerations I carry in, not a verdict:

1. **The "active dev mode" lens (`feedback_active_dev_mode`):** prefer more-ideal over more-expedient. Option A is most-ideal. Option B is most-expedient. Option C is middle.
2. **The "2am test":** which option has the highest probability of being wrong in a way that wakes someone up at 2am? Option B — because "engine bridges" means engine has to emulate phantom-safety at runtime (typed-key lookups that fail at runtime instead of compile time). That's a strictly-larger error surface. Option A trades compile-time strictness for coordination risk; the compile-time win is durable.
3. **The co-decision surface:** this is NOT a priority call I make alone. If security-lead's threat model weighs zeroization-RAII heavily (likely — `SchemeGuard` is an isolated failure point for secret lifetime), that tilts toward A. If architect's migration-risk framing weighs coordination-with-credential-CP6 heavily (likely), that tilts toward C. **My role in Phase 2 is as tie-breaker, not first-mover.**

**My position going into Phase 2 (per consensus-participant rules):** Lean A, with explicit fallback to C (not B). B is the "borrow from future-you" option per `feedback_hard_breaking_changes`; we have license to not take that path. Will not solo-commit in advance of architect + security-lead Phase 2 positions.

**Status:** 🔴 STRUCTURAL (decision outcome shapes the whole cascade) / Phase 2 co-decision owner: architect + tech-lead + security-lead.

---

## 4. TriggerAction cluster mode scope (action vs engine)

### Prompt framing

"Cluster mode coordination, leader election, reconnect, event idempotency" per credential Tech Spec §16.1 П10 — is this action's or engine's?

### Ground truth

Grep on `crates/action/` for `cluster|leader|election|reconnect|idempoten` (case-insensitive) — matches are **docstring references and error messages**, zero trait-surface or runtime-state surface. `crates/engine/` has matches in `engine.rs`, `control_consumer.rs`, `control_dispatch.rs` — these are intra-engine control/dispatch concerns, not cluster coordination.

**Nothing in either crate implements cluster coordination today.** Cluster mode is currently a spec-declared concern, not an implemented concern. (Same position as CP6 credential vocabulary — a spec promise.)

### Right scope call

Cluster mode is engine-layer, not action-layer. Reasoning:

- **Leader election** — cross-node state coordination. Requires consensus primitive (etcd / raft / database lease). Action is per-node; engine is the node-lifecycle owner. Putting leader election in action would require every action to carry consensus-client state. Wrong layer.
- **Reconnect** — transport concern. Webhook reconnect is HTTP-transport (action doesn't own the HTTP server; transport layer does). Poll reconnect is "schedule retry" (engine scheduler concern). Event stream reconnect (SSE, websocket, NATS, etc.) is transport.
- **Event idempotency** — cross-cutting. The *dedup key* is action-author knowledge (what makes this webhook request idempotent?), but the *store* is engine (durable cursor). This is an injected-capability shape, same pattern as `TriggerHealth` in memory `project_health_trait.md`.

### Architectural hooks `TriggerAction` + `TriggerHandler` need

What action-layer *should* expose (Phase 2 scope input):

1. **Idempotency-key declaration** — typed method on `WebhookAction` / `PollAction` that returns `IdempotencyKey` given an event. Engine stores, checks, dedupes before dispatching to action body. Action author supplies the domain knowledge; engine enforces.
2. **Lifecycle hooks for cluster transitions** — `on_leader_acquired` / `on_leader_lost` (optional, default no-op) on `TriggerAction`. Engine calls them during state transitions. Action can invalidate cursors, flush in-flight state, etc. This is `Resource::on_credential_refresh` pattern from minor-findings.
3. **Dedup window declaration** — in `ActionMetadata` (how long to retain idempotency keys). Policy, not runtime — sits next to `IsolationLevel`.

What action-layer *should NOT* expose: leader election trait, reconnect trait, cluster-topology trait. Those belong in engine's trigger-lifecycle orchestrator.

### Verdict

**🟠 COHERENCE, not blocker.** Action needs three small hooks (idempotency-key, lifecycle callbacks, dedup window metadata); engine owns the actual cluster coordination. **Phase 2 scope:** add the three hooks on `TriggerAction` + `TriggerHandler`. The cluster-coordination primitive itself is out of action scope — architect's engine-side Phase 2 input covers it.

---

## 5. Priority calls on load-bearing decisions (from Phase 0 §7 table)

Per Phase 0 §7 these are explicit Phase 2 options. Framing my positions per the brief (solo-decide where tech-lead is sole owner; input-only on co-decision items).

### Credential integration: A/B/C

**Status:** co-decision. Framed in §3 above. No solo call.

### ControlAction canon status

**Solo tech-lead priority call.**

```
Decision: Seal ControlAction + keep as internal DX helper (do NOT revise canon §3.5 to list 5 dispatch primaries).
Why: Dispatch-wise ControlAction IS StatelessAction (adapter erases to StatelessHandler at control.rs:484). Canon §3.5 is correct — 4 dispatch primaries. The public-non-sealed-trait status is a DX overreach, not a new dispatch category. Sealing it closes the "community plugins can implement this" door we accidentally opened without an ADR, and makes the DX tier a closed set owned by nebula-action.
Trade-off: Plugin ecosystem loses the ability to define new ControlAction variants. We believe this is fine — the existing variants (If, Switch, Router, Filter, NoOp, Stop, Fail) cover the graph-topology needs; adding new ones is a canon-level change, not a plugin concern.
Revisit when: A plugin actually needs a novel ControlAction variant AND the variant can't be expressed as a StatelessAction. Prediction: won't happen.
```

Severity: 🟠 COHERENCE.

### DX trait layer scope

**Solo tech-lead priority call (with architect + dx-tester input).**

```
Decision: Keep all 5 DX traits (Paginated, Batch, Webhook, Poll, Control), but seal them and ratify in canon as "erases to primary" tier.
Why: Grep confirms every DX trait erases cleanly to one of the 4 primaries (§1 table). The DX layer is well-tested and reduces per-action boilerplate by 40-80 LOC on average — real authoring win. Collapsing to helper functions loses the trait-surface enforcement point (WebhookConfig signature-policy default, PollAction interval floor). Sealing prevents external ecosystem extension without canon revision.
Trade-off: 10-trait public surface stays; requires canon §3.5 amendment (or explicit "DX tier" paragraph). Does NOT require dispatch-enum extension.
Revisit when: A new DX trait is proposed and can't cleanly slot into the "erases to primary" pattern — that's the signal the pattern is breaking down.
```

Severity: 🟠 COHERENCE.

### `#[action]` macro vs `#[derive]` shape

**Co-decision with architect + rust-senior.** Framed in §2. Downstream of Option A/B/C. No pre-emptive call.

### `ActionResult::Terminate` gating

**Solo tech-lead priority call.**

```
Decision: Feature-gate Terminate behind `unstable-terminate` (mirror the Retry pattern), AND in this redesign cascade, wire it end-to-end in the engine scheduler.
Why: Public-surface-without-engine-support violates canon §4.5 and is the exact "partial work in sibling crates" that feedback_active_dev_mode forbids. Retry is the 🟢 model cited in Phase 0 §6 — apply the same pattern. Active-dev mode says finish the partial work; don't just gate it and call it done. The scheduler wiring is ~engine-internal dispatcher work, not plugin-ecosystem-breaking.
Trade-off: Engine work in the cascade grows by ~engine-dispatcher scheduler changes. Bigger Phase 3+ scope. Acceptable because engine is the critical-path crate anyway (§7 says 27+ import sites touch engine).
Revisit when: The engine scheduler change turns out to be larger than one-PR size — then feature-gate-only is the fallback, with scheduler wiring filed as a tracked follow-up (not a TODO comment, a real issue).
```

Severity: 🟠 COHERENCE (asymmetric discipline is a clarity-compounding problem; Retry-shaped discipline is the standard).

### Lefthook parity policy

**Solo devops priority call; tech-lead input only.**

Input: Per `feedback_lefthook_mirrors_ci`, lefthook pre-push MUST mirror every CI required job. The current `lefthook.yml:45` comment ("Doctests/docs/MSRV remain CI-owned checks") directly contradicts user policy. This is a 🔴 policy violation, not a preference.

**My input to devops:** Fix the divergence. The user memory is explicit. This is not a trade-off to rearbitrate; it's already been arbitrated and the code diverged from the decision. Follow the user memory rule. Severity 🟠 COHERENCE (policy drift).

### `zeroize` workspace pin

**Solo devops priority call.**

Input: Migrate to workspace-inherited. Rationale is in Phase 0 T4 — inline drop of `std` feature + crypto-stack de-unification risk. Low-cost fix, high-value invariant (crypto dep unification is a security hygiene concern). Severity 🟡 PREFERENCE edging 🟠.

### Macro test harness

**Solo devops priority call.**

Input: Add both `trybuild` (compile-fail for attribute rejection rules) AND `macrotest` (expansion snapshot for the 9+ emission paths). C2 (broken `parameters = Type`) would have been caught by macrotest. 359-LOC proc-macro with 9+ rejection rules and zero expansion tests is a 🔴 coverage gap. Costs ~1 day of dev-dependency setup + snapshots. Post-redesign, snapshots become regression guard for the new `#[action]` surface.

---

## 6. Plugin ecosystem migration risk + sequencing assessment

### Blast radius recap (Phase 0 §8)

- 7 direct reverse-deps, 69 source files, 63 public items, ~40 cascaded via `sdk::prelude`, ~55 files touched by rename-only refactor
- `nebula-engine` (27+ import sites) and `nebula-sandbox` (7 files) are heaviest
- `sdk::prelude` is the officially-sanctioned user-contract surface

### Settled vs contested surface

**Settled (won't change materially in redesign):**
- `ActionError` hierarchy — taxonomy is stable and well-used
- `ActionMetadata` core fields (`key`, `version`, ports, schema) — shape is right even if `CheckpointPolicy` is pending
- `ActionResult` variants — `Continue`/`Break`/`Wait`/`Retry`/`Terminate` taxonomy is correct; only gating discipline needs alignment
- `StatelessHandler` / `StatefulHandler` / `TriggerHandler` / `ResourceHandler` dyn contracts (the 4 engine-side dispatch traits) — changing these would break sandbox ABI (out-of-process runner serialization)
- Port system core (`InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`) — missing `Provide` per Phase 0 S5 but the 4 existing kinds are settled
- `ValidSchema` / `Field` / `field_key` — re-exports from `nebula-schema`; stable

**Contested (will change):**
- `ControlAction` public trait status (seal decision above)
- `#[derive(Action)]` macro surface (derive-vs-attribute pending Option A/B/C)
- `CredentialContextExt` three-variants API (Phase 0 S4 + C3 — will change regardless of Option A/B/C path)
- `ctx.resource::<R>(key) -> Box<dyn Any>` untyped API (Phase 0 S4)
- Handler dyn-safety HRTB verbosity (Phase 0 S6 — likely cosmetic via `trait_variant::make` or RTN)
- `ActionResult::Terminate` gating (decision above)
- DataTag hierarchical registry (Phase 0 S5 — absent entirely; if it lands, it's new surface, not change to existing)

**Neither settled nor contested (new):**
- `IdempotencyKey` trait-surface for TriggerAction (§4)
- `on_leader_acquired` / `on_leader_lost` lifecycle hooks on TriggerAction (§4)
- `Provide` port kind (Phase 0 S5)

### Phased migration viability

**Can it be phased?** Yes, but the phases must respect a specific order.

**Critical-path dependencies:**
1. **Option A/B/C decision** blocks macro-shape decision blocks field-rewriting decision blocks plugin migration. This is one sequenced causal chain; you can't parallelize across it.
2. **Credential CP6 implementation landing** (currently not-yet-implemented per §3 grep) blocks Option A. If Phase 2 picks A, action cascade has to wait for or co-land with credential.
3. **Engine dispatcher changes** (ControlAction seal, Terminate wiring, idempotency-key handling) can go first — they don't touch plugin surface.
4. **sandbox ABI** (dyn-handler serialization) should not change in this cascade if possible; if it does, that's a separate sub-cascade with its own review gate.

### Proposed sequencing

**Phase 3A — Engine-internal (no plugin impact):**
- Seal ControlAction, migrate canon §3.5 wording
- Wire `ActionResult::Terminate` scheduler integration
- Add macro test harness (trybuild + macrotest)
- Lefthook parity fix
- Deny.toml layer rule for action (Phase 0 T9)
- `zeroize` workspace pin

**Phase 3B — Action-internal settled items (SDK prelude stable):**
- Fix C2 broken `parameters = Type` path
- Fix C3 type-name-as-key heuristic (deprecate with #[deprecated] if Option B; remove if Option A)
- Add `Provide` port kind
- Add DataTag hierarchical registry
- Fix S6 handler HRTB verbosity (cosmetic)

**Phase 3C — Plugin-breaking (contested surface):**
- Macro shape decision lands
- `CredentialContextExt` replacement lands (Option A/B/C)
- `ctx.resource::<R>(key)` typed-lease replacement lands
- `IdempotencyKey` + lifecycle hooks on TriggerAction
- Plugin migration docs + codemod

Phase 3C is the big plugin-breaking PR. Phases 3A/3B ship before it; they reduce Phase 3C's review surface.

**Can 3C land as one cut?** Yes — `feedback_hard_breaking_changes` licenses this; semver-checks are advisory-only during alpha (Phase 0 T7); 7 reverse-deps is tractable for one coordinated migration day; doing it in pieces creates dual-vocabulary windows we want to avoid per `feedback_incomplete_work`.

### Verdict

**🟠 COHERENCE.** Phased migration is viable with strict ordering (engine-internal → action-internal → plugin-breaking). Phase 3C is the only plugin-breaking cut; it should be one coordinated PR, not spread. Architect's Phase 2 strategy doc will own the detailed sequencing; this is tech-lead input on shape.

---

## 7. Top-N architectural coherence findings + priority calls

| # | Severity | Finding | Priority call |
|---|---|---|---|
| **A1** | 🔴 STRUCTURAL | Option A/B/C for Action × Credential coupling is the critical-path decision; every downstream shape (macro, `CredentialContextExt`, plugin cascade sequencing) is gated on it | **Co-decision** (architect + tech-lead + security-lead). My position: lean A with fallback C (not B). Framed in §3. |
| **A2** | 🟠 COHERENCE | Canon §3.5 drift via 5 DX traits + ControlAction public-non-sealed status | **Solo call:** seal ControlAction; ratify DX tier in canon §3.5 as "erases to primary." §5 decision block. |
| **A3** | 🟠 COHERENCE | `ActionResult::Terminate` asymmetric gating vs Retry | **Solo call:** feature-gate + wire end-to-end in this cascade. §5 decision block. Active-dev-mode applies: don't gate-only. |
| **A4** | 🟠 COHERENCE | `TriggerAction` cluster-mode hooks (idempotency-key, lifecycle, dedup window) | **Phase 2 input:** 3 hooks on action-layer; cluster coordination in engine. §4. |
| **A5** | 🟠 COHERENCE | Phase 3C plugin cascade should be one coordinated cut, not spread | **Phase 2 input:** sequence 3A (engine) → 3B (action-internal) → 3C (plugin-breaking, single cut). §6. |
| **A6** | 🔴 STRUCTURAL | Macro test harness gap is the root cause of C2 (broken `parameters = Type`) shipping | **Solo devops call:** trybuild + macrotest both. §5. Cascade must not ship without this. |
| **A7** | 🟠 COHERENCE | Lefthook policy divergence from user memory rule | **Solo devops call:** fix divergence. §5. Not a re-arbitrated trade-off. |
| **A8** | 🟠 COHERENCE | `#[derive(Action)]` vs `#[action]` attribute is downstream of Option A/B/C; do not pre-empt | **Co-decision blocked on A1.** Recommend: if A chosen, attribute macro with narrow documented rewriting contract (§2). |

---

## 8. Grounding references

- `crates/action/src/lib.rs:11-20` — docstring enumerates 10 surfaces; self-contradicts §3.5 rule
- `crates/action/src/handler.rs:41-50` — `ActionHandler` 4-variant enum (dispatch ground truth)
- `crates/action/src/control.rs:393-431` — `ControlAction` trait
- `crates/action/src/control.rs:484-510` — `ControlActionAdapter` erases to `StatelessHandler`
- `crates/action/src/stateful.rs:170-227` — `impl_paginated_action!` macro (erasure via macro_rules)
- `crates/action/src/webhook.rs:1074` — `WebhookTriggerAdapter impl TriggerHandler`
- `crates/action/src/poll.rs:1290` — `PollTriggerAdapter impl TriggerHandler`
- `crates/action/macros/src/action_attrs.rs:10-33` — 8 attribute keys of `#[derive(Action)]`
- `crates/action/macros/src/action_attrs.rs:129-134` — broken `with_parameters` emission (C2)
- `crates/action/macros/` — confirmed no `tests/` directory (T1 macro harness gap)
- `crates/credential/src/` grep — zero matches for `CredentialRef|SchemeGuard|SlotBinding|AnyCredential|resolve_as_bearer` (CP6 vocabulary not yet implemented anywhere in `src/`)
- `crates/action/src/` grep for `cluster|leader|election|reconnect|idempoten` — zero runtime surface matches (cluster mode is spec-only today)

---

*End of Phase 1 tech-lead architectural coherence input. Phase 2 co-decisions routed via orchestrator.*
