---
# budget-justified: ADR prose document — one contiguous decision record (the deliberate #[async_trait] override for nebula-resource and the carve-out from the workspace "prefer native async-fn-in-trait" idiom), not decomposable code
id: 0094
title: nebula-resource-async-trait-policy
status: accepted
date: 2026-06-12
supersedes: []
amends: []
superseded_by: []
tags: [resource, async-trait, object-safety, idiom, topology, dx]
related:
  - crates/resource/src/topology/contract.rs  # the open Topology<R> trait — #[async_trait]
  - crates/resource/src/resource.rs  # the Provider trait — #[async_trait]
  - docs/adr/0093-resource-teardown-contract-and-foolproofing.md  # the teardown contract this serves
---

# 0094. nebula-resource deliberately uses `#[async_trait]`

## Status

**Accepted** (2026-06-12, owner-directed).

This ADR records a deliberate, standing exception: the `nebula-resource`
lifecycle traits (`Provider`, the open `Topology<R>`, and the per-pattern hook
traits `PoolProvider` / `ResidentProvider` / `BoundedProvider`) use
`#[async_trait]` rather than native async-fn-in-trait (AFIT) / RPITIT. The
workspace coding standard prefers native AFIT on edition 2024; this carve-out
says **do not "modernize" nebula-resource back to native AFIT** without
re-opening this decision.

## Context

The workspace standard (and the path-scoped Rust review persona) prefer native
`async fn` in traits over the `async-trait` crate on Rust ≥ 1.94 / edition 2024,
because AFIT removes a per-call `Box<dyn Future>` allocation and a dependency.
That preference is correct **for the common case**: a trait reached
monomorphically, whose futures do not need to be named or stored behind `dyn`.

`nebula-resource` is not the common case:

1. **Object safety is load-bearing.** The framework erases every registered
   resource to `Arc<dyn ManagedHandle>` (the registry is a heterogeneous map of
   resource rows; `Manager::acquire_any` dispatches through the erased handle).
   `ManagedHandle`'s async methods must therefore be object-safe. Native AFIT
   does **not** produce an object-safe trait whose futures are provably `Send`
   without `dyn*`/RTN machinery that is not yet stable; `#[async_trait]`'s
   boxed-future lowering gives object safety today, for free.

2. **The hooks the framework awaits must be `Send`.** A hook future
   (`Provider::create`, `Topology::on_release`, `BoundedProvider::reset`, …) is
   awaited *inside* the framework's own boxed `Topology` future, which is
   `Send + 'static` so it can be driven on the multi-threaded runtime and held
   across the acquire loop. Native AFIT does not let a trait *require* its
   method futures be `Send`; `#[async_trait]` bakes the `Send` bound in.

3. **The cost is negligible here.** Every lifecycle method does real I/O
   (open a connection, run a health probe, flush on teardown). One boxed
   future per call is lost in the noise next to a network round-trip; this is
   the opposite of a hot, allocation-sensitive inner loop.

4. **Migration off is trivial and local.** `#[async_trait]` is ubiquitous and
   trusted; if a future Rust release makes object-safe `Send` AFIT ergonomic
   (stable RTN / `dyn*`), removing the attribute is a mechanical, in-crate
   change with no API-shape impact on resource authors.

## Decision

Keep `#[async_trait]` crate-wide in `nebula-resource` for the
lifecycle/topology trait surface. The sync hooks (`try_reserve`,
`slot_instance`, `into_instance`, `phase`, `load`, `pools`, `check_cost`,
`teardown_budget`, …) stay plain sync; only the genuinely-async hooks carry the
boxed future.

This is a **carve-out from**, not a repeal of, the workspace
prefer-native-AFIT standard. The standard still governs every other crate.

## Consequences

- Reviewers (human and the path-scoped Rust persona) must treat an
  `#[async_trait]` on a `nebula-resource` lifecycle trait as **intended**, not
  as tech debt to flag. The carve-out is mirrored into the project's
  idiom-currency note so future agent sessions do not churn it back.
- If stable Rust gains object-safe, `Send`-bounded async-fn-in-trait, this ADR
  should be revisited and likely superseded — the migration is mechanical.
- No resource-author-facing API changes: authors already write `async fn`
  bodies in their `impl`s exactly as they would under native AFIT.
