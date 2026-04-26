# Spike iter-3 — post-amendment compose verification NOTES

**Date:** 2026-04-25
**Verdict:** PASS
**Isolated worktree branch:** `worktree-agent-a3ec73dbf722f0095`
**Validation commit:** `10b24616`
**Validation crate:** `scratch/spike-iter-3-shape/` (standalone, not in workspace)
**Toolchain:** workspace pin `1.95.0` per `rust-toolchain.toml`
**Result:** clean `cargo +1.95.0 check`, no warnings.

## Scope

Iter-3 = update + compose verification, **not** full re-spike. The original
spike PASS verdict for shape-only validation (Phase 4 commit `c8aef6a0`)
stands. Iter-3 only verifies the four amendment rounds (Q1 + Q6 + Q7 + Q8)
do not break compose against the existing post-amendment Tech Spec FROZEN CP4
state.

## Eight compose probes

1. **Probe 1** — `StatefulAction` impl with restored `init_state` +
   `migrate_state` hooks (Q7 R1). PASS. Override of `migrate_state` compiles;
   `init_state` mandatory per `#[diagnostic::on_unimplemented]` discipline
   in production parity.
2. **Probe 2** — `TriggerAction` impl with `start` + `stop` lifecycle
   (Q6) + `accepts_events` (Q7 R3) + `idempotency_key` override (Q8 F2)
   + `handle` returning `TriggerEventOutcome::Emit(...)` (Q7 R3). PASS.
3. **Probe 3** — `ResourceAction` impl with `configure` + `cleanup`
   paradigm (Q7 R2). NO `execute` method; no `Input` / `Output` associated
   types. PASS — production-parity shape compiles.
4. **Probe 4** — `StatefulAction + ResourceAction` on the SAME struct
   (`DualAction`). PASS — independent associated-type sets compose without
   trait-coherence collision.
5. **Probe 5** — `WebhookAction` peer-of-Action shape (Q7 R6). Load-bearing
   compose check: a struct that impls `WebhookAction` only (NOT
   `TriggerAction`) compiles. PASS — confirms no TriggerAction subtype
   leak from sealed-DX bound chain.
6. **Probe 6** — `TriggerAction` without `idempotency_key` override (Q8 F2
   default-opt-in). PASS — default `None` is consumed by all triggers
   without explicit override.
7. **Probe 7** — `Arc<dyn StatelessHandler>` / `Arc<dyn StatefulHandler>` /
   `Arc<dyn TriggerHandler>` / `Arc<dyn ResourceHandler>` all dyn-safe under
   `#[async_trait]` (Q1 amendment). PASS — `assert_dyn_handlers()` accepts
   all four.
8. **Probe 8** — Cluster-mode trait placeholders (`CursorPersistence`,
   `LeaderElection`, `ExternalSubscriptionLedger`, `ScheduleLedger`)
   object-safe stand-ins (Q8 F13). PASS — `assert_cluster_traits()`
   accepts `&dyn` references.

## Findings

- **No compile failures** — all probe impls compile clean.
- **No unused-import warnings** — clean `cargo check`.
- **`#[async_trait]` macro layer works as designed** — `Pin<Box<dyn Future +
  Send + 'async_trait>>` returns are emitted internally; the four
  `*Handler` traits are dyn-safe per ADR-0024 contract.
- **Default-opt-in `idempotency_key` confirmed** — Probe 6 omits override
  (default `None` consumed); Probe 2 overrides (returns
  `Some(IdempotencyKey)`); both compile in the same crate without
  coherence collision.
- **Webhook peer-of-Action framing confirmed** — `PureWebhookAction` impls
  `WebhookAction` only; the `+ Action` supertrait bound is sufficient (no
  `+ TriggerAction` propagation). Q7 R6 sealed-DX peer framing holds.

## What this validates vs what it does NOT

**Validated:**

- Trait shapes compose syntactically (compiler accepts the bound chain).
- Default-method shapes work (Q8 F2 idempotency_key default-opt-in).
- Sealed-DX peer framing (Q7 R6) does not leak TriggerAction methods onto
  WebhookAction/PollAction implementors.
- `#[async_trait]` macro emission is dyn-safe (Q1 amendment).
- `TriggerEventOutcome` multiplicity types (Skip / Emit / EmitMany)
  compose with the Q7 R3 `handle()` return type.

