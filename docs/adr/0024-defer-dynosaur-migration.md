---
id: 0024
title: defer-dynosaur-migration
status: accepted
date: 2026-04-20
supersedes: [0014]
superseded_by: []
tags: [traits, async, dyn-compatibility, dependencies, api-design]
related:
  - docs/adr/0014-dynosaur-macro.md
  - docs/superpowers/specs/2026-04-19-rust-feature-adoption-plan.md
  - docs/STYLE.md#1-idioms-we-use
linear: []
---

# 0024. Defer `dynosaur` migration — keep `#[async_trait]` for `dyn`-consumed traits

## Context

[ADR-0014](./0014-dynosaur-macro.md) approved `dynosaur` as the mechanism
for `dyn`-compatible async traits across the workspace. Since then, three
facts changed:

1. **Phase 2 of the 1.75-1.95 feature-adoption plan landed** (PR #507,
   #508, #509). Every trait with **zero** `dyn Trait` call sites — 20
   traits across `runtime`, `credential`, `storage/repos/*` — moved to
   native AFIT (`fn -> impl Future<Output = ...> + Send`). The remaining
   `#[async_trait]` surface is **14 traits, 100 % of whose call sites
   are `Arc<dyn …>`** (per the plan's inventory). `dynosaur`'s core
   selling point — letting generic callers keep the zero-cost path
   while dyn callers pay the box — **has no call sites to serve** for
   these 14.
2. **`async_fn_in_dyn_trait` is still experimental on nightly.** The
   rust-lang tracking issue ([rust-lang/rust#133119][rfc-133119]) lists
   RFC as *not yet accepted* and stabilization is not scheduled. An
   independent project (`assapir/golem` issue #8, 2026-02-18) tested
   nightly 1.95 and reported the feature is marked **incomplete** and
   still emits `E0038: trait is not dyn compatible`. Waiting is not a
   weeks-away bet, but the wait cost for Nebula is **zero**: `async-trait`
   keeps compiling.
3. **`dynosaur` adoption stayed narrow.** At the time of this ADR the
   crate is `v0.3.0`, last released 2025-07-16 (277 days ago, marked
   "aging" by crate-health metrics), with **13 reverse dependencies** on
   crates.io — all niche projects. Surveyed peers:
   - `databend` (query engine, analogous domain) — `async-trait` on
     every `Arc<dyn Catalog>` / `Arc<dyn Table>` seam.
   - `apache/opendal` (storage abstraction, closest analog to
     `storage/repos/*`) — hand-rolls a `Foo` / `FooDyn` pair with
     manual `BoxedFuture` returns; does **not** depend on `dynosaur`.
   - `tokio-rs/axum` — native AFIT for extractors/handlers, manual
     `Pin<Box<dyn Future>>` at the few trait-object sites.
   - `launchbadge/sqlx` — `BoxFuture` / `BoxStream` in trait
     signatures plus `async-trait` in lock.

   No major peer adopted `dynosaur`.

Separately, an audit of `dtolnay/async-trait` upstream issues (7 open at
the time of writing, over a ~5-year history) shows every open report is
a sophisticated generic-lifetime edge case (elided lifetimes across
tuples, higher-rank closure bounds, generic tuple-in-Send shapes) —
none of which match Nebula's trait signatures (simple `async fn foo(&self,
args: T) -> Result<U, E>` per repo method). Risk of hitting an upstream
bug with our shapes is negligible.

## Decision

1. **`#[async_trait]` is the approved mechanism for the 14 remaining
   `dyn`-consumed async traits in Nebula.** The crate stays in
   `[workspace.dependencies]`. Phase 3 of the feature-adoption plan
   (`dynosaur` migration) **is not executed** and is dropped from the
   rollup.

   The 14 traits covered by this decision are the `dyn`-consumed ones
   inventoried in the plan's §Inventory: `TriggerHandler`,
   `CredentialAccessor`, `StatelessHandler`, `ControlQueueRepo`, legacy
   `ExecutionRepo` (`crates/storage/src/execution_repo.rs`), legacy
   `WorkflowRepo` (`crates/storage/src/workflow_repo.rs`),
   `ResourceHandler`, `ResourceAccessor`, `ExecutionEmitter`,
   `StatefulHandler`, `TriggerScheduler`, `AgentHandler`,
   `StatefulCheckpointSink`, `BlobStorage`, `SandboxRunner`. (Phase 2
   has already migrated every non-dyn async trait.)

2. **Do not add `dynosaur` to `[workspace.dependencies]`.** A 0.3 crate
   with 13 reverse-deps does not clear the bar for a load-bearing
   workspace dependency.

3. **Native AFIT remains the default for new traits that are not
   `dyn`-consumed.** Phase 2's outcome stands: `fn foo(&self, ...) ->
   impl Future<Output = ...> + Send` is the house style for traits that
   live behind generics only. Do not regress to `#[async_trait]` on
   these.

4. **`#[async_trait]` is not forbidden for new code when the trait is
   `dyn`-consumed from day one.** The "never introduce `#[async_trait]`"
   rule in [ADR-0014 §Style guidelines](./0014-dynosaur-macro.md) is
   rescinded. Match the surrounding seam: if the author introduces a
   new trait whose concrete consumers will hold it as `Arc<dyn Foo>`,
   `#[async_trait]` is the correct choice. If the trait has generic
   consumers only, use native AFIT.

5. **Re-evaluation triggers.** This ADR is revisited — and likely
   superseded with a follow-up migration — when **any** of the following
   becomes true:

   - `async_fn_in_dyn_trait` stabilizes on stable Rust and Nebula's
     MSRV floor reaches that version. At that point the migration is
     trivial: delete `#[async_trait]` annotations, traits keep
     compiling as `dyn`-compatible natively. No sibling macro required.
   - A real hot-path generic consumer appears for one of the 14 traits
     (e.g. the engine's per-step dispatch loop learns to call a
     `WorkflowRepo` method directly with a concrete type). In that
     narrow case, reach for the **opendal hand-rolled pair pattern**
     (`Foo` native AFIT + `FooDyn` manual `Pin<Box<dyn Future>>`
     sibling) for **that trait only** — not a workspace-wide macro
     migration. This is the bounded-scope fallback; a workspace-wide
     flip back to `dynosaur` remains out of scope.
   - `async-trait` upstream becomes unmaintained (e.g. no release or
     issue response for ≥ 12 months while Rust edition moves forward).
     The low open-issue volume and `dtolnay`'s stewardship make this
     tail-risk, not a live concern.

## Consequences

**Positive**

- **Zero migration work.** The 14 `dyn`-consumed traits keep compiling
  as-is. No 5-PR rollup, no ADR-0023 scheduling pressure on
  `CredentialAccessor`.
- **One async macro in tree, not two.** `async-trait` is already a
  transitive dep via tokio-adjacent crates; not adding `dynosaur`
  keeps the dep graph flatter.
- **Clean supersession path.** When stdlib stabilizes, the migration is
  mechanical — delete attributes; no `DynFoo` renaming sweep across
  consumer crates.

**Negative**

- **Stylistic split.** Phase 2 zero-dyn traits are native AFIT; Phase 3
  dyn traits remain `#[async_trait]`. Readers navigating between a
  `repos/*.rs` trait (native) and a `workflow_repo.rs` legacy trait
  (`#[async_trait]`) see two forms. Canon §11.6 (advertise only what
  is honored) is not violated — both patterns compile and work — but
  code-review ergonomics take a hit. Acknowledged; worth it versus the
  5-PR alternative.
- **`async-trait` boxes generic call sites too.** If a generic call
  site appears on one of the 14 traits later, we pay a `Box` we would
  not pay with `dynosaur`'s static-dispatch path. Re-evaluation
  trigger (2) above covers this.
- **No style unification until stdlib stabilizes.** We are explicitly
  betting on stdlib catching up rather than a third-party macro. If
  `async_fn_in_dyn_trait` stalls for multiple years, the stylistic
  split persists.

**Neutral**

- Runtime cost is **identical** to the `dynosaur` route for the 14
  target traits (every call site is `Arc<dyn>`; both macros box).

## Alternatives considered

- **Execute ADR-0014 as originally written (Phase 3 dynosaur migration).**
  Reject. Unchanged facts: `dynosaur` 0.3 / aging / 13 reverse-deps /
  zero major-peer adoption do not justify a new workspace dependency
  whose core value prop (dual generic + dyn) currently serves zero
  call sites.
- **Hand-rolled opendal-style pair workspace-wide.** Reject as a
  migration; retain as a per-trait fallback under re-evaluation
  trigger (2). Cost is ≈ 14 traits × 8 methods × 2 signatures = ~224
  method signatures of boilerplate, for no current-day win over
  `async-trait`. Opendal made this choice because their trait surface
  is ~3 trait families that are both heavily generic and heavily dyn;
  our 14 are dyn-only.
- **Manual `Pin<Box<dyn Future>>` without the generic sibling.**
  Reject. Identical runtime cost to `async-trait` with worse
  ergonomics (lifetime bookkeeping at every method, `Box::pin` wrap
  in every impl body). No advantage.
- **`trait-variant`.** Reject. Does not solve `dyn`-compatibility;
  ADR-0014 already considered and rejected it for that reason. Still
  true.
- **Wait for `async_fn_in_dyn_trait` without a formal decision.**
  Reject. The in-tree status of ADR-0014 would silently rot the
  spec and block Phase 3 of the adoption plan without an explicit
  L2-invariant update. This ADR exists precisely to close that gap.

## Follow-ups

- Mark ADR-0014 `superseded` in frontmatter (body immutable per
  `docs/adr/README.md`).
- Update [`docs/STYLE.md §1`](../STYLE.md#1-idioms-we-use) to reflect
  the decision — `#[async_trait]` is permitted for `dyn`-consumed
  async traits; native AFIT is the default for generic-only async
  traits.
- Update the adoption plan
  ([`docs/superpowers/specs/2026-04-19-rust-feature-adoption-plan.md`](../superpowers/specs/2026-04-19-rust-feature-adoption-plan.md)):
  Phase 3 goes to **"cancelled (superseded by ADR-0024)"**. Phase 4
  (use<> precise-capture) stays — it remains inline work for Phase 2
  follow-ups. Phase 5 polish items are unaffected.
- Watch rust-lang/rust#133119 and RFC discussions. When the feature
  reaches beta on stable, open a follow-up ADR for the
  `async-trait` → native dyn-AFIT migration.
- **Async closures (RFC 3668, stable 1.85) — Phase 5 production scope
  deferred.** Site audit of the 55 `Box::pin(async move)` matches in
  the 1.75-1.95 adoption rollup (2026-04-20) found that 13 of them wrap
  the `ActionExecutor` type alias
  (`crates/sandbox/src/runner.rs:65-76`, `Arc<dyn Fn(...) -> Pin<Box<dyn
  Future>>>`). Converting these requires `Arc<dyn AsyncFn(...)>` to be
  object-safe, which is **not stable on Rust 1.95** — tracked at
  [rust-lang/rust#132633][rfc-132633] with no stable target. Only
  tests/benches on already-generic callers converted as part of the
  rollup closeout. Revisit the production scope — and the question of
  whether `ActionExecutor` should grow its own ADR to retire the `Fn
  -> Pin<Box<Future>>` shape — when **either** of the following
  happens:
  - `async_fn_in_dyn_trait` /
    [rust#132633][rfc-132633] stabilizes and Nebula's MSRV reaches
    that version — at which point the sandbox alias becomes
    `Arc<dyn AsyncFn(...)>` with no boilerplate change at consumer
    sites.
  - A concrete sandbox-facing requirement (new runner shape,
    alternate transport, sandbox v2) motivates opening that ADR
    independently — in which case async closures fold into that
    migration rather than a standalone chip.

[rfc-133119]: https://github.com/rust-lang/rust/issues/133119
[rfc-132633]: https://github.com/rust-lang/rust/issues/132633
