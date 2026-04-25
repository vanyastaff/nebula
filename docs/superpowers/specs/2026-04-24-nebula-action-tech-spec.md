---
name: nebula-action tech spec (implementation-ready design)
status: FROZEN CP4 2026-04-25
date: 2026-04-24
authors: [architect (drafting); tech-lead (CP gate decider); security-lead (VETO authority on §4 security floor); orchestrator (CP coordination)]
scope: nebula-action redesign cascade Phase 6 — implementation-ready design for the action trait family, the `#[action]` attribute macro, runtime model, security floor, and codemod migration
supersedes: []
related:
  - docs/superpowers/specs/2026-04-24-action-redesign-strategy.md
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/adr/0036-action-trait-shape.md
  - docs/adr/0037-action-macro-emission.md
  - docs/adr/0038-controlaction-seal-canon-revision.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md
  - docs/superpowers/drafts/2026-04-24-nebula-action-redesign/final_shape_v2.rs
---

# nebula-action Tech Spec (implementation-ready design)

## §0 Status, scope, freeze policy

### §0.1 Status progression

This document moves through four checkpoints with parallel reviewer matrices per Strategy §6.3 (line 386-394):

| Checkpoint | Sections | Focus | Status |
|---|---|---|---|
| **DRAFT CP1** | §0–§3 | Status, goals, trait contract, runtime model | locked CP1 |
| **DRAFT CP2** | §4–§8 | Macro emission, test harness, security floor, lifecycle, storage | locked CP2 |
| **DRAFT CP3** | §9–§13 | Public API surface, codemod migration, adapter authoring, ControlAction migration, evolution policy | locked CP3 |
| **FROZEN CP4 2026-04-25** (iterated 2026-04-25 post 11a/11b; tech-lead RATIFY-FREEZE 11c) | §14–§16 | Open items, accepted gaps, handoff, implementation-path framing for Phase 8 user pick | **frozen** |

Inputs are **frozen** at this freeze point: Strategy frozen at CP3 (commit `a38f6f5a`); ADR-0036 status `accepted` 2026-04-25 + ADR-0037 status `accepted` 2026-04-25 (amended-in-place 2026-04-25 per §15.5 enactment) — both flipped on Tech Spec FROZEN CP4 ratification per their respective §Status sections; ADR-0038 retained at `proposed` pending explicit user ratification on canon §3.5 revision (per cascade prompt: surface к user в Phase 8 summary, не auto-flip); Phase 4 spike PASS at commit `c8aef6a0` (worktree-isolated; see [spike NOTES](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md) §5).

### §0.2 What invalidates the freeze

Once frozen at CP4, only an ADR may amend §1–§16. Per Strategy §0 amendment mechanics (line 32-34), the following invalidate the freeze and require an ADR-supersede before Tech Spec ratification:

1. **Strategy revision.** Any §1–§6 change to [`2026-04-24-action-redesign-strategy.md`](2026-04-24-action-redesign-strategy.md) post-FROZEN CP3.
2. **ADR amendment.** Any of ADR-0035 / ADR-0036 / ADR-0037 / ADR-0038 moves from `accepted` to `superseded` or undergoes a non-trivial amendment.
3. **Security floor change.** Any of the four invariant items in §4 (per Strategy §2.12 + §4.4) is relaxed, deferred, or has its enforcement form softened (e.g., "hard removal" → "deprecated shim" — `feedback_no_shims.md` violation).
4. **Spike-shape divergence.** `final_shape_v2.rs` (the shapes Tech Spec §2 freezes verbatim) is re-validated and a different shape is required.

Citations to Strategy / credential Tech Spec / ADRs are pinned at line-number granularity below; if a cited line range moves due to upstream document edits, this Tech Spec must be re-pinned (CHANGELOG entry + reviewer pass).

### §0.3 Authority chain

PRODUCT_CANON > ADRs (0035 / 0036 / 0037 / 0038 cascade-ratifying) > Strategy (frozen CP3) > Tech Spec (this document) > implementation plans. Tech Spec is **implementation-normative** — implementers consume §2 / §3 / §7 (when CP3 lands) / §9 (when CP3 lands) directly without re-deriving from Strategy.

---

## §1 Goals + non-goals

### §1.1 Goals

Each goal binds to one or more Phase 1 Critical findings (CR1–CR11; see [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §4) and Strategy decisions. The list is **load-bearing** — every section of this Tech Spec must trace its content to one of these goals, and any section that does not is out-of-scope.

**G1 — Credential CP6 vocabulary adoption (closes CR1, CR5–CR10).** Action surface adopts the credential Tech Spec CP6 typed-credential paradigm: `CredentialRef<C>` field-level handles, `SlotBinding` registry registration with HRTB `resolve_fn`, `SchemeGuard<'a, C>` RAII at the action body. `ctx.credential::<S>(key)` / `credential_opt::<S>(key)` materialize through macro-emitted slot bindings. Time-to-first-successful credential-bearing action drops from 32 minutes (current; see [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §1, dx-tester reference) to <5 minutes (Strategy §1 target). Per Strategy §3.1 component 2 + ADR-0036 §Decision.

**G2 — Macro emission correctness with regression harness (closes CR2, CR8, CR9, CR11).** `#[derive(Action)]` is replaced by `#[action]` attribute macro per ADR-0036; macro emits regression-covered tokens for the four action shapes; `crates/action/macros/Cargo.toml` gains `[dev-dependencies]` block with `trybuild` + `macrotest`; six probes from Phase 4 spike commit `c8aef6a0` port forward as the production harness baseline (per ADR-0037 §4 table). Three independent agents hitting the same `parameters = Type` emission bug becomes structurally impossible because regression coverage exists.

**G3 — Security must-have floor (non-negotiable invariant, per Strategy §2.12 + §4.4).** Four floor items ship in cascade scope, each with typed error + trace span + invariant check per `feedback_observability_as_completion.md`:

  - JSON depth bomb fix (CR4 / S-J1 — depth cap 128 at every adapter JSON boundary);
  - Cross-plugin shadow attack fix (CR3 / S-C2 — explicit keyed dispatch; **hard removal** of `CredentialContextExt::credential<S>()` no-key heuristic, not `#[deprecated]` shim — `feedback_no_shims.md` + security-lead 03c §1 VETO);
  - `ActionError` Display sanitization (route through `redacted_display()` helper);
  - Cancellation-zeroize test (closes S-C5).

Detail spec is §4 (CP2); §1 binds the floor as a Goal so that any later section that relaxes one of these items invalidates the freeze (per §0.2 item 3).

**G4 — Sealed DX tier ratification (per ADR-0038).** Five DX traits — `ControlAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction` — become sealed per ADR-0038 §1 (sealed-trait pattern via per-capability inner sealed trait, following ADR-0035 §3 convention). Canon §3.5 line 82 revises per ADR-0038 §2 to enumerate the DX tier explicitly; the 4-primary trait family is preserved. Sealing closes the §1(c) governance drift.

**G5 — `*Handler` HRTB modernization (per Strategy §4.3.1).** The four `*Handler` companion traits adopt single-`'a` lifetime + `BoxFut<'a, T>` type alias, replacing `for<'life0, 'life1, 'a>` + `where Self: 'a, 'life0: 'a, 'life1: 'a` boilerplate. Rust 1.95 elision rules accept the single-lifetime form (rust-senior 02c line 55); dyn-safety preserved (rust-senior 02c line 358); ~30-40% LOC reduction across `stateless.rs:313-322`, `stateful.rs:461-472`, `trigger.rs:328-381`, `resource.rs:83-106` plus mirrored adapter sites (rust-senior 02c §8 line 439). Per `feedback_idiom_currency.md` (1.95+ idioms; pre-1.85 HRTB shapes are anti-patterns now).

**G6 — `ActionResult::Terminate` symmetric gating (per Strategy §4.3.2 + §2.3).** Today `ActionResult::Terminate` is a public variant whose engine wiring is "Phase 3 of the ControlAction plan and is not yet wired" (`crates/action/src/result.rs:217`) — a literal canon §4.5 false-capability violation (Strategy §2.3 line 70). Strategy §4.3.2 locks the **principle**: Retry and Terminate share the same gating discipline; either both wire end-to-end or both stay gated-with-wired-stub. CP1 §2.7 picks the concrete path (see §2.7 below).

### §1.2 Non-goals

Each non-goal cites the Strategy §3.4 OUT row (line 165-183) or scope-decision §6 boundary that places it out of scope. This is **honest deferral** per `feedback_active_dev_mode.md` ("before saying 'defer X', confirm the follow-up has a home").

**N1 — Resource integration deeper than ADR-0035 §4.3 rewrite obligation.** Resource-side scope (resource crate redesign, `Resource::on_credential_refresh` full integration, resource cluster-mode coordination) is OUT. Action's responsibility ends at the `ResourceAction::Resource` associated type binding + `CredentialRef<C>` field-zone rewrite per ADR-0036 §3 + the consumer-side completion of ADR-0035's two-trait phantom-shim contract (ADR-0036 §Decision item 4). Strategy §3.4 row "`Resource::on_credential_refresh` full integration" (line 173) names the home: absorbed into resource cascade or co-landed with credential CP6 implementation.

**N2 — DataTag hierarchical registry (58+ tags).** Strategy §3.4 row 1 (line 169) → future port-system sub-cascade. Net-new surface; orthogonal to action core.

**N3 — `Provide` port kind.** Strategy §3.4 row 2 (line 170) → same sub-cascade as DataTag. Net-new; not cascade-gating.

**N4 — Engine cluster-mode coordination implementation.** Strategy §3.4 row 3 (line 171-172) → engine cluster-mode coordination cascade, queued behind credential CP6 implementation cascade per Strategy §6.6 (line 421). Action surfaces three hooks on `TriggerAction` (`IdempotencyKey`, `on_leader_*` lifecycle, `dedup_window` metadata; surface contract only per Strategy §3.1 component 7); engine-side coordination ships in the dedicated cascade. Hook trait shape lock is CP3 §7 scope per Strategy §5.1.5 (line 297).

**N5 — Q1 implementation path decision (paths a/b/c).** Strategy §4.2 framing (line 198-206) presents three implementation paths: (a) single coordinated PR, (b) sibling cascades, (c) phased rollout with B'+ surface commitment. Strategy explicitly states the user picks at Phase 8 (Strategy §4.2 line 206 + §6.5 line 408-413). CP4 §16 frames the choice with concrete criteria (extending Strategy §6.5 table); Tech Spec does NOT pre-pick.

**N6 — `#[trait_variant::make(Handler: Send)]` adoption.** Strategy §3.4 last row (line 182) → separate Phase 3 redesign decision. `trait_variant` would collapse the RPITIT/HRTB split into a single source generating both, breaking existing public `*Handler` trait surface. G5 modernization adopts single-`'a` + `BoxFut<'a, T>` *without* `trait_variant` adoption per rust-senior 02c §6 line 362-380.

**N7 — Sub-spec out-of-scope rows from Strategy §2.12.** S-W2 (webhook hardening cascade), S-C4 (credential CP6 implementation cascade), S-O1/O2/O3 (output-pipeline cascade), S-I2 (sandbox phase-1 cascade), and the §6.7 sunset table (Strategy line 432-440) are all deferred-with-cascade-home per `feedback_active_dev_mode.md` discipline.

---

## §2 Trait contract — full Rust signatures

This is the **signature-locking section**. Each shape below is freeze-grade Rust, compile-checked against [`final_shape_v2.rs`](../drafts/2026-04-24-nebula-action-redesign/final_shape_v2.rs) (the spike's curated extract from commit `c8aef6a0`) — with three deliberate-divergence overlays where the spike's placeholder shape disagrees with the cross-crate authoritative source:

1. **`Input: HasSchema` + `DeserializeOwned` and `Output: Serialize` bounds** are lifted onto the typed trait (§2.2.1 / §2.2.2 / §2.2.4) over the spike's `Send + 'static`-only bounds — closes CR9 (undocumented schema bound) and resolves the "leaky adapter ser/de invariant" rust-senior 02c §3 finding (line 203-217). Spike artefact is informal sourcing for shape; CR-binding is canonical.
2. **`StatefulAction::State` bound chain** (`Serialize + DeserializeOwned + Clone + Send + Sync + 'static`) is lifted from current `crates/action/src/stateful.rs` adapter requirements — spike has `Send + Sync + 'static` only; production engine contract requires the full chain (per §2.2.2 narrative).
3. **`ActionSlots::credential_slots(&self)`** signature aligns with credential Tech Spec §3.4 line 851 (cross-crate authoritative cardinality on receiver) over any earlier no-`&self` form. Spike `final_shape_v2.rs:278` already has `&self`; ADR-0037 §1 example must re-pin to `&self` form (cascade-internal — covered by Tech Spec ratification).

Tech Spec ratification freezes these signatures (with the three deliberate-divergence overlays above); subsequent deviations land as ADR amendments per §0.2 invariant 4. Spike `final_shape_v2.rs` is informal-sourcing (a curated extract that proved compile); the credential Tech Spec, ADRs, and rust-senior 02c findings are canonical for cross-crate / cross-cutting invariants where they conflict with the spike.

### §2.1 Base trait — `Action` (identity + metadata supertrait)

```rust
/// Identity + metadata-bearing marker. User code does NOT implement
/// `Action` directly — the `#[action]` macro emits a **concrete**
/// `impl Action for X` per action, threading `ActionMetadata` from
/// the attribute fields (`#[action(name = …, version = …, parameters = …)]`).
/// The macro also emits `impl ActionSlots for X` from the
/// `credentials(...)` / `resources(...)` zones, so any
/// `#[action]`-decorated struct that also implements one of the four
/// primary dispatch traits below satisfies `Action` structurally.
pub trait Action: ActionSlots + Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
}
```

The supertrait bound on `ActionSlots` makes "no `ActionSlots` impl → no `Action` impl" structural — Probe 3's type-system enforcement layer per ADR-0037 §2 (line 67-70). `Send + Sync + 'static` is required for handler erasure; spike Iter-1 §1.7 confirmed the bound chain compiles via `assert_is_action::<A>()` across all three iter-2 actions.

**`ActionMetadata`** (the return type of `Action::metadata`) is defined at `crates/action/src/metadata.rs` (current shape; field-set lock is CP3 §7 scope per Strategy §5.1.1). The macro emits the metadata literal from `#[action(...)]` attribute fields per ADR-0037 §1.

#### §2.1.1 `ActionSlots` companion trait

```rust
/// Slot bindings emitted by the `#[action]` macro. User code does NOT
/// implement this directly — the macro emits the impl from the
/// `credentials(...)` and `resources(...)` zones declared on the
/// action struct.
///
/// `&self` receiver matches credential Tech Spec §3.4 line 851
/// (cross-crate authoritative shape). The `'static` lifetime on the
/// returned slice survives because the macro emits the slice as a
/// `&'static [SlotBinding]` literal — see §3.1 + ADR-0037 §1.
pub trait ActionSlots {
    fn credential_slots(&self) -> &'static [SlotBinding];
    // Resource-slot companion shape locked at CP3 §7 (currently CP3-deferred
    // per N1 + Strategy §3.4 row "Resource::on_credential_refresh full integration").
    // fn resource_slots(&self) -> &'static [ResourceBinding];
}
```

The `&self` receiver is **deliberate divergence** from spike `final_shape_v2.rs:278` (which has `&self` already, matching this trait) AND from credential Tech Spec §3.4 line 851 (`fn credential_slots(&self) -> &[SlotBinding]`). Tech Spec retains `&'static` on the slice per spike `slot.rs` static-assert (binding is `Copy + 'static` per §3.1) — credential Tech Spec authoritative shape covers the cross-crate contract; spike artefact is informal sourcing for the slice-lifetime detail. CP1 reconciliation: this Tech Spec aligns with credential Tech Spec §3.4 on receiver shape and inherits spike on slice-lifetime; if credential Tech Spec re-pins `&[SlotBinding]` (no `'static`) at any future revision, this Tech Spec re-pins per §0.2 invariant 4.

### §2.2 Four primary dispatch traits

The four primary traits carry the dispatch-shape contract per PRODUCT_CANON §3.5 line 82 (revised by ADR-0038 §2). Each trait uses **RPITIT** (`-> impl Future<Output = ...> + Send + 'a`) to express the body's async return — verified via spike `final_shape_v2.rs:213-262`.

#### §2.2.1 `StatelessAction`

```rust
pub trait StatelessAction: Send + Sync + 'static {
    type Input: HasSchema + DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}
```

`Input: HasSchema` is documented per ADR-0037 §Context (Goal G2 — closes CR9 undocumented bound). `Input: DeserializeOwned` and `Output: Serialize` are **lifted onto the trait** rather than imposed at the adapter site — closes the leaky-adapter-invariant finding from rust-senior 02c §3 (line 203-217) where `with_parameters`-style ser/de bounds surfaced only at registration. Lifting moves the diagnostic from registration site to impl site (better error UX); applies uniformly across `StatelessAction` / `StatefulAction` / `ResourceAction`. `Output: Send + 'static` per spike `final_shape_v2.rs:211` — `Send` for handler erasure, `'static` for serialization through the engine's port projection. Cancellation invariants (G3 floor item 4): the body's `impl Future + Send + 'a` is cancellable at any `.await` point; any `SchemeGuard<'a, C>` borrowed via `ctx.credential::<S>(key)` zeroizes deterministically on drop (§3.4 below).

#### §2.2.2 `StatefulAction`

```rust
pub trait StatefulAction: Send + Sync + 'static {
    type Input: HasSchema + DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        state: &'a mut Self::State,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}
```

`State: Serialize + DeserializeOwned + Clone + Send + Sync + 'static` is the engine's contract — `Serialize` + `DeserializeOwned` for persisted iteration state (per `crates/action/src/stateful.rs:356-383`), `Clone` for retry / redrive, `Send + Sync` for engine-side dispatch through `Arc<dyn StatefulHandler>`, `'static` for adapter erasure. Spike Iter-2 §2.2 (commit `c8aef6a0`'s `iter2_compose.rs::GitHubListReposAction`) verified the bound chain compiles.

State Send-bound discipline: `state: &'a mut Self::State` is borrowed mutably across the body's `.await` points; the `'a` lifetime ties state to the borrow chain, preventing storage in long-lived structs (verified by spike Iter-2 §2.2 against `tokio::select!` cancellation test).

#### §2.2.3 `TriggerAction`

```rust
pub trait TriggerSource: Send + Sync + 'static {
    type Event: Send + 'static;
}

pub trait TriggerAction: Send + Sync + 'static {
    type Source: TriggerSource;     // Probe 2 verifies this is required
    type Error: std::error::Error + Send + Sync + 'static;

    fn handle<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        event: <Self::Source as TriggerSource>::Event,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}
```

`Source: TriggerSource` associated type per spike Probe 2 ([NOTES §1.3](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)) — without it, `impl TriggerAction for X` produces `error[E0046]: not all trait items implemented, missing: Source`. Per spike Iter-2 §2.2 the `<Self::Source as TriggerSource>::Event` projection composes cleanly with the action body's `&'a` borrow chain.

Cluster-mode hooks (`IdempotencyKey`, `on_leader_*`, `dedup_window`) attach as supertrait extensions per Strategy §3.1 component 7 + §5.1.5 — exact trait shape locked at CP3 §7 (this Tech Spec section is foundational; full hook surface is Phase 3+ scope).

#### §2.2.4 `ResourceAction`

```rust
pub trait Resource: Send + Sync + 'static {
    type Credential: Credential;
}

pub trait ResourceAction: Send + Sync + 'static {
    type Resource: Resource;        // Probe 1 verifies this is required
    type Input: HasSchema + DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        resource: &'a Self::Resource,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}
```

`Resource: Resource` associated type per spike Probe 1 ([NOTES §1.2](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)) — `error[E0046]: missing: Resource` without it. Resource is borrowed (`&'a Self::Resource`) — the action body cannot retain the resource past its own lifetime. `Resource::Credential: Credential` ensures resource-credential composition lands at `crates/credential/src/contract` per Strategy §2.8 / credential Tech Spec §3.4 line 807-939.

Resource-credential ownership boundary: the resource holds `SchemeFactory<C>` (per credential Tech Spec §15.7 line 3438-3447); the action body ALWAYS acquires `SchemeGuard<'a, C>` per request. This is N1 (Non-goal): resource-side scope (the `Resource` impl itself, `on_credential_refresh` full integration) is out of this Tech Spec's scope, but the type-level binding here is in scope per ADR-0035 §4.3 rewrite obligation.

### §2.3 `BoxFut<'a, T>` type alias

```rust
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
```

Replaces the `for<'life0, 'life1, 'a>` HRTB boilerplate from the legacy `*Handler` trait surface (per Strategy §4.3.1 line 215-220, rust-senior 02c §6 line 358). This is the **dyn-safe companion** type used by §2.4 `*Handler` traits. The single-`'a` lifetime composes with the action body's borrow chain (spike Iter-2 §2.4 cancellation test); spike `final_shape_v2.rs:38` confirms the alias is well-formed under Rust 1.95 elision (rust-senior 02c line 55).

`BoxFut` is **not** dyn-safe by itself; it is the return shape used by dyn-safe handler trait methods (§2.4). The HRTB used at the credential-resolution layer (§2.5 `ActionHandler` and §3.2 dispatch) uses the same fn-pointer shape per credential Tech Spec §3.4 line 869.

**Crate residence.** `BoxFut<'a, T>` lives in `nebula-action` as the canonical alias for handler returns. Spike `final_shape_v2.rs:38` and credential Tech Spec §3.4 line 869 both use `BoxFuture` (longer name) for the same shape. CP3 §7 confirms the single-home decision: `nebula-action::BoxFut` is the action-side alias; if a shared `nebula-core::BoxFuture` is hoisted in a future cascade, this Tech Spec re-pins per §0.2 invariant 4. For CP1, the alias is action-local — engine adapters that need the same shape should `use nebula_action::BoxFut`, not redeclare.

### §2.4 Four `*Handler` companion traits — dyn-safe parallels

Each primary dispatch trait has a **dyn-safe** companion `*Handler` trait used by the engine's `Arc<dyn XHandler>` storage (per `crates/action/src/handler.rs:39-50`). The HRTB modernization (G5 / Strategy §4.3.1) collapses the legacy quadruple-lifetime boilerplate to single-`'a` + `BoxFut`.

```rust
pub trait StatelessHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        input: serde_json::Value,
    ) -> BoxFut<'a, Result<serde_json::Value, ActionError>>;
}

pub trait StatefulHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        state: &'a mut serde_json::Value,
        input: serde_json::Value,
    ) -> BoxFut<'a, Result<serde_json::Value, ActionError>>;
}

pub trait TriggerHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
    fn handle<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        event: serde_json::Value,
    ) -> BoxFut<'a, Result<(), ActionError>>;
}

pub trait ResourceHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        resource_id: ResourceId,
        input: serde_json::Value,
    ) -> BoxFut<'a, Result<serde_json::Value, ActionError>>;
}
```

JSON-typed input/output at the handler boundary preserves the JSON-level contract `crates/action/src/handler.rs:11-19` documents. Each handler trait is dyn-safe (per rust-senior 02c §6 line 358) — `Arc<dyn StatelessHandler>` continues to compile post-modernization. The `serde_json::from_value` adapter call sites are where G3 floor item 1 (JSON depth cap 128) attaches; detail in §4 (CP2).

### §2.5 `ActionHandler` enum — 4 variants, no Control variant

```rust
#[non_exhaustive]
pub enum ActionHandler {
    Stateless(Arc<dyn StatelessHandler>),
    Stateful(Arc<dyn StatefulHandler>),
    Trigger(Arc<dyn TriggerHandler>),
    Resource(Arc<dyn ResourceHandler>),
}
```

Engine dispatches on the 4-variant enum per current `crates/action/src/handler.rs:39-50` (preserved post-modernization — the variant set is the canon §3.5 dispatch core, and ADR-0038 §1 confirms the DX tier "erases to primary" at runtime).

**No `Control` variant.** `ControlAction` is sealed (§2.6) and erases to `Stateless` via adapter at registration time; the engine never sees a `Control` variant. This is the load-bearing property ADR-0038 §1 / §2 ratifies — adding a primary dispatch trait requires canon revision (§0.2 trigger), but adding a sealed DX trait does NOT (per ADR-0038 §2 revised wording).

### §2.6 Five sealed DX traits per ADR-0038

The DX tier wraps the primary dispatch traits with authoring-friendly shapes that erase to `Stateless` / `Stateful` / `Trigger` at dispatch. Each DX trait is **sealed** per ADR-0038 §1 — community plugin crates may NOT implement it directly; they go through the underlying primary trait + adapter.

Sealing follows the per-capability inner-sealed-trait pattern from [ADR-0035 §3](../../adr/0035-phantom-shim-capability-pattern.md#3-sealed-module-placement-convention) (the post-amendment-2026-04-24-B canonical form). The `mod sealed_dx` is crate-private; each inner `Sealed` trait is `pub` within that module (so the public DX trait's supertrait reference does not trigger `private_in_public`).

```rust
mod sealed_dx {
    // Per-capability inner sealed traits — one per DX trait, per ADR-0035 §3
    // canonical form. Outer `mod sealed_dx` is crate-private; inner traits are
    // pub-within-scope so the public DX trait can reference them as supertrait.
    pub trait ControlActionSealed {}
    pub trait PaginatedActionSealed {}
    pub trait BatchActionSealed {}
    pub trait WebhookActionSealed {}
    pub trait PollActionSealed {}
}

// Erases to Stateless via adapter:
pub trait ControlAction: sealed_dx::ControlActionSealed + StatelessAction { /* ... */ }

// Erase to Stateful via adapter (adapter holds the iteration state):
pub trait PaginatedAction: sealed_dx::PaginatedActionSealed + StatefulAction { /* ... */ }
pub trait BatchAction:     sealed_dx::BatchActionSealed     + StatefulAction { /* ... */ }

// Erase to Trigger via adapter:
pub trait WebhookAction: sealed_dx::WebhookActionSealed + TriggerAction { /* ... */ }
pub trait PollAction:    sealed_dx::PollActionSealed    + TriggerAction { /* ... */ }

// Crate-internal blanket impls: one per DX trait, each authoring eligibility from
// the corresponding primary + `ActionSlots` (so the seal mirrors the §2.1 `Action`
// supertrait chain — without `ActionSlots`, the seal would admit types that cannot
// satisfy `Action`, breaking ADR-0038 §1's "DX tier erases to primary" invariant).
// Spike `final_shape_v2.rs:282` is the canonical bound. Community plugins use the
// primary trait directly + the sealed adapter pattern at registration. Trait-by-trait
// audit of which primary each DX trait wraps is locked at CP3 §7 design time per
// ADR-0038 §Implementation notes ("trait-by-trait audit at Tech Spec §7 design time").
impl<T: StatelessAction + ActionSlots> sealed_dx::ControlActionSealed for T {}
impl<T: StatefulAction  + ActionSlots> sealed_dx::PaginatedActionSealed for T {}
impl<T: StatefulAction  + ActionSlots> sealed_dx::BatchActionSealed     for T {}
impl<T: TriggerAction   + ActionSlots> sealed_dx::WebhookActionSealed   for T {}
impl<T: TriggerAction   + ActionSlots> sealed_dx::PollActionSealed      for T {}
```

**Erasure adapter pattern.** Each DX trait erases to its primary at dispatch through a crate-internal adapter. For example, `ControlAction` erases to `StatelessHandler` via `ControlActionAdapter<A: ControlAction>` that wraps the body's typed `Continue` / `Skip` / `Retry` / `Terminate` result variants into an `ActionResult<Value>` (current shape; adapter detail is §3 / CP3 §9 scope). The adapter is the only path to dispatch — community plugins cannot bypass it because the sealed bound prevents `impl ControlAction for X` outside the crate.

**Community plugin authoring path** (per ADR-0038 §1 + ADR-0038 §Negative item 4). External plugin crates do **NOT** implement any sealed DX trait directly. The five DX shapes — pagination, batch, control-flow, webhook, poll — are authored via `StatelessAction` / `StatefulAction` / `TriggerAction` primary trait impls plus `#[action]` macro attribute zones (`#[action(paginated(cursor = …, page_size = …))]`, `#[action(control_flow = …)]`, etc.; CP2 §4 locks the attribute syntax). The macro emits the appropriate sealed-adapter impl from the cascade-internal `nebula-action::sealed_dx::*` namespace; the engine erases to the primary at dispatch. Migration: code that today writes `impl ControlAction for X` moves to `impl StatelessAction for X` + `#[action(control_flow = …)]` per ADR-0038 §Negative item 4. CP3 §7 surfaces the end-to-end community-plugin example.

### §2.7 `ActionResult` variants — including Terminate decision

This is the **load-bearing decision in CP1**. Strategy §4.3.2 (line 224-229) locked the symmetric-gating principle; CP1 §2.7 picks the concrete path now per the active-dev mode rule (no gate-only-and-defer).

#### §2.7.1 Decision: wire `Terminate` end-to-end

**Picked: wire-end-to-end** for both `ActionResult::Retry` (existing `unstable-retry-scheduler` feature) and `ActionResult::Terminate` (new `unstable-action-scheduler` feature gating both, OR keep separate `unstable-terminate-scheduler` — feature-flag granularity locked at CP3 §9). Either way, both variants graduate from gated-with-stub to wired-end-to-end in cascade scope.

**Evidence in support of wire-end-to-end** (vs the alternative — retire and stay gated-with-wired-stub):

