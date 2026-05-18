---
id: 0014
title: dynosaur-macro
status: superseded
date: 2026-04-19
supersedes: []
superseded_by: [0024]
tags: [traits, async, dyn-compatibility, macros, api-design]
related:
  - docs/STYLE.md#1-idioms-we-use
  - docs/adr/0010-rust-2024-edition.md
  - docs/adr/0024-defer-dynosaur-migration.md
linear:
  - NEB-152
---

# 0014. `dynosaur` for `dyn`-compatible async traits

## Context

Many Nebula trait seams need **both** static dispatch (for the hot path in
the engine / storage layer) **and** `dyn Trait` objects (for the integration
surface — stored handlers, plugin broker boundaries, CLI command dispatch).
Async methods in traits make this awkward:

- **`async fn` in trait** (AFIT) is stable since Rust 1.75 and is the
  ergonomic form we want authors to read and write. It is the static-dispatch
  path.
- AFIT traits are **not** directly `dyn`-compatible. Naïve
  `dyn MyTrait` fails to compile because the compiler cannot pick a concrete
  `impl Future` return type.
- The historical fix — `#[async_trait]` — rewrites every `async fn` to
  return `Pin<Box<dyn Future + Send>>`, which boxes the future **even at
  call sites that never needed `dyn`**, and fights against stable AFIT.

The `dynosaur` macro solves this cleanly: author the trait in idiomatic
AFIT form, apply `#[dynosaur::dynosaur(DynMyTrait)]`, and the macro
generates a second `dyn`-compatible trait whose methods return
`Pin<Box<dyn Future + 'a>>` (or similar). Static callers keep the
zero-cost AFIT path; dynamic callers name the generated `DynMyTrait`.

With MSRV 1.94 (see [ADR-0010](./0010-rust-2024-edition.md)) stable AFIT
is available workspace-wide and this pattern becomes our default.

## Decision

1. **`dynosaur` is the approved mechanism for `dyn`-compatible async
   traits** across the workspace. It replaces `#[async_trait]` in new
   code.
2. **Author the trait in AFIT form.** No `async fn` rewrite, no
   `impl Future` return types unless there is a specific reason (GATs on
   the future, cancellation tokens, etc.). Example:

   ```rust
   #[dynosaur::dynosaur(DynExecutionRepo)]
   pub trait ExecutionRepo: Send + Sync {
       async fn load(&self, id: ExecutionId) -> Result<ExecutionRow, RepoError>;
       async fn transition(&self, row: ExecutionRow) -> Result<(), RepoError>;
   }
   ```

   `&dyn ExecutionRepo` is not valid — the engine uses
   `&dyn DynExecutionRepo` where `dyn` dispatch is needed (e.g. repo
   registries, swappable storage backends), and `impl ExecutionRepo` for
   static call sites (hot path, per-run dispatch).

3. **Static dispatch is the default.** Name `impl TraitName` (or a
   concrete type) in signatures. Use `dyn DynTraitName` only at a
   boundary that stores the trait object (registry, broker, cross-plugin
   boundary).

4. **Do not reintroduce `#[async_trait]`.** Existing call sites that still
   use it are tech debt and should be migrated as they are touched — not
   in a mass refactor.

5. **Keep `Send + Sync` bounds explicit on the trait.** `dynosaur`
   generates the `Dyn*` version preserving these bounds; consumers get
   a compile error if they try to cross thread boundaries with a trait
   missing `Send`. Do not rely on macro defaults.

6. **MSRV gate.** `dynosaur` requires stable AFIT (≥ 1.75); we are on
   1.94 (ADR-0010) so the floor is not an issue. If the workspace MSRV
   ever drops below AFIT support, this ADR must be superseded, not edited.

## Consequences

**Positive**

- Zero-cost static dispatch is preserved at hot-path call sites. The
  engine's per-run dispatch loop does not pay `Box<dyn Future>` tax on
  every step.
- Trait authors write idiomatic AFIT — new contributors do not need to
  learn the `#[async_trait]` desugaring to read core traits.
- `Dyn*` naming keeps the cognitive boundary explicit: a function
  signature that says `DynExecutionRepo` is a deliberate choice to pay
  for dynamic dispatch.

**Negative**

- Consumers see two names per trait (`ExecutionRepo` + `DynExecutionRepo`).
  The `Dyn` prefix is conventional but adds a lookup step when navigating
  code.
- `dynosaur` is a young crate. API breakage would touch every seam using
  it; pinning exact versions in `Cargo.toml` workspace deps is a
  prerequisite (see "Follow-ups").

**Neutral**

- Runtime cost of `dyn DynTraitName` is the usual indirect-call + `Box`
  allocation per async call. It is the same cost `#[async_trait]` forced
  on every call site; with `dynosaur` it is opt-in.

## Alternatives considered

- **Stay on `#[async_trait]`.** Reject. Mandates `Box<dyn Future>` at
  every call site regardless of dispatch kind; fights stable AFIT.
- **Hand-write parallel `DynFoo` wrappers.** Reject. Drift between the
  static and dynamic forms is inevitable; macro-generation removes the
  class of bug.
- **Avoid `dyn` trait objects entirely (pure generics).** Reject.
  Plugin registries, repo swaps, and cross-crate dispatch need stored
  trait objects; forcing everything monomorphic balloons compile time
  and binary size without a real ergonomic win.
- **`trait-variant`.** Similar goal, but as of 2026 its feature set is
  narrower and it pre-dates some stabilized patterns `dynosaur` targets.
  Re-evaluate when / if it grows to cover our cases.

## Style guidelines (summary)

Full rule lives in [`docs/STYLE.md §1 — Idioms we use`](../STYLE.md#1-idioms-we-use):

- Trait is authored in AFIT form; `#[dynosaur::dynosaur(DynFoo)]` generates
  the `dyn`-compatible sibling.
- Static signatures: `impl Foo`. Dynamic boundary: `dyn DynFoo`.
- Never add `#[async_trait]` in new code.

## Follow-ups

- `Cargo.toml` workspace deps: pin `dynosaur = "<exact>"` once adopted
  so a semver bump is an intentional PR, not an accidental `cargo update`.
- Migrate existing `#[async_trait]` trait definitions opportunistically;
  track as low-priority cleanup, not a blocking refactor.
- When `dynosaur` (or an upstream RFC) no longer requires the `Dyn`
  sibling pattern — because the compiler natively supports `dyn Trait`
  on AFIT — open a follow-up ADR and migrate the trait names back to
  the single form.
