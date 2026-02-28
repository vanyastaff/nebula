# Proposals

Use this for non-accepted ideas before they become decisions.

## P001: Unified Memory Runtime Config

Type: Non-breaking

Motivation:

Bootstrap of allocator/pool/cache/budget configs is fragmented in runtime code.

Proposal:

Introduce optional `MemoryRuntimeConfig` that composes existing config objects.

Expected benefits:

Simpler integration and clearer policy ownership in runtime bootstrap.

Costs:

Additional abstraction layer and mapping complexity.

Risks:

Inconsistent defaults between unified and per-module configs.

Compatibility impact:

Non-breaking if existing constructors remain supported.

Status: Review

## P002: Policy-driven Memory Mode Selection

Type: Non-breaking

Motivation:

Different workloads need predictable strategy profiles.

Proposal:

Add policy profiles (`Latency`, `Throughput`, `Reuse`, `Constrained`) that choose internal defaults.

Expected benefits:

Faster and safer tuning for operators.

Costs:

Need strong docs and benchmark-backed defaults.

Risks:

Wrong defaults can degrade production behavior.

Compatibility impact:

Non-breaking if opt-in.

Status: Draft

## P003: Adaptive Pressure Controller

Type: Breaking

Motivation:

Pressure handling is currently signal-first; future may need tighter automation.

Proposal:

Add optional adaptive controller that can enforce budget/pool throttling policies automatically.

Expected benefits:

Better survival under extreme load spikes.

Costs:

Higher behavioral complexity and potential surprises.

Risks:

Implicit throttling may violate caller expectations.

Compatibility impact:

Potentially breaking if enabled by default.

Status: Defer

## P004: Async Contract Unification

Type: Non-breaking

Motivation:

Async surfaces are useful but currently fragmented.

Proposal:

Define one coherent async trait family for pool/cache/budget interactions.

Expected benefits:

Cleaner integration with async runtime crates.

Costs:

Refactoring and migration adapters.

Risks:

Over-constraining sync-first consumers if poorly isolated.

Compatibility impact:

Non-breaking with additive API and adapters.

Status: Review

## P005: Experimental Surface Extraction

Type: Breaking

Motivation:

Main crate size and complexity may outgrow stable-core expectations.

Proposal:

Move unstable experimental modules into sibling crates once contracts stabilize.

Expected benefits:

Tighter stable core and clearer support guarantees.

Costs:

Migration and import-path churn.

Risks:

Fragmented developer experience if split too early.

Compatibility impact:

Major version impact for moved APIs.

Status: Defer
