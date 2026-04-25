---
name: nebula-action tech spec (implementation-ready design)
status: DRAFT CP2 (iterated 2026-04-24)
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
| **DRAFT CP2** (this revision) | §4–§8 | Macro emission, test harness, security floor, lifecycle, storage | active |
| **DRAFT CP3** | §9–§13 | Codemod design, retry-scheduler chosen path, migration, interface | pending |
| **DRAFT CP4 → FROZEN CP4** | §14–§16 | Open items, accepted gaps, handoff, implementation-path framing for Phase 8 user pick | pending |

Inputs are **frozen** at this draft point: Strategy frozen at CP3 (commit pending; see status header of [`2026-04-24-action-redesign-strategy.md`](2026-04-24-action-redesign-strategy.md)); ADR-0036 / ADR-0037 / ADR-0038 in `proposed` (status moves to `accepted` upon Tech Spec ratification — ADR-0036 §Status / ADR-0037 §Status / ADR-0038 §Status); Phase 4 spike PASS at commit `c8aef6a0` (worktree-isolated; see [spike NOTES](../drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md) §5).

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
    /// service identity AND a capability projection. Engine matches both
    /// `cred.metadata().service_key == Some(*service)` AND
    /// `cred.metadata().capabilities_enabled.contains(*capability)`
    /// (credential Tech Spec §9.4 line 2467-2470).
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