1. **Phase 0 finding S3** ([`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §4 row "S3"): `crates/action/src/result.rs:217` documents `Terminate` as "Phase 3 of the ControlAction plan and is not yet wired" — a literal canon §4.5 false-capability violation today. Strategy §2.3 line 70 binds the resolution to wiring discipline.
2. **Tech-lead Phase 1 solo decision** ([`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §7): "feature-gate **AND** wire `Terminate` in cascade, not gate-only-and-defer" — direct cite. The `feedback_active_dev_mode.md` rule binds the wire-end-to-end path.
3. **Strategy §4.3.2 principle (line 226-228)**: "no parallel retry surface; no `Retry` wired and `Terminate` gate-only (asymmetry violates `feedback_active_dev_mode.md`)". Wire-end-to-end is the principled symmetric path.
4. **Scope decision §3 must-have floor**: items 2-3 (CR3 fix + ActionError sanitization) require trace spans + invariant checks per `feedback_observability_as_completion.md`; carrying gated-with-wired-stub for `Retry`+`Terminate` violates the same observability discipline (a gated-stub variant cannot ship trace spans for a code path that doesn't execute).

The alternative (retire + stay gated-with-wired-stub) survives tech-lead's symmetric-gating principle but fails the active-dev rule and is bound only when scheduler infrastructure is **explicitly** out of cascade scope. Strategy §6.9 (line 463-465) confirms scheduler infrastructure ships in cascade scope per the chosen path; CP3 §9 details the engine wiring contract.

#### §2.7.2 Concrete `ActionResult` variants

Below is the full enum signature including the feature gate. The shape is preserved from `crates/action/src/result.rs:55-224` (current production shape) with the `Retry` gate adjusted per §2.7.1.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum ActionResult<T> {
    Success { output: ActionOutput<T> },
    Skip { reason: String, output: Option<ActionOutput<T>> },
    Drop { reason: Option<String> },
    Continue {
        output: ActionOutput<T>,
        progress: Option<f64>,
        #[serde(default, with = "duration_opt_ms")] delay: Option<Duration>,
    },
    Break { output: ActionOutput<T>, reason: BreakReason },
    Branch {
        selected: BranchKey,
        output: ActionOutput<T>,
        alternatives: HashMap<BranchKey, ActionOutput<T>>,
    },
    Route { port: PortKey, data: ActionOutput<T> },
    MultiOutput {
        outputs: HashMap<PortKey, ActionOutput<T>>,
        main_output: Option<ActionOutput<T>>,
    },
    Wait {
        condition: WaitCondition,
        #[serde(default, with = "duration_opt_ms")] timeout: Option<Duration>,
        partial_output: Option<ActionOutput<T>>,
    },

    /// Re-enqueue for engine-driven retry. Feature-gated until scheduler ships;
    /// wire-end-to-end at scheduler landing per Strategy §4.3.2 + §2.7.1.
    #[cfg(feature = "unstable-retry-scheduler")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable-retry-scheduler")))]
    Retry {
        #[serde(with = "duration_ms")] after: Duration,
        reason: String,
    },

    /// End the whole execution explicitly. Feature-gated parallel to Retry per
    /// Strategy §4.3.2 symmetric-gating discipline; wire-end-to-end at scheduler
    /// landing. Engine integration hook locked in §3 (CP1) — the engine's
    /// scheduler consumes this variant identically to Retry's re-enqueue path:
    /// scheduler cancels sibling branches, propagates `TerminationReason` into
    /// audit log per `crates/action/src/result.rs:212-218`.
    #[cfg(feature = "unstable-terminate-scheduler")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable-terminate-scheduler")))]
    Terminate { reason: TerminationReason },
}
```

**Decision (CP1) on feature-flag granularity** — committed: **parallel flags** `unstable-retry-scheduler` + `unstable-terminate-scheduler`. Per Strategy §4.3.2 symmetric-gating discipline (line 222-229) the two variants share gating discipline but the *names* are independently meaningful: `Retry` and `Terminate` consume distinct scheduler subsystems (re-enqueue vs sibling-branch-cancel + termination-audit), so a downstream that wants only one path can compile-time-disable the other. Per §0.2 invariant 4: this Tech Spec freezes the parallel-flag signature; CP3 §9 may amend the *internal scheduler implementation* but cannot rename or unify the public flags without an ADR amendment. (Resolves devops 08e NIT 1: §2.7.2 freeze surface no longer pretends-frozen-but-deferred.)

**Open item §2.7-2** — engine scheduler-integration hook detail (the dispatch path `Retry` + `Terminate` follow into the engine's scheduler module) is referenced in §3 below as "scheduler integration hook" but full detail is CP3 §9 scope. The hook contract is: engine receives `ActionResult::{Retry, Terminate}` from the adapter, persists the per-execution dispatch metadata via `ExecutionRepo` (preserving canon §11.3 idempotency per Strategy §2.5), and routes to the scheduler's re-enqueue path or termination path; CP3 §9 locks the trait surface.

### §2.8 `ActionError` taxonomy — confirm reference-quality

Per rust-senior 02c §7 line 428: "Error taxonomy is **the cleanest part of the crate idiomatically**. Two-axis hint vs classify split is disciplined; `Arc<dyn Error>` for `Clone` is correct; input sanitization is reference-quality; `ActionErrorExt` is DX-justified. The `DisplayError` wrapper is a minor curiosity, not a defect. **No 🔴 findings in error design.**"

CP1 confirms preservation of:

- **`RetryHintCode`** (`crates/action/src/error.rs:31-48`) — engine retry-strategy hints (`RateLimited` / `Conflict` / `AuthExpired` / `UpstreamUnavailable` / `UpstreamTimeout` / `InvalidInput` / `QuotaExhausted` / `ActionPanicked`).
- **`ValidationReason`** (`crates/action/src/error.rs:58-71`) — categorized validation failure reason (`MissingField` / `WrongType` / `OutOfRange` / `MalformedJson` / `StateDeserialization` / `Other`).
- **`<ActionError as nebula_error::Classify>::code()`** (`crates/action/src/error.rs:284-294`) — stable cross-crate taxonomy tag (`ACTION:RETRYABLE`, `ACTION:VALIDATION`, etc.).

The two-axis split (user-supplied hint via `RetryHintCode` ≠ framework classifier via `Classify::code()`) is preserved unchanged.

**Modification in scope of this Tech Spec** (G3 floor item 3): `ActionError` Display routes through `redacted_display()` helper in `tracing::error!` call sites per Strategy §2.6 + §4.4 item 3. Helper crate location is CP2 §4 scope (Strategy §5.1.2 open item — likely `nebula-log` or new `nebula-redact`). Variant set unchanged; only Display surface adjusted.

**`SchemeGuard<'a, C>` is `!Clone`** per credential Tech Spec §15.7 + ADR-0037 §3 — the qualified-syntax probe `<SchemeGuard<'_, C> as Clone>::clone(&guard)` is mandated at test time to catch the auto-deref Clone shadow (where unqualified `guard.clone()` resolves to `Scheme::clone` via `Deref`, defeating the `!Clone` invariant silently). The non-Clone receipt is load-bearing for the cancellation-zeroize invariant in §3.4 — a clonable guard would let the action body retain a copy past scope, defeating the `tokio::select!` cancellation discipline. Probe discipline is enforced at CP2 §8 testing scope.

### §2.9 Input/Output base-trait consolidation analysis

#### §2.9.1 Question

User raised during CP1 iteration: should `type Input` + `type Output` be hoisted into the base `Action` trait (§2.1) as `Action<Input, Output>`, consolidating the per-trait declarations? Four sub-questions:

1. Are `Input`/`Output` positions consistent across the four primary trait variants (`StatelessAction` / `StatefulAction` / `TriggerAction` / `ResourceAction`)?
2. If consistent, is consolidation into a base trait beneficial?
3. Does consolidation preserve ADR-0035 phantom-shim composition (§4.3 action-side rewrite obligation)?
4. If consolidation breaks composition, is a sub-trait pattern (e.g., `ExecutableAction<I, O>: Action`) viable instead?

#### §2.9.1a User pushback during CP2 iteration (verbatim) and resolution

During CP2 iteration the user pushed back on the §2.9 verdict:

> «у TriggerAction тоже есть входные параметры для того чтоб настроить триггер например для RSSTrigger мы можем настроить url, interval допустим. для KafkaTrigger можем настроить канал и действие после ack.»

(Translation: "TriggerAction also has input parameters to configure the trigger — e.g., RSSTrigger can be configured with url + interval; KafkaTrigger with channel + post-ack-action.")

**Resolution: Configuration ≠ Runtime Input.** The user names a real lifecycle artefact (per-instance configuration: RSS url, poll interval; Kafka channel, post-ack handler) but it is not the axis the §2.9 verdict turns on:

1. **Configuration lives in `&self` fields, populated at registration.** Per §4.2 ("Fields outside the zones pass through unchanged"), an action struct may declare ordinary fields — `pub url: String`, `pub interval: Duration`, `pub channel: KafkaChannel` — and the `#[action]` macro emits the struct verbatim with credentials/resources zone-injection composed in. The body methods (`StatelessAction::execute` / `TriggerAction::handle`) read configuration via `&self` (the receiver is `&'a self` per §2.2 RPITIT signatures). `tests/execution_integration.rs:155` is the precedent — `NoOpTrigger { meta: ActionMetadata }` carries configuration in fields. RSSTrigger / KafkaTrigger compose identically.
2. **Configuration schema flows through `ActionMetadata::parameters` (`ValidSchema`) — universally, across all 4 variants.** §4.6.1 binds `#[action(parameters = T)]` to emit `ActionMetadata::with_schema(<T as HasSchema>::schema())` (per `crates/action/src/metadata.rs:292`). This mechanism is **not Trigger-specific** — `parameters = SlackSendInput` works on a `StatelessAction`; `parameters = RSSConfig` works on a `TriggerAction`; same builder, same JSON-schema validation, same UI surface. The schema-zone is universally-keyed. The current `for_stateless` / `for_stateful` / `for_paginated` / `for_batch` helpers at `crates/action/src/metadata.rs:140-222` derive the schema **from `A::Input`** for the three Input-bearing traits as a convenience shortcut; the underlying `with_schema` builder is the universal mechanism and accepts any `ValidSchema` — including a Trigger's externally-supplied configuration schema. (No `for_trigger` helper today is a discoverability gap to address at CP3 §7 ActionMetadata field-set lock, not a structural objection to REJECT.)
3. **Runtime Input is what `execute(.., input)` / `handle(.., event)` parameters carry per dispatch.** `StatelessAction::Input` is "value passed for this dispatch only" (e.g., `SlackSendInput { channel, text }` per dispatch). `TriggerAction::handle`'s parameter is `<Self::Source as TriggerSource>::Event` — projected from the source the trigger listens to (RSS feed payload, Kafka record). Runtime Input comes from a different lifecycle source than configuration; the divergence in §2.9.2 above is over **runtime** Input shape, not configuration shape.

**Verdict on the user's example.** RSS url + interval and Kafka channel are **configuration** (per-instance, registered once, read from `&self` during dispatch, schema declared via `parameters = T` universal zone). They do not break the §2.9 REJECT — REJECT was always about **runtime** `Input`/`Output` consolidation. The user's examples surface a clarification need: the §2.9 framing must distinguish lifecycle phases explicitly. Ratification: REJECT (Option C) preserved; rationale tightened in §2.9.6 below to name the Configuration vs Runtime Input axis. The four trait shapes from `final_shape_v2.rs:209-262` remain the signature-locking source.

**No §2.2 signature ripple.** `final_shape_v2.rs:209-262` does not have a `type Config` on any of the four traits; the spike's PASS is consistent with this resolution. Configuration carrier is `&self`; configuration schema carrier is `ActionMetadata::parameters` via `with_schema`. No new associated type, no signature edit.

**Open item §2.9-1 (CP3 §7 carry).** `ActionMetadata::for_trigger::<A>()` helper — should the metadata-builder convenience layer add a Trigger-shaped helper analogous to `for_stateless` etc.? The current four `for_*` helpers derive from `A::Input`; Trigger has no `Input` so the helper would accept an explicit `parameters_schema: ValidSchema` argument (or a separate `type Config: HasSchema` associated type purely for the helper's discoverability — narrow speculative-DX risk per `feedback_active_dev_mode.md`). CP3 §7 ActionMetadata field-set lock decides; CP2 §2 leaves the universal `with_schema` builder as the ground-truth path.

#### §2.9.2 Consistency check (Q1)

| Variant | `type Input`? | `type Output`? | Execute-shape signature? | Diverging axis |
|---|---|---|---|---|
| `StatelessAction` (§2.2.1) | YES | YES | `execute(&self, ctx, input) -> Future<Result<Output, Error>>` | — |
| `StatefulAction` (§2.2.2) | YES | YES | `execute(&self, ctx, &mut state, input) -> Future<Result<Output, Error>>` | adds `type State` |
| `ResourceAction` (§2.2.4) | YES | YES | `execute(&self, ctx, &resource, input) -> Future<Result<Output, Error>>` | adds `type Resource: Resource` |
| `TriggerAction` (§2.2.3) | **NO** | **NO** | `handle(&self, ctx, event) -> Future<Result<(), Error>>` | input is `<Self::Source as TriggerSource>::Event` (projected via separate trait); output is fixed unit `()` (terminal effect, fire-and-forget per Strategy §3.1 component 7) |

**Verdict on Q1:** NOT uniform. Three of four primaries (Stateless / Stateful / Resource) share the `Input`/`Output` shape. `TriggerAction` diverges on **two** axes:

1. **Input source.** Trigger's "input" is not a free associated type — it is `<Self::Source as TriggerSource>::Event`, a projected type owned by a separate trait. The user-facing input to the trigger body is the event, but the trait's input-shape is `Source: TriggerSource`. Hoisting an `Input` associated type onto `Action` would force `TriggerAction` to either declare `type Input = <Self::Source as TriggerSource>::Event` (redundant projection) or `type Input = ()` (lying about the actual input).
2. **Output absence.** Trigger has no `Output`. The body returns `Result<(), Error>` because triggers fire events into the engine's event channel — they do not produce a value the engine threads forward. Hoisting `Output` onto `Action` would force `type Output = ()` on triggers, which is honest but noise-adding (every trigger declares the same unit type).

Spike `final_shape_v2.rs:209-262` confirms this divergence — the spike's curated extract from commit `c8aef6a0` already reflects the asymmetry, and the four trait definitions there were the shape that compiled end-to-end without consolidation.

#### §2.9.3 Options analysis (Q2-Q4)

**Option (A) — Consolidate `Input`/`Output` into `Action<Input, Output>` base trait.**

```rust
pub trait Action<Input, Output>: ActionSlots + Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
}

pub trait StatelessAction: Action<Self::Input, Self::Output> {
    type Input: HasSchema + DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type Error: ...;
    fn execute<'a>(...) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a;
}

// TriggerAction forced to:
pub trait TriggerAction: Action<<Self::Source as TriggerSource>::Event, ()> {
    type Source: TriggerSource;
    type Error: ...;
    fn handle<'a>(...) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}
```

*Trade-offs.* (+) Single nominal namespace for input/output across the family — reflective tooling that wants "what does this action take + return" has one canonical location. (–) Forces noise on `TriggerAction`: the `<Self::Source as TriggerSource>::Event` projection appears twice (once in `Action<...>` supertrait, once in `handle`'s parameter); `()` Output is meaningless. (–) Adds **two type parameters** to the base `Action` trait — every `dyn Action<I, O>` position must specify both, defeating any homogeneous storage (e.g., a `Vec<Arc<dyn Action<?, ?>>>` cannot exist). The §2.5 `ActionHandler` enum already provides the JSON-erased homogeneous path; adding `Action<I, O>` consolidation neither helps that path nor creates a new useful one. (–) Breaks `Action: ActionSlots + Send + Sync + 'static` simplicity — the `Action` supertrait on `#[action]`-emitted code becomes parameterized, complicating macro emission per ADR-0037 §1.

**Option (B) — Sub-trait pattern: `ExecutableAction<I, O>: Action`.**

```rust
pub trait Action: ActionSlots + Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
}

pub trait ExecutableAction: Action {
    type Input;
    type Output;
}

// StatelessAction / StatefulAction / ResourceAction implement ExecutableAction.
// TriggerAction implements only Action (no Input/Output).
```

*Trade-offs.* (+) Honestly expresses the divergence — the sub-trait names the property "this action shape has Input/Output," and triggers opt out. (+) Reflective tooling that wants "actions with input/output" has a typed predicate (`T: ExecutableAction`). (–) Adds a new trait surface (`ExecutableAction`) that plugin authors must learn, even though they never implement it directly. (–) `#[action]` macro must decide which super-trait to emit; doubles the code-paths in ADR-0037 §1 emission (every per-primary path now also chooses `ExecutableAction` vs not). (–) The benefit ("typed reflective predicate") has no concrete consumer in the current Tech Spec — §3 runtime model goes through `ActionHandler` enum + `serde_json::Value` JSON erasure, not through typed `Input`/`Output` reflection. Adding the trait pre-emptively is **speculative DX surface** per `feedback_active_dev_mode.md` ("before saying 'we will need X', confirm X has a current consumer").

**Option (C) — Reject consolidation; status quo per-trait declaration.**

Each primary declares `type Input` / `type Output` (or omits them, for `TriggerAction`). The four trait shapes mirror their actual semantic divergence: Stateless/Stateful/Resource share an "input → output" shape; Trigger has a different shape ("event → effect").

*Trade-offs.* (+) **Honest.** Each trait reads as what it actually is. `TriggerAction` reads as event-driven; the absence of `Input`/`Output` *signals* the difference at first glance. (+) Macro emission stays simple — ADR-0037 §1 does not need a super-trait choice. (+) No noise on triggers. (+) Spike `final_shape_v2.rs` precedent — the four shapes that compiled were already non-consolidated, and the spike's success at commit `c8aef6a0` validated this shape end-to-end (Probe 1-6 PASS, Iter-2 §2.2 compose PASS, Iter-2 §2.4 cancellation PASS). (–) Apparent symmetry between Stateless/Stateful/Resource gets duplicated three times — but the duplication is only ~2 lines per trait (the `Input`/`Output` declarations), and StatefulAction's `State` + ResourceAction's `Resource` already break the apparent uniformity. (–) Reflective tooling that wants "all actions with input/output" must enumerate three traits explicitly — mild cost, no current consumer.

#### §2.9.4 ADR-0035 composition impact

ADR-0035 §4.3 ("action-side rewrite obligation") binds the `#[action]` macro to translate `CredentialRef<dyn ServiceCapability>` → `CredentialRef<dyn ServiceCapabilityPhantom>` in field-zone rewriting, OR reject the non-phantom form with a guidance diagnostic. The phantom-shim contract is **field-shape-level**: it operates on `CredentialRef<C>` field types declared in `credentials(slot: Type)` zones (per ADR-0036 §Decision item 4 + ADR-0037 §1), not on trait associated types.

- **Option (A):** PRESERVES composition. The `Action<Input, Output>` consolidation operates on associated-type / type-parameter axis, orthogonal to the phantom-shim's field-zone rewriting axis. The macro still emits the same `CredentialRef<dyn ...Phantom>` translation regardless of whether `Input`/`Output` live on `Action` or on the per-primary trait. Mechanically: ADR-0035 §4.3 is satisfied by the same emission contract.
- **Option (B):** PRESERVES composition. Same orthogonality — `ExecutableAction` is a marker over Input/Output presence; phantom-shim operates on field types. No interaction.
- **Option (C):** PRESERVES composition. Status quo — current shape is already what ADR-0035 §4.3 was drafted against. No change required.

**Verdict on ADR-0035:** all three options preserve the §4.3 obligation. The phantom-shim contract is structurally independent of `Input`/`Output` placement. This question does NOT bind the decision.

#### §2.9.5 Decision

**REJECT consolidation. Status quo (Option C) preserved.** (Rationale tightened during CP2 iteration 2026-04-24 per §2.9.1a — explicit Configuration vs Runtime Input axis named; configuration goes through `&self` + `ActionMetadata::parameters` universally; runtime Input divergence is what consolidation cannot honestly resolve.)

#### §2.9.6 Rationale

The analysis surfaces a **shape mismatch** that consolidation cannot honestly resolve. Before the rationale: **the §2.9 axis is Runtime Input/Output, not Configuration.** Per §2.9.1a above, configuration (per-instance settings — RSS url, Kafka channel) lives in `&self` struct fields with schema declared through `ActionMetadata::parameters` via `with_schema` (per `crates/action/src/metadata.rs:292`); this is universal across all 4 variants and orthogonal to consolidation. The shapes below concern runtime Input — what the engine threads to `execute(.., input)` / `handle(.., event)` per dispatch.

1. **Trigger's runtime-input/output divergence is structural, not stylistic.** `TriggerAction` has `type Source: TriggerSource` because triggers are event-driven — the runtime input shape is "event from a source," not "user-supplied parameter." Output is unit because triggers terminate by firing events, not by producing values. Forcing `Action<I, O>` parameterization onto a trigger requires lying (`type Input = ()`) or redundant projection (`<Source as TriggerSource>::Event` repeated in supertrait + body). Both violate `feedback_active_dev_mode.md` ("more-ideal over more-expedient") — the more-ideal shape is to let each trait read as what it actually is.

2. **Sub-trait pattern (Option B) has no current consumer.** `ExecutableAction` would be a new surface area plugin authors must learn (even if only through hover), and the only benefit is reflective predication that no current Tech Spec section requires. §2.5 `ActionHandler` enum + §3 runtime dispatch already JSON-erase through `Arc<dyn StatelessHandler>` etc.; adding a typed `ExecutableAction` predicate does not enable any code path in this redesign. Per `feedback_active_dev_mode.md`, speculative surface area is technical debt — adding it now means ADR-0036 / ADR-0037 must absorb it without a current beneficiary.

3. **Spike validated status quo at commit `c8aef6a0`.** The shape in `final_shape_v2.rs:209-262` is non-consolidated by construction. Spike Iter-2 §2.2 (compose test across the family), Iter-2 §2.4 (cancellation across the family), and Probes 1-6 all PASS under the non-consolidated shape. Consolidation would invalidate the spike's "this compiles end-to-end" property — re-validation would be required, and the cost-benefit does not justify the validation work given the absence of a current consumer.

The apparent symmetry between Stateless/Stateful/Resource is shallower than the shared `Input`/`Output` makes it look — `StatefulAction` adds `type State` (with a heavy bound chain per §2.2.2), `ResourceAction` adds `type Resource: Resource`. Three traits already diverge on their non-Input/Output associated types. Hoisting only the Input/Output overlap would consolidate the cosmetic part while leaving the structural divergence in place — a partial consolidation that doesn't simplify the mental model meaningfully.

#### §2.9.7 Implications

**N/A — REJECT.** Status quo §2.2 signatures preserve verbatim. No refactor checklist. ADR-0036 ratification is unaffected; ADR-0035 §4.3 obligation is satisfied without change. Spike `final_shape_v2.rs:209-262` remains the signature-locking source.

**Re-open trigger.** This decision is reconsidered if either of the following fires:

- A fifth primary dispatch trait is proposed that shares the Stateless/Stateful/Resource Input/Output shape (canon §3.5 revision per §0.2). At four of five sharing the shape, the cost-benefit shifts; consolidation may become principled.
- A concrete consumer for typed Input/Output reflection materializes (e.g., a future dependency-typed resource graph that needs to walk the action family by Input/Output type identity). Current §3 / §4 / §7 do not require this.

Neither trigger condition is anticipated in the active cascade. Re-evaluation is a future ADR concern, not Tech Spec scope.

---

## §3 Runtime model

This section describes how the static signatures from §2 compose at engine-runtime — slot registration, HRTB dispatch, capability-specific resolve helpers, and cancellation safety. The narrative cites credential Tech Spec §3.4 line 807-939 as the load-bearing dispatch source.

### §3.1 `SlotBinding` registry registration

At action registration time, the engine collects each action's `ActionSlots::credential_slots()` static slice and indexes the slot bindings by `(action_key, field_name)` for runtime lookup. The binding is `Copy + 'static` (verified by spike `slot.rs` static assert per [NOTES §1.1](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)) so the static slice storage is well-formed:

```rust
// Per credential Tech Spec §3.4 line 851-863 + §9.4 line 2452 (authoritative
// shape — three matching-pipeline variants) + spike final_shape_v2.rs:43-55:

#[derive(Clone, Copy, Debug)]
pub struct SlotBinding {
    pub field_name: &'static str,
    pub slot_type: SlotType,
    pub resolve_fn: ResolveFn,
}

/// Three-variant matching-pipeline shape mirrors credential Tech Spec §9.4
/// line 2452 verbatim — engine-side `iter_compatible` (credential Tech Spec
/// §9.4 line 2456-2470) dispatches on this enum. Spike `final_shape_v2.rs:64`
/// has a degraded two-variant placeholder; credential Tech Spec is canonical
/// for the runtime registry pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SlotType {
    /// Pattern 1 — concrete `CredentialRef<C>` field. Engine matches by
    /// type-id (not capability).
    Concrete { type_id: TypeId },
    /// Pattern 2 — `CredentialRef<dyn ServicePhantom>` field with both a
    /// service identity AND a capability projection. Engine matches by
    /// `cred.metadata().service_key == Some(*service)` AND the registry-
    /// computed capability set per credential Tech Spec §15.8 (CP5
    /// supersession of §9.4): `RegistryEntry::capabilities.contains(*capability)`
    /// rather than the pre-CP5 plugin-metadata field `capabilities_enabled`
    /// (which is REMOVED in §15.8). Same matching axes; capability authority
    /// shifts from plugin metadata to type-system registration time.
    ServiceCapability { capability: Capability, service: ServiceKey },
    /// Pattern 3 — `CredentialRef<dyn AnyBearerPhantom>` field, capability-only
    /// projection (no service binding). Engine matches purely on capability.
    CapabilityOnly { capability: Capability },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Capability { Bearer, Basic, OAuth2 }
```

`ServiceKey` is defined at `nebula-credential` per credential Tech Spec §9.4; Tech Spec re-uses the same identifier without redeclaration. `TypeId` is `core::any::TypeId`.

**Storage shape.** Engine maintains a registry-time map `Map<ActionKey, &'static [SlotBinding]>` populated when `ActionRegistry::register*` is invoked. Per ADR-0037 §1 the macro emits the `&'static [SlotBinding]` slice as part of the `ActionSlots` impl (the `&self` receiver returns the same `&'static` slice for every action instance); engine's `register*` call sites live in `nebula-engine` (`crates/engine/src/registry.rs` is the current host per Phase 0 audit; exact line range and final host-crate path are CP3 §7 scope — `crates/runtime/` does not exist per Phase 1 workspace audit row 4). Engine iterates the slice once at registration and clones the binding entries (cheap — `SlotBinding: Copy`) into the registry-side index.

**Lifecycle.** Slot registration happens at action-registry-construction time, NOT at execution time. The `&'static [SlotBinding]` slice lives for the entire process; cloning into the registry index is a one-time cost per action. Execution-time lookup is `O(1)` via the `(action_key, field_name)` index. CP3 §9 locks the exact registry trait surface; CP1 locks only the input shape (`&'static [SlotBinding]` from `ActionSlots::credential_slots()`).

### §3.2 HRTB fn-pointer dispatch at runtime

The execution-time path from action body call → `SlotBinding` lookup → HRTB `resolve_fn` invocation → `ResolvedSlot` return → `SchemeGuard<'a, C>` construction follows credential Tech Spec §3.4 line 807-939 verbatim. CP1 documents the HRTB shape; CP3 §9 locks the engine-side wrapper.

**HRTB type alias (load-bearing).** Per credential Tech Spec §3.4 line 869:

```rust
// Single-'ctx HRTB fn pointer; cannot be `for<'ctx> async fn(...)` on Rust 1.95
// (no such syntax — see spike NOTES §4 open question 1). BoxFuture return is
// load-bearing, not a wart.
pub type ResolveFn = for<'ctx> fn(
    ctx: &'ctx CredentialContext<'ctx>,
    key: &'ctx SlotKey,
) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>;
```

**Dispatch path.**

1. **Action body invokes** `let bearer: &BearerScheme = ctx.resolved_scheme(&self.bb)?;` (or equivalent — exact ActionContext API location in credential Tech Spec is open item §5.1.1 of Strategy, deadline before CP3 §7 drafting per Strategy §5.1.1 line 270).
2. **Engine looks up** the `SlotBinding` for `self.bb`'s field via the registry-time index; the binding carries the macro-emitted `resolve_fn: ResolveFn` (per ADR-0037 §1 line 47-63).
3. **Engine invokes** `(binding.resolve_fn)(&credential_ctx, &slot_key)` — HRTB monomorphizes per slot at registration; `BoxFuture` is awaited.
4. **Resolve helper (§3.3)** type-reflects `where C: Credential<Scheme = X>` for compile-time enforcement (per spike Probe 6 per [NOTES §1.5](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md): wrong-Scheme `resolve_as_bearer::<BasicCred>` fails `E0277`).
5. **`ResolvedSlot` returns** (e.g., `ResolvedSlot::Bearer { token: SecretString }`); engine **wraps** in `SchemeGuard<'a, C>` via `SchemeGuard::engine_construct(scheme, &'a credential_ctx)` per credential Tech Spec §15.7 line 3503-3516 iter-3 refinement.
6. **`&'a SchemeGuard<'a, C>`** is exposed to the action body via the ActionContext API. Action body calls `Deref` (`&BearerScheme` directly, per credential Tech Spec §3.4 line 916-925) — never sees `&dyn Phantom` (per credential Tech Spec §3.4 line 928).
7. **On scope exit** (normal completion or cancellation per §3.4 below), `SchemeGuard::Drop` zeroizes deterministically.

**Open item §3.2-1 — `ResolvedSlot` wrap point.** Spike NOTES §4 question 5 surfaces an ambiguity: the credential Tech Spec narrative implies `resolve_fn` returns `ResolvedSlot` and engine wraps in `SchemeGuard` after; the spike's interpretation is "engine-side wrapper, not inside `resolve_fn`." CP3 §9 locks this explicitly per the spike's recommendation. CP1 inherits the spike's interpretation pending CP3 ratification.

### §3.3 `resolve_as_<capability><C>` helpers

The capability-specific resolve helpers live in `nebula-engine` (not `nebula-action`). Strategy §3.1 component 3 names this placement explicitly: "`resolve_as_<capability><C>` helpers, slot binding registration at registry time, HRTB fn-pointer dispatch at runtime." Helper signatures are the resolve-site enforcement gate per credential Tech Spec §3.4 step 3 (line 893).

**Full signatures** (one per canonical scheme; spike `resolve.rs` validated all three):

```rust
// In nebula-engine — engine-internal, called via SlotBinding::resolve_fn
// after macro-emission monomorphization at slot registration.

pub fn resolve_as_bearer<C>(
    ctx: &CredentialContext<'_>,
    key: &SlotKey,
) -> BoxFuture<'_, Result<ResolvedSlot, ResolveError>>
where
    C: Credential<Scheme = BearerScheme>,
{
    Box::pin(async move {
        let cred: &C = ctx.registry.resolve::<C>(&key.credential_key)
            .ok_or(ResolveError::NotFound { key: key.credential_key.clone() })?;
        let state: &C::State = ctx.load_state::<C>(&key.credential_key).await?;
        let scheme: BearerScheme = C::project(state);
        Ok(ResolvedSlot::Bearer { /* projected fields */ })
    })
}

pub fn resolve_as_basic<C>(
    ctx: &CredentialContext<'_>,
    key: &SlotKey,
) -> BoxFuture<'_, Result<ResolvedSlot, ResolveError>>
where
    C: Credential<Scheme = BasicScheme>,
{ /* parallel to bearer */ }

pub fn resolve_as_oauth2<C>(
    ctx: &CredentialContext<'_>,
    key: &SlotKey,
) -> BoxFuture<'_, Result<ResolvedSlot, ResolveError>>
where
    C: Credential<Scheme = OAuth2Scheme>,
{ /* parallel to bearer */ }
```

**Where-clause is load-bearing.** `where C: Credential<Scheme = BearerScheme>` is the **resolve-site enforcement gate** per credential Tech Spec §3.4 step 3 (line 893-903). Engine cannot instantiate the helper with a wrong-Scheme concrete type — `error[E0277]` (subsumes `E0271` per Rust 1.95 diagnostic rendering, per spike NOTES §1.5 Probe 6). The complementary declaration-site phantom check (per credential Tech Spec §3.4 step 1, line 822-842) is the first gate; `where`-clause is the second.

**Scheme types.** `BearerScheme` / `BasicScheme` / `OAuth2Scheme` (canonical schemes per credential Tech Spec §15.5; each `ZeroizeOnDrop`, contains `SecretString` per credential Tech Spec §15.7 line 3414-3415) are the projected types `Credential::Scheme` resolves to.

### §3.4 Cancellation safety guarantees (security floor item 4)

This subsection binds G3 floor item 4 (cancellation-zeroize test) at the design level. Detail spec is §4 (CP2); CP1 locks the invariant.

**Invariant.** When the action body's future is dropped — normal completion, scope exit, OR cancellation under `tokio::select!` — every live `SchemeGuard<'a, C>` zeroizes its underlying `C::Scheme` deterministically before the borrow chain unwinds. Spike Iter-2 §2.4 (3 sub-tests in `cancel_drop_zeroize.rs` + 1 in `cancel_in_action.rs`) confirmed PASS under three drop scenarios (normal, cancellation-via-select, cancellation-after-partial-progress).

**Mechanism.**

1. **`tokio::select!` discipline at action body.** Cancellation is propagated via `CancellationToken` per `crates/action/src/context.rs` (current shape). Action body's `.await` points are cancellation points; the body's outermost `tokio::select!` arm receives cancellation AND zeroizes any in-scope `SchemeGuard` via Drop ordering (no manual cleanup required).
2. **`SchemeGuard` Drop ordering.** Per credential Tech Spec §15.7 line 3412 + spike `scheme_guard.rs:144-151`: `impl Drop for SchemeGuard<'a, C>` runs `self.scheme.zeroize()` **before** scope unwind; the `_lifetime: PhantomData<&'a ()>` ensures `'a` cannot outlive the borrow chain (per credential Tech Spec §15.7 line 3503-3516 iter-3 refinement: engine constructs guard with `&'a CredentialContext<'a>` pinning `'a`).
3. **Zeroize invariant.** `C::Scheme: Zeroize` is required at the bound (spike `scheme_guard.rs:111-113`); canonical schemes (`BearerScheme`, `BasicScheme`, `OAuth2Scheme`) all derive `ZeroizeOnDrop` per credential Tech Spec §15.5. Auto-deref Clone shadow (per ADR-0037 §3 + spike finding #1): the qualified-form probe `<SchemeGuard<'_, C> as Clone>::clone(&guard)` is mandated at test time to catch the violation, since unqualified `guard.clone()` resolves to `Scheme::clone` via auto-deref (silent green-pass risk).

**Test contract.** Cancellation-zeroize test ports forward from spike `tests/cancel_drop_zeroize.rs` + `tests/cancel_in_action.rs` (commit `c8aef6a0`) into `crates/action/tests/`. Three sub-tests minimum (per spike Iter-2 §2.4):
- `scheme_guard_zeroize_on_cancellation_via_select` — guard moved into body future, `tokio::select!` cancel branch fires after 10ms.
- `scheme_guard_zeroize_on_normal_drop` — guard scope-exits normally.
- `scheme_guard_zeroize_on_future_drop_after_partial_progress` — body progresses past one `.await`, cancelled at the second.

**Test instrumentation.** Spike used a global `AtomicUsize` counter; production tests must use either a per-test `ZeroizeProbe: Arc<AtomicUsize>` (test-only constructor variant on `Scheme`) OR `serial_test::serial` per spike finding #2 ([NOTES §3](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)). CP2 §8 locks the choice; CP1 binds the test-set requirement.

---

## §4 `#[action]` attribute macro — full token shape

This section locks the **production emission contract** for the `#[action]` attribute macro per [ADR-0037 §1](../../adr/0037-action-macro-emission.md). ADR-0037 names the load-bearing constraints (HRTB fn-pointer shape, dual enforcement layer, qualified-syntax Clone shadow probe, per-slot perf bound); §4 below is the implementer-grade contract that ADR-0037 ratifies.

The macro replaces `#[derive(Action)]` (current `crates/action/macros/src/derive.rs`-style emission per `crates/action/macros/src/action.rs:39-50`) with an attribute macro that participates in **field-zone rewriting** within the struct definition (per [ADR-0036 §Decision item 1](../../adr/0036-action-trait-shape.md)). Migration is hard-cut — `#[derive(Action)]` ceases to exist post-cascade per ADR-0036 §Negative item 1 + `feedback_hard_breaking_changes.md`. Codemod design lands at CP3 §9 per Strategy §4.3.3.

### §4.1 Attribute parser zones

The macro accepts the following zones per [ADR-0036 §Decision item 1](../../adr/0036-action-trait-shape.md) ("rewriting confined to fields declared inside `#[action(credentials(slot: Type), resources(slot: Type))]` attribute zones"):

```rust
#[action(
    key         = "slack.send",
    name        = "Send Slack Message",
    description = "Sends a message to a Slack channel",
    version     = "2.1",
    parameters  = SlackSendInput,                          // §4.6
    credentials(slack: SlackToken),                         // §4.1.1 zone
    resources(http: HttpClient),                            // §4.1.2 zone
)]
pub struct SlackSendAction {
    // body: only fields rewritten by §4.2 contract live here
}
```

#### §4.1.1 `credentials(...)` zone

Each entry has shape `slot_name: CredentialType`, where:
- `slot_name` is a Rust identifier — becomes the rewritten field name on the struct.
- `CredentialType` is one of three credential-type forms (per credential Tech Spec §3.4 line 851-863 + §3.1 SlotType three-variant matching pipeline):
  - **Pattern 1 — concrete credential type.** `slack: SlackToken` rewrites to `pub slack: CredentialRef<SlackToken>`. Engine matches by `TypeId` per `SlotType::Concrete { type_id }`.
  - **Pattern 2 — service-bound capability.** `gh: dyn ServiceCapability<GitHub, Bearer>` rewrites to `pub gh: CredentialRef<dyn ServiceCapabilityPhantom<GitHub, Bearer>>` per [ADR-0035 §4.3](../../adr/0035-phantom-shim-capability-pattern.md) action-side rewrite obligation. Engine matches both service identity and capability per `SlotType::ServiceCapability { capability, service }`.
  - **Pattern 3 — capability-only.** `bearer: dyn AnyBearer` rewrites to `pub bearer: CredentialRef<dyn AnyBearerPhantom>` per ADR-0035 §1 phantom shim. Engine matches by capability alone per `SlotType::CapabilityOnly { capability }`.

Multiple entries comma-separated. Empty zone (`credentials()`) is permitted (zero-credential action). Omitting the zone entirely is permitted; equivalent to `credentials()` (still emits `ActionSlots` impl with `credential_slots() -> &'static []` empty slice — supertrait satisfaction per §2.1).

#### §4.1.2 `resources(...)` zone

Same shape as `credentials(...)`. Each entry `slot_name: ResourceType` rewrites to `pub slot_name: ResourceRef<ResourceType>` (resource handle per Strategy §3.1 component 2). Resource-slot emission shape is **CP3 §7 scope** per §2.1.1 Open Item — CP2 emits the `resources(...)` zone parsing-only contract; full `ResourceBinding` shape locks at CP3.

#### §4.1.3 Zone parser invariants

- **Duplicate `slot_name` within one zone is `compile_error!`** with span at the second occurrence — preempts confusing `E0428: duplicate field` from the rewritten struct.
- **`slot_name` collides with non-zone field name is `compile_error!`** — e.g., `credentials(http: SlackToken)` plus a struct body field `pub http: u32` triggers parser-level rejection. The rewritten struct cannot contain two fields named `http`.
- **Cross-zone `slot_name` collision is `compile_error!` (added during CP2 iteration 2026-04-24 per dx-tester 09d #1).** A slot name appearing in BOTH the `credentials(...)` zone AND the `resources(...)` zone — e.g., `credentials(http: SlackToken)` + `resources(http: HttpClient)` — would currently fall through to `E0428: duplicate field` after macro emission injects two `http` fields into the rewritten struct. CP2 commits the parser-level invariant: cross-zone slot-name collision is preempted with span at the second-zone occurrence and message `note: slot name 'http' is also declared in 'credentials(...)' zone — slot names must be unique across all zones`. The macro maintains a single `HashSet<Ident>` of declared slot names across the parse pass, populated as zones are walked; a second insert returns the prior span for the diagnostic.
- **Unknown attribute key is `compile_error!`** — e.g., `#[action(unkown = ...)]` rejects. `#[action]` is `#[non_exhaustive]`-like at the parser level; new keys land via ADR amendment.

### §4.2 Field-rewriting contract

Per [ADR-0036 §Decision item 1](../../adr/0036-action-trait-shape.md) — **rewriting is confined to fields declared inside the `credentials(...)` / `resources(...)` zones**. Fields outside the zones pass through unchanged.

```rust
// User writes:
#[action(
    key  = "ex.do",
    name = "Example",
    credentials(slack: SlackToken),
)]
pub struct ExampleAction {
    pub config: ExampleConfig,        // NOT rewritten — passes through
    pub max_retries: u32,             // NOT rewritten — passes through
}

// Macro emits:
pub struct ExampleAction {
    pub slack: ::nebula_credential::CredentialRef<SlackToken>,  // injected from zone
    pub config: ExampleConfig,        // pass-through
    pub max_retries: u32,             // pass-through
}
```

**Why narrow.** ADR-0036 §Negative item 2 + `feedback_idiom_currency.md`: pervasive struct-level rewriting harms LSP / grep / IDE hover semantics ("why does this `String` field act like `&str`?" mysteries). Narrow zone-bounded rewriting keeps non-zone fields visible-meaning-preserved while opt-in zones gain typed-handle injection.

**Field ordering.** Zone-injected fields appear **before** struct-body fields in the rewritten struct (preserves source-readable struct iteration order across multiple credential slots — first slot first). Plugin authors must not rely on field order semantically; Tech Spec does not commit to ordering stability across versions.

### §4.3 Per-slot emission

For each `credentials(...)` zone entry, the macro emits the slot binding as a **`SlotBinding` const slice entry** in the `ActionSlots::credential_slots()` body. The HRTB `resolve_fn` is selected by macro pattern-match on the credential type — the macro picks `resolve_as_bearer::<C>` / `resolve_as_basic::<C>` / `resolve_as_oauth2::<C>` from `nebula-engine` per the credential's `Scheme` associated type (the macro reads `<C as Credential>::Scheme = X` at emission time and selects the matching helper).

```rust
// For #[action(credentials(slack: SlackToken))] where SlackToken: Credential<Scheme = BearerScheme>:
impl ::nebula_action::ActionSlots for SlackSendAction {
    fn credential_slots(&self) -> &'static [::nebula_action::SlotBinding] {
        const SLOTS: &[::nebula_action::SlotBinding] = &[
            ::nebula_action::SlotBinding {
                field_name: "slack",
                slot_type: ::nebula_action::SlotType::Concrete {
                    type_id: ::core::any::TypeId::of::<SlackToken>(),
                },
                resolve_fn: ::nebula_engine::resolve_as_bearer::<SlackToken> as ::nebula_action::ResolveFn,
            },
        ];
        SLOTS
    }
}
```

Per [ADR-0037 §1](../../adr/0037-action-macro-emission.md) — `&'static [SlotBinding]` storage is well-formed because `SlotBinding: Copy + 'static` (verified by spike `slot.rs` static assert per [NOTES §1.1](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)). The `resolve_fn` HRTB type alias `for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>` (per §3.2 + credential Tech Spec §3.4 line 869) is the load-bearing shape; `resolve_as_bearer::<SlackToken>` coerces to `ResolveFn` because the function-pointer-as-`Self` coercion preserves the HRTB quantification — verified by spike Iter-2 §2.2 / §2.3 (the const-slot-slices that include `resolve_as_basic::<C>` and `resolve_as_oauth2::<C>` in real action emissions, all compiling at commit `c8aef6a0`). Probe 6 is the **wrong-Scheme rejection** gate (`resolve_as_bearer::<BasicCred>` fires `E0277` when `BasicCred::Scheme = BasicScheme`, not `BearerScheme`), confirming the coercion is constrained to matching Schemes per spike NOTES §1.5; the right-Scheme coercion path is the Iter-2 §2.2/§2.3 evidence, not Probe 6 itself.

**Pattern 2 / Pattern 3 dispatch table.** When the macro sees `slack: dyn ServiceCapability<X, Y>` (Pattern 2) or `bearer: dyn AnyBearer` (Pattern 3), the resolve fn is selected by the **capability marker** projected from the phantom-shim trait per ADR-0035 §1 — `ServiceCapabilityPhantom<X, Bearer>` selects `resolve_as_bearer`; `Basic` capability selects `resolve_as_basic`; `OAuth2` selects `resolve_as_oauth2`. The macro reads the capability marker from the trait's associated `const CAPABILITY: Capability` (per credential Tech Spec §15.5) at emission time.

### §4.4 Dual enforcement layer for declaration-zone discipline

Per [ADR-0036 §Decision item 3](../../adr/0036-action-trait-shape.md) + [ADR-0037 §2](../../adr/0037-action-macro-emission.md). Both layers ship in production:

#### §4.4.1 Type-system layer (always on, structural)

A struct that declares a `CredentialRef<C>` field outside the `credentials(...)` zone has **no `ActionSlots` impl emitted** by the macro (the macro only emits `ActionSlots` for the rewritten struct, and the rewritten struct's fields come from the zone, not the body). The struct cannot satisfy the `Action: ActionSlots + Send + Sync + 'static` supertrait (per §2.1) — registration via `ActionRegistry::register*` is rejected at compile time with `error[E0277]: trait bound X: Action not satisfied`. Spike Probe 3 confirmed this layer is type-system-enforceable per [NOTES §1.4](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md).

This is the **structural ground**: even if a malicious or buggy author bypasses the proc-macro layer (e.g., hand-implements `ActionSlots` on a bare-`CredentialRef` struct), they still hit the type system. ADR-0036 §Negative item 2 names this property as load-bearing.

#### §4.4.2 Proc-macro layer (DX, helpful diagnostic)

When the macro parses an `#[action]` invocation, it walks the struct body and detects any field whose type is `CredentialRef<_>` (or its dyn-shaped equivalents) that is NOT also declared in the `credentials(...)` zone. On such a field, the macro emits `compile_error!("did you forget to declare this credential in `credentials(slot: Type)`?")` with span pointing at the offending field. This fires **before** the type-system layer would error — cleaner DX per [ADR-0037 §2](../../adr/0037-action-macro-emission.md) bullet 2.

```rust
// User writes (mistake):
#[action(key = "x", name = "X")]
pub struct BadAction {
    pub slack: CredentialRef<SlackToken>,   // forgot zone declaration
}

// Macro emits compile_error! with span on `slack`:
//   error: did you forget to declare this credential in `credentials(slot: Type)`?
```

Both layers are intentionally redundant at the catch-the-bug level. The type-system layer is the structural truth; the proc-macro layer optimizes the diagnostic (per ADR-0036 §Negative item 2 — "removing either weakens the contract").

#### §4.4.3 No `ActionSlots` impl outside zones — invariant statement

The macro never emits `impl ActionSlots for X` from anything other than the `credentials(...)` zone declaration. There is no public `ActionSlots` derive, no manual-implementation ergonomic. Hand-implementing `ActionSlots` is technically possible (the trait is `pub`) but discouraged with rustdoc + spike Probe 4 / 5 invariants:

- A hand-rolled `impl ActionSlots for X { fn credential_slots(&self) -> &'static [SlotBinding] { &[] } }` compiles but produces a slot-less action — `ctx.credential::<S>(key)` calls fail at runtime with `ResolveError::NotFound` because no binding exists.
- A hand-rolled impl with non-empty slots referencing a `resolve_fn` that does not match the credential's `Scheme` triggers `error[E0277]` at registration time per spike Probe 6 (resolve-site enforcement gate per §3.3).

**Open item §4.4-1** — should the trait be sealed (per ADR-0035 §3 sealed convention) to prevent hand-implementation entirely? CP3 §9 considers; CP2 leaves `ActionSlots` `pub` because the macro is the recommended path and the spike + ADR-0037 §1 do not require seal.

### §4.5 Per-slot emission cost bound

Per [ADR-0037 §5](../../adr/0037-action-macro-emission.md) + spike §2.5 ([NOTES](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)):

| Component | LOC emitted (one Bearer slot) |
|---|---|
| Field rewrite (1 slot) | 1 |
| `ActionSlots` impl with const slice (1 entry) | ~15 |
| `Action` impl (metadata literal) | ~10 |
| `DeclaresDependencies` impl (replaces hand-written) | ~10 |
| Primary trait impl (`StatelessAction`) with body wrapper | ~25 |
| Existing metadata + `OnceLock` machinery (parity with old) | ~10 |
| **Total per first slot** | **~71 LOC** |

**Naive ratio vs old `#[derive(Action)]`:** 3.2x (71 LOC new / ~22 LOC old per spike §2.5).

**Adjusted ratio (net of user-code absorbed):** **1.6-1.8x**. The old shape required user-written `impl StatelessAction for X { type Input = ...; ... fn execute(...) -> impl Future { async move { /* logic */ } } }` + hand-written `impl DeclaresDependencies for X` referencing `CredentialRef` fields by hand. The new macro absorbs both (~20-25 LOC user effort per old action). Adjusted ratio is the net new emission per equivalent user-effort baseline.

**Linear scaling per additional slot.** Each extra `credentials(...)` zone entry adds ~10 LOC to `ActionSlots::credential_slots()` (one `SlotBinding` literal per slot). For N=3 slots, expect ~91 LOC (71 + 2 × 10). This is the **per-slot gate**, not a per-action gate — Tech Spec §4.5 commits to "per-slot emission ≤10 LOC beyond the first" rather than "per-action emission ≤X LOC." Verifiable via `cargo expand` measurement at any later point per ADR-0037 §5 Positive item 6.

**CI gate.** Macrotest snapshots (§5.2) lock the per-slot byte-budget at the snapshot level — drift fires a snapshot diff. CP3 §9 proposes whether the gate hard-fails CI (recommended) or warns; CP2 commits the snapshot mechanism, not the CI policy.

### §4.6 Parameters / version / schema attribute handling

#### §4.6.1 Phase 0 C2 broken `parameters = Type` path — fix

**Current bug.** `crates/action/macros/src/action_attrs.rs:129-134` emits `.with_parameters(<#ty>::parameters())` in `metadata_init_expr()`. The target method `ActionMetadata::with_parameters()` **does not exist** in `crates/action/src/metadata.rs` (verified `grep with_parameters` — zero matches). The actual builder API is `ActionMetadata::with_schema(schema: ValidSchema)` at `crates/action/src/metadata.rs:292`. Existing actions using `parameters = Type` produce a broken expansion that would fail to compile if exercised — silently dropped because no production `#[derive(Action)]` invocation reaches the parameters-arm in test fixtures (Strategy §1(b) emission-bug class — three independent agents hit this without regression-test coverage).

**Fix in CP2 emission contract.** The `#[action]` macro emits `.with_schema(<#ty as ::nebula_schema::HasSchema>::schema())` for `parameters = Type` per the existing builder contract:

```rust
// Current (BROKEN):  .with_parameters(<#ty>::parameters())
// New (CORRECT):     .with_schema(<#ty as ::nebula_schema::HasSchema>::schema())
```

This aligns with `ActionMetadata::for_stateless::<A>()` at `crates/action/src/metadata.rs:176, 191, 206, 221` which already projects `<A::Input as nebula_schema::HasSchema>::schema()` through `with_schema`. The macro-emitted form is structurally equivalent (extracts schema from the parameters type, threads through the existing builder).

**Compile-fail probe.** §5.3 Probe 7 (added beyond ADR-0037 §4's six-probe table; new) asserts: a `parameters = Type` where `Type` does NOT implement `HasSchema` produces `error[E0277]: trait bound Type: HasSchema not satisfied` at the macro expansion site. Catches the "forgot `#[derive(HasSchema)]`" common case with a typed diagnostic, instead of a confusing "no method named `with_parameters`" — i.e., the **diagnostic surfaces the actual bound that's missing**, not the macro-internal method choice.

#### §4.6.2 `version = "X.Y[.Z]"` parsing

Preserved verbatim from current `crates/action/macros/src/action_attrs.rs:51-54, 200+` (`parse_version` helper). Default `"1.0"` if absent. Threading: `.with_version_full(::semver::Version::new(major, minor, patch))` per `crates/action/macros/src/action_attrs.rs:142`.

#### §4.6.3 `description` doc-fallback

Preserved per `crates/action/macros/src/action.rs:26-31` — if `description` attribute is absent, the macro falls back to the struct's `///` doc-string (joined non-empty lines). Same behavior as current `#[derive(Action)]`.

### §4.7 String-form `credential = "key"` rejection

#### §4.7.1 Current silent-drop bug

`crates/action/macros/src/lib.rs:31-32` documents: "`credential = "key"` (string) is ignored; use `credential = CredentialType` for type-based refs." The macro at `crates/action/macros/src/action_attrs.rs:58, 61` uses `get_type_skip_string("credential")?` — string-form value is **silently dropped** (no error, no warning). Phase 1 dx-tester finding 6 surfaced this as a real DX trap (plugin authors who write `credential = "slack_token"` get zero diagnostic feedback; their action ships with no credential dependency, fails at runtime with `ResolveError::NotFound`).

#### §4.7.2 Fix in CP2 emission contract — hard `compile_error!`

The `#[action]` macro rejects string-form values for `credential`, `credentials`, `resource`, `resources` keys with `compile_error!("the `credential` attribute requires a type, not a string. Use `credential = SlackToken`, not `credential = \"slack_token\"`. The credential's key is provided by `<C as Credential>::KEY`.")` — span at the offending string literal.

```rust
// User writes:
#[action(credential = "slack_token", ...)]   // <- compile_error!
//                    ^^^^^^^^^^^^^

// Diagnostic:
//   error: the `credential` attribute requires a type, not a string.
//          Use `credential = SlackToken`, not `credential = "slack_token"`.
//          The credential's key is provided by `<C as Credential>::KEY`.
```

**Why hard-error not warning.** Per `feedback_no_shims.md` + `feedback_observability_as_completion.md` — silent-drop is the worst possible UX (no DoD invariant check). Hard-error gives a clean migration signal; codemod (CP3 §9) auto-rewrites `credential = "key"` to `credentials(<inferred slot name>: <inferred type>)` form where the type is recoverable from explicit registration sites; otherwise emits a manual-review marker.

**Open item §4.7-1** — Inference success rate for codemod auto-rewrite needs measurement. Strategy §4.3.3 codemod transform 3 names "Codemod must error on remaining call sites with crisp diagnostic, not silently rewrite"; CP3 §9 quantifies the inference success rate against the 7 reverse-deps before committing to auto-rewrite vs manual-marker default.

---

## §5 Macro test harness

This section locks the **production regression harness** that closes Phase 0 T1 + Strategy §1(b). Currently `crates/action/macros/Cargo.toml` (verified at this commit, lines 19-25) has **no `[dev-dependencies]` block** — no `trybuild`, no `macrotest`, no compile-fail coverage. Three independent agents hit emission bugs (CR2 / CR8 / CR9 / CR11) because the regression-coverage hole made it structurally possible. CP2 §5 closes this hole.

### §5.1 `Cargo.toml` `[dev-dependencies]` addition

CP2 commits the dev-deps block to `crates/action/macros/Cargo.toml`:

```toml
[dev-dependencies]
trybuild = "1.0.99"        # compile-fail harness; pinned major version
macrotest = "1.2"          # snapshot harness for emission stability — bumped from 1.0.13 during CP2 iteration 2026-04-24 per devops 09e #1 (current crates.io max 1.2.1; minor-pin tracks latest stable per `feedback_idiom_currency.md`)
```

**Pinning rationale.** `trybuild` 1.0.99 is the latest stable as of cascade close; `macrotest` 1.2 (current crates.io max 1.2.1, minor-pin) tracks latest stable per `feedback_idiom_currency.md` (1.0.13 → 1.2 bump committed during CP2 iteration 2026-04-24 per devops 09e #1).

**Workspace-pin posture (corrected during CP2 iteration 2026-04-24 per devops 09e #2).** Today `trybuild` already has **two** workspace consumers (`crates/schema/Cargo.toml:40` `trybuild = "1"`; `crates/validator/Cargo.toml:46` `trybuild = "1"`); admitting `crates/action/macros` raises consumer count to **three**. Per `feedback_boundary_erosion.md` + version-cohesion discipline, three crate-local pins risk version-skew across compile-fail surfaces. CP3 §9 has a forward-track decision: (a) promote `trybuild` to a workspace dep (`[workspace.dependencies] trybuild = "1.0.99"`) and rewrite all three consumers to `trybuild = { workspace = true }`; (b) keep crate-local pins and document the cohesion expectation in a workspace-level cargo-deny check. CP2 commits the localized pin for the macro crate (preserves spike-validated shape); CP3 §9 picks a/b. The earlier "only consumer" framing is corrected — `trybuild` is the third consumer, not the first.

**Open item §5.1-1** — `cargo-public-api` snapshot for the macro crate is **out of scope** per ADR-0037 §4 ("macro test harness ships with implementation"). Surface stability is at the trait level (§2), not the proc-macro internal token level. CP3 §9 may revisit if reviewer flags.

### §5.2 Harness layout

```
crates/action/macros/
├── Cargo.toml                          (gains §5.1 dev-deps)
├── src/                                (existing macro source)
└── tests/                              (NEW)
    ├── compile_fail.rs                 (trybuild driver — runs all probes)
    ├── compile_fail/
    │   ├── probe_1_resource_no_resource.rs    + .stderr
    │   ├── probe_2_trigger_no_source.rs       + .stderr
    │   ├── probe_3_bare_credential_ref.rs     + .stderr
    │   ├── probe_4_scheme_guard_clone.rs      + .stderr
    │   ├── probe_5_scheme_guard_retain.rs     + .stderr
    │   ├── probe_6_wrong_scheme.rs            + .stderr
    │   └── probe_7_parameters_no_schema.rs    + .stderr  (NEW; §4.6.1)
    ├── expansion.rs                    (macrotest driver — runs all snapshots)
    └── expansion/
        ├── stateless_bearer.rs         (input)  + stateless_bearer.expanded.rs (snapshot)
        ├── stateful_oauth2.rs          (input)  + stateful_oauth2.expanded.rs
        └── resource_basic.rs           (input)  + resource_basic.expanded.rs
```

Layout mirrors spike commit `c8aef6a0` `tests/compile_fail/` (for the trybuild side) plus a **macrotest expansion side** newly added in CP2 to lock per-slot emission stability per §4.5. Snapshot files commit alongside source per `feedback_lefthook_mirrors_ci.md` discipline (CI runs `cargo nextest run -p nebula-action-macros --profile ci`; snapshots fail if drift).

### §5.3 6-probe port from spike commit `c8aef6a0` + Probe 7

Each probe ports from spike `tests/compile_fail/probe_{1..6}_*.rs` (commit `c8aef6a0`) into `crates/action/macros/tests/compile_fail/`. Probe 7 is **new** in CP2 per §4.6.1.

| Probe | Asserts | Expected diagnostic | Source |
|---|---|---|---|
| 1 | `ResourceAction` impl missing `Resource` assoc type | `E0046` | spike NOTES §1.2 |
| 2 | `TriggerAction` impl missing `Source` assoc type | `E0046` | spike NOTES §1.3 |
| 3 | Bare `CredentialRef<C>` field outside `credentials(...)` zone | `E0277` (type-system layer per §4.4.1) **AND** `compile_error!` (proc-macro layer per §4.4.2) | spike NOTES §1.4 + ADR-0037 §2 |
| 4 | `<SchemeGuard<'_, C> as Clone>::clone(&guard)` qualified-syntax probe — see §5.4 | `E0277` (`SchemeGuard: !Clone`) | spike NOTES §1.5 + ADR-0037 §3 |
| 5 | `SchemeGuard` retention beyond `'a` lifetime (`MisbehavingPool { cached: Option<SchemeGuard<'static, C>> }`) | `E0597` — borrowed value does not live long enough | spike NOTES §1.5 |
| 6 | Wrong-Scheme `resolve_as_bearer::<BasicCred>` (where `BasicCred::Scheme = BasicScheme`, not `BearerScheme`) | `E0277` (subsumes `E0271` per Rust 1.95 diagnostic rendering) | spike NOTES §1.5 + §3.3 |
| **7** (new) | `parameters = Type` where `Type: !HasSchema` | `E0277: HasSchema not satisfied` (typed bound, not "no method `with_parameters`") | §4.6.1 |

**Probe 5 / Probe 6 cross-crate dependency.** These probes exercise `SchemeGuard<'a, C>` + `resolve_as_bearer::<C>` shapes that live in `nebula-credential` + `nebula-engine` (per §3.3 placement). The macro-tests crate must depend on both for compile-fixtures to resolve. CP3 §7 confirms the dev-deps wiring — for CP2 we commit the dev-dep entries to `crates/action/macros/Cargo.toml`:

```toml
[dev-dependencies]
trybuild = "1.0.99"
macrotest = "1.2"
nebula-action = { path = ".." }                    # action surface (Action trait, ActionSlots, etc.)
nebula-credential = { path = "../../credential" }  # SchemeGuard, Credential, CredentialRef
nebula-engine = { path = "../../engine" }          # resolve_as_bearer/_basic/_oauth2 helpers
```

**Open item §5.3-1 — RESOLVED at CP2 iteration 2026-04-24 per rust-senior 09b #1.** `nebula-engine` as a dev-dep on `nebula-action-macros` is the **committed path**, not the stub-helper alternative. Rationale: spike Probe 6 needs the **real** `resolve_as_bearer::<C>` helper from `nebula-engine` to verify the wrong-Scheme bound mismatch (Probe 6 fires on `BasicCred::Scheme = BasicScheme` against `resolve_as_bearer::<BasicCred>`); a stub-helper test fixture would mirror the function signature but lose the property the probe actually exercises (real bound coercion against the real HRTB shape coerces correctly to `ResolveFn`, only failing for wrong-Scheme — that's the property under test).

**Companion commitment — `deny.toml` wrappers amendment.** `deny.toml` enumerates per-crate dependency-direction wrappers; admitting `nebula-engine` as a dev-dep on `nebula-action-macros` requires adding `nebula-action-macros` to the deny-config wrapper list with an inline reason. CP2 commits the amendment shape (CP3 §9 lands the `deny.toml` edit alongside the macro-crate dev-deps wiring):

```toml
# deny.toml (CP3 amendment shape — wrapper entry for nebula-action-macros):
# Justification: dev-only dependency on nebula-engine for compile-fail Probe 6
# (real `resolve_as_bearer::<C>` HRTB coercion bound-mismatch verification).
# Stub-helper alternative loses real-bound verification — see Tech Spec §5.3-1.
```

This is not a layering violation in the ordinary sense (it's `[dev-dependencies]` only, not a runtime dependency cycle), but `feedback_boundary_erosion.md` discipline requires explicit acknowledgement. The dev-only direction is preserved at runtime — `nebula-action-macros` builds without `nebula-engine` in its production dependency closure; only the test target pulls it in. CP3 §9 lands the `deny.toml` wrapper entry + verifies via `cargo deny check` post-amendment.

### §5.4 Auto-deref Clone shadow probe — qualified-syntax form

Per [ADR-0037 §3](../../adr/0037-action-macro-emission.md) + spike finding #1 ([NOTES §3](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)):

The naive form `let g2 = guard.clone();` does NOT compile-fail for `SchemeGuard<'_, C>`. Mechanism: `SchemeGuard: Deref<Target = C::Scheme>`, and canonical schemes (`BearerScheme`, `BasicScheme`, `OAuth2Scheme`) all derive `Clone` for ergonomics (per credential Tech Spec §15.5). Auto-deref resolves `guard.clone()` against `Scheme` — produces a Scheme clone (which is itself a leak — `Scheme` contains `SecretString`, also `Clone`). The compile-fail probe **silently green-passes** while the `SchemeGuard: !Clone` invariant is violated by user code.

**Production probe form (mandated by CP2 + ADR-0037 §3):**

```rust
// crates/action/macros/tests/compile_fail/probe_4_scheme_guard_clone.rs
use nebula_action::{action, ActionContext};
use nebula_credential::{CredentialRef, SchemeGuard};
use slack_creds::SlackToken;

#[action(key = "ex.do", name = "Ex", credentials(slack: SlackToken))]
pub struct ExAction;

async fn body(ctx: &ActionContext<'_>, action: &ExAction) {
    let guard: &SchemeGuard<'_, SlackToken> = ctx.resolved_scheme(&action.slack).unwrap();
    // The qualified form bypasses auto-deref and exercises SchemeGuard's Clone (which doesn't exist):
    let _g2 = <SchemeGuard<'_, SlackToken> as Clone>::clone(guard);  // E0277 fires here
}
```

**Why qualified-form is mandatory.** The unqualified form `guard.clone()` is the user-trap shape (auto-deref to `Scheme::clone`). The qualified form `<SchemeGuard<'_, C> as Clone>::clone(&guard)` skips method resolution to `Scheme::clone` because the explicit trait projection forces the resolver to look only at `SchemeGuard`'s `Clone` impl — which does not exist. `error[E0277]: trait bound SchemeGuard<'_, SlackToken>: Clone not satisfied` fires.

##### §5.4-companion Author-trap regression-lock probe (dx-tester 09d #2)

The qualified-form probe asserts `SchemeGuard: !Clone` is **structurally enforced**, but does NOT regression-lock the **author-trap** itself — the silent-pass shape that real users would write. A second probe is added during CP2 iteration 2026-04-24 to lock the trap behavior explicitly:

```rust
// crates/action/macros/tests/compile_pass/probe_4b_scheme_guard_clone_unqualified.rs
// NOTE: this is a compile-PASS test (not compile-fail) under trybuild's `pass`
// directory — its purpose is to regression-lock the AUTO-DEREF SILENT-PASS
// shape: the unqualified form compiles. The behavioral consequence (a Scheme
// clone that DEFEATS the !Clone invariant) must be caught by a runtime
// assertion in §6.4 cancellation-zeroize tests OR a clippy lint at the
// emission boundary (CP3 §9 design scope).
use nebula_action::{action, ActionContext};
use nebula_credential::{CredentialRef, SchemeGuard};
use slack_creds::SlackToken;

#[action(key = "ex.do", name = "Ex", credentials(slack: SlackToken))]
pub struct ExAction;

async fn body(ctx: &ActionContext<'_>, action: &ExAction) {
    let guard: &SchemeGuard<'_, SlackToken> = ctx.resolved_scheme(&action.slack).unwrap();
    // The unqualified form auto-derefs to Scheme::clone — compiles silently.
    // This probe regression-LOCKS the silent-pass behavior: if the auto-deref
    // pathway were closed (e.g., by a future hand-off impl SchemeGuard: !Deref<Target = Scheme>),
    // this probe fails and the dual-probe pair (this + qualified §5.4) is re-derived.
    let _scheme_clone = guard.clone();   // compiles; produces Scheme clone via Deref + Scheme::Clone
}

fn main() {}
```

The pair (qualified-form compile-fail + unqualified-form compile-pass) makes the silent-pass shape **observable** at the test surface. CP3 §9 design scope: decide whether a clippy-lint at the macro emission boundary should warn on `<SchemeGuard as Deref>::deref().clone()` paths (would surface the trap to authors before runtime). CP2 commits the dual-probe regression-lock; the lint is a separate forward-track item.

#### §5.4.1 Soft amendment к credential Tech Spec §16.1.1 probe #7 — flagged, not enacted

Credential Tech Spec §16.1.1 probe #7 (line 3756) currently specifies:

> | 7 | `tests/compile_fail_scheme_guard_clone.rs` | `let g2 = guard.clone()` on `SchemeGuard` | `E0599` — no method `clone` |

This is the **silent-pass shape** flagged by spike finding #1. Per ADR-0037 §3 + ADR-0035 amended-in-place precedent, this is a **soft amendment candidate** к credential Tech Spec — the probe form should re-pin to:

> | 7 | `tests/compile_fail_scheme_guard_clone.rs` | `<SchemeGuard<'_, C> as Clone>::clone(&guard)` qualified-syntax form on `SchemeGuard` | `E0277` — `Clone` bound not satisfied (subsumes naive `E0599` because qualified form bypasses auto-deref) |

**This Tech Spec FLAGS the amendment** but does NOT enact it. Per ADR-0035 amended-in-place precedent, cross-crate amendments to credential Tech Spec are coordinated via the credential Tech Spec author (architect). Tech Spec ratification (CP4) records the amendment as an outstanding cross-cascade item; the amendment lands as a credential Tech Spec inline edit + CHANGELOG entry, not via a new ADR.

**Forward-track to credential Tech Spec author.** During CP4 cross-section pass: surface §16.1.1 probe #7 as soft amendment candidate; coordinate with credential Tech Spec author to land the amendment inline (per the §0.2 precedent — "*Amended by ADR-0037, 2026-04-24*" prefix at the §16.1.1 probe #7 row, plus updated diagnostic column). Until amendment lands, the production credential probe at `crates/credential/tests/compile_fail_scheme_guard_clone.rs` would use the unqualified form (silent-pass risk). The action-side probe (§5.4 above) catches the violation independently.

### §5.5 Macrotest expansion snapshots

Per §4.5 — three snapshot fixtures lock per-slot emission stability:

- `expansion/stateless_bearer.rs` — minimal `#[action(credentials(slack: SlackToken))]` + `StatelessAction` impl. Snapshot: ~71 LOC expanded.
- `expansion/stateful_oauth2.rs` — `#[action(credentials(gh: GitHubOAuth2))]` + `StatefulAction` impl with state. Snapshot: ~85 LOC expanded (state-handling adds ~14 LOC).
- `expansion/resource_basic.rs` — `#[action(credentials(pg: PostgresBasicCred), resources(pool: PostgresPool))]` + `ResourceAction` impl. Snapshot: ~95 LOC expanded (resource-handling + credential composition).

**CI policy.** `cargo nextest run -p nebula-action-macros --profile ci` includes the expansion snapshot tests (`macrotest::expand_args` per macrotest 1.2 API; CP3 §9 verifies `expand_args` shape against macrotest 1.2.x — flag if signature drifted from 1.0.13). Snapshot drift fails CI; intentional regeneration via `MACROTEST=overwrite cargo test -p nebula-action-macros`. Per `feedback_lefthook_mirrors_ci.md`, lefthook pre-push must mirror this.

---

## §6 Security must-have floor (CO-DECISION territory)

This section is **co-decision tech-lead + security-lead**. Authority sourcing (corrected during CP2 iteration 2026-04-24 per spec-auditor 09a #2):

- **Co-decision authority** — Strategy §4.4 (security must-have floor invariant verbatim, lines 245-254) + 03c §1 VETO + §1 G3 freeze invariant. Strategy §6.3 lines 386-394 is the per-CP **reviewer matrix** table (CP2a / CP2b reviewer routing), NOT the co-decision authority basis.
- Strategy §4.4 binds the four floor items as invariants per `feedback_observability_as_completion.md` ("typed error + trace span + invariant check are DoD"). §1 G3 already binds the items as freeze invariants per §0.2 item 3; §6 below locks the **concrete implementation forms** that close the security 03c §1 VETO conditions and the CP2 readiness gaps from 08c §CP2.

Security-lead retains **implementation-time VETO authority** on shim-form drift per security 03c §1 + §1 G3 + Strategy §4.4 item 2. Items below explicitly call out the VETO trigger language (verbatim from 03c) on §6.2 to make the boundary unambiguous.

### §6.1 JSON depth cap (128) implementation

Closes **S-J1 (CR4)** per Strategy §2.12 item 1 + 03c §2 item 1. Depth cap **128** at every adapter JSON boundary.

**Cap origin (corrected during CP2 iteration 2026-04-24 per spec-auditor 09a #3).** Cap = 128 originates from Strategy §2.12 item 1 / 03-scope-decision §3 must-have floor (action-adapter boundary). The existing `check_json_depth` primitive at `crates/action/src/webhook.rs:1378-1413` is **parameter-driven** (`max_depth: usize`) and has **no hardcoded cap** — webhook.rs:331-345 *recommends* `max_depth: 64` for webhook bodies (smaller real-payload-grounded cap), distinct from the action-adapter floor. The action-adapter §6 sites adopt cap=128 per Strategy must-have, not because the existing primitive enforces it; the primitive is reused as the depth-counting engine, not as the cap-source.

#### §6.1.1 Apply sites — exact line numbers

| Site | File | Line (current shape) | Boundary |
|---|---|---|---|
| `StatelessActionAdapter::execute` | `crates/action/src/stateless.rs` | line 370 (`from_value(input)`) | input deserialization |
| `StatefulActionAdapter::execute` (input) | `crates/action/src/stateful.rs` | line 561 (`from_value(input.clone())`) | input deserialization |
| `StatefulActionAdapter::execute` (state) | `crates/action/src/stateful.rs` | line 573 (`from_value::<A::State>(state.clone())`) | state deserialization (closes S-J2 simultaneously per 03c §1) |

Webhook body deserialization at `crates/api/src/services/webhook/transport.rs` already pre-bounds via `body_json_bounded` (uses `check_json_depth` per `crates/action/src/webhook.rs:1378-1413`); CP2 §6.1 verifies this site is unchanged. CP3 §9 confirms.

#### §6.1.2 Mechanism choice — pre-scan via existing `check_json_depth` primitive (with two pre-CP3 amendments)

Per security 03c §2 item 1 — two acceptable mechanisms: `serde_stacker::Deserializer` wrap, or pre-scan via existing `check_json_depth` primitive before `from_value`. CP2 commits to **pre-scan via existing primitive** with two amendments to the primitive itself, both committed at this Tech Spec section (not deferred):

##### §6.1.2-A Visibility — promote `check_json_depth` to `pub(crate)` (security-lead 09c §6.1-A)

Today the primitive is fn-private at `crates/action/src/webhook.rs:1378` (`fn check_json_depth(...)`). The `crate::webhook::check_json_depth(...)` call from §6.1.2 below is **not callable** at that visibility — it would force CP3 implementer drift toward re-implementation in `stateless.rs` / `stateful.rs`, defeating the **single-audited-primitive** rationale that justified preferring this path over `serde_stacker`. CP2 commits the visibility change explicitly: `pub(crate) fn check_json_depth(...)`. The primitive remains crate-internal (no external API surface widening); `pub(crate)` is the minimum-visibility form that preserves the single-audit-point property.

##### §6.1.2-B Return signature — `Result<(), DepthCheckError>` carrying `{observed, cap}` (security-lead 09c §6.1-B)

Today the primitive returns `Result<(), serde_json::Error>` and the cap-exceeded error message is a `format!("webhook body JSON exceeds max depth {max_depth}")` string baked into the `serde_json::Error` payload. To ship a typed `ValidationReason::DepthExceeded { observed, cap }` per `feedback_observability_as_completion.md` (DoD: typed error + trace span + invariant check), the primitive must surface both `observed` and `cap` as integer fields, not as a stringified message. CP2 commits the amendment:

```rust
// crates/action/src/webhook.rs (amended primitive — pre-CP3 visibility + return-shape):
pub(crate) struct DepthCheckError { pub observed: u32, pub cap: u32 }

pub(crate) fn check_json_depth(bytes: &[u8], max_depth: u32) -> Result<(), DepthCheckError> {
    // ... existing byte-walker preserved; on cap-exceed, returns
    //     Err(DepthCheckError { observed: depth as u32, cap: max_depth })
    // instead of a serde_json::Error::custom(format!(...)).
}
```

The `webhook.rs:345` caller (`body_json_bounded`) re-wraps `DepthCheckError` into the existing `serde_json::Error::custom(...)` form to preserve its public API contract; no public-facing API change at the webhook boundary. Action-adapter sites (§6.1.1 below) construct `ActionError::validation` from the typed pair directly. `max_depth` parameter promoted from `usize` to `u32` to match the typed-error fields and avoid platform-width drift in observability sinks.

##### §6.1.2-C Apply-site shape (post-amendments)

```rust
// In stateless.rs:369 (before `from_value`):
let input_bytes = serde_json::to_vec(&input).map_err(|e| {
    ActionError::validation("input", ValidationReason::MalformedJson, Some(e.to_string()))
})?;
crate::webhook::check_json_depth(&input_bytes, 128).map_err(|DepthCheckError { observed, cap }| {
    ActionError::validation(
        "input",
        ValidationReason::DepthExceeded { observed, cap },
        Some(format!("input depth {observed} exceeds cap {cap}")),
    )
})?;
let typed_input: A::Input = serde_json::from_slice(&input_bytes).map_err(...)?;
```

##### §6.1.2-D Rationale + caveat (preserved)

**Rationale.** `check_json_depth` already exists at `crates/action/src/webhook.rs:1378-1413` — adding `serde_stacker` would expand the dep surface (one new transitive dep per `feedback_boundary_erosion.md`). The pre-scan adds one byte-encoding round-trip per dispatch (small cost; alternative is `serde_stacker` wrap which carries ~equivalent allocation cost). The primitive is already audited (used in webhook body bounding); §6.1.2-A + §6.1.2-B preserve that single-audit-point property by amending the primitive itself rather than re-implementing.

**Caveat.** `check_json_depth` operates on bytes, but `from_value` operates on `serde_json::Value`. The pre-scan requires a `to_vec` round-trip (line 1 of §6.1.2-C). Alternative: re-implement a `Value`-walking depth check in a new primitive (`check_value_depth(&Value, 128)`). CP3 §9 picks; CP2 commits to **byte-pre-scan path** (lower implementation cost; existing primitive). Rust-senior CP2 review: flag if `Value`-walking is preferred.

#### §6.1.3 Typed error variant + observability

Per `feedback_observability_as_completion.md`, the depth-exceeded path ships with:

- **Typed error variant.** `ValidationReason::DepthExceeded { observed: u32, cap: u32 }` added to `crates/action/src/error.rs` `ValidationReason` enum (currently has `MissingField` / `WrongType` / `OutOfRange` / `MalformedJson` / `StateDeserialization` / `Other` per §2.8). Variant is `#[non_exhaustive]`-safe (existing enum is `#[non_exhaustive]` per `crates/action/src/error.rs:58-71`).
- **Trace span.** `tracing::warn!(action = %meta.key, observed_depth, cap = 128, "input depth cap exceeded")` at the rejection site.
- **Invariant check.** A unit test `depth_cap_rejects_at_128` constructs a 129-deep nested JSON object and asserts the dispatch path returns `ActionError::Validation { reason: DepthExceeded { .. }, .. }`.

Apply discipline at all three sites (stateless input, stateful input, stateful state). CP3 §9 codemod design + observability-spans wiring.

### §6.2 Explicit-key credential dispatch — HARD REMOVAL of `CredentialContextExt::credential<S>()`

Closes **S-C2 (CR3)** per Strategy §2.12 item 2 + 03c §1 VETO + §1 G3 freeze invariant. **Hard removal**, NOT `#[deprecated]`. **Security-lead implementation-time VETO authority retained.**

#### §6.2.1 Current shape — to be deleted entirely

Currently at `crates/action/src/context.rs:635-668`. The method body uses `std::any::type_name::<S>()` → `rsplit("::").next()` → `to_lowercase()` to derive a credential key from the type name. Phase 1 02b §2.2 detailed the cross-plugin shadow attack (S-C2) — `plugin_a::OAuthToken` and `plugin_b::oauth::OAuthToken` both map to key `"oauthtoken"` per the heuristic; whichever credential the engine registered first under that key is what both plugins resolve.

#### §6.2.2 Mechanism — Option (a): hard delete

Per security 03c §1 + 08c §Gap 1 Option (a) **preferred** — delete the method from `CredentialContextExt`. Old call sites get `error[E0599]: no method named credential found for type X` at compile time (not warning). Codemod (CP3 §9) rewrites to:

```rust
// OLD: ctx.credential::<SlackToken>()                   (no key — silent shadow attack vector)
// NEW: ctx.resolved_scheme(&self.slack)                 (typed slot reference; macro-emitted in §4.1.1)
```

Where `self.slack: CredentialRef<SlackToken>` was emitted by `#[action(credentials(slack: SlackToken))]` per §4.3, and `ctx.resolved_scheme(&CredentialRef<C>) -> Result<&SchemeGuard<'a, C>, ResolveError>` is the new ActionContext API surface (location pending Strategy §5.1.1 — pinned at credential Tech Spec §2.6 or §3 before CP3 §7 drafting).

#### §6.2.3 Why NOT `#[deprecated]` (VETO trigger — verbatim from 03c §1)

Quoted from security 03c §1.B:

> Critical: the deprecation must be **enforced at type level** (compile error or method removal), **NOT** a `#[deprecated]` attribute that lets old code keep compiling. A `#[deprecated]` warning is NOT structural elimination — the attack vector still ships.

And from 03c §4 handoff:

> If tech-lead and architect converge on B' and the implementation later attempts to ship a `#[deprecated]` instead of hard-removing the no-key `credential<S>()` method: I will VETO the landing.

**This Tech Spec commits to hard-removal.** Any implementation-time deviation toward `#[deprecated]` shim form invalidates the freeze per §0.2 item 3 ("'hard removal' → 'deprecated shim' — `feedback_no_shims.md` violation") AND triggers security-lead implementation VETO per 03c §1.

#### §6.2.4 Migration codemod scope

Per Strategy §4.3.3 transform 3 + 08c §Gap 1 — codemod must error on remaining call sites with crisp diagnostic, not silently rewrite. Manual-review marker for each call site:

- **Auto-rewritable**: call sites with explicit type annotation `ctx.credential::<SlackToken>()` where `SlackToken` is a known concrete credential type registered in the workflow's manifest. Codemod rewrites to `ctx.resolved_scheme(&self.<inferred_slot_name>)` after auto-injecting the appropriate `credentials(<slot>: SlackToken)` zone in the action's `#[action(...)]` attribute.
- **Manual-review marker**: call sites with type erasure or unknown type — codemod emits `// TODO(action-cascade-codemod): manual rewrite required — see CP3 §9 codemod runbook` plus the original line.

CP3 §9 details runbook + reverse-dep coverage. CP2 §6.2 commits the hard-removal contract; codemod design is CP3.

#### §6.2.5 Companion HARD-REMOVAL — `credential_typed::<S>(key)` retained

The remaining method `credential_typed<S>(key: &str)` (`crates/action/src/context.rs:563-632` from earlier read; verified at this commit) is **explicit-key by design** — caller supplies the key, no type-name heuristic, no shadow attack. **Retained** (not removed). Codemod transform 2 (Strategy §4.3.3) rewrites `credential_typed` call sites to `resolved_scheme` form per CP6 vocabulary; CP3 §9 picks whether `credential_typed` is also removed (deprecation-free transition to `resolved_scheme`-only) OR stays as a side-channel for non-`#[action]` consumers. CP2 §6.2 leaves this question open — security-lead 03c VETO applies only to the no-key heuristic, not to `credential_typed`.

**Open item §6.2-1** — `credential_typed::<S>(key)` retention vs removal — CP3 §9 picks. Security-neutral (no shadow attack vector); architectural cleanliness question (one credential-resolution API surface vs two).

### §6.3 `ActionError` Display sanitization via `redacted_display()` helper

Closes **S-O4 (partially)** + **S-C3 (module-path leak via type-name)** per Strategy §2.12 item 3 + §2.6 + 03c §2 item 3.

#### §6.3.1 Apply sites — exact line numbers

| Site | File | Line |
|---|---|---|
| Stateful adapter error-path log | `crates/action/src/stateful.rs` | line 609-615 (`tracing::error!(action_error = %action_err, ...)`) |
| Stateless adapter error log | `crates/action/src/stateless.rs` | line 382 — verify exact form (currently emits `ActionError::fatal(format!("output serialization failed: {e}"))` per `crates/action/src/stateless.rs:382`; no direct `tracing::error!(action_error = %e)` at this line) |

**Note on stateless apply site.** Per re-verification at `crates/action/src/stateless.rs:380-385` at this commit, the line emits an `ActionError::fatal(format!(...))` with `e.to_string()` — the leak vector is the `e: serde_json::Error`'s `Display` (which can include path / value information from the offending JSON). The sanitization wraps the *outgoing error string*, not just the `tracing::error!` call. CP3 §9 confirms exact wrap-site; CP2 §6.3 commits the requirement: every path that emits an `ActionError` whose `Display` could leak credential material or module-path information **must** route through `redacted_display()`.

##### §6.3.1-A Pre-`format!` sanitization wrap-form (security-lead 09c §6.3-A)

The sanitization point must be **the `serde_json::Error`'s own Display, before it enters the `format!` argument list**. Wrapping the outer string after `format!` interpolation is too late — `format!("output serialization failed: {e}")` invokes `e`'s Display impl directly; if `e: serde_json::Error` reveals path / value details, the leak ships in the formatted string before any outer-string sanitizer runs. CP2 commits the wrap-form:

```rust
// stateless.rs:382 (current — leaks via `e`'s Display):
ActionError::fatal(format!("output serialization failed: {e}"))

// CP2 / CP3 emission contract — sanitize `e` BEFORE format! interpolates:
ActionError::fatal(format!(
    "output serialization failed: {}",
    nebula_redact::redacted_display(&e)
))
```

The `redacted_display(&e)` call returns a `String` (not `impl Display`) — it consumes `e`'s Display through the redaction filter, then the outer `format!` interpolates the already-sanitized string. This shape applies to every emit site where the embedded error's Display impl is the leak surface (not only `serde_json::Error` — any error whose Display could include credential material, module-path identity, or `SecretString`-bearing field accessors). CP3 §9 enumerates the full apply-site list across `crates/action/src/`.

#### §6.3.2 Helper crate location — co-decision: `nebula-redact` (NEW dedicated crate)

Per security 08c §Gap 3 — security-lead position:

> prefer `nebula-redact` as a dedicated, reviewable surface (single audit point); `nebula-log` as a co-resident is acceptable but mixes redaction policy with logging policy.

**CP2 commits to `nebula-redact` (NEW dedicated crate)**, not `nebula-log` co-resident. Rationale:

1. **Single audit point.** Redaction policy is a security-critical surface (any change is potential leak introduction); a dedicated crate with `security-lead` as required CODEOWNER aligns with `feedback_active_dev_mode.md` (DoD includes typed error + trace span + invariant check — redaction policy is the invariant for log-content surface).
2. **Layering.** `nebula-log` is a logging facade; redaction is a content-rule that operates on values flowing through ANY surface (logs, error messages, audit trails, metric tags). Co-resident in `nebula-log` would force `nebula-error`-side error sanitization to depend on `nebula-log` (inverted dependency).
3. **Review surface.** A standalone crate has its own `cargo doc`, its own test surface, its own changelog. Reviewers can audit the redaction policy in isolation.

**Crate stub.**

```rust
// crates/redact/src/lib.rs (NEW)
//! Single-audit-point redaction policy for log / error / audit-trail surfaces.
//!
//! `redacted_display(&dyn Display) -> String` returns the input's `Display`
//! with credential-bearing patterns stripped per §1 redaction rules.

pub fn redacted_display<T: ?Sized + std::fmt::Display>(value: &T) -> String {
    // §1: strip module-path prefixes (`plugin_x::module_y::CredType` → `CredType`)
    // §2: strip type-name patterns matching credential-bearing types
    // §3: replace `SecretString`-bearing field accessors with `[REDACTED]`
    // ... (impl details: CP3 §9)
}

#[cfg(test)]
mod tests { /* invariant tests per §6.3.3 */ }
```

**Open item §6.3-1** — full redaction rule set (which substring patterns) is CP3 §9 design scope; CP2 commits crate location + helper signature only.

#### §6.3.3 Typed observability discipline

Per `feedback_observability_as_completion.md`:

- **Typed error.** `ActionError::Display` impl wraps internal Display through `redacted_display()` (or a new `ActionError::redacted_display()` method). Verified in unit tests `actionerror_display_strips_module_paths` + `actionerror_display_redacts_secret_string_field`.
- **Trace span.** `tracing::error!(action_error = %e.redacted_display(), ...)` at every emit site (stateful.rs:609-615, stateless.rs:382, plus any future emit sites).
- **Invariant check.** Property test: for any `ActionError` containing a `SecretString`-bearing variant, `e.to_string().contains("REDACTED")` AND `!e.to_string().contains(<actual_secret_value>)`.

### §6.4 Cancellation-zeroize test (closes S-C5)

Closes **S-C5** per Strategy §2.12 item 4 + 03c §2 item 4 + §1 G3 freeze invariant. Design-level invariant locked at §3.4; §6.4 commits the implementation form.

#### §6.4.1 Test location

`crates/action/tests/cancellation_zeroize.rs` (NEW) — tests directory at the action crate's integration-test layer, NOT inside `crates/action/src/testing.rs` (which is a public test-helpers module per `crates/action/src/testing.rs` shape — keeping integration tests out of the public API per `feedback_boundary_erosion.md`).

The three sub-tests from spike Iter-2 §2.4 port forward verbatim per §3.4 Test contract:

- `scheme_guard_zeroize_on_cancellation_via_select` — guard moved into body future, `tokio::select!` cancel branch fires after 10ms.
- `scheme_guard_zeroize_on_normal_drop` — guard scope-exits normally.
- `scheme_guard_zeroize_on_future_drop_after_partial_progress` — body progresses past one `.await`, cancelled at the second.

#### §6.4.2 ZeroizeProbe instrumentation choice — per-test (closes 08c §Gap 4)

Per security 08c §Gap 4 — security-lead position:

> prefer `ZeroizeProbe` per-test instrumentation — global counters create test-coupling antipatterns and cross-test contamination on flaky CI runs. `serial_test::serial` is acceptable but slows test parallelism.

**CP2 commits to per-test `ZeroizeProbe: Arc<AtomicUsize>` (test-only constructor variant on `Scheme`).**

```rust
// In nebula-credential's test surface (cfg(test) / pub-cfg(test-helpers)):
impl<C: Credential> SchemeGuard<'a, C> {
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn engine_construct_with_probe(
        scheme: C::Scheme,
        ctx: &'a CredentialContext<'a>,
        probe: Arc<AtomicUsize>,
    ) -> Self { ... }  // bumps `probe` on Drop instead of (or in addition to) global

    // Production constructor unchanged.
    pub(crate) fn engine_construct(scheme: C::Scheme, ctx: &'a CredentialContext<'a>) -> Self { ... }
}
```

Each cancellation-zeroize test creates its own `Arc<AtomicUsize>` probe, threads through `engine_construct_with_probe`, asserts probe count. No cross-test contamination; tests parallelize freely; no `serial_test::serial` needed.

**Cross-crate amendment.** This is a **soft amendment к credential Tech Spec §15.7** — adds the `engine_construct_with_probe` test-only constructor variant. Same precedent as §5.4.1 — flagged here, NOT enacted by this Tech Spec. CP4 cross-section pass surfaces; credential Tech Spec author lands the inline edit.

#### §6.4.3 Assertions

Per spike Iter-2 §2.4 contract:

- After cancellation fires (whether via `tokio::select!`, normal scope exit, or partial-await drop), the per-test `Arc<AtomicUsize>` probe count is exactly `1` (one zeroize call per guard instance, per test).
- The action body's `.await` point is interruptible — if the body progresses past one `.await` and is cancelled at the second, the guard's Drop still fires before scope unwind (Probe 5 retention check is the complementary compile-time gate).

**Open item §6.4-1** — `tokio::time::pause()` vs real-clock 10ms in cancellation tests — choice impacts test wall-clock duration. CP3 §9 picks (recommendation: `tokio::time::pause()` for deterministic cancellation timing).

### §6.5 Forward-track to CP3 §9 — cross-tenant `Terminate` boundary

Per security 08c §Gap 5 — **NOT-CP2-SCOPE; locked to CP3 §9.**

Quoted from 08c §Gap 5:

> §2.7-2 line 397-398 leaves the engine scheduler-integration hook open: "scheduler cancels sibling branches, propagates `TerminationReason` into audit log." Security-relevant: cross-tenant cancellation is a new attack surface introduced by the wire-end-to-end pick. CP3 §9 must explicitly state: "`Terminate` from action A in tenant T cancels sibling branches **only within tenant T's execution scope**; engine MUST NOT propagate `Terminate` across tenant boundaries." This is a tenant-isolation invariant, not a Strategy decision — CP3 §9 must lock the engine-side check (likely `if termination_reason.tenant_id != sibling_branch.tenant_id { ignore }`).

CP2 §6.5 commits to CP3 §9 lock; this Tech Spec section flags the requirement, the engine-side enforcement form is CP3 scope. Open item §6.5-1 tracks.

---

## §7 Action lifecycle / execution

This section ties the static signatures (§2) and runtime model (§3) into the per-dispatch execution flow. CP2 commits the execution-time path; CP3 §9 details the engine-side wiring.

### §7.1 Adapter execute path with SlotBinding resolution flow

The adapter (`StatelessActionAdapter` / `StatefulActionAdapter` / `TriggerActionAdapter` / `ResourceActionAdapter` per §2.4 handler companions) is the dyn-erasure boundary between the engine's `Arc<dyn StatelessHandler>` storage and the user-typed `StatelessAction` impl. Execution flow per dispatch:

1. **Engine dispatches to handler.** `ActionHandler::Stateless(handler).execute(&ctx, input_json)` (per §2.5) calls into the dyn-typed handler.
2. **Adapter deserializes typed input.** Per §6.1.2 — the adapter pre-scans `input_json` for depth (cap 128 — closes S-J1), then `from_value(input_json)` into `A::Input`. Failure → `ActionError::Validation { reason: DepthExceeded | MalformedJson, .. }` (typed; per §6.1.3).
3. **Adapter resolves credential slots.** For each `SlotBinding` in `A::credential_slots()` (per §3.1 + §4.3), the adapter invokes `(binding.resolve_fn)(&ctx.creds, &slot_key)` — HRTB monomorphizes per slot at registration; `BoxFuture` is awaited; `ResolvedSlot` returns. Engine wraps in `SchemeGuard<'a, C>` per §3.2 step 5 (wrap-point CP3 §9 scope; CP1 inherits spike interpretation).
4. **Adapter invokes typed action body.** `action.execute(&typed_ctx, typed_input)` → `impl Future<Output = Result<A::Output, A::Error>> + Send + 'a`. Body runs to completion or cancels per §3.4.
5. **Adapter serializes typed output.** `to_value(output)` produces `serde_json::Value` for the engine's port projection. Output serialization failure → `ActionError::Fatal { ... }`.
6. **Adapter returns `ActionResult<Value>`.** Per §2.7.2 variants — engine consumes `Continue` / `Skip` / `Branch` / etc. Wire-gated `Retry` / `Terminate` per §2.7.1 (feature flags `unstable-retry-scheduler` + `unstable-terminate-scheduler`).

**Stateful adapter divergence.** Per `crates/action/src/stateful.rs:548-625` (current shape, preserved post-modernization): adapter additionally pre-scans `state_json` for depth (cap 128 — closes S-J2 simultaneously), `from_value::<A::State>(state.clone())` (with `migrate_state` fallback per `crates/action/src/stateful.rs:573-582`); after body returns, adapter writes `to_value(&typed_state)` back to `*state` and propagates serialization-failure via `ActionError::fatal` per §6.3. CP3 §9 details exact ordering.

### §7.2 SchemeGuard<'a, C> RAII flow per credential Tech Spec §15.7

The `SchemeGuard<'a, C>` lifecycle is **owned by the credential Tech Spec**; this Tech Spec cites the contract verbatim and does NOT restate.

**Authoritative source.** Credential Tech Spec §15.7 lines **3394-3516**:
- §15.7 line 3394-3429 — `SchemeGuard<'a, C: Credential>` definition: `!Clone`, `ZeroizeOnDrop`, `Deref<Target = C::Scheme>`, lifetime parameter.
- §15.7 line 3437-3447 — `SchemeFactory<C>` companion: long-lived resource hooks, fresh `SchemeGuard` per acquire.
- §15.7 line 3503-3516 (iter-3 refinement) — engine constructs guard with `&'a CredentialContext<'a>` pinning `'a`; `_lifetime: PhantomData<&'a ()>` alone does NOT prevent retention; the construction signature does.

**Action-side implications (the slice this Tech Spec is responsible for):**

- Action body sees `&'a SchemeGuard<'a, C>` via `ctx.resolved_scheme(&self.<slot>)`. The reference cannot outlive `ctx`'s lifetime; retention attempt fails at compile time per Probe 5 (E0597).
- `SchemeGuard: Deref<Target = C::Scheme>` — action body interacts with the projected scheme directly (`bearer_scheme.token`, etc. per credential Tech Spec §15.5). The dyn-shape `&dyn Phantom` is never exposed (per credential Tech Spec §3.4 line 928).
- On scope exit (normal or cancellation), Drop runs `scheme.zeroize()` deterministically before the borrow chain unwinds (per credential Tech Spec §15.7 line 3412 + §3.4 cancellation contract).

No action-side amendment to §15.7 contract. **Two** soft amendments к credential Tech Spec are surfaced by this Tech Spec — listed together for §15 cross-section integrity:

1. **§5.4.1** — credential Tech Spec §16.1.1 probe #7 — qualified-syntax form `<SchemeGuard<'_, C> as Clone>::clone(&guard)` replaces naive `guard.clone()` to defeat auto-deref silent-pass (per ADR-0037 §3 + spike finding #1).
2. **§6.4.2** — credential Tech Spec §15.7 — `engine_construct_with_probe` test-only constructor variant added on `SchemeGuard<'a, C>` to thread per-test `Arc<AtomicUsize>` zeroize probe (closes 08c §Gap 4 per-test instrumentation preference).

Both amendments are FLAGGED, NOT ENACTED. CP4 cross-section pass surfaces; credential Tech Spec author lands the inline edits per ADR-0035 amended-in-place precedent. §15 open items track both.

### §7.3 Per-action error propagation discipline

Per §2.8 + §6.3 — `ActionError` taxonomy preserved (rust-senior 02c §7 line 428: "cleanest part of the crate idiomatically"); only Display surface routes through `redacted_display()`.

**Propagation points within the adapter execute path:**

| Failure | Variant | Site |
|---|---|---|
| Input depth cap exceeded | `Validation { reason: DepthExceeded, .. }` | §7.1 step 2 (depth pre-scan) |
| Input deserialization fails | `Validation { reason: MalformedJson, .. }` | §7.1 step 2 (`from_value`) |
| State deserialization fails (stateful) | `Validation { reason: StateDeserialization, .. }` | §7.1 step 2 (stateful) |
| Credential slot resolve fails (`ResolveError::NotFound`) | `Fatal { ... }` (mapped at adapter; CP3 §9 may introduce typed `Resolve` variant) | §7.1 step 3 |
| Action body returns `Err(A::Error)` | propagated as `ActionError` per `From<A::Error>` impl | §7.1 step 4 |
| Output serialization fails | `Fatal { ... }` | §7.1 step 5 |
| Cancellation fires | adapter does NOT propagate as error — body's `Drop` runs; engine sees task cancellation per `tokio::JoinHandle::abort` | §3.4 + §7.1 step 4 |

**Open item §7.3-1** — `ResolveError` mapping to `ActionError` taxonomy — should `ResolveError::NotFound` map to `Fatal` (current) or new `Resolve` typed variant? CP3 §9 picks. Security-neutral.

### §7.4 Result type variants handling

Per §2.7.2 — `ActionResult<T>` variants. Adapter's `try_map_output` (per `crates/action/src/stateless.rs:380-384`) maps the inner `T` through `to_value` to produce `ActionResult<Value>`. Engine consumes per variant:

- `Success { output }` — engine threads `output` to next node per port-projection.
- `Skip { reason, output }` — engine skips node; emits skip event with reason.
- `Drop { reason }` — engine drops execution at this node (no output).
- `Continue { output, progress, delay }` — engine re-enqueues with optional delay (per stateful iteration); same dispatch path as `Retry` for the wired-out shape.
- `Break { output, reason }` — engine breaks loop.
- `Branch { selected, output, alternatives }` — engine routes to selected branch.
- `Route { port, data }` — engine routes to specific port.
- `MultiOutput { outputs, main_output }` — engine emits to multiple ports.
- `Wait { condition, timeout, partial_output }` — engine waits on condition.
- **`Retry { after, reason }`** — feature-gated `unstable-retry-scheduler` per §2.7.1; engine's scheduler re-enqueues (CP3 §9 wires).
- **`Terminate { reason }`** — feature-gated `unstable-terminate-scheduler` per §2.7.1; engine's scheduler cancels sibling branches AND propagates `TerminationReason` to audit log (CP3 §9 wires; cross-tenant boundary check per §6.5).

**Wire-end-to-end commitment.** Per §2.7.1 + Strategy §4.3.2 — Retry + Terminate share gating discipline; both wire end-to-end at scheduler landing. CP3 §9 details engine scheduler-integration hook trait surface.

---

## §8 Storage / state

Storage boundaries between action persistence and engine persistence. CP2 locks the action-side contract; engine-side persistence (`crates/storage/`) is **out of action's direct concern** — cited as cross-ref only.

### §8.1 Action-side persistence

#### §8.1.1 State JSON via `StatefulAction`

Per `crates/action/src/stateful.rs:573-582` (current shape, preserved):

- `StatefulAction::State` associated type bounds: `Serialize + DeserializeOwned + Clone + Send + Sync + 'static` (per §2.2.2; lifted onto trait per CP1 iteration to close rust-senior 08b 🔴 leaky-adapter-invariant).
- Adapter persists state via JSON serialization (`to_value(&typed_state)`) at the end of each dispatch; engine writes the resulting `serde_json::Value` to `ExecutionRepo` (canon §11.3 idempotency).
- Migration path: `StatefulAction::migrate_state(state: serde_json::Value) -> Option<Self::State>` (per `crates/action/src/stateful.rs:573-582`) consulted only when `from_value::<A::State>(state.clone())` fails — version-skew between stored checkpoint and current State schema.
- Depth cap (§6.1) applies to state deserialization (`from_value(state.clone())`) — closes S-J2 simultaneously per 03c §1.

#### §8.1.2 Trigger cursor via `PollAction`

`PollAction` is a sealed DX trait (per §2.6) erasing to `TriggerAction`. PollAction-shaped triggers track cursor position via the underlying `TriggerAction::handle` fire-and-forget event surface; cursor itself is engine-managed (per Strategy §3.1 component 7 — cluster-mode dedup window, idempotency key).

CP3 §7 locks the `PollAction` trait shape (sealed-DX trait-by-trait audit per ADR-0038 §Implementation notes); CP2 commits "cursor lives at engine, not action body."

#### §8.1.3 Macro-emitted slot bindings

Per §3.1 + §4.3 — `&'static [SlotBinding]` slices live for the entire process; engine copies the binding entries into the registry-side index at `ActionRegistry::register*` time. **No per-execution persistence** — slot bindings are static-shape, registry-time.

### §8.2 Runtime-only state (NOT persisted)

#### §8.2.1 Handler cache

`ActionHandler::{Stateless, Stateful, Trigger, Resource}(Arc<dyn ...Handler>)` (per §2.5) is constructed at registration time via the adapter pattern — handlers wrap user-typed actions per §7.1 step 1. The handler `Arc` lives for the registry's lifetime; not per-execution; not persisted.

#### §8.2.2 SchemeGuard borrows

`SchemeGuard<'a, C>` is **borrow-lifetime-scoped** per §7.2 + credential Tech Spec §15.7 line 3503-3516. Every dispatch acquires fresh; never persisted. Cancellation-zeroize (§6.4) ensures deterministic cleanup.

#### §8.2.3 ActionContext borrows

`ActionContext<'a>` per spike `final_shape_v2.rs:205-207` carries `&'a CredentialContext<'a>` (the credential context borrow). Lifetime `'a` is the dispatch's borrow chain — `ActionContext` is constructed per dispatch and cannot be retained across dispatches. CP3 §7 locks the exact ActionContext API location per Strategy §5.1.1.

### §8.3 Boundary with engine persistence

The action crate's responsibility ends at:

- **Typed serialization shape** of `Input`, `Output`, `State`, `Event`, `Error` per §2.2.
- **JSON adapter contract** at `*Handler::execute` (per §2.4 — `serde_json::Value` in/out).
- **`SlotBinding` static metadata** per §3.1 + §4.3.

Engine-side persistence (`crates/storage/`, `crates/engine/src/storage/`, `ExecutionRepo`) consumes the action's serialized shapes and persists per canon §11.3 (idempotency) + §6 (engine guarantees). Action does NOT depend on `crates/storage/` — engine bridges via the `*Handler` dyn-erasure boundary (per §2.4).

**Cross-ref.** CP3 §9 details engine scheduler-integration hook (per §7.4 wire-end-to-end commitment); this includes how `Retry` / `Terminate` re-enqueue / cancellation persistence interacts with `ExecutionRepo`. CP2 §8 commits the boundary; CP3 wires.

---

## §9 Public API surface

This section locks the **observable public surface** of `nebula-action` post-cascade. CP1 §2 already locked the trait signatures; CP3 §9 is the surface-area inventory: what's exported, what's added, what's removed, what's reshuffled. The Phase 0 reverse-deps fingerprint ([`01b-workspace-audit.md`](../drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md) §9 line 252-329) is the baseline — 7 direct reverse-deps (engine, api, sandbox, sdk, plugin, cli + action-macros sibling), 69 source files importing `nebula_action::*`, 63 public items re-exported through `crates/action/src/lib.rs`, ~40+ items cascading through `nebula-sdk::prelude`. The redesign treats `nebula-sdk::prelude` (`crates/sdk/src/prelude.rs:15-33`) as the **public contract surface** per audit finding §9 🟠 MAJOR.

**Semver posture (per §13.1).** Pre-1.0 alpha breaks are acceptable per `feedback_hard_breaking_changes.md`; semver-checks advisory-only per Phase 0 audit T7. CP3 §9 enumerates the surface honestly so post-1.0 callers have a stable target.

### §9.1 Four primary trait surface — unchanged at trait level

The four primary dispatch traits (`StatelessAction` / `StatefulAction` / `TriggerAction` / `ResourceAction` per §2.2) remain `pub` at the trait level. Per [ADR-0036 §Neutral item 2](../../adr/0036-action-trait-shape.md) line 80: "Public API surface of the 4 dispatch traits is unchanged at the trait level — only the macro that constructs implementations changes shape."

**What CP3 §9 confirms:**

- **Trait identity preserved.** `pub trait StatelessAction`, `pub trait StatefulAction`, `pub trait TriggerAction`, `pub trait ResourceAction` exported from `crates/action/src/{stateless,stateful,trigger,resource}.rs` per current shape (verified via `crates/action/src/lib.rs:91-153` re-export block per Phase 0 audit §9 line 308).
- **Signature additions per CP1 §2.2.** Three deliberate-divergence overlays from CP1 §2 (`Input: HasSchema + DeserializeOwned`, `Output: Serialize`, `StatefulAction::State: Serialize + DeserializeOwned + Clone + ...`) lift bound chains onto the typed traits. **Semver impact: per-feature additions are non-breaking** at the trait-impl level (existing impls already satisfy these bounds via the adapter contract; the lift makes them surface at impl site instead of registration site — DX win, not surface change). **Removals would break** — none are proposed.
- **`*Handler` companion shape** per CP1 §2.4 — single-`'a` lifetime + `BoxFut<'a, T>` per Strategy §4.3.1 modernization; trait-by-trait dyn-safe form preserved (per [rust-senior 02c §6 line 358](../drafts/2026-04-24-nebula-action-redesign/02c-idiomatic-review.md)). The HRTB collapse from `for<'life0, 'life1, 'a>` to single-`'a` is a **structural simplification, not a surface rename** — `Arc<dyn StatelessHandler>` continues to compile; existing `impl StatelessHandler for X` blocks pre-modernization need re-pin to the new shape (codemod transform T4, §10.2).

### §9.2 Five sealed DX trait surface (sealed pattern per ADR-0038)

Five DX specialization traits — `ControlAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction` — become sealed per [ADR-0038 §1](../../adr/0038-controlaction-seal-canon-revision.md) line 49-70. Trait identifiers remain `pub` (community plugin code that names them in trait bounds compiles); the **implementation surface** is sealed via per-capability inner sealed traits (per ADR-0035 §3 + ADR-0038 §1):

```rust
// Surface visible to community plugins (unchanged identifiers; sealed implementation):
pub trait ControlAction:    sealed_dx::ControlActionSealed    + StatelessAction { /* ... */ }
pub trait PaginatedAction:  sealed_dx::PaginatedActionSealed  + StatefulAction  { /* ... */ }
pub trait BatchAction:      sealed_dx::BatchActionSealed      + StatefulAction  { /* ... */ }
pub trait WebhookAction:    sealed_dx::WebhookActionSealed    + TriggerAction   { /* ... */ }
pub trait PollAction:       sealed_dx::PollActionSealed       + TriggerAction   { /* ... */ }
```

**Community plugin migration target — reaffirmed.** Per CP1 §2.6 + [ADR-0038 §Negative item 4](../../adr/0038-controlaction-seal-canon-revision.md) line 111: code that today writes `impl ControlAction for X` moves to `impl StatelessAction for X` + `#[action(control_flow = …)]` attribute. The macro emits the sealed `ControlActionAdapter` from cascade-internal `nebula-action::sealed_dx::*` namespace. Migration codemod T6 in §10.2 covers the common case.

**Trait-by-trait audit status.** Per ADR-0038 Implementation note ("trait-by-trait audit at Tech Spec §7 design time"): all five DX traits use the §2.6 blanket-impl shape `impl<T: PrimaryTrait + ActionSlots> sealed_dx::TraitSealed for T {}` — the supertrait chain mirrors the §2.1 `Action: ActionSlots + Send + Sync + 'static` discipline. ControlAction wraps Stateless; Paginated/Batch wrap Stateful; Webhook/Poll wrap Trigger. CP3 §9 confirms the audit closes; exact attribute-zone syntax for each (`#[action(control_flow = …)]`, `#[action(paginated(cursor = …))]`, etc.) is CP4 §15 housekeeping not §9 scope.

### §9.3 `prelude.rs` re-export reshuffle

Phase 0 audit §9 line 295 enumerates the current `nebula-sdk::prelude` 40+ re-exports (`crates/sdk/src/prelude.rs:15-33`); `crates/action/src/prelude.rs:1-54` is the action-side prelude (subset of `lib.rs:91-153` exports). CP3 §9 locks the delta:

#### §9.3.1 Removed (hard-cut per `feedback_no_shims.md`)

| Removed item | Source (current) | Replacement | Rationale |
|---|---|---|---|
| `CredentialContextExt::credential<S>()` (no-key heuristic) | `crates/action/src/context.rs:635-668` | `ctx.resolved_scheme(&self.<slot>)` (typed slot ref via `#[action(credentials(...))]` zone) | §6.2 hard removal per security 03c §1 VETO; cross-plugin shadow attack S-C2 / CR3 |
| `CredentialContextExt::credential_typed<S>(key)` *(retention TBD)* | `crates/action/src/context.rs:563-632` | Same `ctx.resolved_scheme(&self.<slot>)` form — **OR retained as side-channel for non-`#[action]` consumers** | §6.2.5 + §6.2-1 open item; CP3 §9 picks (recommendation: remove — unify on `resolved_scheme`; retention adds two parallel APIs with no current consumer) |
| `CredentialGuard<S>` legacy guard type | `crates/credential/src/guard.rs` (legacy) | `SchemeGuard<'a, C>` per credential Tech Spec §15.7 | Cross-crate transition per §7.2; legacy guard goes away post-CP6-implementation |
| `nebula_action_macros::Action` (derive) | `crates/action/src/lib.rs:91-153` | `nebula_action_macros::action` (attribute) | ADR-0036 §Decision item 1 (hard break per ADR-0036 §Negative item 1) |

**Decision lock for `credential_typed` (§6.2-1 closed at CP3 §9).** Recommendation: **remove** alongside `credential<S>()`. Rationale: (a) explicit-key form is achievable via `ctx.resolved_scheme(&CredentialRef::from_key(key))` for the rare non-`#[action]` consumer (e.g., dynamic test harness construction); (b) two parallel APIs (`resolved_scheme(&CredentialRef<C>)` typed-handle vs `credential_typed::<C>(key: &str)` string-key) bifurcate authoring guidance; (c) Phase 0 audit shows zero `nebula-sdk::prelude` re-export of `credential_typed` — surface-area cost is internal-only. Tech-lead ratifies at CP3 close.

#### §9.3.2 Added

| Added item | Host crate / module | Notes |
|---|---|---|
| `ActionSlots` (trait) | `nebula-action::ActionSlots` per §2.1.1 | Macro-emitted-only; hand-impl discouraged via doc + Probe 4/5/6 invariants. Sealing decision §9.4 below + §4.4-1 open item |
| `BoxFut<'a, T>` (type alias) | `nebula-action::BoxFut` per §2.3 | **Single home (resolves §9.4 forward-track from CP1 rust-senior 08b 🟡).** Cross-doc / sibling-crate references go through this single home; engine adapters that need the same shape `use nebula_action::BoxFut`, do not redeclare. Spike `final_shape_v2.rs:38` and credential Tech Spec §3.4 line 869 both name `BoxFuture` — Tech Spec re-pins both to `nebula-action::BoxFut` per CP3 §9 single-home decision. **Rationale for the `BoxFut` (vs spike's `BoxFuture`) name:** (a) shape alignment with Strategy §4.3.1 single-`'a` modernization (`BoxFut<'a, T>` has exactly one lifetime — the spike-original `BoxFuture` from `futures::future::BoxFuture` is `Pin<Box<dyn Future<Output = T> + Send + 'static>>` with implicit `'static`, semantically distinct from this alias); (b) avoids name conflict with `futures::future::BoxFuture` so plugin authors who `use futures::future::BoxFuture;` separately do not collide with `use nebula_action::BoxFut;`; (c) precedent: spike `final_shape_v2.rs:38` chose this short alias and Phase 4 PASS validated the shape (see [spike NOTES §5](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md)). (Future cascade may hoist `nebula-core::BoxFuture`; per §0.2 invariant 4 that re-pin is in scope of Tech Spec amendment.) |
| `SlotBinding`, `SlotType`, `Capability`, `ResolveFn` | `nebula-action::SlotBinding` etc. per §3.1 | Macro-emitted into `ActionSlots::credential_slots()` const slice; community plugins do not construct `SlotBinding` manually. `SlotType` and `Capability` are `#[non_exhaustive]` per CP1 iteration |
| `redacted_display!` macro export *(or `redacted_display` fn)* | `nebula-redact::redacted_display` per §6.3.2 | NEW dedicated crate; `nebula-action` does NOT re-export — `redacted_display` is consumed at error-emit sites in `crates/action/src/{stateless,stateful}.rs`, NOT in the public action API surface. Open question §9.3-1: does `nebula-sdk::prelude` re-export `redacted_display` for community plugin authors who need it for their own error sanitization? CP4 §16 picks; default position **NO** (community plugins should depend on `nebula-redact` directly if they need it — keeps the single audit point) |
| `ValidationReason::DepthExceeded { observed: u32, cap: u32 }` | `nebula-action::ValidationReason` (variant added) per §6.1.3 | `#[non_exhaustive]`-safe per `crates/action/src/error.rs:58-71` |
| `DepthCheckError { observed: u32, cap: u32 }` *(internal)* | `nebula-action::webhook::DepthCheckError` (`pub(crate)` per §6.1.2-A) | Crate-internal only; not in public surface; webhook caller re-wraps to preserve public webhook API |

#### §9.3.3 Reshuffled

- **`SchemeGuard<'a, C>` re-export** lives in `nebula-credential` (per credential Tech Spec §15.7 line 3394-3429). `nebula-action::prelude` re-exports through canonical credential path: `pub use nebula_credential::SchemeGuard;` (NOT a hand-vendored alias). The credential side is the single home; action-side prelude points there. Closes Phase 0 §9 finding "redaction policy in `nebula-log` would force `nebula-error`-side error sanitization to depend on `nebula-log` (inverted dependency)" — same principle: vocabulary type lives where it's defined; consumers cite, not vendor.
- **`CredentialRef<C>`** moves analogously — re-export through `nebula-credential` (per credential Tech Spec §3.5 + Strategy §3.2 placement lock). `nebula-action` consumes; does not own.
- **40+ SDK prelude items** per Phase 0 audit §9 line 295 stay re-exported through `nebula-sdk::prelude`. Codemod transform T6 (§10.2) flags reverse-dep import sites for review when prelude paths change. Migration guide (§10.4) lists added/removed/renamed pairs.

### §9.4 Builder/macro convenience methods — what's exposed in `nebula-sdk::prelude`

`nebula-sdk::prelude` is the **community plugin author's entry point**. Per Phase 0 audit §9 finding 🟠 MAJOR (line 331): "any rename/relocation in action cascades directly to `nebula-sdk::prelude::*`, which is the officially-sanctioned user-facing API." CP3 §9 specifies what surface a community plugin author sees in one `use nebula_sdk::prelude::*;`:

**Exposed in prelude (community plugin authoring path):**

- The four primary traits (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`) — direct impl target.
- `ActionContext<'a>` — per CP1 §2.1 receiver type; community plugins use `ctx.resolved_scheme(&self.<slot>)` only. Internal API like `ctx.creds: &'a CredentialContext<'a>` (per spike `final_shape_v2.rs:205-207`) is `pub(crate)` field — surface is `pub` type with `pub(crate)` field access; the community-author method is `resolved_scheme`. **Internal storage shape (field vs method) may be revisited in CP4 §15 cross-section; community-facing API at `resolved_scheme(&self.<slot>)` is locked** — the `pub(crate)` field is unreachable from outside the crate, so any future field-to-method reshape is a crate-internal refactor, not a public-surface change.
- `ActionResult<T>`, `ActionOutput<T>`, `ActionError`, `ValidationReason`, `RetryHintCode`, `BreakReason`, `TerminationCode`, `TerminationReason` — per CP1 §2.7-§2.8 + current `crates/action/src/result.rs` shape.
- `#[action]` attribute macro re-export from `nebula_action_macros::action` (replacing the legacy `Action` derive).
- The five DX trait identifiers (`ControlAction`, `PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`) — bound-shape only; community plugins do not impl directly per §9.2 / ADR-0038.
- Test harness types (`TestContextBuilder`, `SpyEmitter`, `SpyLogger`, `SpyScheduler`, `StatefulTestHarness`, `TriggerTestHarness`) — per `crates/action/src/prelude.rs:42-45` + Phase 0 audit §9 list.

**Lower-level access (not in prelude — `use nebula_action::*` for these):**

- `StatelessHandler`, `StatefulHandler`, `TriggerHandler`, `ResourceHandler` (the dyn-safe companion traits per §2.4) — engine-side / sandbox-side ABI; community plugins use the typed primary trait, not the handler.
- `ActionHandler` enum (per §2.5) — engine dispatch surface; not authored by community plugins.
- `*ActionAdapter` types (`StatelessActionAdapter`, `StatefulActionAdapter`, `TriggerActionAdapter`, `ResourceActionAdapter`, `ControlActionAdapter`) — engine-internal adapter pattern per §11; community plugins do not see these directly.
- `SlotBinding`, `SlotType`, `Capability`, `ResolveFn` — macro-emitted internals per §3.1; not authored by hand.

**ActionSlots seal decision (closes §4.4-1 open item).** CP3 §9 commits to **leave `ActionSlots` `pub` (NOT sealed)**. Rationale: (a) the `#[action]` macro is the recommended path and the dual enforcement layer (§4.4) makes hand-implementation observable in tests (Probe 4/5/6 fire on shape violations); (b) sealing would force a parallel `mod sealed_action_slots` pattern with no current consumer benefit beyond purity; (c) advanced internal-Nebula crates may need to hand-impl `ActionSlots` for special-cases (e.g., engine-internal `MetaAction` test fixtures) — sealing closes that door without a current upside. The doc comment per §4.4.3 already says "hand-implementing is technically possible (the trait is `pub`) but discouraged with rustdoc + spike Probe 4/5 invariants." Tech-lead ratifies at CP3 close.

### §9.5 Cross-tenant `Terminate` boundary lock (security 08c Gap 5)

This subsection closes [security 08c §Gap 5](../drafts/2026-04-24-nebula-action-redesign/08c-cp1-security-review.md) line 109-111 forward-tracked from CP2 §6.5. The contract is engine-side enforcement; action authors do not see tenant scope (it is an engine-internal invariant).

#### §9.5.1 Engine-side enforcement contract

**Invariant (verbatim language closing 08c Gap 5):**

> `Terminate` from action A in tenant T cancels sibling branches **only within tenant T's execution scope**; engine MUST NOT propagate `Terminate` across tenant boundaries.

#### §9.5.2 Mechanism

Tenant scope check at scheduler dispatch path **before** fanning `Terminate` to siblings:

1. Adapter returns `ActionResult::Terminate { reason }` from action A (under feature `unstable-terminate-scheduler` per §2.7.1 + §2.7.2).
2. Engine's scheduler-integration hook (per §7.4 wire-end-to-end commitment + §2.7-2 forward-track) receives the variant alongside the dispatch context that produced it. The dispatch context carries `tenant_id` (per engine's existing tenant-isolation discipline; engine-side type, not surfaced to action authors).
3. Scheduler enumerates sibling branches eligible for cancellation. For each candidate sibling branch B with dispatch-context `tenant_id_B`:
   - **If `tenant_id_B == tenant_id_A`**: enqueue cancellation; emit `tracing::info!(tenant_id, terminating_action = %A, sibling_branch = %B, reason = %reason, "sibling branch cancelled by Terminate")`.
   - **If `tenant_id_B != tenant_id_A`**: SKIP the cancellation; emit `tracing::warn!(tenant_id_termination_source = %tenant_id_A, tenant_id_sibling = %tenant_id_B, "cross-tenant Terminate ignored — sibling branch in different tenant scope")`. Counter `nebula_action_terminate_cross_tenant_blocked_total{tenant_origin, tenant_target}` increments per `feedback_observability_as_completion.md`.
4. Cross-tenant Terminate **does NOT fail the originating action** — the originating action's `Terminate { reason }` propagates normally within its own tenant scope; cross-tenant siblings are simply un-reachable via this mechanism. **However:** if the scheduler-integration hook's pre-fan validation surfaces ANY structural error in the Terminate dispatch (e.g., malformed `TerminationReason`, scheduler unavailable, persistence backend failure for audit log), the originating action receives `ActionResult::Terminate { reason }` → engine maps to `Fatal` per §7.3 propagation table. **Cross-tenant ignore is silent (not Fatal); structural errors are Fatal.** This split is the active-dev-mode-coherent shape: cross-tenant is a known-policy boundary with observable telemetry, not an action-author error.

#### §9.5.3 Why cross-tenant Terminate must NOT silently cross AND must NOT silently no-op

Two reject paths considered and reasoning:

- **REJECT — silent cross-tenant cancel.** If `Terminate` from tenant T cancels a tenant T'-owned branch, T can mount denial-of-service against T'-owned executions by emitting `Terminate` from any action in T's scope that the engine's scheduler-integration sibling fan-out treats as eligible. **Tenant isolation invariant violated** per security 08c Gap 5 verbatim language. Rejected.
- **REJECT — silent cross-tenant no-op without telemetry.** If cross-tenant Terminate is silently dropped without trace span / counter, attempted misbehavior is **structurally invisible** to the security ops surface. Per `feedback_observability_as_completion.md` ("typed error + trace span + invariant check are DoD"), the observable shape is mandatory. Rejected.

The accepted mechanism (§9.5.2 step 3) makes the cross-tenant skip **observable via `tracing::warn!` + counter** without elevating it to a fatal action error.

#### §9.5.4 Action-author surface

Action authors see no tenant scope. The action body returns `Ok(ActionResult::Terminate { reason })`; the engine handles tenant-scope filtering at the scheduler. This preserves the §1 G3 floor item discipline: action authors author within their own tenant scope; tenant-isolation is engine-internal contract per Nebula's threat model.

#### §9.5.5 Implementation note

The scheduler-integration hook trait surface (per §7.4 + §2.7-2 forward-track) must expose `tenant_id` in the dispatch context to make this check possible. CP3 §9 commits the contract; the exact engine trait shape (`SchedulerIntegrationHook::on_terminate(&self, dispatch_ctx: &DispatchContext, reason: TerminationReason) -> Result<(), SchedulerError>` — or analogous) is engine-side scope per §7.4 cross-ref. This Tech Spec defines what the action surface produces (`ActionResult::Terminate { reason }`); engine cascade defines how the scheduler consumes it (with the §9.5.2 tenant-scope filter as a non-negotiable invariant).

**Security-lead implementation-time VETO retained.** Per [security 08c §Gap 5](../drafts/2026-04-24-nebula-action-redesign/08c-cp1-security-review.md) line 109-111: any implementation-time deviation from §9.5.1's invariant language ("engine MUST NOT propagate `Terminate` across tenant boundaries") triggers security-lead VETO. The wording "MUST NOT propagate" is normative — softening to "should not" or "by default does not" is a freeze invariant 3 violation per §0.2.

---

## §10 Migration plan (codemod runbook)

This section locks the **codemod runbook** for migrating the 7 reverse-deps off pre-cascade shape onto the CP1+CP2-locked surface. Strategy §4.3.3 (line 231-243) locks the codemod **scope** at five mechanical transforms; CP3 §10 designs the runbook (script shape, transform list, dry-run output format) per Strategy §4.3.3 line 243 explicitly delegating "design only" to Tech Spec §9. Codemod **execution** (running the script on 7 reverse-deps) is post-cascade per Strategy §3.4 OUT row.

### §10.1 Reverse-deps inventory (verbatim from Phase 0 audit §9)

| Consumer | Cargo declared at | Source files | Risk |
|---|---|---|---|
| `crates/action/macros` | self-reference via `nebula-action-macros` | sibling proc-macro | intra-action |
| `crates/engine` | `Cargo.toml:27` | 27+ import sites in `engine.rs`, `runtime.rs`, `registry.rs`, `error.rs`, `stream_backpressure.rs` | 🔴 HEAVY (per Phase 0 §10) |
| `crates/api` | `Cargo.toml:35` | 4 files (webhook transport + tests) | 🟡 LIGHT |
| `crates/sandbox` | `Cargo.toml:16` | 7 files (`runner`, `remote_action`, `handler`, `process`, `in_process`, `discovery`, `discovered_plugin`) | 🟠 MODERATE — dyn-handler ABI |
| `crates/sdk` | `Cargo.toml:17` | 5 files; full re-export at `src/lib.rs:47` (`pub use nebula_action;`) + 40+ items in `prelude.rs:15-33` | 🟠 MODERATE — public contract |
| `crates/plugin` | `Cargo.toml:22` | 2 src + 1 test (`Action`, `ActionMetadata`, `DeclaresDependencies`) | 🟡 LIGHT |
| `apps/cli` | `Cargo.toml:66` | 5 files (`actions.rs`, `dev/action.rs`, `run.rs`, `watch.rs`, `replay.rs`) | 🟡 LIGHT |

**Doc-only references** (no compile coupling, no codemod required): `crates/workflow/connection.rs` (rustdoc only), `crates/storage/execution_repo.rs:425` (rustdoc only), `crates/execution/status.rs:146` (rustdoc only). Migration guide (§10.4) flags these for review-only update; no automated transform.

### §10.2 Codemod transforms (T1-T6)

Per Strategy §4.3.3 transforms 1-5 are the **minimal complete set**; Tech Spec §10 may add transforms during design without re-opening Strategy (per Strategy §4.3.3 line 243). CP3 §10 names six transforms with the following Strategy → Tech Spec mapping (NOT 1:1):

- **Strategy 1** (`#[derive]` → `#[action]`) → **T1** (verbatim scope).
- **Strategy 2** (`ctx.credential_by_id` / `ctx.credential_typed` / `ctx.credential::<S>` → unified API) + **Strategy 3** (no-key heuristic hard removal) → **T2** (collapsed; per §6.2.4 hard-removal commitment).
- **Strategy 4** (`[dev-dependencies]` block — Cargo.toml hygiene) → **NOT a code-edit transform**; lands at CP2 §5.1.
- **Strategy 5** (`nebula-sdk::prelude` re-export reshuffle) → **NOT a code-edit transform per se**; covered by §9.3 reshuffled list.

**T3** (`Box<dyn>` → `Arc<dyn>` safety net), **T4** (HRTB collapse to `BoxFut<'a, T>`), **T5** (`redacted_display!` wrap), and **T6** (ControlAction → StatelessAction migration) are added at Tech Spec design level per Strategy §4.3.3 line 243 license — not derived from Strategy 1-5. T6 in particular is added per ADR-0038 §Negative item 4 for the sealed-DX migration. (Earlier Tech-Spec mention of T1-T5 in CP1+CP2 forward-tracks predated this disambiguation; the CP3 mapping above is authoritative.)

| Transform | What it rewrites | Source pattern | Target pattern | Auto / Manual | Notes |
|---|---|---|---|---|---|
| **T1** | `#[derive(Action)]` → `#[action(...)]` | `#[derive(Action)] struct X { ... }` + `#[nebula(key = ...)]` companion attribute | `#[action(key = ..., name = ..., credentials(slot: Type))] struct X { ... }` (zone-injected fields) | **AUTO** for happy-path; manual review for `parameters = T` arm (T2-related) | Attribute extraction from prior derive form; codemod parses `#[nebula(...)]` companions and folds into `#[action(...)]` |
| **T2** | `ctx.credential::<S>()` (no-key, S-C2) → `ctx.resolved_scheme(&self.<slot>)` | Type-name heuristic call site | Typed slot-ref through macro-emitted `credentials(slot: Type)` zone | **MANUAL REVIEW** required for each call site; AUTO for cases with explicit type annotation `ctx.credential::<SlackToken>()` AND `SlackToken` registered in workflow manifest | §6.2.4 + §4.7-1 open item — codemod errors on remaining call sites with crisp diagnostic per Strategy §4.3.3 transform 3; emits `// TODO(action-cascade-codemod): manual rewrite required — see §10.4 migration guide` markers |
| **T3** | `Box<dyn StatelessHandler>` → `Arc<dyn StatelessHandler>` (where ABI shape changed) | Sandbox in-process / out-of-process runners; engine handler storage | Per CP1 §2.5 `ActionHandler::Stateless(Arc<dyn StatelessHandler>)` form (current shape preserved; transform applies only to legacy `Box<dyn>` patterns if any) | **AUTO** | Verified against current `crates/action/src/handler.rs:39-50` — `Arc<dyn>` is already canonical; transform is a safety net for any pre-cascade `Box<dyn>` patterns the codemod surfaces |
| **T4** | HRTB `for<'life0, 'life1, 'a>` patterns → `BoxFut<'a, T>` alias | Hand-written `*Handler` impls (rare; mostly engine-internal) | Single-`'a` + `BoxFut` per CP1 §2.4 + Strategy §4.3.1 modernization | **AUTO** | Token-form rewrite; per Phase 0 audit Phase 1 02c §6 line 358 the cut is ~8 lines per handler trait; codemod validates dyn-safety preserved |
| **T5** | `tracing::error!(action_error = %e)` → `redacted_display!` wrap form | Adapter error log sites (`stateful.rs:609-615`, `stateless.rs:382` per §6.3.1) + any custom `tracing::error!(error = %ActionError, ...)` patterns in reverse-deps | `tracing::error!(action_error = %nebula_redact::redacted_display(&e), ...)` per §6.3.1-A | **MANUAL REVIEW** required — non-`ActionError` Display sites need case-by-case check per §6.3.1-A "every error whose Display could include credential material, module-path identity, or `SecretString`-bearing field accessors" | Cannot mechanically distinguish leak-prone from safe; codemod marks all `tracing::error!(.. = %e)` sites for review |
| **T6** | `impl ControlAction for X` → `impl StatelessAction for X` + `#[action(control_flow, ...)]` | ControlAction direct-impl call sites (community plugin migration target per ADR-0038 §Negative item 4) | StatelessAction primary + control-flow attribute zone (flag form per §12.2; CP4 §15 may revisit `control_flow = SomeStrategy` config form) | **MIXED** — **AUTO** for the trivial pass-through case (default mode); **MANUAL REVIEW** marker for control-flow-specific behavior (custom Continue/Skip/Retry reasons; Terminate interaction; test fixtures) | New at CP3; per ADR-0038 §Negative item 4 ("Codemod can cover the common case; edge cases (control-flow-specific behavior) need hand migration") — codemod attempts AUTO first, falls back to MANUAL marker on edge-case detection |

#### §10.2.1 Codemod execution model

The codemod is a `cargo`-style binary `nebula-action-codemod` (location: `tools/codemod/` — exact crate name and host TBD per Phase 0 / orchestrator) operating per-crate via `cargo metadata` to walk a target workspace. Per-transform modes:

- **AUTO mode (default for T1, T3, T4)** — codemod rewrites in place; surfaces a unified diff via `--dry-run` flag for review before commit.
- **MANUAL-REVIEW mode (default for T2, T5)** — codemod inserts `// TODO(action-cascade-codemod): ...` marker + leaves the original line unchanged; reviewer applies the rewrite by hand using the marker hint.
- **MIXED mode (T6)** — codemod attempts AUTO first for trivial pass-through (`impl ControlAction { fn execute(...) -> ControlOutcome }` → `impl StatelessAction` + `#[action(control_flow, ...)]` zone, body preserved); falls back to MANUAL-REVIEW marker on edge-case detection (custom Continue/Skip/Retry reason variants; ActionResult::Terminate interaction; test fixtures exercising `impl ControlAction` directly via mock dispatch).

**Idempotent.** Re-running the codemod on already-migrated code is a no-op (no marker insertion if the new pattern is already present).

### §10.3 Per-consumer migration step counts

Estimates per Phase 0 §10 line 346-356 blast-radius weight (range includes the "Blast-radius weight by consumer" header at line 346), refined by transform applicability:

| Consumer | T1 | T2 | T3 | T4 | T5 | T6 | Notes |
|---|---|---|---|---|---|---|---|
| `nebula-engine` | 0 (no `#[derive(Action)]` in engine source) | ~5 sites | ~3 sites (any pre-cascade `Box<dyn>`) | ~10 sites in `engine.rs` / `runtime.rs` / `registry.rs` | ~5-7 sites | 0 (engine doesn't impl ControlAction) | 27+ import sites total per Phase 0 §9 line 273 |
| `nebula-api` | 0 | ~1 site (webhook transport) | 0 | ~2 sites | ~1 site | 0 | 4 files; 🟡 LIGHT per Phase 0 §10 |
| `nebula-sandbox` | 0 | 0 (sandbox sees JSON adapter only) | ~3 sites (runner) | ~4 sites | ~2 sites | 0 | 7 files; 🟠 MODERATE per Phase 0 §10 — dyn-handler ABI |
| `nebula-sdk` | 0 (sdk re-exports; no actions) | 0 | 0 | 0 | 0 | 0 | 5 files but mostly re-export changes; covered by §9.3 reshuffle, not codemod transforms |
| `nebula-plugin` | ~1-2 sites (test fixtures) | 0 | 0 | 0 | 0 | 0 | 3 files; 🟡 LIGHT per Phase 0 §10 |
| `apps/cli` | ~3 sites (CLI dev fixtures) | ~1-2 sites | 0 | 0 | ~1 site | 0 | 5 files; 🟡 LIGHT |
| `crates/action/macros` (sibling) | self-reference; no transform | 0 | 0 | 0 | 0 | 0 | Macro crate itself; covered by §5 harness landing |

**Aggregate touch count.** ~55 file edits across 6 crates + 1 app per Phase 0 §10 line 358; codemod converts most into mechanical edits, leaving ~12-20 manual-review sites (mostly T2 + T5).

### §10.4 Plugin author migration guide (community plugins)

This is the **community plugin author runbook** — separate from the internal-Nebula crate migration above. Community plugins are crates outside the Nebula workspace that depend on `nebula-action` / `nebula-sdk::prelude` and ship `#[derive(Action)]`-decorated structs.

**Steps per plugin crate:**

1. **Bump `nebula-action` and `nebula-sdk` to the new release.** Per Cargo.toml; CP3 §13 evolution policy locks the crate version posture. Plugin authors reading `crates.io` see the new shape on bump.
1.5. **Add `semver = { workspace = true }` (or `semver = "1"` for non-workspace plugins) to consumer crate's `Cargo.toml`.** The `#[action]` macro emits `::semver::Version::new(major, minor, patch)` with the absolute `::semver::` path (per §4.6.2 line 920 + Phase 1 finding CC1); the macro is not auto-importable. Without this dep declared at the consumer's own Cargo.toml level, the first compile after migration fails with `error[E0433]: cannot find 'semver' in the crate root`. Re-export of `semver` through `nebula-action::__private::semver` is CP4 §15 housekeeping scope; for CP3, the consumer-side dep declaration is the smaller fix.
2. **Run the codemod (§10.2) in `--dry-run` mode** against the plugin crate. Surfaces all proposed transforms; no edits applied.
3. **Review the dry-run diff.** AUTO transforms (T1, T3, T4) rewrite mechanically; review for unintended catches. MANUAL-REVIEW transforms (T2, T5, T6) print TODO markers — plugin author applies the rewrite by hand following the marker hint.
4. **Apply codemod (`--apply`).** AUTO transforms land; MANUAL markers remain for hand-review.
5. **Resolve manual markers.** For each `// TODO(action-cascade-codemod): ...` marker:
   - **T2 (`ctx.credential::<S>()` removal)**: replace with `ctx.resolved_scheme(&self.<slot_name>)?` where `<slot_name>` matches a `#[action(credentials(<slot_name>: <Type>))]` declaration on the action struct. If the credential type is unknown at the call site, the plugin author must add an explicit `credentials(...)` zone to the action's `#[action(...)]` attribute first.
   - **T5 (`redacted_display!` wrap)**: confirm the error type's Display can leak credential material; if yes, wrap; if no (e.g., a numeric error code), leave unchanged.
   - **T6 (ControlAction → StatelessAction)**: rewrite `impl ControlAction for X` to `impl StatelessAction for X` + `#[action(control_flow, ...)]` zone (flag form, matching §12.2 example) on the struct attribute. Control-flow-specific behavior (custom `Continue` / `Skip` / `Retry` reasons) ports per-case. (CP4 §15 may revisit flag form vs `control_flow = SomeStrategy` config form — the spelling will land via §9.2 trait-by-trait audit closure; for CP3, flag form is the consistent placeholder across §10.2 / §10.4 / §12.2.)
6. **Run plugin's tests against the new release.** New compile-fail probes (§5.3 Probes 1-7) catch shape violations early. Cancellation-zeroize test (§6.4) validates `SchemeGuard` discipline.
7. **Update plugin's documentation.** README + examples reference the new `#[action]` attribute shape; remove stale references to `#[derive(Action)]` + `ctx.credential::<S>()`.

**Migration guide artefacts.** A `MIGRATION.md` ships in `crates/action/` alongside the cascade-landing PR documenting steps 1-7 with worked examples. Per `feedback_active_dev_mode.md` ("DoD includes migration guide for breaking changes"), the guide is a cascade-landing artefact, not a follow-up.

**Estimated migration cost per plugin.** Trivial plugin (1 stateless action, 1 credential): **<30 minutes**. Complex plugin (5+ actions, mixed credential / resource patterns, custom error sanitization): **2-4 hours** — bulk in MANUAL-REVIEW resolution + test re-run.

### §10.5 Auto-vs-manual breakdown summary

Per devops CP2 09e NIT 4 (CP3-track):

- **Automatable**: T1 (`#[derive]` → `#[action]`), T3 (`Box` → `Arc` legacy pattern), T4 (HRTB collapse). ~70% of total transforms by site count.
- **Manual review**: T2 (no-key credential removal), T5 (`redacted_display` wrap). ~30% of total transforms; mostly concentrated in engine + plugin authoring code.
- **Mixed**: T6 (ControlAction migration) — AUTO for trivial pass-through (default mode); MANUAL marker on edge-case detection per §10.2.1. Per ADR-0038 §Negative item 4 ("common case auto / edge cases manual").

Per-consumer share differs from this workspace-aggregate ratio: a trivial community plugin (1 stateless action, 1 credential, no `ctx.credential::<S>()` call sites) approaches ~100% AUTO; a heavy reverse-dep like `nebula-engine` (~10 T4 sites + ~5 T2 sites + ~5-7 T5 sites) sits closer to 50/50. The 70/30 figure is by file-touch count across the workspace, not per-consumer; per `feedback_active_dev_mode.md` ("DoD includes migration guide"), §10.4 sets per-plugin expectations explicitly.

Auto-vs-manual ratio aligns with Strategy §4.3.3 transform 3 ("Codemod must error on remaining call sites with crisp diagnostic, not silently rewrite") — the manual sites are where hard-removal discipline (per §6.2 + `feedback_no_shims.md`) requires human judgment, not silent rewrite. Automating those would be a `feedback_no_shims.md` violation.

---

## §11 Adapter authoring contract

This section locks the **adapter pattern** that bridges the typed action body (per CP1 §2.2) and the dyn-erased handler surface (per CP1 §2.4 / §2.5). Adapters are the dyn-erasure boundary; they are emitted by the `#[action]` macro for community plugins (§11.1) and authored by hand only in narrow internal-Nebula cases (§11.2).

### §11.1 Adapter macro emission semantics — `#[action]` macro is THE adapter

For community plugins, **`#[action]` is the adapter**. The macro emits both the typed-impl side and the dyn-erased side; community plugins do not write adapter code.

Per [ADR-0036 §Decision item 2](../../adr/0036-action-trait-shape.md) the macro emits:

- `Action` impl (identity + metadata supertrait satisfaction per §2.1)
- `ActionSlots` impl with `credential_slots() -> &'static [SlotBinding]` per §2.1.1 + §3.1
- `DeclaresDependencies` impl (replaces hand-written) per `crates/action/src/lib.rs` legacy shape
- The primary trait body wrapper (`StatelessAction` / `StatefulAction` / etc.) — connects user-typed `execute<'a>(...) -> impl Future + Send + 'a` to the adapter's `BoxFut<'a, ...>` conversion
- Adapter wiring — the `*ActionAdapter<A>` instance constructed at registration time so `Arc<dyn StatelessHandler>` can wrap the user-typed action

**Adapter as a generic wrapper.** Adapters are crate-local types per `crates/action/src/{stateless,stateful,trigger,resource}.rs` current shape:

```rust
// In crates/action/src/stateless.rs (current shape, preserved post-modernization):
pub struct StatelessActionAdapter<A: StatelessAction> { /* metadata + zero-cost wrapper */ }

impl<A: StatelessAction + ActionSlots> StatelessHandler for StatelessActionAdapter<A> {
    fn metadata(&self) -> &ActionMetadata { /* ... */ }
    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        input: serde_json::Value,
    ) -> BoxFut<'a, Result<serde_json::Value, ActionError>> {
        Box::pin(async move {
            // 1. Pre-scan input depth (cap 128) per §6.1.2-C
            // 2. Deserialize typed input
            // 3. Resolve credential slots per §3.1
            // 4. Invoke user-typed body: action.execute(ctx, input).await
            // 5. Serialize typed output
            // 6. Return ActionResult<Value> per §7.1
        })
    }
}
```

Community plugins write `#[action(...)] struct MyAction { ... } impl StatelessAction for MyAction { ... }` — the macro emits the `StatelessActionAdapter` instantiation. Plugin code never names `StatelessActionAdapter` directly.

### §11.2 Internal-Nebula adapter authoring (when `#[action]` is insufficient)

A small set of internal-Nebula contexts may need to author adapters by hand:

- **Engine-internal `MetaAction` test fixtures** that exercise dispatch shape without going through the `#[action]` macro (e.g., property tests on `ActionHandler` enum dispatch).
- **Sandbox out-of-process runner** (`crates/sandbox/`) that may need a custom `*HandlerProxy<A>` adapter shape for cross-process serialization (per Phase 0 §9 line 288 — "spans both in-process and out-of-process runners").
- **Future custom DX shapes** authored within `nebula-action` itself before sealed-DX expansion lands (rare; cascade-internal only).

For these cases, the **sealed_dx adapter pattern from [ADR-0038 §1](../../adr/0038-controlaction-seal-canon-revision.md)** is the contract:

```rust
// Crate-internal adapter authoring path (mod sealed_dx::* private):
mod sealed_dx {
    pub trait MyCustomShapeSealed {}
}

pub struct MyCustomShapeAdapter<A: StatelessAction> {
    inner: A,
    metadata: ActionMetadata,
}

// Crate-internal blanket impl seals authoring eligibility:
impl<T: StatelessAction + ActionSlots> sealed_dx::MyCustomShapeSealed for T {}

// Adapter implements the dyn-safe primary handler:
impl<A: StatelessAction + ActionSlots> StatelessHandler for MyCustomShapeAdapter<A> {
    /* ... same shape as §11.1 adapter ... */
}
```

The seal prevents external crates from authoring adapter parallels; `pub use` from `crates/action/src/lib.rs` is restricted to the adapter type identifier, not the sealed inner trait.

### §11.3 Adapter responsibilities (load-bearing contract)

Every adapter — macro-emitted or hand-authored — discharges these responsibilities:

#### §11.3.1 Serialize/deserialize boundary (`Input` ↔ JSON)

Per CP1 §2.4 + CP2 §6.1 + §7.1:

- **Input pre-scan**: depth cap 128 via `crate::webhook::check_json_depth(&input_bytes, 128)` per §6.1.2-C; failure → `ActionError::Validation { reason: ValidationReason::DepthExceeded { observed, cap }, .. }`.
- **Input deserialize**: `serde_json::from_slice::<A::Input>(&input_bytes)` per §6.1.2-C; failure → `ActionError::Validation { reason: ValidationReason::MalformedJson, .. }`.
- **Output serialize**: `to_value(typed_output)` per §7.1 step 5; failure → `ActionError::Fatal { ... }` per §7.3 propagation table; wrapped through `redacted_display(&e)` per §6.3.1-A.
- **Stateful adapters additionally** pre-scan + deserialize state JSON per §7.1 stateful-divergence (`crates/action/src/stateful.rs:573-582` shape; closes S-J2 simultaneously per 03c §1).

#### §11.3.2 Error propagation (CP2 §6.3 sanitization)

Per §6.3.1 + §7.3 propagation table:

- Every `ActionError` emit site that includes a foreign error's `Display` (e.g., `serde_json::Error`, `ResolveError`, user-supplied `A::Error`) wraps the foreign Display through `nebula_redact::redacted_display(&e) -> String` per §6.3.1-A pre-`format!` wrap-form.
- `From<A::Error>` impl is provided by the user-typed `A::Error: std::error::Error + Send + Sync + 'static`; per CP1 §2.8 the `ActionErrorExt` companion trait drives the typed → `ActionError` conversion.
- Cancellation does NOT propagate as an error per §7.3 — adapter does not catch the body's `tokio::JoinHandle::abort` signal; engine sees task cancellation directly.

#### §11.3.3 Cancellation safety (CP2 §6.4 ZeroizeProbe contract surface)

Per §3.4 + §6.4 + spike Iter-2 §2.4 (commit `c8aef6a0`):

- The adapter's `BoxFut<'a, ...>` body is cancellable at any `.await` point. The body's outermost `tokio::select!` discipline at the action body propagates cancellation (per §3.4 mechanism item 1).
- All `SchemeGuard<'a, C>` instances in scope at cancellation drop deterministically, zeroizing their underlying `C::Scheme` before borrow-chain unwind. Adapter does NOT manually clean up guards — Drop runs naturally per credential Tech Spec §15.7.
- Test contract (§6.4): adapters are exercised by the three sub-tests (`scheme_guard_zeroize_on_cancellation_via_select`, `scheme_guard_zeroize_on_normal_drop`, `scheme_guard_zeroize_on_future_drop_after_partial_progress`) per §6.4.1.
- ZeroizeProbe instrumentation: per-test `Arc<AtomicUsize>` per §6.4.2 (closes 08c §Gap 4); adapters are constructed in tests via `engine_construct_with_probe` test-only constructor variant (soft-amendment к credential Tech Spec §15.7 per §6.4.2).

**Open item §11.3-1.** Adapter performance budget — per-dispatch overhead (input serialization round-trip + depth pre-scan + slot resolution + output serialization). CP2 §6.1.2-D names the byte-pre-scan cost as "small (`to_vec` round-trip)"; CP3 §11 forward-tracks: a microbenchmark for a representative `StatelessAction` (e.g., the spike's iter-2 Action A `SlackSendAction` shape) lands as part of CP4 §15 housekeeping. CP3 §11 commits the responsibility table; perf measurement is CP4 / implementation-time scope.

---

## §12 ControlAction + DX migration

This section locks the **ControlAction migration contract** per [ADR-0038](../../adr/0038-controlaction-seal-canon-revision.md). CP1 §2.6 already locked the sealed-DX trait family; CP3 §12 details the migration path from today's `pub trait ControlAction { ... }` (non-sealed, public-impl-allowed) to the sealed shape, and from the community plugin author perspective.

### §12.1 Sealed adapter pattern (verbatim from [ADR-0038 §1](../../adr/0038-controlaction-seal-canon-revision.md))

Per ADR-0038 §1 line 49-70 (verbatim):

> `ControlAction` becomes a sealed trait — community plugin crates may NOT implement it directly. Sealing follows the per-capability inner-sealed-trait pattern from ADR-0035 §3:
>
> ```rust
> mod sealed_dx {
>     pub trait ControlActionSealed {}
>     pub trait PaginatedActionSealed {}
>     pub trait BatchActionSealed {}
>     pub trait WebhookActionSealed {}
>     pub trait PollActionSealed {}
> }
>
> pub trait ControlAction: sealed_dx::ControlActionSealed { /* ... */ }
> ```
>
> The blanket `impl<T: StatelessAction> sealed_dx::ControlActionSealed for T {}` (or analogous, depending on the wrap shape) ensures only `StatelessAction` implementors gain `ControlAction` membership via the **adapter pattern** — community plugins use `StatelessAction` as the primary dispatch trait + adapter to gain `ControlAction` semantics. Internal Nebula crates may continue to author `ControlAction`-using actions through the sealed adapter.

CP1 §2.6 refines the blanket-impl to require `+ ActionSlots` per spike `final_shape_v2.rs:282`: `impl<T: StatelessAction + ActionSlots> sealed_dx::ControlActionSealed for T {}`. This Tech Spec preserves that refinement in §9.2 + §12.

### §12.2 Community plugin DX flow

Community plugin authors **do NOT implement any sealed DX trait directly**. Per [ADR-0038 §Negative item 4](../../adr/0038-controlaction-seal-canon-revision.md) line 111 (verbatim):

> Two-step user-facing migration: code that today does `impl ControlAction for X` must move to `impl StatelessAction for X` + sealed adapter. Codemod can cover the common case; edge cases (control-flow-specific behavior) need hand migration. In-cascade per scope §1.6.

**Concrete community-plugin authoring shape (post-cascade):**

```rust
use nebula_sdk::prelude::*;

#[action(
    key  = "myplugin.control_example",
    name = "Control Example",
    control_flow,                   // <- attribute zone signals control-flow shape; macro emits ControlActionAdapter wiring
    credentials(api: ApiToken),
)]
pub struct ControlExampleAction;

impl StatelessAction for ControlExampleAction {
    type Input = ControlExampleInput;
    type Output = ControlOutcome;       // <- typed control-flow output
    type Error = MyPluginError;

    fn execute<'a>(
        &'a self,
        ctx: &'a ActionContext<'a>,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a {
        async move {
            // Plugin-author logic returns ControlOutcome::{Continue, Skip, Retry, ...}
            // The macro-emitted ControlActionAdapter erases ControlOutcome to ActionResult<Value>
            Ok(ControlOutcome::Continue { /* ... */ })
        }
    }
}
```

The `#[action(control_flow)]` zone is the signal to the macro to emit the `ControlActionAdapter<ControlExampleAction>` wrapper per §11. Plugin author never names `ControlAction` trait or `ControlActionAdapter` type — both are cascade-internal per ADR-0038 §1. (The exact attribute zone syntax — `control_flow` flag vs `control_flow = SomeStrategy` config — is CP4 §15 scope per §9.2 trait-by-trait audit closure.)

### §12.3 Internal Nebula crate migration (engine + sandbox)

Engine + sandbox already implement at the **handler level** (`StatelessHandler`, `StatefulHandler`, etc. per `crates/action/src/handler.rs:39-50` + Phase 0 §9 line 288) — they consume `Arc<dyn StatelessHandler>`-erased actions, not typed `ControlAction` impls. The sealed DX is **mostly additive for community visibility**, not a refactor for engine / sandbox internal dispatch.

What changes for internal Nebula crates:

- **`crates/action/src/control.rs`** — current `pub trait ControlAction { ... }` becomes sealed per §12.1; existing internal `impl ControlAction for X` patterns (if any in `crates/action/src/` itself or test fixtures) re-pin via T6 codemod (§10.2). Per Phase 0 audit no external ControlAction implementors are tracked in Strategy §1(c); migration surface is small.
- **`crates/action/src/lib.rs:4`** library docstring becomes truthful per ADR-0038 §Neutral item 2 — currently self-contradicts "adding a trait requires canon revision" (line 4 verbatim) while re-exporting 10 traits; post-cascade it states the actual shape (4 primary + 5 sealed DX).
- **PRODUCT_CANON §3.5 line 82** — wording revision per [ADR-0038 §2](../../adr/0038-controlaction-seal-canon-revision.md). Inline canon edit lands as PR alongside ADR-0038 ratification per ADR-0038 Implementation note.

### §12.4 Codemod coverage (T6) — common-case automation

Per §10.2 transform T6 + ADR-0038 §Negative item 4 ("Codemod can cover the common case; edge cases need hand migration"):

**Auto-rewrite path (T6 AUTO mode):**

```rust
// Source pattern:
impl ControlAction for MyAction {
    fn execute(...) -> ControlOutcome { /* ... */ }
}

// Target pattern (codemod-rewritten):
impl StatelessAction for MyAction {
    type Input = MyActionInput;
    type Output = ControlOutcome;
    type Error = MyPluginError;
    fn execute<'a>(&'a self, ctx: &'a ActionContext<'a>, input: Self::Input)
        -> impl Future<Output = Result<Self::Output, Self::Error>> + Send + 'a
    {
        async move { /* original body */ }
    }
}
// + #[action(control_flow, ...)] zone added to the struct attribute
```

**Manual-review path (T6 MANUAL-REVIEW mode):**

- Custom `Continue` / `Skip` / `Retry` reason variants — codemod cannot mechanically determine the user's intent.
- Interaction with `ActionResult::Terminate` — manual confirmation of the wire-end-to-end discipline per §2.7.1.
- Test fixtures that exercise `impl ControlAction` directly (e.g., via mock dispatch) — codemod marks; reviewer rewrites tests to use `StatelessAction` + adapter dispatch.

CP3 §12.4 commits T6 in §10.2 codemod transform list. Plugin author migration guide §10.4 step 5 references the T6-specific rewrite pattern.

---

## §13 Evolution policy

This section locks the **post-cascade evolution discipline** for `nebula-action`. Phase 0 audit T7 (semver-checks advisory-only during alpha) sets the current posture; CP3 §13 makes explicit how `nebula-action` evolves through deprecation, breaking changes, and per-crate versioning.

### §13.1 Deprecation policy

**Pre-1.0 alpha-cycle posture (current).** Hard breaking changes are acceptable per `feedback_hard_breaking_changes.md`. Per Phase 0 §10 line 362-363: "`semver-checks.yml:27` is advisory-only during alpha" + "feedback memory `feedback_hard_breaking_changes.md` + `feedback_adr_revisable.md` + `feedback_bold_refactor_pace.md` all align: hard breaking changes are acceptable right now for spec-correct outcomes; the blast radius just sets the scale, not the gate."

**Post-1.0 posture.** Once `nebula-action` graduates to 1.0 (per ADR-0021 crate publication policy — out of cascade scope per §13.3), breaking changes require:
1. **Deprecation cycle** of one minor release, NOT shim form per `feedback_no_shims.md`. Deprecation is `#[deprecated(since = "X.Y.0", note = "...")]` on the type / method, with a clear migration target named in the note.
2. **Major-bump landing** at the next major release, with a CHANGELOG entry citing the ADR that ratified the breaking change.
3. **Codemod artefact** for any non-trivial migration (the `#[derive(Action)]` → `#[action]` precedent at §10).

**Forward-track for §6.2-1 (`credential_typed::<S>(key)` decision).** The §9.3.1 recommendation **remove** is consistent with the pre-1.0 alpha posture (hard breaking change acceptable now). Post-1.0 a similar removal would require a deprecation cycle. If the user / tech-lead picks **retain** instead of **remove** at CP3 close, the retention is a permanent surface element subject to the post-1.0 deprecation discipline above.

### §13.2 Breaking-change policy

Two classes of breaking change:

#### §13.2.1 Spec-level (ADR amendment-in-place per ADR-0035 precedent)

Per ADR-0035 amended-in-place precedent (referenced throughout CP2 §5.4.1 + §6.4 cross-crate amendments + §7.2): for breaking changes that refine existing decisions without paradigm shift, ADR amendment-in-place is the mechanism. Examples (from this Tech Spec):

- ADR-0037 §1 `SlotBinding` shape divergence (CP2 §15 forward-track) — `field_name` vs `key`, capability folded into `SlotType` enum vs separate field. Lands as `*Amended by ADR-0037 inline edit, 2026-04-24*` prefix at §1; CHANGELOG entry.
- Soft amendments к credential Tech Spec §16.1.1 probe #7 (qualified-syntax form per §5.4.1) and §15.7 (`engine_construct_with_probe` test-only constructor per §6.4.2) — flagged in this Tech Spec, NOT enacted; CP4 cross-section pass surfaces; credential Tech Spec author lands the inline edit.

**Discipline.** Amendment-in-place preserves the ADR's identity (decision name unchanged); it adds a dated annotation and updates the affected paragraph(s). Surrounding decisions are not retracted.

#### §13.2.2 Paradigm shift (full ADR supersession)

For breaking changes that **invalidate** the prior decision rather than refine it, full ADR supersession applies per ADR-0035 / ADR-0036 / ADR-0037 / ADR-0038 own status mechanics: `superseded` status set + `supersedes:` / `superseded_by:` cross-reference in frontmatter.

Examples (none active in current Tech Spec scope; all hypothetical):

- If a future Rust release adds `for<'ctx> async fn(...)` syntax (per [ADR-0037 §Negative item 1](../../adr/0037-action-macro-emission.md) line 121), the HRTB fn-pointer shape may be superseded by an `async fn` pointer shape. ADR-0037 supersession lands; macro emission shifts.
- If a future cluster-mode coordination cascade re-shapes `TriggerAction` cluster hooks from supertrait extensions to a separate `ClusterAware` trait (per N4 + Strategy §3.4 row), that's a paradigm shift; new ADR + supersession.

**Discipline.** Supersession pre-empts amendment-in-place when the original decision's name no longer fits the new shape — e.g., "ADR-0036 action-trait-shape" describes attribute-macro-replaces-derive; if a future cascade restores derives via a different mechanism, supersession (not amendment) is the honest record.

### §13.3 Versioning per crate publication

**Out of cascade scope per ADR-0021.** Crate publication policy (when `nebula-action` ships to crates.io, what version it ships at, what its semver-checks posture is post-publication) is governed by ADR-0021. CP3 §13.3 explicitly defers; the action redesign cascade lands changes at 0.1.0-cycle pre-publication shape.

**Cross-ref.** When `nebula-action` is queued for crates.io publication (post-cascade housekeeping), the publication PR should cite this Tech Spec §9 public surface as the at-publication contract. Pre-publication shape is mutable per §13.1; post-publication shape locks to the §13.1 post-1.0 posture.

### §13.4 CP1 hygiene fold-in vs out-of-cascade scope

CP1 + CP2 carry-forward devops nit-list (T4 / T5 / T9 from CP1 09e — minor; deferred to CP3 §13 fold-in or CP4 §16 explicit-pointer):

#### §13.4.1 T4 — `zeroize` workspace=true pin (cascade-scope absorb)

**Context ([`01b-workspace-audit.md`](../drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md) §1 finding 🟠 MAJOR line 44).** `crates/action/Cargo.toml:36` pins `zeroize = { version = "1.8.2" }` inline; workspace declares `zeroize = { version = "1.8.2", features = ["std"] }` at root `Cargo.toml:116`. Inline pin silently drops the `std` feature and de-unifies the version in the feature-unification graph.

**Decision.** **Cascade-scope absorb.** The `nebula-action` Cargo.toml edit lands at implementation time per §10 codemod runbook Step 1 (cascade-landing PR includes Cargo.toml hygiene). Edit:

```toml
# crates/action/Cargo.toml (CP3 §13 amendment shape):
# Before:  zeroize = { version = "1.8.2" }                              # line 36, drops std feature
# After:   zeroize = { workspace = true }                                # workspace-pinned with std feature
```

**Why cascade-scope.** `zeroize` is the canonical zeroize dependency for `SchemeGuard<'a, C>` per credential Tech Spec §15.7 + spike `final_shape_v2.rs:95`; the `std` feature is required for `Vec<u8>` zeroization patterns the credential / action surfaces cross-reference. De-unifying the version risks crypto-dep version skew — shared with `crates/credential`, `crates/api`, webhook verification path per [`01b-workspace-audit.md`](../drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md) §1.

#### §13.4.2 T5 — `lefthook.yml` doctests/msrv/doc parity (out of action cascade scope)

**Context ([`01b-workspace-audit.md`](../drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md) §11 finding 🟠 MAJOR line 376).** `lefthook.yml:45` does not mirror CI's `doctests` / `msrv` / `doc` jobs; action has 20+ doctests per workspace audit; `feedback_lefthook_mirrors_ci.md` discipline names this as a divergence.

**Decision.** **Out of action cascade scope; separate housekeeping PR.** Per Phase 1 02d (workspace hygiene scope split) + `feedback_lefthook_mirrors_ci.md`, lefthook parity is workspace-wide concern and lands as its own housekeeping PR independent of the action redesign. Action cascade cites the gap; the fix is owned by devops with target sunset window per `feedback_lefthook_mirrors_ci.md` discipline (≤2 release cycles).

**Forward-pointer.** Migration guide §10.4 step 7 ("Run plugin's tests against the new release") implicitly relies on lefthook parity for plugin authors who use lefthook locally. If a community plugin author hits a CI-pre-push divergence (e.g., a doctest fails in CI but passed pre-push), the housekeeping PR is the resolution path — not an action cascade re-open.

#### §13.4.3 T9 — `deny.toml` layer-enforcement rule for `nebula-action` (cascade-scope absorb)

**Context ([`01b-workspace-audit.md`](../drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md) §11 finding 🟠 MAJOR line 379).** `deny.toml` has positive bans for `engine` / `storage` / `sandbox` / `sdk`; `nebula-action` relies on implicit correctness — no positive ban rule. Action cascade introduces a `[dev-dependencies]` `nebula-engine` edge (per §5.3-1 rust-senior 09b #1 resolution) which compounds the missing guardrail.

**Decision.** **Cascade-scope absorb.** Per CP2 §5.3-1 commitment ("CP3 §9 lands the `deny.toml` edit alongside the macro-crate dev-deps wiring"), the `deny.toml` amendment lands at implementation time alongside the cascade PR. **Two edits, both targeting the existing `[bans] deny = [...]` block at `deny.toml:48-81`:**

```toml
# deny.toml (CP3 §13 amendment shape) — both edits target existing [bans] deny = [...] block:

# ---- Edit 1: extend the existing nebula-engine wrappers list (deny.toml:59-66). ----
# Current shape:
#   { crate = "nebula-engine", wrappers = ["nebula-cli", "nebula-api"], reason = "..." }
# Amended shape (add "nebula-action-macros" to the wrappers list):
{ crate = "nebula-engine", wrappers = [
    "nebula-cli",
    # Dev-only: `crates/api/tests/knife.rs` ... (existing comment preserved verbatim)
    "nebula-api",
    # Dev-only: `nebula-action-macros` `[dev-dependencies]` `nebula-engine` for
    # compile-fail Probe 6 (real `resolve_as_bearer::<C>` HRTB coercion bound-mismatch
    # verification per Tech Spec §5.3-1; stub-helper alternative rejected per CP2 §5.3-1).
    "nebula-action-macros",
], reason = "Engine is exec-layer orchestration; business/core crates must not depend on it" }

# ---- Edit 2: NEW positive ban for nebula-action runtime layer (T9 full intent). ----
# Symmetric with existing engine / sandbox / storage / sdk / plugin-sdk bans at deny.toml:52-81.
# Per Phase 0 audit §11 row 9 + Strategy §1.6 layer ordering: nebula-action is the business-trait
# layer; engine / api / storage / sandbox / sdk / plugin-sdk are upward layers and MUST NOT be
# runtime deps of nebula-action.
{ crate = "nebula-action", wrappers = [
    # All current reverse-deps that are allowed to depend on nebula-action go here.
    # Initial list per Phase 0 audit §9 reverse-deps inventory:
    "nebula-engine",
    "nebula-sandbox",
    "nebula-api",
    "nebula-sdk",
    "nebula-plugin",
    "nebula-cli",
    "nebula-action-macros",  # sibling test crate
], reason = "Action is business-trait layer; upward layers (engine/api/storage/sandbox/sdk/plugin-sdk) must not be runtime deps of nebula-action — symmetric with §1.6 layer ordering" }
```

**Verification.** `cargo deny check` post-amendment per CP2 §5.3-1; lefthook + CI both run `cargo deny`. Edit 1 closes the `nebula-engine` dev-dep wrapper omission; Edit 2 closes Phase 0 §11 row 9 (T9 full intent: positive ban on `nebula-action` symmetric with engine/sandbox/storage/sdk/plugin-sdk rules).

#### §13.4.4 `nebula-redact` workspace integration (cascade-scope absorb — preliminary)

**Context.** §6.3.2 (CP2) commits to creating `nebula-redact` as a NEW dedicated crate; §11.3.2 names `nebula_redact::redacted_display(&e)` in production code (adapter responsibility table); §9.3.2 added list re-exports `nebula-redact::redacted_display` through call sites. **`nebula-redact` does not exist in the current workspace** (verified: `crates/redact/` absent; root `Cargo.toml [workspace] members` does not list it; `[workspace.dependencies]` has no `nebula-redact` entry). Without explicit absorption, the cascade-landing engineer may either (a) land action changes that fail to compile (missing `nebula_redact` dep) or (b) land the redact crate as a separate PR (cascade-fragmentation per `feedback_active_dev_mode.md` "finish partial work in sibling crates").

**Decision.** **Cascade-scope absorb (preliminary — must land BEFORE or atomic-with the action cascade PR per `feedback_active_dev_mode.md` DoD).** Four atomic edits in the cascade-landing PR:

1. **`crates/redact/Cargo.toml`** — new manifest with name `nebula-redact`, version `0.1.0`, edition pinned to workspace.
2. **`crates/redact/src/lib.rs`** — public surface: `redacted_display` function (per §6.3.2 specification).
3. **Root `Cargo.toml`** — add `crates/redact` to `[workspace] members`; add `nebula-redact = { path = "crates/redact" }` (or workspace-version pin) to `[workspace.dependencies]`.
4. **`deny.toml`** — **no new ban needed** (`nebula-redact` is a leaf utility crate consumed by `nebula-action` only; no upward-layer guardrail required at this stage). The Edit 1 + Edit 2 amendments to `[bans] deny = [...]` from §13.4.3 above are unaffected.

**Why preliminary, not strictly cascade-scope.** The crate's existence is a precondition for the cascade landing — `crates/action/src/{stateless,stateful}.rs` error-emit sites (per §6.3.1-A) reference `nebula_redact::redacted_display`. Per `feedback_no_shims.md` discipline, no stub / empty-shell `nebula-redact` lands ahead of the actual implementation; the substantive `redacted_display` body lands atomic with the call sites that consume it.

**Verification.** Post-cascade, `cargo build --workspace` resolves `nebula_redact::redacted_display` at every call site enumerated in §11.3.2 / §6.3.1-A; `cargo deny check` passes (no new ban; existing rules unaffected).

#### §13.4.5 Summary of T4 / T5 / T9 / `nebula-redact` dispositions

| Item | Disposition | Lands at |
|---|---|---|
| **T4** `zeroize` workspace=true pin | Cascade-scope absorb | `crates/action/Cargo.toml` edit alongside cascade PR |
| **T5** `lefthook.yml` parity | Out of cascade scope | Separate housekeeping PR (devops-owned) |
| **T9** `deny.toml` layer-enforcement | Cascade-scope absorb | `deny.toml` edits alongside macro-crate dev-deps wiring (per CP2 §5.3-1) — wrappers-list extension to existing `nebula-engine` rule + NEW positive ban for `nebula-action` runtime layer |
| **`nebula-redact`** workspace integration (NEW crate) | Cascade-scope absorb (preliminary — atomic with cascade PR) | `crates/redact/{Cargo.toml,src/lib.rs}` + root `Cargo.toml` `[workspace] members` + `[workspace.dependencies]` (no new `deny.toml` ban — leaf utility) |

CP4 §16 picks up only T5 as a sunset-tracked item; T4 + T9 + `nebula-redact` are absorbed.

---

## §14 Cross-references

This section consolidates every load-bearing cross-document reference the Tech Spec depends on at line-number granularity. CP4 binds these so that future re-pin events (per §0.2 invariants) trigger explicit reviewer pass rather than silent drift. Every citation below has a corresponding `grep`-able anchor in the cited document at draft time.

### §14.1 ADR matrix

| ADR | Status | Cited by | Relationship to Tech Spec |
|---|---|---|---|
| **ADR-0035** phantom-shim capability pattern | `accepted` (amended 2026-04-24-B post iter-2; 2026-04-24-C post iter-3) | §2.1 supertrait shape; §4.1.1 credential type-pattern dispatch; §11.2 sealed-DX adapter pattern; §13.2.1 amendment-in-place precedent (load-bearing for §15.5 ADR-0037 amendment); §13.4.4 `nebula-redact` `feedback_no_shims.md` discipline | Sets the cross-cascade precedent for amend-in-place vs supersession (ADR-0035 §Status block records iter-2-B and iter-3-C amendments — same mechanism §15.5 enacts on ADR-0037) |
| **ADR-0036** action trait shape | `proposed` (status moves to `accepted` at Tech Spec ratification per §0.1 line 35) | §1 G1/G2 closures; §2.1.1 `ActionSlots` companion trait shape; §4 attribute macro narrow-zone rewriting contract; §11.1 macro-emitted adapter shape | Trait shape is signature-locked at §2; macro emission contract grounds in this ADR per ADR-0036 §Decision items 1-4 |
| **ADR-0037** action macro emission | `proposed` → `proposed (amended-in-place 2026-04-25)` per §15.5 enactment | §3.1 `SlotBinding` shape (post-amendment); §4.3 per-slot emission table; §4.4 dual enforcement layer; §5.3 6-probe port (production harness); §5.4 qualified-syntax probe form | **Amended-in-place this CP per §15.5 — see §15.5.1 enactment**; pre-amendment shape diverged from credential Tech Spec §9.4 line 2452 |
| **ADR-0038** ControlAction seal + canon §3.5 revision | `proposed` (status moves to `accepted` at canon §3.5 revision PR ratification — see §16.5) | §1 G4 sealed DX tier ratification; §2.6 five sealed DX traits; §9.2 sealed DX surface; §10.2 T6 codemod transform (community migration target per ADR-0038 §Negative item 4); §12 ControlAction migration | Canon §3.5 line 82 revision PR is a §16.5 cascade-final precondition (separate from Tech Spec ratification but co-gating) |

**Phantom-shim composition note.** ADR-0036 + ADR-0037 + ADR-0038 all compose with [ADR-0035](../../adr/0035-phantom-shim-capability-pattern.md) at `SchemeGuard<'a, C>` + `RefreshDispatcher::refresh_fn` HRTB shape per credential Tech Spec §15.12.3. The four-ADR composition is structural; this Tech Spec inherits the composition without re-deriving. Per §0.2 invariant 2: any ADR moving from `accepted` to `superseded` (or undergoing non-trivial amendment beyond §15.5's enactment) re-opens this Tech Spec for re-pin.

### §14.2 Strategy parent-doc cross-refs (frozen CP3)

| Strategy § | Tech Spec sections citing | Purpose |
|---|---|---|
| §1 problem framing (line 70) — `Terminate` literal canon §4.5 false-capability violation | §1 G6 + §2.7.1 | G6 binds wire-end-to-end resolution to canon §4.5 violation |
| §2.12 must-have floor (line 247-254) | §1 G3 four-item floor + §6 implementation forms | Verbatim must-have-floor cite (per `feedback_observability_as_completion.md` integration) |
| §3.1 component 2-7 (post-pick A' decomposition) | §1 G1/G5 + §2.1.1 + §3.1 + §4 macro emission | A' eight-component decomposition grounds Tech Spec §1-§7 sections |
| §3.4 OUT row table (line 165-183) | §1.2 N1-N7 non-goals | Each non-goal cites Strategy §3.4 OUT row verbatim (honest-deferral discipline) |
| §4.2 implementation paths (a)/(b)/(c) (line 198-206) | §1.2 N5 + §16.1 path framing | Strategy frames the choice; CP4 §16 records criteria; user picks at Phase 8 |
| §4.3.1 `*Handler` HRTB modernization in-scope (line 213-220) | §1 G5 + §2.4 | Modernization decision; ~30-40% LOC reduction sourced from rust-senior 02c §8 line 439 |
| §4.3.2 retry-scheduler symmetric-gating principle (line 222-229) | §1 G6 + §2.7.1 | Strategy locks principle; Tech Spec §2.7.1 picks wire-end-to-end concrete path |
| §4.3.3 codemod transforms 1-5 minimal-complete-set (line 231-243) | §10.2 T1-T6 (T1-T2 mapped from Strategy 1-3; T3-T6 added at Tech Spec design-level per Strategy §4.3.3 line 243 license) | Codemod design ground; Strategy 4-5 are not code-edit transforms (covered by CP2 §5.1 dev-deps + CP3 §9.3 prelude reshuffle) |
| §4.4 security floor invariant (line 247-254) | §6 (CO-DECISION territory) | §6 implementation form must not relax (per §0.2 invariant 3) |
| §6.4 concerns register lifecycle | §16.5 cascade-final readiness (no 🔴 unresolved at Phase 8) | Path (c) viability gate compounds with concerns register state |
| §6.5 a/b/c decision tree (line 408-413) | §16.1 path table | Tech Spec §16.1 extends Strategy §6.5 with concrete cascade-final criteria |
| §6.6 cross-crate coordination (line 416-426) | §16.1 path (b)/(c) viability gate | Credential CP6 cascade slot commitment is path (b)/(c) precondition; silent-degradation guard active |
| §6.8 B'+ contingency (line 443-461) | §16.4 rollback strategy | B'+ activation is architect+tech-lead co-decision per §6.8 — orchestrator does NOT silently activate |
| §6.9 retry-scheduler chosen path locus (line 463-465) | §1 G6 + §2.7.1 | Strategy locks principle, Tech Spec picks path (wire-end-to-end) |

### §14.3 Credential Tech Spec cross-refs

| Credential Tech Spec § | Tech Spec citation point | Status |
|---|---|---|
| §2.7 line 486-528 — `#[action]` macro `dyn ServiceCapability` → `dyn ServiceCapabilityPhantom` rewrite | §4.1.1 credential type-pattern dispatch | Cross-crate authoritative for Pattern 2/3 rewriting |
| §3.4 line 807-939 — Pattern 2 dispatch narrative (4-step path) | §3.2 dispatch path (steps 1-7) | Cited verbatim, not restated |
| §3.4 line 851-863 — `ActionSlots::credential_slots(&self)` cardinality on receiver | §2.1.1 + §3.1 — reconciled to `&self` form | Cross-crate authoritative (deliberate divergence #3 per §2 preamble); spike `final_shape_v2.rs:278` already aligned |
| §3.4 line 869 — load-bearing HRTB declaration `for<'ctx> fn(&'ctx CredentialContext<'ctx>, &'ctx SlotKey) -> BoxFuture<'ctx, Result<ResolvedSlot, ResolveError>>` | §3.2 `ResolveFn` type alias + ADR-0037 §1 (post-amendment) | Cross-crate authoritative; same shape as `RefreshDispatcher::refresh_fn` (§7.1) |
| §7.1 — `RefreshDispatcher::refresh_fn` HRTB pattern | §3.2 narrative parallel | Compositional cross-ref only |
| §9.4 line 2452 → **§15.8 (CP5 supersession of §9.4)** — `SlotType` three-variant matching pipeline (`Concrete`, `ServiceCapability`, `CapabilityOnly`) | §3.1 `SlotType` enum (lines 619-633) | **Cross-crate authoritative — load-bearing for §15.5 ADR-0037 amendment.** Matching-axis shape preserved verbatim across supersession (`SlotType::Concrete / ServiceCapability / CapabilityOnly` axes per §15.8 line 3522 "Same `SlotType::Concrete / ServiceCapability / CapabilityOnly` matching axes"); only the **filter authority source** shifts plugin-metadata `capabilities_enabled` → registry-computed `RegistryEntry::capabilities` (per credential Tech Spec §15.8 line 3520-3528) |
| §9.4 line 2456-2470 → **§15.8 (CP5 supersession)** — engine-side `iter_compatible` dispatch on `SlotType` | §3.1 storage-shape narrative | Cross-crate authoritative (engine-side runtime pipeline). CP5 canonical body at §15.8 consults `RegistryEntry::capabilities` rather than `cred.metadata().capabilities_enabled` (§9.4 supersede block line 2446) |
| §15.7 line 3394-3429 — `SchemeGuard<'a, C>` decision (`!Clone`, `ZeroizeOnDrop`, `Deref`, lifetime parameter) | §7.2 RAII flow + §3.4 cancellation safety | Cross-crate authoritative; cited verbatim, not restated |
| §15.7 line 3438-3447 — `SchemeFactory<C>` for re-acquisition by long-lived resources | §7.2 narrative cross-ref | Out-of-scope per §1.2 N1; cited for compositional completeness |
| §15.7 line 3503-3516 — Iter-3 lifetime-pin refinement (`engine_construct(scheme, &'a credential_ctx)`) | §3.2 step 5 (`SchemeGuard::engine_construct`) | Cross-crate authoritative |
| §15.12.3 line 3689 — Iter-3 Gate 3 sub-trait × phantom-shim composition validation | §0.1 inputs frozen-at footer + ADR-0035 amendment 2026-04-24-C citation | Validation evidence chain |
| **§15.7 — soft amendment for `engine_construct_with_probe` test-only constructor variant** | §6.4.2 + §15.4 forward-track | **Soft amendment — FLAGGED, NOT ENACTED** per §15.4 |
| **§16.1.1 probe #7 (line 3756) — `compile_fail_scheme_guard_clone.rs` shape** | §5.4.1 + §15.3 forward-track | **Soft amendment — FLAGGED, NOT ENACTED** per §15.3 |

**Re-pin obligation.** Per §0.2 invariants 2 + 4: if any cited line range moves due to upstream credential Tech Spec edit, this Tech Spec must be re-pinned (CHANGELOG entry + reviewer pass). Reference [`reference_credential_tech_spec_pins.md`](../../../.claude/agent-memory-local/architect/reference_credential_tech_spec_pins.md) for line-pin governance.

### §14.4 Phase 1 register cross-refs (CR1-CR11 closure traceability)

Per [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-action-redesign/02-pain-enumeration.md) §4 Critical findings:

| CR ID | Subject | Closed by | Verification |
|---|---|---|---|
| **CR1** | Typed credential surface unrealized | §1 G1 + §3.1 + §4.1.1 + §11.1 | `#[action]` macro emits `&'static [SlotBinding]` per ADR-0037 §1 (post-amendment); spike Iter-1 PASS |
| **CR2** | Macro emission shape regressions | §1 G2 + §5 macro test harness + ADR-0037 §4 6-probe table | trybuild + macrotest dev-deps; 6 probes ported from spike commit `c8aef6a0` |
| **CR3 / S-C2** | Cross-plugin shadow attack | §1 G3 + §6.2 hard removal of `CredentialContextExt::credential<S>()` | Verbatim VETO trigger language cited (security-lead 03c §1) |
| **CR4 / S-J1** | JSON depth bomb (no cap) | §1 G3 + §6.1 depth cap 128 implementation | Apply sites: `stateless.rs:370`, `stateful.rs:561, 573`; typed `ValidationReason::DepthExceeded` |
| **CR5** | Credential CP6 vocabulary not adopted | §1 G1 + §2.1.1 + §3.1 | Adoption complete at §2-§3 |
| **CR6** | `ctx.credential_*` API fragmentation | §1 G1 + §6.2 + §10.2 T2 | Unified API per credential Tech Spec §3.4; codemod migration path |
| **CR7** | Canon §3.5 governance debt (ControlAction not seal-ratified) | §1 G4 + §2.6 + ADR-0038 + §16.5 canon §3.5 revision PR | Sealed-DX trait family + canon line 82 revision |
| **CR8** | `parameters = Type` macro emits non-existent `with_parameters()` | §1 G2 + §4.6.1 fix + §5.3 Probe 7 | Macro emits `with_schema(<T as HasSchema>::schema())` per `crates/action/src/metadata.rs:292` |
| **CR9** | Undocumented `Input: HasSchema` bound | §1 G2 + §2 deliberate-divergence #1 (lift bounds onto typed trait) | §2.2.1 / §2.2.2 / §2.2.4 lift `Input: HasSchema + DeserializeOwned` |
| **CR10** | `*Handler` HRTB pre-1.85 boilerplate | §1 G5 + §2.4 single-`'a` + `BoxFut<'a, T>` alias | rust-senior 02c §8 line 439 ~30-40% LOC reduction |
| **CR11** | Three independent agents repeating same emission bug | §1 G2 + §5 production harness + §5.5 macrotest snapshots | Regression coverage makes the bug class structurally impossible |

**CC1 (carry-forward from Phase 1 dx-tester).** `semver` consumer-side dep declaration → §10.4 step 1.5 closure (per CP3 iteration 2026-04-24 dx-tester 10d R2).

### §14.5 Phase 0 evidence

[`01b-workspace-audit.md`](../drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md) audit findings cited at line-number granularity (workspace / CI / tooling tier of Phase 0; the source-tier is [`01-current-state.md`](../drafts/2026-04-24-nebula-action-redesign/01-current-state.md), cited only where source-side findings ground the row):

- **`01b-workspace-audit.md` §1 line 44** — `zeroize` workspace-pin drift → §13.4.1 T4 cascade-scope absorb
- **`01-current-state.md` §2 finding C1** — derives structurally cannot do field-type rewriting → Strategy §3 A' selection (informs §1 G1)
- **`01b-workspace-audit.md` §9 line 252-329** — reverse-deps inventory verbatim → §10.1 reverse-deps table
- **`01b-workspace-audit.md` §10 line 346-356** — blast-radius weight by consumer → §10.3 per-consumer step counts
- **`01b-workspace-audit.md` §11 line 376** — `lefthook.yml` doctests/msrv/doc parity gap → §13.4.2 T5 out-of-cascade-scope
- **`01b-workspace-audit.md` §11 line 379 row 9** — `nebula-action` runtime-layer positive ban absent → §13.4.3 T9 cascade-scope absorb (Edit 2 — symmetric with engine/sandbox/storage/sdk/plugin-sdk rules)
- **Phase 0 T1 finding** — `crates/action/macros/Cargo.toml` lacks `[dev-dependencies]` block → §5.1 macro test harness landing

**T-disposition table** (per CP3 §13.4.5 final form):

| T | Phase 0 origin | Disposition | Lands at |
|---|---|---|---|
| T1 (`zeroize` workspace=true) | §1 line 44 | Cascade-scope absorb | `crates/action/Cargo.toml` edit alongside cascade PR |
| T4 = §13.4.1 above (different Phase 0 source) | §1 line 44 | (same — clarification: T4 codemod transform vs T-disposition; see §10.2) | — |
| T5 (`lefthook.yml` parity) | §11 line 376 | Out of cascade scope | Separate housekeeping PR (devops-owned; sunset ≤2 release cycles per `feedback_lefthook_mirrors_ci.md`) |
| T9 (`deny.toml` layer-enforcement) | §11 line 379 row 9 | Cascade-scope absorb | `deny.toml` edits per §13.4.3 (wrappers-list extension + NEW positive ban) |
| `nebula-redact` workspace integration | §6.3.2 helper crate decision | Cascade-scope absorb (preliminary) | Atomic with cascade PR per §13.4.4 |

**Hygiene-T vs codemod-T naming caveat.** §13.4 enumerates hygiene items (T4/T5/T9 from CP1 09e devops review); §10.2 enumerates codemod transforms (T1-T6). The two T-namespaces overlap by accident only — T4 in §10.2 (HRTB collapse codemod transform) is unrelated to T4 in §13.4.1 (`zeroize` workspace-pin hygiene). CP4 explicitly disambiguates: §10.2 transforms are `T1`-`T6`; §13.4 hygiene items are `T4 (zeroize)` / `T5 (lefthook)` / `T9 (deny.toml)`.

---

## §15 Open items resolution

This section walks every open item raised across CP1-CP3 to decided / deferred-with-trigger status. Carry-forward items get explicit closure rows; one cross-crate amendment is enacted in this CP (§15.5 ADR-0037 §1 SlotBinding shape per §0.2 invariant 2). Two soft amendments to credential Tech Spec are flagged (NOT enacted) per ADR-0035 amended-in-place precedent — credential Tech Spec author lands the inline edit during cross-section pass.

### §15.1 Strategy + CP1-CP3 open-items walkthrough

**Strategy §5 carry-forward (Strategy line 469-479):**

| Open item | Status | Closure point |
|---|---|---|
| §4.3.2 — `unstable-retry-scheduler` wire vs gated-stub | **CLOSED** | §1 G6 + §2.7.1 (wire-end-to-end picked) |
| §4.3.3 — codemod transform list locked at scope; Tech Spec design without re-opening Strategy | **CLOSED** | §10.2 T1-T6 (T6 added at design level per Strategy §4.3.3 line 243 license) |
| §5.1.1 — ActionContext API location in credential Tech Spec | **DEFERRED-WITH-TRIGGER** | §15.8 cross-section coordination row; landing point is credential Tech Spec author edit, not action-side |
| §5.1.2 — `redacted_display()` hosting crate decision | **CLOSED** | §6.3.2 NEW dedicated `nebula-redact` crate; §13.4.4 workspace integration |
| §5.1.4 — B'+ activation criteria detail | **CLOSED at Strategy §6.8** | §16.4 rollback strategy cross-ref |
| §5.1.5 — cluster-mode hooks final trait shape | **DEFERRED-WITH-TRIGGER** | §15.8 — TriggerAction cluster-mode hooks final trait shape per Strategy §5.1.5; deferred to Tech Spec §7 (which CP3 closed at §2.2.3 with `IdempotencyKey`/`on_leader_*`/`dedup_window` hook surface; full trait shape is engine-cascade scope per §1.2 N4) |
| §5.2 — spike DONE criteria | **CLOSED** | Spike PASS commit `c8aef6a0` per §0.1 inputs frozen-at footer |
| §2.11 amendment-pending — `feedback_idiom_currency.md` + `feedback_observability_as_completion.md` explicit citation roll-up | **CLOSED** | §1 G3 invokes `feedback_observability_as_completion.md`; §1 G5 invokes `feedback_idiom_currency.md`; both load-bear in this Tech Spec at §1 lock |
| CP3 §6 inventory bookkeeping (~8 forward-promises) | **CLOSED at Strategy §6 frozen CP3** | All eight sub-promises mapped at Strategy frozen CP3 line 525 |

**CP1 §15 carry-forward (Tech Spec line 2106-2125):**

| Open item | Status | Closure point |
|---|---|---|
| §1.2 / N5 — paths a/b/c framing | **CLOSED at §16.1** | This CP |
| §2.2.3 — TriggerAction cluster-mode hooks final trait shape | **DEFERRED-WITH-TRIGGER** | §15.8 — engine cluster-mode coordination cascade (§1.2 N4) |
| §2.6 / §9.2 — DX trait blanket-impl trait-by-trait audit | **DEFERRED-WITH-TRIGGER** | §15.8 — exact `#[action(...)]` attribute zone spelling for each DX trait; CP4 housekeeping |
| §3.1 — engine `ActionRegistry::register*` call-site exact line range + final host-crate path | **DEFERRED-WITH-TRIGGER** | §15.8 — engine cascade handoff; `crates/runtime/` does not exist (Phase 1 audit row 4) — current host is `crates/engine/src/runtime/registry.rs` (CP1 §3.1 cited `crates/engine/src/registry.rs`; the file lives under the `runtime/` submodule per `Glob crates/engine/src/registry*` returning no top-level match), exact line range CP4 §15.8 row |
| §3.2 — ActionContext API location in credential Tech Spec | **DEFERRED-WITH-TRIGGER** | §15.8 — coordination with credential Tech Spec author |

**CP2 §15 carry-forward (Tech Spec line 2157-2196):**

| Open item | Status | Closure point |
|---|---|---|
| §4.4-1 — `ActionSlots` trait sealing | **CLOSED CP3 §9.4** | leave `pub` (NOT sealed); dual enforcement layer makes hand-impl observable |
| §4.7-1 — codemod auto-rewrite vs manual-marker for `credential = "key"` | **CLOSED CP3 §10.2 T2** | MIXED: AUTO for explicit type annotation; MANUAL marker otherwise |
| §5.1-1 — `cargo-public-api` snapshot for macro crate | **DEFERRED-WITH-TRIGGER** | §15.8 — out of cascade scope per ADR-0037 §4; future macro-evolution housekeeping |
| §5.3-1 — `nebula-engine` as dev-dep on `nebula-action-macros` | **CLOSED CP2 iteration 2026-04-24** | committed; `deny.toml` wrappers amendment landed CP3 §13.4.3 |
| §5.4.1 — soft amendment к credential Tech Spec §16.1.1 probe #7 | **FLAGGED NOT ENACTED — see §15.3** | Soft amendment to credential Tech Spec; this CP records, does not enact |
| §6.1.2 — byte-pre-scan vs `Value`-walking primitive | **CLOSED CP2 §6.1.2** | byte-pre-scan path (existing `check_json_depth` primitive); rust-senior CP2 review confirmed acceptable |
| §6.2-1 — `credential_typed::<S>(key)` retention vs removal | **CLOSED CP3 §9.3.1** | REMOVE alongside `credential<S>()`; explicit-key form via `ctx.resolved_scheme(&CredentialRef::from_key(key))` |
| §6.3-1 — full `redacted_display()` rule set | **CLOSED CP3 §9 design + CP2 §6.3.1-A wrap-form** | Pre-`format!` sanitization wrap-form is the single rule; substring patterns deferred to `nebula-redact` crate's internal evolution post-cascade |
| §6.4-1 — `tokio::time::pause()` vs real-clock 10ms | **CLOSED CP3 §9** | recommendation `tokio::time::pause()` for deterministic cancellation timing |
| §6.4 cross-crate amendment к credential Tech Spec §15.7 | **FLAGGED NOT ENACTED — see §15.4** | Soft amendment; cross-section pass surfaces |
| §6.5 — cross-tenant `Terminate` boundary | **CLOSED CP3 §9.5** | engine-side enforcement contract; verbatim "MUST NOT propagate" invariant; security-lead implementation-time VETO retained |
| §7.3-1 — `ResolveError::NotFound` mapping to `ActionError` taxonomy | **DEFERRED-WITH-TRIGGER** | §15.8 — security-neutral; can land at implementation time without re-opening Tech Spec |
| §7.1 step 3 / §3.2-1 — `ResolvedSlot` engine-side wrap point | **PARTIAL CLOSURE CP3 §11.3.1 + §3.2 step 5** | Adapter responsibility: engine wraps after `resolve_fn` returns; explicit wrap-point in engine code is engine-cascade scope |
| §2.9-1 — `ActionMetadata::for_trigger::<A>()` helper | **CLOSED — see §15.6** | Closes carry-forward; no helper builder added |

**CP3 §15 carry-forward (Tech Spec line 2234-2264):**

| Open item | Status | Closure point |
|---|---|---|
| §9.3-1 — `nebula-sdk::prelude` re-export of `redacted_display` for community plugin authors | **CLOSED — see §15.8** | NO; community plugins depend on `nebula-redact` directly to preserve single audit point |
| §11.3-1 — adapter performance microbenchmark | **DEFERRED-WITH-TRIGGER** | §15.8 — CP4 / implementation-time housekeeping; not blocking |
| §12 `#[action(control_flow)]` attribute zone syntax — exact spelling | **DEFERRED-WITH-TRIGGER** | §15.8 — flag form is consistent CP3-CP4 placeholder; canonical spelling lands via §9.2 trait-by-trait audit closure |
| Forward-track items (a)-(j) at CP3 CHANGELOG (current line 2598-2607) | Each enumerated below | §15.7 / §15.8 |

### §15.2 Q2 ActionResult::Terminate decision recorded (cascade-scope-final)

**Decision (recorded CP1 §2.7.1 — re-affirmed at CP4):** wire-end-to-end. Both `ActionResult::Retry` (existing `unstable-retry-scheduler`) and `ActionResult::Terminate` (new `unstable-terminate-scheduler`) graduate from gated-with-stub to wired-end-to-end in cascade scope. Parallel feature flags (committed CP1 §2.7.2 line 438) — each flag gates one variant; the two consume distinct scheduler subsystems (re-enqueue vs sibling-branch-cancel + termination-audit). Per §0.2 invariant 4: this Tech Spec freezes the parallel-flag signature; CP3 §9 may amend the *internal scheduler implementation* but cannot rename or unify the public flags without an ADR amendment.

**Cross-tenant Terminate** — §9.5 engine-side enforcement contract; verbatim invariant "engine MUST NOT propagate `Terminate` across tenant boundaries" (security 08c §Gap 5 line 109-111); silent skip with `tracing::warn!` + counter on cross-tenant block; structural errors are Fatal. Security-lead implementation-time VETO retained on §9.5.1 invariant language.

### §15.3 Cross-crate soft amendment к credential Tech Spec §16.1.1 probe #7 — FLAGGED, NOT ENACTED

Per §5.4.1 — credential Tech Spec §16.1.1 probe #7 (line 3756) currently specifies the unqualified `let g2 = guard.clone()` form which is **silent-pass** per spike finding #1 (auto-deref Clone shadow on `SchemeGuard<'a, C>`).

**Amendment candidate (form):**

> | 7 | `tests/compile_fail_scheme_guard_clone.rs` | `<SchemeGuard<'_, C> as Clone>::clone(&guard)` qualified-syntax form on `SchemeGuard` | `E0277` — `Clone` bound not satisfied (subsumes naive `E0599` because qualified form bypasses auto-deref) |

**Status: FLAGGED, NOT ENACTED in this Tech Spec.** Per ADR-0035 amended-in-place precedent (ADR-0035 §Status block records iter-2-B and iter-3-C amendments), cross-crate amendments to credential Tech Spec are coordinated by the credential Tech Spec author. This Tech Spec ratification (CP4 freeze) records the amendment as outstanding cross-cascade item; the amendment lands as credential Tech Spec inline edit ("*Amended by Tech Spec [`2026-04-24-nebula-action-tech-spec.md`](2026-04-24-nebula-action-tech-spec.md) §15.3, 2026-04-25*" prefix at the §16.1.1 probe #7 row, plus updated diagnostic column) + CHANGELOG entry, not via a new ADR.

**Until amendment lands, the production credential probe at `crates/credential/tests/compile_fail_scheme_guard_clone.rs` would use the unqualified form** (silent-pass risk). The action-side probe (§5.4) catches the violation independently — so this gap is not action-cascade-blocking, only credential-side-soft-degradation.

**Coordination owner.** Credential Tech Spec author. **Trigger:** CP4 cross-section pass (this CP). **Sunset window:** ≤1 release cycle (per ADR-0035 amendment-in-place tempo precedent).

### §15.4 Cross-crate soft amendment к credential Tech Spec §15.7 — FLAGGED, NOT ENACTED

Per §6.4.2 — credential Tech Spec §15.7 currently does not include the `engine_construct_with_probe` test-only constructor variant on `SchemeGuard<'a, C>`. CP2 §6.4.2 commits per-test `ZeroizeProbe: Arc<AtomicUsize>` instrumentation as the cancellation-zeroize test contract (closes 08c §Gap 4); this requires the test-only constructor.

**Amendment candidate (form):**

```rust
// In nebula-credential's test surface (cfg(any(test, feature = "test-helpers"))):
impl<C: Credential> SchemeGuard<'a, C> {
    #[cfg(any(test, feature = "test-helpers"))]
    pub fn engine_construct_with_probe(
        scheme: C::Scheme,
        ctx: &'a CredentialContext<'a>,
        probe: Arc<AtomicUsize>,
    ) -> Self { ... }  // bumps `probe` on Drop instead of (or in addition to) global

    // Production constructor unchanged.
    pub(crate) fn engine_construct(scheme: C::Scheme, ctx: &'a CredentialContext<'a>) -> Self { ... }
}
```

**Status: FLAGGED, NOT ENACTED in this Tech Spec.** Same precedent as §15.3 — credential Tech Spec author lands the inline edit per ADR-0035 amended-in-place tempo. Until amendment lands, the cancellation-zeroize test (`crates/action/tests/cancellation_zeroize.rs`) cannot construct `SchemeGuard` with a per-test probe, so the test must be gated `#[cfg(feature = "test-helpers")]` until the credential-side feature lands.

**Coordination owner.** Credential Tech Spec author. **Trigger:** CP4 cross-section pass (this CP). **Sunset window:** ≤1 release cycle, atomic with §15.3 if practical (both surface in same cross-section pass).

### §15.5 ADR-0037 §1 SlotBinding shape amendment-in-place — ENACTED

Per CP2 line 2179 ("ADR-0037 §1 SlotBinding shape divergence — amendment-in-place trigger"): ADR-0037 §1 currently shows `SlotBinding { key, slot_type, capability, resolve_fn }` with separate `capability` field; this Tech Spec §3.1 (lines 606-633) folds capability into the `SlotType` enum per credential Tech Spec §9.4 line 2452 three-variant matching pipeline (`Concrete { type_id }`, `ServiceCapability { capability, service }`, `CapabilityOnly { capability }`). The pre-amendment shape is a **§0.2 invariant 2 trigger** if not landed before Tech Spec ratification.

**Supersession acknowledgement (CP4 iterated 2026-04-25).** Credential Tech Spec §9.4 was superseded by §15.8 in credential Tech Spec CP5 (2026-04-24). The §9.4 supersede blocks at credential Tech Spec lines 2376 and 2446 redirect readers to §15.8 (line 3520-3528) as the canonical shape. The amendment justification is **shape-preserving across the supersession**: §15.8 explicitly preserves the `SlotType::Concrete / ServiceCapability / CapabilityOnly` matching axes verbatim (credential Tech Spec line 3522 "Same `SlotType::Concrete / ServiceCapability / CapabilityOnly` matching axes"); the supersession shifts only the **filter authority source** (plugin-metadata `capabilities_enabled` → registry-computed `RegistryEntry::capabilities` at `register<C>` time). Capability-authority-source orthogonality means the §15.5 SlotBinding amendment-in-place is substantively correct under both pre-CP5 and post-CP5 forms. CP4 §14.3 row 7 re-pins the citation to §15.8 (CP5 supersession of §9.4); §3.1 SlotType doc comment likewise re-pins.

#### §15.5.1 Enactment

This CP **enacts** the ADR-0037 §1 amendment-in-place per ADR-0035 precedent. The enactment is recorded as a separate edit on `docs/adr/0037-action-macro-emission.md` co-landing with this CP4 draft:

- **§1 SlotBinding shape** rewritten from `SlotBinding { key, slot_type, capability, resolve_fn }` (separate `capability` field) to `SlotBinding { field_name, slot_type, resolve_fn }` (capability folded into `SlotType` enum per credential Tech Spec §9.4 three-variant matching pipeline → §15.8 CP5 supersession preserves the same matching axes; capability authority source shifts plugin-metadata → `RegistryEntry::capabilities` registry-computed).
- **Status header** changed from `proposed` to `proposed (amended-in-place 2026-04-25)` per ADR-0035 precedent block style.
- **CHANGELOG entry added** at ADR top: "Amended-in-place 2026-04-25 per Tech Spec CP4 §15.5 to fold capability into `SlotType` enum, aligning with credential Tech Spec §9.4 line 2452 authoritative three-variant matching pipeline. Per ADR-0035 amended-in-place precedent."
- **Field name reconciliation.** Pre-amendment `key: "slack"` (string-typed) is reconciled to `field_name: "slack"` (`&'static str`-typed) — matches Tech Spec §3.1 line 608 + spike `final_shape_v2.rs:43-55`. The runtime semantic is unchanged (the `field_name` identifies the action struct field by name); the rename clarifies that this is the *Rust field name*, not a credential `SlotKey`.
- **Cross-section consistency.** ADR-0037 §3 Auto-deref Clone shadow probe section references `SlotBinding` shape only in the body — qualified-syntax probe form is preserved. ADR-0037 §4 Macro test harness section references the 6-probe table — `SlotBinding` shape not load-bearing for individual probe rows. ADR-0037 §5 Emission perf bound section refers to `LOC of macro-emitted region` — `SlotBinding` shape not load-bearing for naive-vs-adjusted ratio.

**Why amend-in-place vs supersede.** Per ADR-0035 §Status block: "Post iter-2 amendments applied (canonical-form corrections, not stylistic)" — same shape-correction discipline applies here. The `capability` field divergence is a structural inconsistency (cross-crate authoritative source wins), not a paradigm shift; supersede would be disproportionate per `feedback_adr_revisable.md` precedent for ADR-0035 itself.

**Status invariant.** Per §0.2 invariant 2: ADR-0037 amendment is recorded as `proposed (amended-in-place 2026-04-25)`. This Tech Spec ratification (CP4 freeze) is conditional on ADR-0037 amendment landing — verified in §16.5 cascade-final precondition. ADR-0037 status moves to `accepted` upon Tech Spec ratification per §0.1 line 35.

### §15.6 §2.9-1 forward-track — closure (CP2 carry-forward)

**Decision:** keep universal `with_schema` builder pattern; no `ActionMetadata::for_trigger::<A>()` helper added. Per §2.9.5 / §2.9.6 — Configuration vs Runtime Input axis distinction makes the universal pattern correct; trigger-shape-specific helper would over-specialize. Configuration lives in `&self` struct fields per §4.2 ("Fields outside the zones pass through unchanged") + schema declared via `ActionMetadata::parameters` per `crates/action/src/metadata.rs:292`. Closes CP2 §15 carry-forward.

### §15.7 CP1 hygiene T-disposition ratification

| Item | Disposition (CP3 §13.4.5) | CP4 ratification |
|---|---|---|
| **T4** `zeroize` workspace=true pin | Cascade-scope absorb | **RATIFIED** — lands in cascade-landing PR per §13.4.1 |
| **T5** `lefthook.yml` parity | Out of cascade scope | **RATIFIED** — separate housekeeping PR (devops-owned); sunset ≤2 release cycles per `feedback_lefthook_mirrors_ci.md`; CP4 §16 sunset-tracked |
| **T9** `deny.toml` layer-enforcement | Cascade-scope absorb | **RATIFIED** — wrappers-list extension + NEW positive ban per §13.4.3 |
| **`nebula-redact`** workspace integration | Cascade-scope absorb (preliminary) | **RATIFIED** — atomic with cascade PR per §13.4.4 |

Disposition table is final at CP4 freeze; no further T-item movement post-freeze per §0.2 invariant 1.

### §15.8 Remaining minor open items walkthrough (deferred-with-trigger registry)

Each item below has a **trigger** (when it surfaces for resolution), an **owner** (who resolves), and a **scope** (action-cascade-internal vs cross-cascade vs implementation-time). None block this Tech Spec freeze.

| Open item | Trigger | Owner | Scope |
|---|---|---|---|
| **(a)** §1.2 N5 paths a/b/c framing | Phase 8 cascade summary | architect (frames) → user picks | Cascade-final |
| **(b)** §2.2.3 TriggerAction cluster-mode hooks final trait shape | Engine cluster-mode coordination cascade activation | engine cascade architect | Out-of-this-cascade per §1.2 N4 |
| **(c)** §2.6 / §9.2 DX trait blanket-impl trait-by-trait audit | Implementation-time finalization | rust-senior | Cascade-internal housekeeping |
| **(d)** §3.1 engine `ActionRegistry::register*` call-site exact line range + final host-crate path | Engine cascade handoff | engine cascade architect | Out-of-this-cascade |
| **(e)** §3.2 ActionContext API location in credential Tech Spec | Cross-section pass with credential Tech Spec author | architect + credential Tech Spec author | Cross-cascade |
| **(f)** §15.3 + §15.4 cross-crate soft amendments | CP4 cross-section pass (this CP) | credential Tech Spec author | Cross-cascade |
| **(g)** ADR-0037 §1 SlotBinding shape amendment-in-place | ENACTED §15.5 (this CP) | architect | Action-cascade-internal — **ENACTED** |
| **(h)** §10 codemod implementation host crate (`tools/codemod/` placeholder) | Implementation-time | devops + architect | Cascade-final housekeeping |
| **(i)** §13.4.2 T5 lefthook parity | Sunset ≤2 release cycles | devops | Out-of-cascade-scope |
| **(j)** §11.3-1 adapter perf microbenchmark + §13.3 crate publication policy ADR-0021 cross-ref | Implementation-time | rust-senior + architect | Cascade-final housekeeping |
| **§5.1-1** `cargo-public-api` snapshot for macro crate | Future macro-evolution housekeeping | rust-senior | Out-of-cascade-scope |
| **§7.3-1** `ResolveError::NotFound` → `ActionError` taxonomy mapping | Implementation-time | rust-senior | Cascade-final housekeeping (security-neutral) |
| **§9.3-1** `nebula-sdk::prelude` re-export of `redacted_display` | Closed: NO — community plugins depend on `nebula-redact` directly | architect | Single-audit-point preserved |
| **§12 `#[action(control_flow)]` attribute syntax — exact spelling** | §9.2 trait-by-trait audit closure | rust-senior | Cascade-final housekeeping; flag form is CP3-CP4 placeholder |

**Per `feedback_active_dev_mode.md` discipline:** every deferred-with-trigger row above has a named trigger + owner + scope. No silent deferral. Per the same discipline, none of these items is implementation-blocking — Phase 8 cascade summary surfaces them as the residual ledger after cascade close.

---

## §16 Implementation handoff

This section frames the post-cascade-freeze handoff. CP4 records the structure; the user picks Q1 path (a)/(b)/(c) at Phase 8 cascade summary per Strategy §6.5 (line 408-413) — Tech Spec presents, does NOT pre-pick.

**Phase numbering anchor.** Phase references in §16 (Phase 6 ratification / Phase 7 cross-section / Phase 8 cascade summary) live in cascade-orchestrator territory, defined in [Strategy §6](../specs/2026-04-24-action-redesign-strategy.md). This Tech Spec uses them as named hand-off points without redefinition.

### §16.1 PR wave plan — Q1 implementation path options

Per Strategy §4.2 (line 198-206) + §6.5 (line 408-413), three implementation paths frame the cascade-final user pick. CP4 §16.1 extends the Strategy table with concrete cascade-final criteria:

| Path | Shape | Best when | Cascade-final criteria |
|---|---|---|---|
| **(a) Single coordinated PR** | One PR landing both crates' CP6 vocabulary + engine wiring + plugin migration in lockstep. ~18-22 agent-days full impl per architect 03a §1; 8-12d per tech-lead round 1 (gap reflects whether codemod + plugin migration are counted). | Credential cascade owner + action-crate owner have concurrent bandwidth committed; reviewer headcount available for one large landing | Reviewer load: 27+ engine import sites + 7 reverse-deps in one diff; ratification depends on tech-lead capacity to absorb single-PR review surface |
| **(b) Sibling cascades — credential leaf-first; action consumer-second in lockstep** | Credential CP6 implementation cascade lands `CredentialRef<C>` / `SlotBinding` / `SchemeGuard` / `SchemeFactory` / `RefreshDispatcher` first. Action consumer cascade adopts CP6 vocabulary second, lockstep. Each fits normal autonomous budget. | Owner bandwidth permits leaf-first; cascade sequencing tolerable; reviewer load amortized over two diffs | Sequencing friction during the gap (CP6 surface lands before action consumer; engine sees CP6-shaped credential surface without action consumer); tech-lead round 2 preference if credential-crate owner has bandwidth |
| **(c) Phased B'+ surface commitment — NOT VIABLE WITHOUT credential CP6 cascade slot** | Action ships CP6 API surface (the user-visible types) with delegating internals while credential cascade lands CP6 internals; plugin authors do not re-migrate. Action's `resolve_as_<capability><C>` thunk is the only action-side bridge. | Credential cascade owner cannot start CP6 implementation immediately but slot is committed; plugin authors need CP6 surface immediately for downstream work | **VIABILITY GATE — per Strategy §6.6 (line 416-426) silent-degradation guard:** committed credential CP6 cascade slot (named owner + scheduled date + queue position in `docs/tracking/cascade-queue.md` — or equivalent location, orchestrator picks at Phase 8 per Strategy §6.6 last paragraph) MUST exist before activation. Absent slot ⇒ path (c) NOT VIABLE; user pick narrows to (a) or (b). Architect+tech-lead co-decision required per Strategy §6.8 (line 459) — orchestrator does NOT silently activate |

**(c) viability gate cross-ref.** Path (c) requires B'+ activation; B'+ activation is not user-pickable in isolation per Strategy §6.8 co-decision rule. The cascade-final readiness check at §16.5 confirms slot status before path (c) is offered as user choice.

**This Tech Spec presents the table; user picks at Phase 8 cascade summary.** Per §1.2 N5 — Tech Spec does NOT pre-pick.

### §16.2 Codemod ship plan

Per CP3 §10.2 T1-T6 — codemod design lands as cascade artefact:

- **Deliverable form.** `cargo`-style binary `nebula-action-codemod` (host: `tools/codemod/` placeholder per §10.2.1; exact crate name + binary location is §15.8 row (h)). AUTO mode default for T1/T3/T4; MANUAL-REVIEW marker for T2/T5; MIXED for T6 per ADR-0038 §Negative item 4.
- **Idempotent re-run.** Codemod re-runs are no-ops if pattern already migrated (per §10.2.1).
- **Automated step counts per consumer.** Per CP3 §10.3 table:
  - `nebula-engine` — 27+ import sites; ~10 T4 + ~5 T2 + ~5-7 T5 sites; closer to 50/50 AUTO/MANUAL ratio
  - `nebula-api` — ~4 sites total; 1 T2 + 2 T4 + 1 T5
  - `nebula-sandbox` — 7 files; ~3 T3 + ~4 T4 + ~2 T5; dyn-handler ABI moderate risk
  - `nebula-sdk` — re-export-only; covered by §9.3 reshuffle, not codemod transforms
  - `nebula-plugin` — ~1-2 T1 sites; trivial
  - `apps/cli` — ~3 T1 + ~1-2 T2 + ~1 T5
- **Aggregate.** ~55 file edits across 6 crates + 1 app per [`01b-workspace-audit.md`](../drafts/2026-04-24-nebula-action-redesign/01b-workspace-audit.md) §10 line 358; ~70/30 AUTO/MANUAL workspace-aggregate per §10.5.
- **`MIGRATION.md` artefact.** Ships in `crates/action/` alongside cascade-landing PR per §10.4 last paragraph; documents steps 1-7 with worked examples per `feedback_active_dev_mode.md` ("DoD includes migration guide for breaking changes").

### §16.3 Definition of done (PR-wave-level)

Per `feedback_active_dev_mode.md` ("Active dev ≠ prod release; never settle for green tests / cosmetic / quick win / deferred"), the cascade-landing PR is DONE when ALL of the following land in cascade scope:

1. **All 11 🔴 from Phase 1 02-pain-enumeration §4 closed in code.** CR1-CR11 verifiable per §14.4 closure traceability table.
2. **Security must-have floor (4 items) verified via tests.**
   - **CR4 / S-J1** JSON depth bomb fix — depth cap 128 at every adapter JSON boundary per §6.1; typed `ValidationReason::DepthExceeded { observed, cap }` per §6.1.3
   - **CR3 / S-C2** cross-plugin shadow attack fix — hard removal of `CredentialContextExt::credential<S>()` per §6.2 (NOT `#[deprecated]` shim — `feedback_no_shims.md` + security-lead 03c §1 VETO retained)
   - **`ActionError` Display sanitization** — `redacted_display(&e)` wrap at every error-emit site per §6.3
   - **Cancellation-zeroize test** — three sub-tests per §6.4.1; per-test `Arc<AtomicUsize>` probe per §6.4.2
3. **Macro test harness landed.** `crates/action/macros/tests/` per §5.2; six probes ported from spike commit `c8aef6a0` + Probe 7 (`parameters = Type` no-`HasSchema` rejection) per §5.3; macrotest expansion snapshots per §5.5.
4. **Sealed DX adapter pattern landed.** Five sealed DX traits (`ControlAction` / `PaginatedAction` / `BatchAction` / `WebhookAction` / `PollAction`) per §2.6 + §9.2; sealed_dx adapter pattern per §11.2 + §12.1; canon §3.5 line 82 revision PR co-lands per §16.5.
5. **7 reverse-deps migrated.** Per §10.3 + §10.5 — engine + api + sandbox + sdk + plugin + cli + macros sibling crate via codemod + manual review.
6. **`nebula-redact` workspace integration landed.** Per §13.4.4 — four atomic edits (new `crates/redact/Cargo.toml` + `src/lib.rs`; root `Cargo.toml [workspace] members` + `[workspace.dependencies]`; no new `deny.toml` ban — leaf utility). Atomic with cascade PR.
7. **`deny.toml` positive ban for `nebula-action` runtime layer landed.** Per §13.4.3 — wrappers-list extension to existing `nebula-engine` rule + NEW positive ban for `nebula-action` runtime layer (Edit 1 + Edit 2). Symmetric with engine/sandbox/storage/sdk/plugin-sdk rules.

**Plus implicit:** ADR-0036 / ADR-0037 / ADR-0038 status moves from `proposed` to `accepted` upon Tech Spec ratification (per §0.1 line 35); ADR-0037 has `proposed (amended-in-place 2026-04-25)` qualifier per §15.5 enactment.

### §16.4 Rollback strategy if soak period reveals issues

Two distinct rollback layers, applied per failure mode:

**Layer 1 — Feature-flag gate path (symmetric `unstable-retry-scheduler` + `unstable-terminate-scheduler`).** If post-cascade soak surfaces issues with `Retry` or `Terminate` end-to-end wiring (e.g., scheduler-integration hook bug, cross-tenant boundary violation slipping through), feature-flag-gate the variant in question via the existing `unstable-retry-scheduler` / `unstable-terminate-scheduler` flags. Per §2.7.1 wire-end-to-end commitment — flags exist precisely for this rollback shape; gating one or both does NOT require Tech Spec amendment, only PRODUCT_CANON §11.2 status revert. Variant signatures remain frozen per §0.2 invariant 4.

**Layer 2 — Reverse-codemod for transform regressions.** If post-cascade soak reveals codemod transforms (T1-T6) introduced regressions in reverse-deps, the codemod ships a reverse mode (`nebula-action-codemod --reverse`) that inverts AUTO transforms (T1, T3, T4, T6 trivial-pass-through case) per their unidirectional shape. MANUAL-REVIEW transforms (T2, T5, T6 edge-case markers) cannot be reverse-codemodded — manual revert required. Reverse-codemod is a §15.8 row (h) housekeeping commitment landing alongside the codemod itself; not a separate cascade.

**Strategy §6.8 B'+ contingency activation** — if post-cascade soak reveals A' is structurally untenable (e.g., HRTB shape stops compiling against future Rust release; macro emission contract requires fundamental rework), B'+ activation is the architect+tech-lead co-decision rollback path. Strategy §6.8 (line 443-461) records the criteria and rollback shape; this is an out-of-scope-for-Tech-Spec recovery layer (Strategy-level reversal).

### §16.5 Pre-implementation checklist (cascade-final)

Per Strategy §6.5 (line 406-413) + §6.6 (line 416-426) + §6.8 (line 443-461) + this Tech Spec §0.1 + §15.5 + §15.8 — the cascade-final readiness check before Phase 8 user pick:

- [ ] **Tech Spec ratified.** This Tech Spec FROZEN CP4 per §0.1 status table (status moves to `FROZEN CP4 2026-04-25` after CP4 review + iterate cycle).
- [ ] **ADR-0036 / ADR-0037 / ADR-0038 status moved to `accepted`** per §0.1 line 35 (status moves at Tech Spec ratification).
- [ ] **ADR-0037 §1 SlotBinding shape amendment-in-place landed.** Per §15.5.1 enactment — verified by `grep`-able anchor at ADR-0037 §Status block ("amended-in-place 2026-04-25") + §1 SlotBinding shape matching this Tech Spec §3.1 line 606-633.
- [ ] **Canon §3.5 revision PR ratified.** Per ADR-0038 §2 — canon §3.5 line 82 revises to enumerate the DX tier explicitly. This is a separate PR from cascade-landing; precondition for ADR-0038 status moving to `accepted`.
- [ ] **Credential CP6 cascade slot status confirmed.** Per Strategy §6.6 (line 416-426) — slot row in [`docs/tracking/cascade-queue.md`](../../tracking/cascade-queue.md) (or equivalent location — orchestrator picks at Phase 8 per Strategy §6.6 last paragraph; the file does not exist on disk at draft time per audit verification) with three required fields: named owner + scheduled date + queue position. **Required if user picks path (b) or (c).** Not required for path (a).
- [ ] **§15.3 + §15.4 cross-crate soft amendments к credential Tech Spec coordinated.** Per cross-section pass (this CP); credential Tech Spec author lands inline edits per ADR-0035 amended-in-place tempo.
- [ ] **Phase 1 register state verified — no 🔴 unresolved.** Per Strategy §6.4 (line 396-402) — concerns register lifecycle activated only if Phase 1 surfaced unresolved 🔴; absence confirmed.

**User pick at Phase 8 cascade summary** — orchestrator surfaces the (a)/(b)/(c) table from §16.1 with each row's cascade-final criteria status (e.g., "(c) NOT VIABLE: credential CP6 cascade slot uncommitted in cascade-queue.md"). Per Strategy §4.2 line 206 + §6.5 line 408-413: Strategy does not pre-pick; orchestrator does not pre-pick; user picks.

---

### Open items raised this checkpoint (CP1)

- §1.2 / N5 — paths a/b/c implementation pick framing in CP4 §16; user picks at Phase 8 (Strategy §4.2 line 198-206 + §6.5 line 408-413). Track for CP4.
- §2.2.3 — TriggerAction cluster-mode hooks final trait shape (Strategy §5.1.5 line 297) — CP3 §7 scope.
- §2.2.4 — Resource-side scope (full `Resource::on_credential_refresh` integration) is N1 / OUT (Strategy §3.4 line 173); confirm boundary at CP4 cross-section pass.
- §2.6 — DX trait blanket-impl trait-by-trait audit (which primary each DX wraps; ADR-0038 §Implementation notes "trait-by-trait audit at Tech Spec §7 design time") — CP3 §7 scope.
- §2.7-1 — **RESOLVED at CP1 iteration (2026-04-24)** — feature-flag granularity committed to parallel flags `unstable-retry-scheduler` + `unstable-terminate-scheduler` per Strategy §4.3.2 symmetric-gating; CP3 §9 may amend internal scheduler implementation but not flag names without ADR amendment.
- §2.7-2 — engine scheduler-integration hook trait surface (`Retry` + `Terminate` dispatch path into the scheduler module) — CP3 §9 scope.
- §2.8 — `redacted_display()` helper crate location (Strategy §5.1.2 open item, line 274-275) — CP2 §4 scope; deadline before CP2 §4 drafting.
- §2.8 / §3.4 — `SchemeGuard<'a, C>` non-Clone qualified-syntax probe + `ZeroizeProbe` test instrumentation (per-test atomic vs `serial_test::serial`) — CP2 §8 scope (security-lead 08c forward-track).
- §3.1 — engine `ActionRegistry::register*` call-site exact line range + final host-crate path (`nebula-engine` likely; `crates/runtime/` confirmed non-existent per Phase 1 audit) — CP3 §7 scope.
- §3.2-1 — `ResolvedSlot` wrap point (engine-side wrapper vs inside `resolve_fn`); spike NOTES §4 question 5 — CP3 §9 scope.
- §3.2 — ActionContext API location in credential Tech Spec (Strategy §5.1.1, line 268-270; deadline before CP3 §7 drafting) — coordination required between architect + credential Tech Spec author before CP3 unblocks.
- §3.4 — cancellation-zeroize test instrumentation choice (per-test probe vs `serial_test::serial`) — CP2 §8 scope.

**Forward-track for CP2 / CP3 (security-lead 08c + rust-senior 08b prep notes):**
- CP2 §4 — hard-removal mechanism for no-key `credential<S>()` (security-lead 08c §1 VETO already binding via G3 floor item 2; mechanism specifics deferred to CP2).
- CP2 §4 — JSON depth-cap mechanism choice (custom deserializer vs library; `serde_json` `Value::deserialize_depth_limited` candidate).
- CP3 §9 — cross-tenant Terminate boundary (security-lead 08c §2 — `Terminate` must not propagate across tenant isolation; engine-side scheduler enforcement detail).
- CP3 §7 — `BoxFut` vs `BoxFuture` single-home decision: confirm `nebula-action::BoxFut` is canonical OR hoist shared `nebula-core::BoxFuture` (rust-senior 08b 🟡).

### CHANGELOG — CP1

CP1 iteration 2026-04-24 (post 5-reviewer-matrix; spec-auditor REVISE / rust-senior RATIFY-WITH-NITS / security-lead ACCEPT-WITH-CONDITIONS / dx-tester REVISE / devops RATIFY-WITH-NITS):
- §2.0 — replaced unconditional "compile-checked against final_shape_v2.rs" with three deliberate-divergence overlays (HasSchema/ser-de bounds, State bound chain, ActionSlots `&self` receiver). Closes spec-auditor 🟠 HIGH "compile-check warrant false."
- §2.1 — corrected doc comment: `#[action]` macro emits a **concrete** `impl Action for X` per action (not a blanket — `Action::metadata` is non-trivial). Added `ActionMetadata` host-crate cite + CP3 §7 lock note. Closes dx-tester R2.
- §2.1.1 (new subsection) — `ActionSlots` companion trait defined with `&self` receiver per credential Tech Spec §3.4 line 851. Closes dx-tester R1 (blocking) + spec-auditor 🔴 BLOCKER on `credential_slots()` 3-way signature divergence.
- §2.2.1 / §2.2.2 / §2.2.4 — lifted `Input: DeserializeOwned` and `Output: Serialize` bounds onto the typed traits (uniform across Stateless/Stateful/Resource). Closes rust-senior 08b 🔴 "adapter ser/de bound asymmetry persists."
- §2.3 — added `BoxFut` crate-residence note + CP3 §7 single-home cross-ref. Closes rust-senior 08b 🟡 + tracks devops 08e.
- §2.6 — added `+ ActionSlots` to all five sealed-DX blanket impls per spike `final_shape_v2.rs:282`. Closes spec-auditor 🔴 BLOCKER on §2.6 / §2.1 supertrait-chain mismatch.
- §2.6 — added "Community plugin authoring path" paragraph naming the migration target (`StatelessAction` / `StatefulAction` + `#[action(...)]` attribute zones) per ADR-0038 §1 + §Negative item 4. Closes dx-tester R3 (blocking).
- §3.1 — `SlotType` enum gained `Concrete { type_id }` + `service: ServiceKey` field on `ServiceCapability` + `CapabilityOnly { capability }` per credential Tech Spec §9.4 line 2452 authoritative shape. Closes spec-auditor 🔴 BLOCKER on `SlotType::ServiceCapability` payload silent degradation.
- §3.1 — added `#[non_exhaustive]` to `Capability` and `SlotType` per dx-tester R7. Storage-shape paragraph re-pinned to `nebula-engine` (`crates/engine/src/registry.rs`) — `crates/runtime/` does not exist (Phase 1 audit row 4). Closes devops 08e NIT 3.
- §2.7.2 — committed feature flag rename: `unstable-action-scheduler` → `unstable-terminate-scheduler` (parallel to `unstable-retry-scheduler`) per Strategy §4.3.2 symmetric-gating. Resolves §2.7-1 open item; closes devops 08e NIT 1 (freeze surface no longer pretends-frozen-but-deferred).
- §2.8 — added `SchemeGuard<'a, C>` non-Clone cross-ref + ADR-0037 §3 qualified-syntax probe discipline. Closes rust-senior 08b 🟠.
- §15 open items — closed §2.7-1; added forward-track entries for CP2/CP3 security-lead + rust-senior prep notes.

CP1 single-pass draft 2026-04-24:
- §0 — status, scope, freeze policy locked per Strategy §0 amendment mechanics.
- §1 — Goals (G1–G6) + Non-goals (N1–N7), each cited to Phase 1 CR-handle / Strategy §X / scope-decision §Y.
- §2 — Trait contract: `Action` base + 4 primary dispatch traits (Stateless/Stateful/Trigger/Resource) + `BoxFut<'a, T>` alias + 4 dyn-safe `*Handler` companions + 4-variant `ActionHandler` enum (no Control variant) + 5 sealed DX traits (per ADR-0038 §1 + ADR-0035 §3 sealed convention) + `ActionResult` with Terminate decision (wire-end-to-end picked per §2.7.1) + `ActionError` taxonomy preserved per rust-senior 02c §7 line 428.
- §3 — Runtime model: SlotBinding registry registration + HRTB fn-pointer dispatch + `resolve_as_<capability><C>` helpers + cancellation safety guarantees (security floor item 4 invariant locked, detail spec deferred to CP2 §4).
- Compile-checked all signatures against [`final_shape_v2.rs`](../drafts/2026-04-24-nebula-action-redesign/final_shape_v2.rs); see §0.2 invariant 4 — divergences would invalidate freeze.

### Handoffs requested — CP1

- **spec-auditor** — please audit §0–§3 for: (a) cross-section consistency (every forward reference to §4 / §7 / §9 / §16 marked deferred, not dangling); (b) every claim grounded in Strategy / credential Tech Spec / ADRs / spike artefacts at line-number citation granularity; (c) signature compile-check against [`final_shape_v2.rs`](../drafts/2026-04-24-nebula-action-redesign/final_shape_v2.rs) (any divergence flagged for either deliberate-divergence justification or correction); (d) terminology alignment with `docs/GLOSSARY.md`; (e) confirm §2.7.1 wire-end-to-end pick aligns with Strategy §4.3.2 + Phase 1 tech-lead solo decision (no asymmetry).
- **tech-lead** — please review §1 Goals (G1–G6) for completeness and §2.7.1 Terminate decision (wire-end-to-end vs retire). Solo-decider authority on G6 / §2.7.1; CP1 lock requires explicit ratification. Flag any §1 Goal that should be a Non-goal (or vice versa) under your active-dev framing.
- **rust-senior** — please confirm §2.2 RPITIT signatures + §2.4 BoxFut handler shapes align with rust-senior 02c §6 LOC payoff framing (single-`'a` + `BoxFut<'a, T>` type alias; `#[trait_variant::make]` not adopted per N6). Flag any §2 signature that contradicts 02c findings.
- **dx-tester** — please review §2 from a newcomer's perspective (do the four primary traits + sealed DX tier + `ActionResult` variants present a coherent authoring surface? does the signature load-up in §2.2–§2.6 absorb in one read?). Flag §1 G1 traceability (does the typed surface in §2 actually close the 32-min → <5-min friction Strategy §1 names?).

### Open items raised this checkpoint (CP2)

- §4.4-1 — `ActionSlots` trait sealing decision (prevent hand-implementation entirely vs leave `pub`) — CP3 §9 scope.
- §4.7-1 — Codemod auto-rewrite vs manual-marker default for `credential = "key"` string-form rejection; inference success rate measurement against 7 reverse-deps — CP3 §9 scope.
- §5.1-1 — `cargo-public-api` snapshot for macro crate stability — CP3 §9 may revisit; out of scope for CP2 per ADR-0037 §4.
- §5.3-1 — **RESOLVED at CP2 iteration 2026-04-24** per rust-senior 09b #1 — `nebula-engine` as dev-dep on `nebula-action-macros` is the committed path; companion `deny.toml` wrappers amendment lands at CP3 §9 (wrapper entry + inline rationale). Stub-helper alternative rejected (loses real-bound verification of Probe 6).
- §5.4.1 — **Soft amendment к credential Tech Spec §16.1.1 probe #7** (qualified-syntax form `<SchemeGuard<'_, C> as Clone>::clone(&guard)` replacing naive `guard.clone()` to avoid auto-deref silent-pass) — flagged, NOT enacted. CP4 cross-section pass surfaces; credential Tech Spec author lands inline edit per ADR-0035 amended-in-place precedent.
- §6.1.2 — `check_json_depth` byte-pre-scan vs `Value`-walking primitive — CP3 §9 picks; CP2 commits byte-pre-scan path (lower implementation cost; existing primitive). Rust-senior CP2 review: flag if `Value`-walking preferred.
- §6.2-1 — `credential_typed::<S>(key)` retention vs removal — CP3 §9 picks. Security-neutral; architectural cleanliness question.
- §6.3-1 — Full `redacted_display()` rule set (which substring patterns) — CP3 §9 design scope. CP2 commits crate location (`nebula-redact` NEW dedicated crate) + helper signature only.
- §6.4-1 — `tokio::time::pause()` vs real-clock 10ms in cancellation tests — CP3 §9 picks (recommendation: `pause()` for deterministic cancellation timing).
- §6.4 cross-crate amendment — soft amendment к credential Tech Spec §15.7 (add `engine_construct_with_probe` test-only constructor variant); flagged, NOT enacted. Same precedent as §5.4.1 — CP4 cross-section pass surfaces.
- §6.5 — Cross-tenant `Terminate` boundary — locked to CP3 §9 per security 08c §Gap 5; engine-side enforcement form (`if termination_reason.tenant_id != sibling_branch.tenant_id { ignore }` or equivalent).
- §7.3-1 — `ResolveError::NotFound` mapping to `ActionError` taxonomy (`Fatal` vs new typed `Resolve` variant) — CP3 §9 picks. Security-neutral.
- §7.1 step 3 — `ResolvedSlot` engine-side wrap point (inside `resolve_fn` vs after) — inherited from §3.2-1 CP1 open item; CP3 §9 scope.

**Items added during CP2 iteration 2026-04-24 (5-reviewer consolidation + user §2.9 reconsideration):**
- §2.9-1 — `ActionMetadata::for_trigger::<A>()` helper question (does the metadata-builder convenience layer need a Trigger-shaped helper analogous to `for_stateless` etc.?) — CP3 §7 ActionMetadata field-set lock decides; CP2 §2 leaves universal `with_schema` builder as ground-truth path.
- §6.1.2-A / §6.1.2-B — `check_json_depth` `pub(crate)` visibility commit + `Result<(), DepthCheckError { observed, cap }>` return-shape amendment, both committed (closes security-lead 09c §6.1-A + §6.1-B). CP3 §9 lands the `webhook.rs` edits + `body_json_bounded` re-wrap shim. NOT a forward-track open item — the commitment is in §6.1.2 above.
- §6.3.1-A — pre-`format!` sanitization wrap-form for `serde_json::Error` Display (closes security-lead 09c §6.3-A). CP3 §9 enumerates the full apply-site list across `crates/action/src/`. NOT a forward-track — the wrap-form is committed.
- §4.1.3 (new bullet) — cross-zone slot-name collision invariant added (closes dx-tester 09d #1). NOT a forward-track — the parser invariant is committed.
- §5.4-companion — author-trap regression-lock probe (unqualified `guard.clone()` compile-pass) added (closes dx-tester 09d #2). Companion clippy-lint at macro emission boundary forward-tracked to CP3 §9 design scope.
- **ADR-0037 §1 SlotBinding shape divergence — amendment-in-place trigger (rust-senior 09b #3).** ADR-0037 §1 currently shows `SlotBinding { key, slot_type, capability, resolve_fn }` with separate `capability` field; this Tech Spec §3.1 correctly folds capability into the `SlotType` enum per credential Tech Spec §9.4. Per ADR-0035 amended-in-place precedent, ADR-0037 §1 must be amended to mirror Tech Spec §3.1's `SlotBinding { field_name, slot_type, resolve_fn }` shape (capability lives inside `SlotType::ServiceCapability { capability, service }` and `SlotType::CapabilityOnly { capability }` variants). **FLAGGED, NOT ENACTED** — CP2 does not edit frozen ADRs (per task constraint); enactment is Phase 8 cross-section pass with ADR-0037 amended-in-place + CHANGELOG entry. This is a §0.2 invariant 2 trigger if not landed before Tech Spec ratification — Phase 8 must enact OR this Tech Spec must re-pin §3.1 to ADR-0037's current shape (rejected — credential Tech Spec §9.4 wins per cross-crate authoritative-source rule).

**Items deferred from CP1 still un-homed (devops 09e from CP1 nit-list — minor):**
- T4 — `[dev-dependencies]` addition mechanics (specific `cargo add` sequence vs hand-edit) for `crates/action/macros/Cargo.toml` — CP4 §16 fold-or-doc. Not blocking.
- T5 — workspace-vs-crate-local pin choice for `trybuild` (now three consumers per devops 09e #2 above) — CP3 §9 picks; CP4 §16 fold if undecided.
- T9 — `lefthook.yml` mirror entries for new macro-crate test commands — CP4 §16 fold-or-doc; lands alongside macro-crate landing.

**Items forward-pointing CP4 cross-section (dx-tester 09d minor — preserved):**
- (a) §2.9.1a / §2.9.6 axis naming — confirm CP4 cross-section pass still surfaces Configuration vs Runtime Input distinction in §1 G2 / G6 traceability.
- (b) §4.1.3 cross-zone collision invariant — confirm the parser invariant test ports through to §5.3 compile-fail probe coverage at CP3 §9.
- (c) §5.4-companion dual-probe regression — confirm both qualified-form (compile-fail) and unqualified-form (compile-pass) live in the same `tests/` directory tree at CP3 §9 layout finalization.

**Forward-track for CP3 (carry-forward + new):**
- CP3 §9 — engine scheduler-integration hook trait surface (`Retry` + `Terminate` dispatch path); cross-tenant `Terminate` boundary per §6.5; `Retry` / `Terminate` re-enqueue / cancellation persistence with `ExecutionRepo` per §7.4 + §8.3.
- CP3 §9 — codemod runbook for `credential<S>()` no-key removal (per §6.2.4); auto-rewrite vs manual-marker classification across 7 reverse-deps.
- CP3 §9 — `redacted_display()` full rule set + invariant tests (per §6.3.3).
- CP3 §7 — `PollAction` sealed-DX trait shape lock per ADR-0038 §Implementation notes (cursor management; cluster-mode hooks).
- CP3 §7 — `ActionSlots` sealing decision per §4.4-1.

### CHANGELOG — CP2

CP2 iteration append 2026-04-24 (post 5-reviewer-matrix consolidation: spec-auditor 09a / rust-senior 09b / security-lead 09c / dx-tester 09d / devops 09e + user §2.9 reconsideration):
- Status header — `DRAFT CP2` → `DRAFT CP2 (iterated 2026-04-24)`.
- §2.9.1a (new subsection) — user verbatim pushback on §2.9 REJECT verdict (RSS url + interval, Kafka channel + post-ack examples) recorded; resolution: Configuration ≠ Runtime Input axis named explicitly. Configuration lives in `&self` struct fields per §4.2 ("Fields outside the zones pass through unchanged") + schema declared via `ActionMetadata::parameters` universal `with_schema` builder per `crates/action/src/metadata.rs:292`. REJECT (Option C) preserved; rationale tightened. New open item §2.9-1 (CP3 §7) — `for_trigger::<A>()` metadata-builder helper question.
- §2.9.5 / §2.9.6 — verdict annotation + rationale prelude refined to name Configuration vs Runtime Input axis.
- §6 (header) — co-decision authority sourcing corrected per spec-auditor 09a #2: Strategy §4.4 (security floor invariant) + 03c §1 VETO + §1 G3 (NOT Strategy §6.3 lines 386-394 which is reviewer-matrix table). Wording revised.
- §6.1 — cap=128 attribution corrected per spec-auditor 09a #3: cap origin is Strategy §2.12 / scope §3 must-have floor (action-adapter boundary), NOT existing `check_json_depth` primitive (which is parameter-driven, no hardcoded cap; webhook recommends 64).
- §6.1.2 — restructured into §6.1.2-A / -B / -C / -D subsections committing two pre-CP3 amendments to `check_json_depth` per security-lead 09c §6.1-A + §6.1-B: (A) `pub(crate)` visibility promotion (closes single-audit-point CP3 implementer drift); (B) typed `DepthCheckError { observed, cap }` return-shape (enables `ValidationReason::DepthExceeded { observed, cap }` per `feedback_observability_as_completion.md`); `max_depth` parameter promoted to `u32`. `body_json_bounded` re-wraps to preserve public webhook API.
- §6.3.1-A (new sub-subsection) — pre-`format!` sanitization wrap-form for `serde_json::Error` Display per security-lead 09c §6.3-A: sanitize embedded error before `format!` interpolation (Display impl is the leak surface, not outer string). `nebula_redact::redacted_display(&e) -> String` consumes Display through redaction filter.
- §4.3 — Probe 6 citation corrected per rust-senior 09b #2: Probe 6 is the wrong-Scheme rejection gate (NOTES §1.5); HRTB-coercion shape comes from spike Iter-2 §2.2/§2.3 (`resolve_as_basic`/`resolve_as_oauth2` in const-slot-slices). Both citations now appear in the rationale paragraph.
- §5.3-1 — RESOLVED at iteration per rust-senior 09b #1: `nebula-engine` as dev-dep on `nebula-action-macros` is the committed path; `deny.toml` wrappers amendment shape committed (CP3 §9 lands the inline edit). Stub-helper alternative rejected (loses real-bound verification of Probe 6).
- §4.1.3 — added cross-zone slot-name collision invariant per dx-tester 09d #1 (`HashSet<Ident>` populated across all zones; collision diagnostic with prior-occurrence span).
- §5.4-companion (new sub-subsection) — author-trap regression-lock probe per dx-tester 09d #2: dual-probe pair (qualified-form compile-fail + unqualified-form compile-pass) makes silent-pass shape observable. CP3 §9 forward-track for clippy-lint at macro emission boundary.
- §5.1 / §5.3 / §5.5 — `macrotest = "1.0.13"` → `macrotest = "1.2"` per devops 09e #1 (current crates.io max 1.2.1; minor-pin tracks latest stable). All three occurrences updated. CP3 §9 verifies `expand_args` API shape against macrotest 1.2.x.
- §5.1 — pinning rationale corrected per devops 09e #2: `trybuild` has TWO existing workspace consumers (`crates/schema/Cargo.toml:40` + `crates/validator/Cargo.toml:46`); admitting `crates/action/macros` makes three. Earlier "only consumer" framing replaced with explicit consumer count + workspace-pin posture decision (CP3 §9 picks workspace-dep promotion vs crate-local pins).
- §7.2 — second cross-crate amendment к credential Tech Spec §15.7 (the §6.4.2 `engine_construct_with_probe` test-only constructor) reconciled in the §15-list paragraph per spec-auditor 09a #1: BOTH soft amendments (§5.4.1 + §6.4.2) listed together for §15 cross-section integrity.
- §15 — §5.3-1 marked RESOLVED; six items added during CP2 iteration (§2.9-1, §6.1.2-A/-B status, §6.3.1-A status, §4.1.3 cross-zone status, §5.4-companion status, ADR-0037 §1 amendment-in-place trigger); three items deferred from CP1 still un-homed (T4/T5/T9); three items forward-pointing CP4 cross-section.

CP2 single-pass append 2026-04-24:
- Status header — `DRAFT CP1 (iterated 2026-04-24)` → `DRAFT CP2`. §0 status table — `(this revision)` annotation moved from CP1 row to CP2 row; CP1 row marked `locked CP1`.
- §4 added — `#[action]` attribute macro full token shape per ADR-0037: §4.1 attribute parser zones (credentials/resources with three credential-type patterns per credential Tech Spec §3.4 + ADR-0035 phantom shim); §4.2 narrow field-rewriting contract (zone-confined, non-zone fields pass through); §4.3 per-slot emission with HRTB `resolve_fn` dispatch table; §4.4 dual enforcement layer (type-system + proc-macro `compile_error!` per ADR-0036 §Decision item 3 + ADR-0037 §2); §4.5 per-slot emission cost bound (1.6-1.8x adjusted ratio per ADR-0037 §5; verified against spike §2.5); §4.6 broken `parameters = Type` path fix (current macro emits non-existent `with_parameters()`; new emits `with_schema(<T as HasSchema>::schema())` per `crates/action/src/metadata.rs:292` builder); §4.7 string-form `credential = "key"` rejection as hard `compile_error!` (closes Phase 1 dx-tester finding 6 silent-drop).
- §5 added — Macro test harness: §5.1 `Cargo.toml` `[dev-dependencies]` addition (`trybuild = "1.0.99"` + `macrotest = "1.0.13"` pinned); §5.2 harness layout (`crates/action/macros/tests/`); §5.3 6-probe port from spike commit `c8aef6a0` + new Probe 7 (`parameters = Type` no-`HasSchema` rejection); §5.4 auto-deref Clone shadow probe in qualified-syntax form per ADR-0037 §3 + spike finding #1; §5.4.1 **soft amendment к credential Tech Spec §16.1.1 probe #7** flagged (NOT enacted — CP4 cross-section pass coordinates); §5.5 macrotest expansion snapshots (3 fixtures locking per-slot emission stability).
- §6 added — Security must-have floor (CO-DECISION tech-lead + security-lead per Strategy §6.3): §6.1 JSON depth cap 128 implementation (apply sites at `stateless.rs:370`, `stateful.rs:561, 573`; mechanism = pre-scan via existing `check_json_depth` primitive at `webhook.rs:1378-1413`; typed `ValidationReason::DepthExceeded` variant added); §6.2 **HARD REMOVAL** of `CredentialContextExt::credential<S>()` no-key heuristic (NOT `#[deprecated]` — security-lead 03c §1 VETO trigger cited verbatim); §6.3 `ActionError` Display sanitization via `redacted_display()` helper hosted in **NEW dedicated `nebula-redact` crate** (closes 08c §Gap 3 — single-audit-point reasoning); §6.4 cancellation-zeroize test in `crates/action/tests/cancellation_zeroize.rs` with **per-test `ZeroizeProbe: Arc<AtomicUsize>`** instrumentation (closes 08c §Gap 4 — preferred over `serial_test::serial`); §6.5 cross-tenant `Terminate` boundary forward-tracked to CP3 §9 (closes 08c §Gap 5).
- §7 added — Action lifecycle / execution: §7.1 adapter execute path with SlotBinding resolution flow (6 steps); §7.2 SchemeGuard<'a, C> RAII flow per credential Tech Spec §15.7 lines 3394-3516 (cited verbatim, not restated); §7.3 per-action error propagation discipline (failure-to-variant table); §7.4 ActionResult variants handling (engine-side dispatch table; wire-end-to-end commitment per §2.7.1).
- §8 added — Storage / state: §8.1 action-side persistence (state JSON via StatefulAction; trigger cursor via PollAction; macro-emitted slot bindings static-shape only); §8.2 runtime-only state (handler cache; SchemeGuard borrows; ActionContext borrows); §8.3 boundary with engine persistence (cross-ref to `crates/storage/`; engine bridges via `*Handler` dyn-erasure per §2.4).
- §15 (open items) — CP1 carry-forward preserved; CP2 open items added (13 items) + forward-track for CP3 (5 items).

### Handoffs requested — CP2

- **tech-lead** — please review §6 co-decision items: (1) §6.1.2 JSON depth-cap mechanism (pre-scan via existing `check_json_depth` primitive vs `serde_stacker` wrap — security-neutral; rust-senior call); (2) §6.2 hard-removal mechanism (Option (a) delete-method preferred per 03c §1 + 08c §Gap 1; security-lead VETO retained on shim regression); (3) §6.3.2 `redacted_display()` helper crate location (NEW dedicated `nebula-redact` crate per security-lead 08c §Gap 3 single-audit-point); (4) §6.4.2 ZeroizeProbe per-test instrumentation (preferred over `serial_test::serial` per 08c §Gap 4). Solo-decider authority on §6 co-decision points; CP2 lock requires tech-lead explicit ratification.
- **security-lead** — please verify §6 floor implementation forms: (1) §6.1 depth cap 128 at all three sites (stateless input + stateful input + stateful state) — verify S-J1 + S-J2 closure simultaneously per 03c §1; (2) §6.2 hard-removal language (no `#[deprecated]` regression — VETO trigger language cited verbatim from 03c §1.B); (3) §6.3 `redacted_display()` helper location (`nebula-redact` NEW crate vs `nebula-log` co-resident — confirm single-audit-point reasoning aligns with 08c §Gap 3 preference); (4) §6.4 per-test `ZeroizeProbe` choice (closes 08c §Gap 4); (5) §6.5 cross-tenant `Terminate` boundary forward-tracked to CP3 §9 (confirm CP3 lock language is what 08c §Gap 5 requested). VETO authority retained on shim-form drift in CR3 fix per 03c §1 + §1 G3.
- **rust-senior** — please confirm: (1) §4.5 per-slot emission cost bound (1.6-1.8x adjusted ratio) aligns with ADR-0037 §5 + spike §2.5 measurements; (2) §5.3-1 `nebula-engine` as dev-dep on `nebula-action-macros` does NOT introduce cycle / boundary-erosion; (3) §6.1.2 byte-pre-scan vs `Value`-walking primitive — flag if `Value`-walking preferred for performance / clarity reasons; (4) §6.3 `redacted_display()` helper signature (`fn redacted_display<T: ?Sized + Display>(value: &T) -> String`) is the right shape vs alternatives (e.g., `RedactedDisplay<'a, T>` newtype wrapper).
- **dx-tester** — please review §4.6.1 + §4.7 from authoring-friction perspective: (1) does the typed `parameters = Type` requires-`HasSchema` diagnostic surface the actual missing bound clearly (vs the legacy "no method `with_parameters`" confusion)? (2) does the string-form `credential = "key"` `compile_error!` message (line "the `credential` attribute requires a type, not a string. Use `credential = SlackToken`...") give a clean migration signal? Flag any DX-friction the diagnostics produce in newcomer scenarios.
- **spec-auditor** — please audit §4–§8 for: (a) cross-section consistency (every forward reference to CP3 / CP4 marked deferred, not dangling — 13 CP2 open items + 5 forward-track CP3 items); (b) every claim grounded in code (file:line citations) / canon / ADR / Strategy / spike artefacts / security 03c+08c at line-number granularity; (c) §5.4 + §6.4 cross-crate amendments к credential Tech Spec §16.1.1 + §15.7 are FLAGGED only (no inline credential Tech Spec edit performed by this Tech Spec); (d) terminology alignment with `docs/GLOSSARY.md`; (e) §6 co-decision items match security 03c VETO conditions verbatim (especially §6.2 hard-removal vs `#[deprecated]` language).

### Open items raised this checkpoint (CP3)

**Items resolved at CP3 §9-§13 drafting (closed in this revision):**
- §2.7-2 — engine scheduler-integration hook trait surface — **PARTIALLY CLOSED at §9.5**. Cross-tenant boundary locked at §9.5; full engine-side trait surface (`SchedulerIntegrationHook::on_terminate(...)` shape) is engine-cascade scope per §9.5.5. Tech Spec scope: action surface produces `ActionResult::Terminate`; engine cascade designs the scheduler consumer.
- §3.2-1 — `ResolvedSlot` engine-side wrap point (inside `resolve_fn` vs after) — **PARTIALLY CLOSED at §11.3.1 + §3.2 step 5**. Adapter responsibility table commits "engine wraps after `resolve_fn` returns `ResolvedSlot`" per spike interpretation; explicit wrap-point in engine code is engine-cascade scope.
- §4.4-1 — `ActionSlots` trait sealing decision — **CLOSED at §9.4**. Decision: leave `pub`, NOT sealed. Rationale: `#[action]` macro is recommended path; dual enforcement layer (§4.4) makes hand-impl observable; advanced internal-Nebula contexts may need hand-impl for special cases. Tech-lead ratifies at CP3 close.
- §6.2-1 — `credential_typed::<S>(key)` retention vs removal — **CLOSED at §9.3.1**. Recommendation: REMOVE alongside `credential<S>()`. Rationale: explicit-key form achievable via `ctx.resolved_scheme(&CredentialRef::from_key(key))`; two parallel APIs bifurcate authoring guidance; zero `nebula-sdk::prelude` re-export presence. Tech-lead ratifies at CP3 close.
- §6.5 / §9.5 — cross-tenant `Terminate` boundary — **CLOSED at §9.5**. Engine-side enforcement contract locked: tenant scope check at scheduler dispatch path before fanning Terminate to siblings; cross-tenant skip is silent (telemetry observable via `tracing::warn!` + counter); structural errors are Fatal. Security-lead implementation-time VETO retained on §9.5.1 invariant language ("MUST NOT propagate").
- §7.3-1 — `ResolveError::NotFound` mapping to `ActionError` taxonomy — **CARRIED FORWARD to CP4 §16** (security-neutral; not implementation-blocking; can land at implementation time without re-opening Tech Spec).
- §10 codemod transforms named — **CLOSED at §10.2** (T1-T6).
- §11.3 adapter responsibility contract — **CLOSED at §11.3** (serialize/deserialize, error propagation, cancellation safety).
- §13.4 T4 / T9 cascade-scope absorb — **CLOSED at §13.4** (T4 `zeroize` workspace=true; T9 `deny.toml` wrappers-list extension + new positive ban for `nebula-action` runtime layer per devops 10e #2 critical iteration). T5 (`lefthook.yml` parity) explicitly out-of-cascade-scope per §13.4.2.
- `nebula-redact` workspace integration (NEW crate creation + workspace member add) — **CLOSED at §13.4.4** (cascade-scope absorb preliminary; atomic with cascade PR per devops 10e #1 critical iteration; closes "compile-fail blocker" gap that CP3 single-pass §13.4 had silently dropped despite CP2 09e flagging it).

**Items added during CP3 §9-§13 drafting:**
- §9.3-1 — `nebula-sdk::prelude` re-export of `redacted_display` for community plugin authors — CP4 §16 picks; default position NO (community plugins depend on `nebula-redact` directly to preserve single audit point).
- §11.3-1 — Adapter performance microbenchmark (per-dispatch overhead: input ser round-trip + depth pre-scan + slot resolution + output ser) — CP4 §15 housekeeping; CP3 §11 commits responsibility table only.
- §12 `#[action(control_flow)]` attribute zone syntax — exact spelling (`control_flow` flag vs `control_flow = SomeStrategy` config) — CP4 §15 scope per §9.2 trait-by-trait audit closure.

**Forward-track for CP4 §14-§16:**
- (a) §1.2 / N5 — paths a/b/c implementation pick framing in CP4 §16; user picks at Phase 8 per Strategy §4.2 line 198-206 + §6.5 line 408-413.
- (b) §2.2.3 — TriggerAction cluster-mode hooks final trait shape per Strategy §5.1.5 line 297 — CP4 §15 scope (deferred from CP3 §7 per ADR-0038 trait-by-trait audit closure plus §1.2 N4 boundary).
- (c) §2.6 / §9.2 — DX trait blanket-impl trait-by-trait audit completion — CP4 §15 confirms exact `#[action(...)]` attribute zone spellings for each DX trait.
- (d) §3.1 — engine `ActionRegistry::register*` call-site exact line range + final host-crate path — CP4 §15 confirms `nebula-engine::registry` surface as engine-cascade-handoff item.
- (e) §3.2 — ActionContext API location in credential Tech Spec (Strategy §5.1.1) — coordination with credential Tech Spec author; CP4 cross-section pass surfaces.
- (f) §5.4.1 + §6.4.2 cross-crate amendments к credential Tech Spec §16.1.1 + §15.7 — CP4 cross-section pass surfaces both for credential Tech Spec author inline edit per ADR-0035 amended-in-place precedent.
- (g) ADR-0037 §1 SlotBinding shape divergence amendment-in-place — CP2 §15 forward-track preserved; Phase 8 enacts inline ADR edit + CHANGELOG entry. Per §0.2 invariant 2, must land before Tech Spec ratification.
- (h) §10 codemod implementation host crate — `tools/codemod/` placeholder; CP4 §15 confirms exact crate name + binary location.
- (i) §13.4.2 T5 lefthook parity — separate housekeeping PR (devops-owned); CP4 §16 sunset-tracked per `feedback_lefthook_mirrors_ci.md`.
- (j) §11.3-1 + §13.3 — adapter perf microbenchmark + crate publication policy cross-ref to ADR-0021.

### CHANGELOG — CP3

CP3 single-pass append 2026-04-24:
- Status header — `DRAFT CP2 (iterated 2026-04-24)` → `DRAFT CP3`. §0 status table — `(this revision)` annotation moved from CP2 row to CP3 row; CP2 row marked `locked CP2`.
- §9 added — Public API surface: §9.1 four primary trait surface unchanged at trait level (per ADR-0036 §Neutral item 2); semver impact (per-feature additions OK; removals would break — none proposed); §9.2 five sealed DX trait surface (sealed per ADR-0038 §1; community migration target reaffirmed — `StatelessAction` primary + `#[action(control_flow = …)]`); §9.3 `prelude.rs` re-export reshuffle (removed: legacy `CredentialContextExt::credential` no-key + `credential_typed` recommendation REMOVE + `CredentialGuard` legacy + `nebula_action_macros::Action` derive; added: `ActionSlots` + `BoxFut` single-home + `SlotBinding`/`SlotType`/`Capability`/`ResolveFn` + `redacted_display!` from `nebula-redact` + `ValidationReason::DepthExceeded` + `DepthCheckError` internal; reshuffled: `SchemeGuard` and `CredentialRef` re-exported through canonical credential path); §9.4 builder/macro convenience methods exposed in `nebula-sdk::prelude` (community plugin authoring path) vs lower-level access (`nebula-action::*` for Handler types / Adapter types / SlotBinding internals); ActionSlots seal decision CLOSED — leave `pub` (NOT sealed) per §4.4-1; §9.5 cross-tenant `Terminate` boundary lock (security 08c Gap 5) — engine-side enforcement contract with verbatim "MUST NOT propagate" invariant language; mechanism (tenant-scope check at scheduler dispatch path before fanning Terminate); silent skip with telemetry; structural errors Fatal; security-lead implementation-time VETO retained.
- §10 added — Migration plan (codemod runbook): §10.1 reverse-deps inventory (verbatim from Phase 0 §9 line 252-329); §10.2 codemod transforms T1-T6 (T6 added at CP3 for ControlAction → StatelessAction migration per ADR-0038 §Negative item 4; T1/T3/T4 AUTO; T2/T5/T6 MANUAL-REVIEW); §10.2.1 codemod execution model (`cargo`-style binary `nebula-action-codemod`; AUTO mode = unified-diff via `--dry-run`; MANUAL-REVIEW mode = TODO marker insertion; idempotent re-runs); §10.3 per-consumer migration step counts (engine ~30 sites; api ~5; sandbox ~9; sdk re-export-only; plugin ~2; cli ~6); §10.4 plugin author migration guide (7 steps; <30min trivial / 2-4hr complex; `MIGRATION.md` ships in `crates/action/`); §10.5 auto-vs-manual breakdown (~70% auto / ~30% manual review).
- §11 added — Adapter authoring contract: §11.1 `#[action]` macro IS the adapter for community plugins (zero hand-authoring); §11.2 internal-Nebula adapter authoring path via sealed_dx pattern from ADR-0038 §1; §11.3 adapter responsibilities (serialize/deserialize boundary with depth cap 128 + `redacted_display(&e)` wrap; error propagation per §6.3 + §7.3; cancellation safety via Drop ordering + ZeroizeProbe per-test instrumentation per §6.4.2); §11.3-1 perf microbenchmark forward-track to CP4 §15.
- §12 added — ControlAction + DX migration: §12.1 sealed adapter pattern verbatim from ADR-0038 §1 (with CP1 §2.6 refinement adding `+ ActionSlots`); §12.2 community plugin DX flow (concrete `#[action(control_flow)]` example); §12.3 internal Nebula crate migration (engine + sandbox already at handler level; sealed DX is mostly additive for community visibility); §12.4 codemod coverage T6 — auto-rewrite path for trivial pass-through; manual-review for custom Continue/Skip/Retry reason variants + Terminate interaction + test fixtures.
- §13 added — Evolution policy: §13.1 deprecation policy (pre-1.0 alpha hard breaking changes acceptable per `feedback_hard_breaking_changes.md`; post-1.0 deprecation cycle + major-bump + codemod artefact); §13.2 breaking-change policy (spec-level via ADR amendment-in-place per ADR-0035 precedent; paradigm shift via full ADR supersession); §13.3 versioning per crate publication — out of cascade scope per ADR-0021 cross-ref; §13.4 CP1 hygiene fold-in (T4 `zeroize` workspace=true cascade-scope absorb; T5 `lefthook.yml` out-of-cascade-scope; T9 `deny.toml` layer-enforcement cascade-scope absorb with wrapper entry for `nebula-action-macros` dev-dep on `nebula-engine`); disposition summary table (re-numbered to §13.4.5 at 2026-04-24 iteration after `nebula-redact` absorption section §13.4.4 inserted per devops 10e #1).
- §15 (open items) — CP3 closures recorded (§2.7-2 partial / §3.2-1 partial / §4.4-1 / §6.2-1 / §6.5 / §10 transforms / §11.3 / §13.4 T4+T9); CP3-new items added (§9.3-1 / §11.3-1 / §12 attribute syntax); 10-item CP4 forward-track including ADR-0037 §1 amendment-in-place trigger preserved from CP2.

CP3 iteration append 2026-04-24 (post 5-reviewer-matrix consolidation: spec-auditor 10a PASS-WITH-NITS / rust-senior 10b RATIFY-WITH-NITS / security-lead 10c ACCEPT (no edits) / dx-tester 10d RATIFY-WITH-NITS / devops 10e RATIFY-WITH-NITS):
- Status header — `DRAFT CP3` → `DRAFT CP3 (iterated 2026-04-24)`. Stays DRAFT until CP4 freeze.
- §9.3.2 — `BoxFut` rename rationale added (rust-senior 10b #1): three-bullet why-not-`BoxFuture` clause covers (a) Strategy §4.3.1 single-`'a` shape alignment, (b) avoidance of `futures::future::BoxFuture` import collision, (c) spike `final_shape_v2.rs:38` precedent. Closes rust-senior 🟠 DATED.
- §9.4 — `ActionContext` field-vs-method clarity (rust-senior 10b #2): "CP4 §15 may revisit field-vs-method shape" → "Internal storage shape may be revisited in CP4 §15 cross-section; community-facing API at `resolved_scheme(&self.<slot>)` is locked." Removes spurious public-surface uncertainty (field is `pub(crate)`, unreachable from outside crate). Closes rust-senior 🟠 DATED.
- §10.2 prose — Strategy §4.3.3 → Tech Spec mapping disambiguated (spec-auditor 10a 🟠): T1 = Strategy 1; T2 = Strategy 2+3 collapsed; T3/T4/T5/T6 added at Tech Spec design level per Strategy §4.3.3 line 243 license (NOT 1:1 with Strategy 1-5). Closes spec-auditor 🟠 HIGH on misleading "T1-T5 from Strategy" framing.
- §10.2 T6 row — verdict normalized to **MIXED** (AUTO default for trivial pass-through; MANUAL marker on edge-case detection) per ADR-0038 §Negative item 4. Closes spec-auditor 10a 🟠 HIGH on §10.2 vs §10.5 internal contradiction. §10.2.1 default-mode line revised to enumerate the MIXED behaviour explicitly.
- §10.2 T6 + §10.4 step 5 T6 — `control_flow` attribute syntax unified to flag form (`control_flow,`) across §10.2, §10.4, §12.2 per dx-tester 10d R1 (§12.2 example was already flag form; §10.2 + §10.4 had `control_flow = ...` key=value form). CP4 §15 still owns the final spelling decision; flag form is the consistent CP3 placeholder.
- §10.3 — Phase 0 §10 line range corrected `347-356` → `346-356` (spec-auditor 10a 🟡 off-by-one; range now includes the "Blast-radius weight by consumer" header at line 346).
- §10.4 — added step 1.5 `semver` Cargo.toml dep instruction per dx-tester 10d R2 (Phase 1 CC1 carry-forward): macro emits unqualified `::semver::Version` path; consumer crate must declare `semver = { workspace = true }` or `semver = "1"` directly. Re-export of `semver` through `nebula-action::__private::semver` deferred to CP4 §15 housekeeping. Closes dx-tester 🟠 first-compile-fails-without-this NIT.
- §10.5 — bucket entry refined: T6 reclassified from "Manual review" to new **Mixed** sub-bucket per ADR-0038 §Negative item 4 + spec-auditor 10a 🟠 closure. Per-consumer share clarification added per rust-senior 10b 🟡: 70/30 figure is workspace-aggregate file-touch count; trivial plugin ~100% AUTO; heavy engine consumer closer to 50/50.
- §12.3 — false file-line citation `crates/action/src/lib.rs:14` → `:4` (spec-auditor 10a #1 🟠). Verified: line 4 is `Canon §3.5 (trait family; adding a trait requires canon revision)` (the contradicting docstring); line 14 is `StatelessAction` enumeration. Closes spec-auditor 🟠 HIGH on cross-doc reference resolution.
- §13.4.3 — `deny.toml` edit shape rewritten per devops 10e #2 critical: Edit 1 changed from "parallel `nebula-engine` deny entry" (would duplicate-rule conflict against existing `deny.toml:59-66`) to **wrappers-list extension** of the existing rule (adds `nebula-action-macros` to existing `["nebula-cli", "nebula-api"]` wrappers list); Edit 2 added — **NEW positive ban for `nebula-action` runtime layer** symmetric with engine/sandbox/storage/sdk/plugin-sdk rules per Phase 0 §11 row 9 T9 full intent. Closes devops 🟠 HIGH critical (deny.toml syntax was wrong + full T9 intent was missing).
- §13.4.4 (NEW subsection) + §13.4.5 (renumbered from prior §13.4.4) — `nebula-redact` workspace integration absorbed per devops 10e #1 critical: §13.4.4 commits four atomic edits (new `crates/redact/Cargo.toml` + `src/lib.rs`; root `Cargo.toml [workspace] members` + `[workspace.dependencies]`; no new `deny.toml` ban — leaf utility); §13.4.5 disposition table extended with fourth row. Closes devops 🟠 HIGH critical (cascade-landing PR would compile-fail without nebula-redact workspace member).

### Handoffs requested — CP3

- **devops** — please review §10 codemod runbook + §13.4 hygiene fold-in: (1) §10.2 transform table T1-T6 with AUTO / MANUAL-REVIEW classification — flag any transform where the auto/manual split is wrong (e.g., T3 should be MANUAL-REVIEW because of in-process/out-of-process ABI nuance?); (2) §10.2.1 execution model (`cargo`-style binary; idempotent re-run; `--dry-run` flag) — flag if alternative shape preferred (e.g., `cargo` subcommand vs standalone bin); (3) §10.3 per-consumer step counts — verify estimates against Phase 0 §10 audit blast-radius weights; (4) §13.4.1 T4 `zeroize` workspace=true edit + §13.4.3 T9 `deny.toml` wrapper entry — confirm both can land in the cascade PR without separate housekeeping; (5) §13.4.2 T5 `lefthook.yml` parity decision (out-of-cascade-scope) — confirm the separate housekeeping PR has a target sunset window per `feedback_lefthook_mirrors_ci.md`.
- **rust-senior** — please confirm: (1) §9.1 trait-level surface unchanged claim — verify `Input: HasSchema + DeserializeOwned + Send + 'static` etc. lifts are non-breaking at the impl level (existing impls already satisfy via adapter contract); (2) §9.3.1 `credential_typed::<S>(key)` removal recommendation — flag if a legitimate non-`#[action]` consumer exists that would force retention; (3) §11.1 macro-emitted adapter shape — confirm the `StatelessActionAdapter<A>` example matches current `crates/action/src/stateless.rs` shape post-modernization; (4) §11.2 sealed_dx adapter authoring pattern for internal-Nebula crates — confirm the `mod sealed_dx { pub trait MyCustomShapeSealed {} }` shape composes with the §2.6 sealed-DX pattern without conflict; (5) §13.2 amendment-in-place vs supersession discipline — flag if the ADR-0035 precedent is mis-cited.
- **spec-auditor** — please audit §9–§13 for: (a) cross-section consistency — every forward reference to CP4 / engine cascade marked deferred, not dangling (§9.5.5 engine trait surface; §11.3-1 perf microbench; §13.3 ADR-0021 cross-ref); (b) every claim grounded in code (file:line) / canon / ADR / Strategy / Phase 0 audit at line-number granularity (Phase 0 §9 reverse-deps verbatim; security 08c Gap 5 verbatim language; ADR-0038 §1 / §Negative item 4 verbatim); (c) §9.5 cross-tenant Terminate engine-side enforcement contract uses VETO trigger language verbatim (NOT paraphrased); (d) terminology alignment with `docs/GLOSSARY.md` (especially "tenant scope", "scheduler-integration hook", "sealed DX", "adapter responsibilities"); (e) §10 codemod transforms T1-T6 trace to specific reverse-deps + Phase 0 §10 blast-radius weights; (f) §13.4 T4/T5/T9 dispositions consistent with Phase 0 audit findings (§1 line 44; §11 line 376, line 379).
- **security-lead** *(focused review on §9.5 only)* — please verify §9.5 cross-tenant `Terminate` boundary closure of 08c §Gap 5: (1) §9.5.1 invariant language quotes 08c §Gap 5 line 109-111 verbatim — confirm the wording matches your veto trigger position; (2) §9.5.2 mechanism (tenant scope check at scheduler dispatch path; cross-tenant skip silent with telemetry; structural error Fatal) — confirm the silent-skip path does NOT enable any tenant-isolation bypass; (3) §9.5.3 reject paths (silent cross-tenant cancel REJECT; silent no-op without telemetry REJECT) — confirm the threat-model coverage; (4) §9.5.5 implementation-time VETO retained on "MUST NOT propagate" — confirm the wording is the right strength. **Out of scope** for this review: §10 codemod, §11 adapter contract (already accepted at CP2 §6.4 + §7), §12 / §13 (no new security surface).

### Open items raised this checkpoint (CP4)

CP4 §15 walks all CP1-CP3 carry-forward to decided / deferred-with-trigger status. No new open items raised at CP4 drafting — the §15.8 deferred-with-trigger registry is the residual ledger. Each row has trigger + owner + scope per `feedback_active_dev_mode.md`.

**Items deferred-with-trigger (residual ledger from §15.8 — none implementation-blocking):**
- (b) §2.2.3 TriggerAction cluster-mode hooks final trait shape — engine cluster-mode coordination cascade scope (§1.2 N4)
- (c) §2.6 / §9.2 DX trait blanket-impl trait-by-trait audit — implementation-time housekeeping (rust-senior owner)
- (d) §3.1 engine `ActionRegistry::register*` exact line range + final host-crate path — engine cascade handoff
- (e) §3.2 ActionContext API location in credential Tech Spec — cross-section pass with credential Tech Spec author
- (h) §10 codemod implementation host crate — cascade-final housekeeping (devops + architect)
- (i) §13.4.2 T5 `lefthook.yml` parity — out-of-cascade-scope; sunset ≤2 release cycles
- (j) §11.3-1 adapter perf microbenchmark + §13.3 ADR-0021 cross-ref — cascade-final housekeeping
- §5.1-1 `cargo-public-api` snapshot for macro crate — out-of-cascade-scope; future macro-evolution housekeeping
- §7.3-1 `ResolveError::NotFound` → `ActionError` taxonomy mapping — cascade-final housekeeping (security-neutral)
- §12 `#[action(control_flow)]` exact spelling — cascade-final housekeeping (flag form is CP3-CP4 placeholder)

**Items enacted this CP (no longer open):**
- (g) ADR-0037 §1 SlotBinding shape amendment-in-place — **ENACTED** at §15.5.1 (ADR file edit lands co-with this CP4 draft)

**Items flagged-not-enacted (cross-cascade coordination):**
- (f) §15.3 + §15.4 cross-crate soft amendments к credential Tech Spec §16.1.1 / §15.7 — owner: credential Tech Spec author; trigger: cross-section pass (this CP); sunset ≤1 release cycle

**Items closed at §15:**
- §1.2 / N5 paths a/b/c framing — **CLOSED at §16.1**; user picks at Phase 8
- §15.6 — `ActionMetadata::for_trigger::<A>()` helper question CLOSED (universal `with_schema` retained)
- §15.7 — T4 / T5 / T9 / `nebula-redact` dispositions RATIFIED
- §9.3-1 — `nebula-sdk::prelude` re-export of `redacted_display` — CLOSED (NO; community plugins depend on `nebula-redact` directly)

### CHANGELOG — CP4

CP4 single-pass append 2026-04-25:
- Status header — `DRAFT CP3 (iterated 2026-04-24)` → `DRAFT CP4`. §0 status table — `(this revision)` annotation moves from CP3 row to CP4 row; CP3 row marked `locked CP3`.
- §14 added — Cross-references: §14.1 ADR matrix (4 ADRs); §14.2 Strategy parent-doc cross-refs (frozen CP3) — 13 rows mapping Strategy § to Tech Spec sections; §14.3 credential Tech Spec cross-refs — 13 rows including 2 soft-amendment-flagged rows; §14.4 Phase 1 register cross-refs — CR1-CR11 closure traceability table; §14.5 Phase 0 evidence — 7 line-pinned audit findings + T-disposition table + hygiene-T vs codemod-T naming caveat.
- §15 added — Open items resolution: §15.1 walkthrough of Strategy §5 + CP1-CP3 carry-forward (~25 items); §15.2 Q2 Terminate decision recorded (wire-end-to-end + cross-tenant boundary lock); §15.3 cross-crate soft amendment к credential Tech Spec §16.1.1 probe #7 (FLAGGED, NOT ENACTED); §15.4 cross-crate soft amendment к credential Tech Spec §15.7 (FLAGGED, NOT ENACTED); §15.5 ADR-0037 §1 SlotBinding shape amendment-in-place ENACTED with §15.5.1 enactment record; §15.6 §2.9-1 forward-track closed (no helper added); §15.7 CP1 hygiene T-disposition ratified (T4 + T5 + T9 + `nebula-redact`); §15.8 remaining minor open items walkthrough (deferred-with-trigger registry — 13 lettered (a)-(j) rows + 4 §-prefixed rows = 17 entries total; "14 rows" framing in earlier draft replaced after audit verification).
- §16 added — Implementation handoff: §16.1 PR wave plan presenting Q1 paths (a)/(b)/(c) per Strategy §6.5 — extends Strategy table with cascade-final criteria column; (c) viability gate cross-ref to §6.6 silent-degradation guard; §16.2 codemod ship plan with per-consumer automated step counts; §16.3 definition of done (PR-wave-level) — 7-item DoD checklist + implicit ADR status moves; §16.4 rollback strategy — Layer 1 feature-flag gate path (symmetric `unstable-action-scheduler` + `unstable-retry-scheduler`); Layer 2 reverse-codemod for transform regressions; Strategy §6.8 B'+ contingency activation as Strategy-level reversal layer; §16.5 pre-implementation checklist (cascade-final) — 7-item readiness checkbox list including ADR-0037 amendment-in-place verification.
- §15.5.1 ENACTMENT — separate edit on `docs/adr/0037-action-macro-emission.md` co-lands with this CP4 draft per §15.5 enactment record. Status moves `proposed` → `proposed (amended-in-place 2026-04-25)`; §1 SlotBinding shape rewritten to fold capability into `SlotType` enum per credential Tech Spec §9.4; CHANGELOG entry added at ADR top citing Tech Spec CP4 §15.5 as enactment trigger.

CP4 iteration append 2026-04-25 (post spec-auditor 11a REVISE — 3 🔴 mechanical pin-fixes + 3 🟠 + 3 actionable 🟡; security-lead 11b ACCEPT, no edits required):
- Status header — `DRAFT CP4` → `DRAFT CP4 (iterated 2026-04-25)`. Stays DRAFT until tech-lead freeze ratification.
- 🔴 #1 (file path) — §14.5 evidence list re-pinned from `01-current-state.md` to `01b-workspace-audit.md` for the workspace/CI/tooling-tier line ranges (§1 line 44 zeroize, §9 line 252-329 reverse-deps, §10 line 346-356 blast-radius, §11 line 376 lefthook, §11 line 379 deny.toml). §13.4.1 / §13.4.2 / §13.4.3 source-attribution paragraphs likewise re-pinned. §16.2 aggregate cite ("~55 file edits per Phase 0 §10 line 358") re-pinned to `01b-workspace-audit.md`. Closes spec-auditor 11a 🔴 #1.
- 🔴 #2 (supersede-stale §9.4) — §14.3 row 7 + row 8 re-pinned to `§15.8 (CP5 supersession of §9.4)` with explicit shape-preservation note (matching axes preserved; capability authority shifts plugin-metadata `capabilities_enabled` → registry-computed `RegistryEntry::capabilities`). §15.5 + §15.5.1 add supersession-acknowledgement paragraph and update §15.5.1 enactment-bullet citation. §3.1 SlotType `ServiceCapability` doc comment (lines 624-628) re-pinned: removes `cred.metadata().capabilities_enabled.contains(*capability)` reference; cites §15.8 + `RegistryEntry::capabilities`. ADR-0037 line 86 doc comment likewise re-pinned (separate Edit on ADR file; ADR amendment-in-place qualifier preserved). Closes spec-auditor 11a 🔴 #2.
- 🔴 #3 (flag name) — §16.4 line 2423 parenthetical corrected from `(symmetric \`unstable-action-scheduler\` + \`unstable-retry-scheduler\`)` to `(symmetric \`unstable-retry-scheduler\` + \`unstable-terminate-scheduler\`)`. Body text was already correct; parenthetical now matches. Aligns with §0.2 invariant 4 freeze on parallel-flag signature per CP1 §2.7.2 line 2478 + Strategy line 432-438. Closes spec-auditor 11a 🔴 #3.
- 🟠 #1 (CP1 path forwarded into CP4 §15.1) — §15.1 row "§3.1 — engine `ActionRegistry::register*` ... current host is `crates/engine/src/registry.rs`" amended in place: notes the file lives under `runtime/` submodule (`crates/engine/src/runtime/registry.rs`); CP1 line 642 + CP1 CHANGELOG line 2477 left untouched per CP1-locked discipline (audit footnote: `Glob crates/engine/src/registry*` returns no top-level match; the file's actual host is the `runtime/` submodule). Closes spec-auditor 11a 🟠 #1.
- 🟠 #2 (§14.3 row 11 status-cell consistency) — `Soft amendment — flagged, NOT enacted` → `Soft amendment — FLAGGED, NOT ENACTED` (bold form aligned to row 12 / probe #7 row). Closes spec-auditor 11a 🟠 #2.
- 🟠 #3 (cascade-queue.md hedge) — §16.1 (c) viability gate row + §16.5 cascade-queue checkbox both append "(or equivalent location — orchestrator picks at Phase 8 per Strategy §6.6 last paragraph)" hedge mirroring Strategy §6.6 framing; §16.5 also notes "the file does not exist on disk at draft time per audit verification". Closes spec-auditor 11a 🟠 #3.
- 🟡 #1 (§15.1 cross-line drift "2253-2263") — re-pinned from "line 2253-2263" to "CP3 CHANGELOG (current line 2598-2607)" per audit verification. Closes spec-auditor 11a 🟡 #1.
- 🟡 #4 (CHANGELOG row count §15.8) — count framing corrected from "14 rows" to "13 lettered (a)-(j) rows + 4 §-prefixed rows = 17 entries total" matching the actual table shape. Closes spec-auditor 11a 🟡 #4.
- 🟡 #5 (Phase numbering anchor) — §16 prelude gains "Phase numbering anchor" sentence pointing to Strategy §6 as the canonical Phase 6/7/8 definitions source; once at top of §16, no per-section repetition. Closes spec-auditor 11a 🟡 #5.
- 🟡 #2 / #3 / #6 — no edits required per audit (pure presentation; close with 🔴 #2 fix; optional symmetry move). Audit explicitly notes these as "current form is correct" or "could move ... but optional".
- security-lead 11b ACCEPT — confirms §16.3 DoD item 2 (4 must-have floor items as cascade-landing-PR obligations); §15 §6.x closure complete (CLOSED or FLAGGED-NOT-ENACTED with named owner/trigger/sunset); VETO retention reinforced across §0.2 invariant 3 + §6.2.3 + §16.3 item 2. No edits required from security-lead review.

### Handoffs requested — CP4

- **spec-auditor** — please **full cross-CP audit** before freeze: (a) cross-section consistency — every forward reference resolves to actual content (especially §14.1 ADR matrix → all four ADRs at correct status; §14.2 / §14.3 / §14.4 / §14.5 cross-refs all `grep`-able at cited line ranges); (b) every claim grounded in code (file:line) / canon / ADR / Strategy / Phase 0 audit / spike artefacts at line-number granularity; (c) §15.5.1 ADR-0037 amendment-in-place actually enacted in `docs/adr/0037-action-macro-emission.md` (verify status header + §1 SlotBinding shape + CHANGELOG entry); (d) §15.3 + §15.4 soft amendments are FLAGGED only (no inline credential Tech Spec edit performed by this Tech Spec); (e) terminology alignment with `docs/GLOSSARY.md`; (f) §16.1 (a)/(b)/(c) framing aligns with Strategy §4.2 + §6.5 verbatim shape; (g) §16.5 pre-implementation checklist captures every cascade-final precondition (Tech Spec ratified + 4 ADRs accepted + ADR-0037 amendment-in-place + canon §3.5 revision PR + cred CP6 cascade slot + soft amendments coordinated + concerns register clean); (h) line-count budget — CP4 §14-§16 within 200-300 line target.
- **tech-lead** — please ratify §15 closures (especially §15.5 ADR-0037 amendment-in-place enactment) and §16 framing (especially §16.3 7-item DoD checklist as the cascade-landing PR-wave-level definition of done, and §16.4 rollback strategy layer split). Solo-decider authority on §15.5 enactment (cascade-internal cross-cutting); CP4 freeze requires tech-lead explicit ratification. **Note:** §16.1 user-facing path framing is presented at Phase 8 cascade summary — Tech Spec presents, user picks; tech-lead ratifies the framing shape, not the user's pick.
- **security-lead** *(no new security surface introduced at CP4 §14-§16; §9.5 cross-tenant boundary already accepted CP3 — confirmation review)* — please confirm §16.3 DoD item 2 (security must-have floor 4 items) verifies all four CR4/S-J1, CR3/S-C2, ActionError sanitization, cancellation-zeroize tests as cascade-landing-PR-DoD obligations (none deferred to follow-up). VETO authority retained on shim-form drift in CR3 fix per `feedback_no_shims.md` + 03c §1.


