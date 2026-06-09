---
# budget-justified: ADR prose document — one contiguous decision record (crate-placement rationale + Bevy prior-art table + symmetric-API contract), not decomposable code
id: 0090
title: metadata-crate-placement-and-symmetric-api
status: accepted
date: 2026-06-09
supersedes: []
amends: []
superseded_by: []
tags: [crate-boundaries, metadata, layering, api, dx, prelude, compile-time, bevy]
related:
  - CLAUDE.md  # Layered Dependency Map
  - crates/core/CLAUDE.md  # vocabulary-only charter
  - crates/metadata/CLAUDE.md
  - crates/action/CLAUDE.md
  - crates/credential/CLAUDE.md
  - crates/resource/CLAUDE.md
  - crates/sdk/CLAUDE.md
---

# 0090. `nebula-metadata` stays a focused Core-layer crate; metadata DX via prelude, not a merge; symmetric by-value metadata API

## Status

**Accepted** (2026-06-09).

Two questions were posed: (1) should `nebula-metadata` be **merged into
`nebula-core`**, and (2) make the metadata accessor/constructor API
**symmetric** across `Action` / `Credential` / `Resource` for DX. The answer to
(1) is **no — keep it a separate crate**; (2) is **done**, and is independent of
(1).

## Context

`nebula-metadata` (Core tier) owns the shared catalog-leaf vocabulary:
`BaseMetadata<K>`, the `Metadata` trait (single required `base()` + 11 defaulted
accessors), `Icon` / `MaturityLevel` / `DeprecationNotice`, `validate_base_compat`,
and the `PluginManifest` container descriptor. It depends on `nebula-core` +
`nebula-schema` (only for the `ValidSchema` field on `BaseMetadata`) +
`nebula-error`. Consumers: `action`, `credential`, `resource`, `plugin`,
`plugin-sdk`, `sandbox`.

The merge proposal ("move it into `nebula-core`") has a real, non-obvious cost.
`nebula-core` is the **universal bottom crate** — every crate depends on it —
and its charter (`crates/core/CLAUDE.md`) is explicit: *"vocabulary only: no
validation (`nebula-schema`/`nebula-validator`) … do not pull those concerns
down here."* Merging `metadata` into `core` forces `core → nebula-schema →
{validator, expression}`. Measured blast radius: **5 crates depend on `core` but
not on `schema` today** — `storage-port` (chartered "no sqlx, no upward deps,
minimal", ADR-0072), `storage`, `tenancy`, `workflow`, `execution` — and none of
them use metadata. The merge would inject an expression-evaluator subtree into
all five, and into every `core`-relink fan-out. The cycle is clean (validator /
expression do not depend on `core`), and `cargo deny` needs no wrappers edit
(`core`/`metadata`/`schema` are all broadly importable) — so the merge is
*mechanically* possible; it is the *coupling* that disqualifies it.

Prior art (Bevy, ~59-crate workspace; tokio; large-Rust-workspace practice) is
unanimous and directly analogous:

| Nebula | Bevy analogue | Lesson |
|--------|---------------|--------|
| `nebula-metadata` (type-descriptor crate) | `bevy_reflect` (type metadata/reflection) | Kept a **separate** crate; the heavier core depends **on** it (`bevy_ecs → bevy_reflect`), never merged — *"would force every reflection consumer to take a dependency on ECS."* |
| `nebula-core` (universal bottom) | `bevy_platform` / the deleted `bevy_core` | Bevy **deleted** `bevy_core` (issue #16892) — *"a utilities crate, which we would like to avoid"* — dissolving its types **outward** into focused homes. Shared types move **out of** catch-all bottoms, never **into** them. |
| "single clean import" DX | `bevy` + `bevy_internal` umbrella, `bevy::prelude` | DX is a **re-export** problem (facade/prelude), solved **without** collapsing the crate graph. |

The one case Bevy *did* fold a type into a bottom crate (`Name → bevy_ecs`) was
gated on two tests: universally applicable **and** "wouldn't change the
dependency graph." The metadata-into-core merge fails **both** (5 non-users; it
injects a heavy subtree).

Crucially, the symmetric-API goal is **independent** of crate placement: it
edits `ActionMetadata` / `CredentialMetadata` / `ResourceMetadata` in their own
crates, which reference `BaseMetadata` by import path regardless of where
`BaseMetadata` lives. The merge would only change *where `BaseMetadata`
physically lives* — a cosmetic import path — at the coupling cost above.

## Decision

**D1 — Do not merge.** `nebula-metadata` remains a focused Core-layer crate.
`nebula-core` keeps its vocabulary-only charter; the 6-layer DAG and the 5
non-user crates are untouched.

**D2 — Symmetric by-value accessor.** `Action::metadata()` changes from
`fn metadata() -> &'static ActionMetadata` to **`fn metadata() -> ActionMetadata`**
(by value), matching `Credential::metadata()` and `Resource::metadata()` which
were already by-value. This deletes the `static`/`OnceLock` boilerplate every
action author hand-wrote. The engine-internal object-safe surfaces
(`ActionFactory`, the `Erased*` traits, the typed→Handler adapters) keep
returning `&ActionMetadata` backed by an internal cache (`OnceLock` on
long-lived factories, a stored field on per-dispatch adapters) — a cold-author /
hot-engine split, not part of the symmetric public contract.

