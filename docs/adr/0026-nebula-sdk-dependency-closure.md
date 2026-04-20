---
id: 0026
title: nebula-sdk-dependency-closure
status: proposed
date: 2026-04-20
supersedes: []
superseded_by: []
tags: [workspace, packaging, publish, crates-io, semver]
related:
  - docs/adr/0021-crate-publication-policy.md
  - crates/sdk/Cargo.toml
  - crates/sdk/src/lib.rs
  - docs/MATURITY.md
linear: []
---

# 0026. nebula-sdk dependency closure for crates.io publication

## Context

[ADR-0021](./0021-crate-publication-policy.md) flipped the workspace default
to `publish = false` and named the **initial published set**:

1. `nebula-sdk` — integration-author façade
2. `nebula-core` — primitive ID types re-exported via sdk
3. `nebula-error` — shared error taxonomy
4. `nebula-resilience` — standalone retry / bulkhead primitives
5. `nebula-credential` — security-primitives surface (`KeyProvider` seam)

Plus the proc-macro companions `nebula-error-macros` and
`nebula-credential-macros`.

ADR-0021 §Consequences-Negative flagged the follow-up this ADR closes:

> `nebula-sdk`'s current dependencies on `nebula-action`, `nebula-credential`,
> `nebula-resource`, `nebula-schema`, `nebula-workflow`, `nebula-plugin`,
> `nebula-validator` (see `crates/sdk/Cargo.toml`) would force publication of
> those crates, or their removal from the published sdk's dependency set:
> crates.io rejects publishing a crate whose dependencies are non-registry
> (path-only / unpublished), regardless of whether their types appear in the
> publisher's public API.

A read of `crates/sdk/Cargo.toml` and `crates/sdk/src/lib.rs` confirms:

- sdk directly depends on **eight** `nebula-*` crates: `nebula-core`,
  `nebula-action`, `nebula-workflow`, `nebula-schema`, `nebula-credential`,
  `nebula-plugin`, `nebula-resource`, `nebula-validator`.
- sdk re-exports **all of them** at the crate root (`pub use nebula_action;`
  …) and again in `prelude` (~80 symbols pulled in by `use
  nebula_sdk::prelude::*;`).
- sdk's builders and test harness return upstream types:
  `ActionBuilder::build() -> nebula_action::ActionMetadata`,
  `WorkflowBuilder::build() -> nebula_workflow::WorkflowDefinition`,
  `TestRuntime::run_stateful<A: nebula_action::StatefulAction>(...)` — the
  bounds on the public runtime come straight from `nebula-action`.
- sdk's **macros** expand into upstream namespaces inside downstream crates:
  - `simple_action!` → `$crate::nebula_action::{Action, ActionMetadata,
    ActionDependencies, StatelessAction, Context, ActionResult,
    ActionError}` and `$crate::nebula_core::action_key!`
  - `params!` → `$crate::nebula_schema::value::FieldValues`

Removing any of these directly deletes documented user-facing behavior — the
macros, the builder return types, the `TestRuntime` bounds — so "tighten
sdk's re-exports" cannot mean "drop them from `Cargo.toml`" without a rewrite
of each affected module.

### The closure is wider than ADR-0021 anticipated

A second finding surfaced while sizing this follow-up: the published-set
problem is **not** limited to sdk's six non-registry direct deps.
`cargo publish` rejects a crate whose `[dependencies]` include any path-only
entry with no `version = "…"` pointing at a registry release — so every
transitive dep of every published crate must itself be on the registry.

Walking the workspace `Cargo.toml` files:

| Published in ADR-0021 | Path-only `nebula-*` deps                                                       |
| --------------------- | ------------------------------------------------------------------------------- |
| `nebula-credential`   | `metadata`, `eventbus`, `telemetry`, `metrics`, `schema` (+ `core`, `resilience`) |
| `nebula-core`         | —                                                                               |
| `nebula-error`        | —                                                                               |
| `nebula-resilience`   | —                                                                               |
| `nebula-sdk`          | `action`, `workflow`, `schema`, `plugin`, `resource`, `validator` (+ `core`, `credential`) |

