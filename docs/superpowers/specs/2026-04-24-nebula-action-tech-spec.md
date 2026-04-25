---
name: nebula-action tech spec (implementation-ready design)
status: DRAFT CP1 (iterated 2026-04-24)
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
| **DRAFT CP1** (this revision) | §0–§3 | Status, goals, trait contract, runtime model | active |
| **DRAFT CP2** | §4–§8 | Security floor, lifecycle, storage, observability, testing | pending |
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

**REJECT consolidation. Status quo (Option C) preserved.**

#### §2.9.6 Rationale

The analysis surfaces a **shape mismatch** that consolidation cannot honestly resolve:

1. **Trigger's input/output divergence is structural, not stylistic.** `TriggerAction` has `type Source: TriggerSource` because triggers are event-driven — the input shape is "event from a source," not "user-supplied parameter." Output is unit because triggers terminate by firing events, not by producing values. Forcing `Action<I, O>` parameterization onto a trigger requires lying (`type Input = ()`) or redundant projection (`<Source as TriggerSource>::Event` repeated in supertrait + body). Both violate `feedback_active_dev_mode.md` ("more-ideal over more-expedient") — the more-ideal shape is to let each trait read as what it actually is.

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

### Open items raised this checkpoint

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