**NOT validated (scope-limited per iter-3 charter):**

- Engine-side semantic correctness (e.g., "engine actually consults
  `accepts_events()` before dispatching"). Engine cascade scope per Tech
  Spec §1.2 N4.
- Cluster-mode trait method bodies. Doc-only contracts per §3.7; engine
  cascade implements bodies.
- Cancellation safety mid-`.await` for the new lifecycle methods (Q6
  `start` / `stop`). Carry-over from iter-1/2 spike PASS for the
  cancellation invariant per Tech Spec §3.4 — same `tokio::select!` test
  pattern applies but is NOT re-run in iter-3 (compose-only scope).
- Macro emission (`#[action]` attribute zone parser). Macro emission
  spike is outside the trait-shape spike scope.

## Files

- **v3 artefact** (main worktree, for orchestrator commit):
  `docs/superpowers/drafts/2026-04-24-nebula-action-redesign/final_shape_v3.rs`
  — 839 lines (vs v2's 284; the +555 line delta is amendment commentary +
  Q7 R6 sealed-DX peer-trait shapes that v2 collapsed into TriggerAction +
  Q8 F9 / F12 / F13 new types).
- **v2 artefact** (UNCHANGED — historical):
  `docs/superpowers/drafts/2026-04-24-nebula-action-redesign/final_shape_v2.rs`
  — 284 lines.
- **iter-3 spike crate** (isolated worktree, validation only; commit
  `10b24616` on branch `worktree-agent-a3ec73dbf722f0095`):
  `scratch/spike-iter-3-shape/Cargo.toml` + `scratch/spike-iter-3-shape/src/lib.rs`
  — ~565 lines of source + 8-line Cargo.toml.

## Diff summary v3 vs v2

| Area | v2 shape | v3 shape | Amendment |
|------|----------|----------|-----------|
| `StatelessAction` | unchanged | unchanged + `HasSchema` / `DeserializeOwned` / `Serialize` bounds (Tech Spec CP1 lift) | (v2-correct; v3 documents bound lift) |
| `StatefulAction` | only `execute` | + `init_state` + `migrate_state(default None)` | Q7 R1 |
| `TriggerAction::handle` return | `Result<(), Error>` | `Result<TriggerEventOutcome, Error>` | Q7 R3 |
| `TriggerAction` lifecycle | absent | + `start` + `stop` adjacent to `handle` | Q6 |
| `TriggerAction` event-gate | absent | + `accepts_events() -> bool` (default false) | Q7 R3 |
| `TriggerAction` idempotency | absent | + `idempotency_key(&self, event) -> Option<IdempotencyKey>` (default None) | Q8 F2 |
| `TriggerEventOutcome` enum | absent | NEW: `Skip` / `Emit(Value)` / `EmitMany(Vec<Value>)` | Q7 R3 |
| `IdempotencyKey` newtype | absent | NEW | Q8 F2 |
| `ResourceAction` | `Input` + `Output` + `execute(.., resource: &Resource, input)` | `configure(&self, ctx) -> Future<Resource>` + `cleanup(&self, resource, ctx) -> Future<()>`; NO `execute` / `Input` / `Output` | Q7 R2 |
| `*Handler` companion traits | hand-written `BoxFut<'a, T>` per method | `#[async_trait::async_trait]` annotation; `async fn` syntax | Q1 |
| `TriggerHandler::handle_event` | `serde_json::Value` event | `TriggerEvent` envelope (`Box<dyn Any> + TypeId`) | Q7 R5 |
| `TriggerHandler::accepts_events` | absent | + default `false` | Q7 R5 |
| `ResourceHandler` | `execute(ctx, resource_id, input)` | `configure(.., ctx) -> Box<dyn Any+Send+Sync>` + `cleanup(box, ctx)`; NO `execute` | Q7 R4 |
| `WebhookAction` bound | `: WebhookActionSealed + TriggerAction` (subtrait) | `: WebhookActionSealed + Action + Send + Sync + 'static` (peer) | Q7 R6 |
| `PollAction` bound | `: PollActionSealed + TriggerAction` (subtrait) | `: PollActionSealed + Action + Send + Sync + 'static` (peer) | Q7 R6 |
| `WebhookAction` shape | absent in v2 (collapsed to TriggerAction subtype) | `type State` + `on_activate` / `handle_request` / `on_deactivate` / `config` | Q7 R6 |
| `PollAction` shape | absent in v2 (collapsed to TriggerAction subtype) | `type Cursor: Default` + `type Event` + `poll_config` / `validate` / `initial_cursor` / `poll` | Q7 R6 |
| `ActionMetadata::max_concurrent` | absent | + `Option<NonZeroU32>` field | Q8 F9 |
| `NodeDefinition::action_version` | absent | + `semver::Version` field (engine-cascade scope; surface obligation here) | Q8 F12 |
| `CursorPersistence` / `LeaderElection` / `ExternalSubscriptionLedger` / `ScheduleLedger` | absent | NEW: 4 doc-only trait placeholders in `nebula-engine` | Q8 F13 |

## What stays the same (carried over verbatim from v2)

- `CredentialRef<C>` (§1) — Credential Tech Spec §3.5 typed handle.
- `SlotBinding` + `ResolveFn` HRTB (§2) — Credential Tech Spec §3.4.
- `BoxFut` / `BoxFuture` alias survives — narrower scope (HRTB-only); see
  Tech Spec §2.3 amendment narrative.
- `SchemeGuard<'a, C>` lifetime-pinning (§3) — credential Tech Spec §15.7
  iter-3 refinement.
- `SchemeFactory<C>` (§4) — credential Tech Spec §15.7.
- `ActionContext<'a>` (§5).
- `ActionHandler` enum 4-variant shape (no `Control` variant per ADR-0038).
- `ControlAction` / `PaginatedAction` / `BatchAction` DX traits — already
  carried v2 shape per CP3 §7.
- `ActionSlots` macro-emitted-only blanket marker (§6 in v2; §12 in v3).

## Why no full re-spike

The original spike (Phase 4 commit `c8aef6a0`) was a **shape-only
validation** crate: it verified that the trait bound chains compile, that
RPITIT `impl Future + Send + 'a` returns are well-formed under the
workspace toolchain pin, and that the four primary dispatch traits are
mutually composable on a single struct (Probe 4 carry-over from
iter-1/2). Iter-3 amendments are:

- **Additive** (Q6 lifecycle, Q7 R1 / R3 / R6 method additions, Q8 F9 / F13
  / F2): adding methods + types to traits whose bound chains the original
  spike already verified compile. The compose check is whether the new
  methods compile against the existing bound chain — Probes 1 / 2 / 3 / 4
  / 5 cover this.
- **Substitutive** (Q1 macro adoption, Q7 R2 / R4 / R5 paradigm shifts):
  replacing one shape (hand-written `BoxFut`, JSON event boundary) with
  another (`#[async_trait]`, `Box<dyn Any>` + `TriggerEvent` envelope).
  The substitution is structurally equivalent at the runtime layer per
  Tech Spec §2.4 equivalence note (heap allocation per call unchanged,
  cancel-safety unchanged); the compose check is whether the new shape
  is dyn-safe — Probe 7.

A full re-spike (rebuilding the entire `scratch/spike-action-credential/`
crate with cancellation-safety tests, `tokio::select!` drop tests, etc.) is
**not justified** for additive + substitutive amendments to a previously
validated shape. The iter-3 standalone crate (730 lines) is sufficient
evidence for the amendment delta; the iter-1/2 spike PASS verdict for
cancellation invariants stands per Tech Spec §0.1 inputs frozen-at footer
scope clarification.

## Handoff notes

- **For orchestrator:** v3 artefact is at
  `docs/superpowers/drafts/2026-04-24-nebula-action-redesign/final_shape_v3.rs`
  in main worktree (un-committed). v2 untouched. NOTES file is this document.
- **For Tech Spec maintainers:** Tech Spec §7 (which currently cites
  `final_shape_v2.rs:213-262` for trait shape evidence) should re-cite
  `final_shape_v3.rs` lines 200-340 (trait shape block) post-amendment.
  Spike commit reference moves from `c8aef6a0` to `c8aef6a0` (original
  shape) + `10b24616` (compose verification of amendments).
- **For future cascades:** the iter-3 standalone crate at
  `scratch/spike-iter-3-shape/` on isolated worktree branch
  `worktree-agent-a3ec73dbf722f0095` can be ported to the post-cascade
  test crate as compose-regression evidence; not load-bearing for
  cascade landing.