And once we add sdk's six:

| New entrant       | Path-only `nebula-*` deps                                                       |
| ----------------- | ------------------------------------------------------------------------------- |
| `nebula-action`   | `metadata`, `credential`, `schema`, `resource` (+ `core`)                       |
| `nebula-workflow` | — (only `core`)                                                                 |
| `nebula-schema`   | `validator`                                                                     |
| `nebula-plugin`   | — (only `core`)                                                                 |
| `nebula-resource` | `metrics`, `resilience`, `metadata`, `schema`, `telemetry` (+ `core`)           |
| `nebula-validator`| — (only `core`)                                                                 |

The first-release closure for ADR-0021's own initial set **already requires**
`nebula-metadata`, `nebula-eventbus`, `nebula-telemetry`, and `nebula-metrics`
on the registry — without any sdk involvement — because `nebula-credential`
pulls them in. Adding sdk pulls in everything else. The closure lands at
**fifteen production crates**, not eleven.

### Canon and maturity constraints

`PRODUCT_CANON.md §3.5` names the five integration primitives: **Action,
Credential, Resource, Schema, Plugin**. `crates/sdk/README.md` restates this
as a contract ("[L1-§3.5] The SDK surface covers the five integration
concepts … adding a sixth requires canon revision (§0.2)."). **Dropping any
of the five from sdk is a canon violation**; `nebula-validator` is the only
direct dep of sdk that is *not* one of the five.

Current `docs/MATURITY.md` rows for the closure (API stability column):

| Crate              | API stability |
| ------------------ | ------------- |
| `nebula-action`    | frontier      |
| `nebula-credential`| frontier *(already published under ADR-0021)* |
| `nebula-eventbus`  | stable        |
| `nebula-metadata`  | frontier      |
| `nebula-metrics`   | stable        |
| `nebula-plugin`    | partial       |
| `nebula-resource`  | frontier      |
| `nebula-schema`    | frontier      |
| `nebula-sdk`       | partial *(already published under ADR-0021)* |
| `nebula-telemetry` | stable        |
| `nebula-validator` | frontier      |
| `nebula-workflow`  | stable        |

ADR-0021 §Consequences-Neutral calls `frontier` + `publish = true` a red
flag. The initial set already carries one such crate (`nebula-credential`);
closing the closure adds five more frontier crates and one partial. This ADR
treats that gap explicitly rather than silently.

## Decision

**Option C (hybrid): publish the full sdk dependency closure, tighten the
sdk re-export surface where canon allows, and commit the first release train
to a `0.x` pre-stable semantic.**

### 1. Tighten: drop `nebula-validator` from sdk's direct deps and prelude

- Remove `nebula-validator = { path = "../validator" }` from
  `crates/sdk/Cargo.toml`.
- Remove `pub use nebula_validator;` from `crates/sdk/src/lib.rs`.
- Remove `pub use nebula_validator::Validator;` and
  `pub use nebula_validator::foundation::{Validate, ValidateExt};` from
  `crates/sdk/src/prelude.rs`.
- Update `crates/sdk/README.md` to drop the `nebula_validator` row from the
  "Top-level re-exports" list.

**Rationale.** `nebula-validator` is not one of the canon §3.5 integration
primitives, and none of the four runnable examples
(`examples/hello_action.rs`, `paginated_users.rs`, `batch_products.rs`,
`poll_habr.rs`) imports `Validator` / `Validate` / `ValidateExt` through
sdk. Field-level validation is already reachable through
`nebula_schema::{ValidValues, ResolvedValues}`, which stays in sdk's
prelude. Dropping validator removes one frontier crate from sdk's docs.rs
landing page with no known external consumer impact.

Publishing of `nebula-validator` itself is still required (see §2) because
`nebula-schema` depends on it — but `nebula-validator` is no longer part of
sdk's *advertised* surface, so its eventual SemVer churn does not register
as a sdk-level break.

### 2. Expand: publish the remaining closure (10 crates + macros)

The following crates flip to `publish = true` with this ADR serving as
condition (b) per ADR-0021 §2:

**§3.5 integration primitives (via sdk):**

| Crate              | Rationale (short)                                                              |
| ------------------ | ------------------------------------------------------------------------------ |
| `nebula-action`    | Canon §3.5 primitive; `simple_action!` macro, `ActionBuilder`, `TestRuntime`   |
| `nebula-workflow`  | `WorkflowBuilder::build()` return type; canon backbone for multi-node flows   |
| `nebula-schema`    | Canon §3.5 primitive; `params!` macro, Field types, proof-token pipeline      |
| `nebula-plugin`    | Canon §3.5 primitive; plugin packaging surface                                |
| `nebula-resource`  | Canon §3.5 primitive; resource lifecycle surface                              |

**Closure forced by `cargo publish` (no path-only deps allowed):**

| Crate              | Why it's in the closure                                                         |
| ------------------ | ------------------------------------------------------------------------------- |
| `nebula-metadata`  | Direct dep of already-published `nebula-credential`, plus `action` / `resource` |
| `nebula-eventbus`  | Direct dep of already-published `nebula-credential`                             |
| `nebula-telemetry` | Direct dep of already-published `nebula-credential`, plus `resource`            |
| `nebula-metrics`   | Direct dep of already-published `nebula-credential`, plus `resource`            |
| `nebula-validator` | Direct dep of `nebula-schema` (forced; sdk no longer re-exports per §1)         |

**Proc-macro companions** (per ADR-0021 §3 — "a proc macro is useless
without its host crate"; this ADR is their condition (b) too):
`nebula-action-macros`, `nebula-schema-macros`, `nebula-plugin-macros`,
`nebula-resource-macros`, `nebula-validator-macros`, and the
`nebula-sdk/macros-support` helper crate if its host is
`nebula-sdk`. Each carries a `# publish-justification: ADR-0026` comment in
the pattern ADR-0021 §5 mandates.

**The published production set grows from 5 to 15**. Including
proc-macros and `macros-support`, roughly 22 crates. This expands the public
SemVer surface beyond ADR-0021's "one gate" ideal; §3 below commits the
first release train to `0.x` to contain the blast radius.

### 3. Pre-1.0 release train policy

The initial `cargo publish` sweep targets `0.1.0` for every crate in §2.
SemVer 2.0.0 §4 permits breaking changes in `0.y.z` minor bumps, which
aligns the registry contract with the `frontier` / `partial` MATURITY rows
we are committing to. This buys the workspace churn budget without the
false advertisement of a 1.0 commitment on unstable surfaces.

**The 1.0 cut for any crate in §2 gates on its MATURITY row reaching
`stable`** (API column). Specifically:

- Cutting `nebula-sdk 1.0` requires *all five* §3.5 primitives —
  `nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-schema`,
  `nebula-plugin` — to be `stable` in `docs/MATURITY.md`, plus
  `nebula-workflow` (already stable).
- `nebula-validator`, `nebula-metadata`, `nebula-eventbus`,
  `nebula-telemetry`, `nebula-metrics` cut 1.0 on their own schedules; they
  are deep in the closure but not advertised on sdk's surface.

This ties the SemVer promise back to the canon's own truthfulness contract
(§11.6: no advertising capabilities code does not deliver).

### 4. Enforcement

ADR-0021 §5 mandates a CI check that a `# publish-justification:` comment
accompanies every `publish = true` flip. The follow-up PR that flips the 15
+ proc-macro companions adds `# publish-justification: ADR-0026` comments
above each opt-in. No change to the check itself; this ADR extends its
input set.

### 5. Explicitly out of scope

- **Tightening of `nebula-credential`'s dependency graph.** That credential
  depends on `metadata`, `eventbus`, `telemetry`, `metrics` is orthogonal
  to sdk and was a pre-existing unclosed follow-up in ADR-0021. This ADR
  resolves the *publication* closure by publishing those four; it does not
  propose narrowing credential's internal deps. If that becomes desirable,
  it lands in a separate ADR.
- **Canon §3.5 expansion.** sdk keeps re-exporting the five canon
  primitives; dropping Resource or Plugin from sdk would require a canon
  revision (§0.2) and is not on the table.
- **`nebula-plugin-sdk`.** Out-of-process plugin protocol; `deny.toml`
  already restricts its consumers to `nebula-sandbox`. Not in the sdk
  closure, not covered here. Stays `publish = false` per ADR-0021's default.

## Consequences

**Positive**

- Unblocks the first `cargo publish` run against crates.io. Today no crate
  in ADR-0021's published set can actually push — the closure tightened by
  this ADR makes the path complete.
- sdk's advertised contract (canon §3.5) stays intact; no integration
  author workflow breaks.
- Dropping `nebula-validator` from sdk's prelude removes one frontier
  surface from sdk's docs.rs landing page without touching anything the
  examples already use.
- `0.x` pre-stable framing aligns the registry promise with the MATURITY
  dashboard. No "stable on crates.io, frontier in MATURITY" contradiction.

**Negative**

- Public SemVer surface grows from 5 prod crates (per ADR-0021) to 15, plus
  ~6 proc-macro companions. ADR-0021's "one gate, not 25" ideal is diluted
  from 5:25 to 15:25 — still better than the accidental-public baseline,
  but not the 1:25 the audit envisioned.
- 5 of 10 new production crates (`action`, `schema`, `resource`,
  `validator`, `metadata`) are `frontier` today; `plugin` is `partial`. We
  commit to SemVer on APIs we also document as moving. `0.x` containment
  helps but does not eliminate the coordination cost of minor bumps.
- docs.rs must stay green for 15 crates, not 5. Every crate needs a
  doc-clean public surface with no broken intra-doc links and a
  `package.metadata.docs.rs` block.
- External consumers of any `frontier` crate published here will see
  minor-version breakage. This is SemVer-legal in `0.x`, but the social
  cost is nonzero — every `0.x` bump is a user-visible event.

**Neutral**

- `deny.toml` is unchanged. `wrappers` rules govern who may depend on what
  inside the workspace; this ADR governs who may be depended on from the
  registry. Same split ADR-0021 drew.
- MATURITY.md rows are not auto-flipped by publication. `nebula-action` at
  `frontier` and `publish = true` coexist — the flag is that 1.0 is gated
  by `stable`, and minor-version breakage inside `0.x` is legal.
- `nebula-sdk` being `partial` in MATURITY is consistent with this ADR's
  `0.1.0` framing. No MATURITY row edits are required by this decision
  itself.

## Alternatives considered

### Option A — Tighten sdk to own-types

Reduce sdk's dependency set to `nebula-core` + `nebula-error` +
`nebula-credential`, and have sdk own wrapper types for the integration
primitives instead of re-exporting upstream.

**Rejected** — cost out of proportion to value. Implementation would
require:

- Rewriting `ActionBuilder::build()` to return a sdk-owned
  `ActionMetadata` mirror, with a conversion layer into
  `nebula_action::ActionMetadata` at the engine boundary.
- Rewriting `TestRuntime` trait bounds against sdk-owned `Action` /
  `StatelessAction` / `StatefulAction` traits.
- Rewriting `simple_action!` and `params!` to expand into sdk-owned
  namespaces, breaking every example downstream.
- Violating canon §3.5 unless all five primitives are mirrored — which
  doubles the workspace's integration-type surface for no external demand.

And the payoff is a sdk that looks deceptively independent but is still
coupled to upstream through the conversion layer. We are in alpha with
zero external integration-author consumers; wrapping now to dodge a
SemVer commitment we have not yet made is premature.

### Option B — Expand without tightening

Keep `nebula-validator` in sdk's re-exports and direct deps; publish all
six sdk direct deps plus the forced closure.

**Rejected** — leaves validator on sdk's docs.rs surface unnecessarily.
Validator is not a canon §3.5 primitive, is `frontier` in MATURITY, and is
not imported through sdk by any runnable example. Keeping it costs a
public-surface advertisement we will have to honor in minor bumps, for no
current user benefit. The small tightening in §1 of the Decision costs
almost nothing and removes a future SemVer liability.

### Option B′ — Only publish sdk's six direct deps, not the closure

Publish `action`, `workflow`, `schema`, `plugin`, `resource`, `validator`
(sdk's directs) but leave `metadata`, `eventbus`, `telemetry`, `metrics`
as `publish = false`.

**Rejected** — `cargo publish` rejects it. `nebula-credential` (already
published in ADR-0021's initial set) depends on all four of those
infrastructure crates with path-only entries; the registry will not accept
a credential upload until they exist on the registry too. This option
doesn't compile with the registry's own rules.

### Option D — Defer first publish

Keep `publish = false` on the whole closure until the MATURITY rows reach
`stable`, then re-open ADR-0021.

**Rejected** — ADR-0020 (library-first GTM) and ADR-0021 both commit the
project to a library-first shipment posture. Deferring publish until every
frontier crate stabilizes is open-ended and misaligns the docs-first
rollout with the library-first strategy. `0.x` pre-stable publication is
the industry-standard answer to the exact tension this option tries to
avoid.

## Follow-ups

- **Implementation PR** — one commit touching:
  - 10 `Cargo.toml` files in §2: add `publish = true` + `#
    publish-justification: ADR-0026` comment block.
  - Proc-macro companions (`*-macros` for each new host) get the same
    treatment.
  - `crates/sdk/Cargo.toml`: remove `nebula-validator = { path =
    "../validator" }`.
  - `crates/sdk/src/lib.rs`: remove `pub use nebula_validator;`.
  - `crates/sdk/src/prelude.rs`: remove the two `nebula_validator`
    re-exports.
  - `crates/sdk/README.md`: drop `nebula_validator` from the top-level
    re-exports list.
  - Every `[dependencies]` path-only entry for a now-published crate must
    grow a `version = "0.1.0"` field so `cargo publish` accepts it.

- **CI check registration (ADR-0021 §5)** — no new check needed; the
  `# publish-justification:` scanner covers this ADR's opt-ins
  automatically. Verify in the implementation PR that the check passes.

- **Release-train script / topological order** — first publish must go in
  dependency order (`core`, `error` → `metadata`, `metrics`, `telemetry`,
  `eventbus`, `resilience` → `schema`, `validator` → `credential`,
  `action`, `workflow`, `plugin`, `resource` → `sdk`). A thin `xtask`
  helper that reads the workspace graph and emits the order will pay for
  itself on every release; tracked as a separate follow-up, not a
  blocker.

- **MATURITY row flip for sdk's 1.0 cut** — when `nebula-action`,
  `nebula-schema`, `nebula-resource`, `nebula-plugin` reach `stable`, open
  the ADR cutting `nebula-sdk 1.0` and supersede the `0.x` policy in §3.

- **`nebula-credential` internal deps** — optional follow-up ADR that
  narrows credential's dep graph (drop `metadata`, `eventbus`, etc. if
  they are internal-only). Independent of this ADR; revisit after the
  first publish cycle.

- **Docs-sync pass** — after the implementation PR, run the checklist in
  [`.claude/skills/docs-sync/SKILL.md`](../../.claude/skills/docs-sync/SKILL.md):
  MATURITY.md unchanged (publication is orthogonal, §3 of Decision),
  `INTEGRATION_MODEL.md` unchanged (surface is the same five primitives),
  crate READMEs list registry versions where they are now available.