**D3 — Unified constructors where semantically sound.** All three expose
`new(…)`, `for_<entity>::<T>()` (schema derived from the companion type), and
`from_key(&key)`. `Action`'s four `for_stateless` / `for_stateful` /
`for_paginated` / `for_batch` collapse into a single `for_action::<A>` (every
`Action` has `Input: HasSchema`). `Resource` is the reference shape and is
unchanged.

`Credential` is a **deliberate, documented partial exception**: its required
`pattern` (`AuthPattern`, no meaningful default) makes a `from_key` constructor
meaningless, and its imperative `Option`-field `builder()` (used on the
icon/doc-url path) is *not* reshaped to the seeded `builder(key, name,
description)` form of the other two. The accessor (by value) and `for_credential`
/ `new` are symmetric; the constructor difference is intrinsic to the type, not
drift, and is documented on `CredentialMetadata`.

**D4 — DX via prelude, not a merge.** The "single clean import" is delivered
through `nebula-sdk`'s existing facade: `nebula_sdk::prelude` now re-exports the
shared `Metadata` trait + `BaseMetadata` + value types and all three
`XMetadata`, so `use nebula_sdk::prelude::*` yields the full symmetric metadata
surface from one import (the Bevy/tokio way). `nebula-sdk` gains a direct
`nebula-metadata` dependency (Core → API, downward; no `deny.toml` change).

## Consequences

- **Positive.** `nebula-core` stays the lean universal vocabulary crate its
  charter promises; the enforced layer DAG and the 5 innocent `core`-dependents
  are untouched; no schema/validator/expression subtree enters the universal
  relink fan-out. Action authors lose ~4 lines of `OnceLock` boilerplate per
  impl. The three catalog leaves now present an identical `metadata()` accessor
  and a near-identical constructor set; the prelude gives one-import DX.
- **Breaking (0.x, intended).** `Action::metadata()` return type changed —
  every hand-written `impl Action` (and the `#[derive(Action)]` macro, the
  `simple_action!` macro) was migrated to by-value. `ActionMetadata`'s four
  `for_*` constructors collapsed to `for_action`. No external semver guarantee
  is in force pre-1.0.
- **Neutral.** `nebula-metadata` keeps its `schema` edge; the option to push it
  lower by genericizing `BaseMetadata.schema` was considered and rejected as
  unnecessary scope (no consumer needs `BaseMetadata` without a schema today).
- **Guidance recorded.** Crate count is not the metric for this workspace — DAG
  shape is (wide/layered, thin stable bottom, heavy deps at the leaves). Merge
  only *false* splits (always co-used, same dependency weight and change
  cadence, no DAG-width gain). `sdk` / `api` are functional top crates, **not**
  umbrellas; the workspace's facade need is met by the `sdk` **prelude**, not a
  new umbrella crate.

## Alternatives considered

- **(B) Merge into `nebula-core` + rewrite the charter.** Rejected: the
  "god-core" anti-pattern; injects `schema → {validator, expression}` into 5
  non-user crates and the universal relink fan-out, to change a cosmetic import
  path. Requires inverting `core`'s own stated charter — a signal of fighting
  the architecture.
- **(C) Merge into `nebula-schema` instead.** Less wrong (spares `core`) but
  still collapses two genuinely separate concerns with different change cadence
  (the validation/expression engine vs. the catalog-descriptor vocabulary), and
  serves the DX goal no better than the prelude.
- **(D) Genericize / make optional `BaseMetadata.schema` so metadata can sit
  lower without the validator/expression weight.** A valid technique, but
  unnecessary: nothing keeps `metadata` from where it is, and `schema` is a
  required, used field today. Banked for if a truly minimal consumer ever needs
  `BaseMetadata` without a schema.
- **(A-partial) Force full constructor symmetry onto `Credential`
  (`from_key` + seeded `builder`).** Rejected for the documented reasons in D3:
  `from_key` is semantically invalid without an `AuthPattern`, and the builder
  reshape is high-churn (≈15 call sites, incl. in-flight `credential-builtin`)
  for marginal symmetry.
