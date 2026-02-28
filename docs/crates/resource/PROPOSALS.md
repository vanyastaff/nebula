# Proposals

Use this for non-accepted ideas before they become decisions.

## P001: Typed Resource Keys

Type: Breaking

Motivation:

Reduce runtime ID mismatch and improve compile-time safety.

Proposal:

Introduce `ResourceKey<T>` and dual-mode registry API (`&str` + typed) for one major cycle.

Expected benefits:

Clearer contracts for action/runtime integration and safer refactors.

Costs:

Additional generic surface, migration complexity for plugin-driven dynamic flows.

Risks:

Over-constraining dynamic workflows if migration is forced too early.

Compatibility impact:

Major if string-only API is removed.

Status: Draft

## P002: Acquire Policy Profiles

Type: Non-breaking

Motivation:

Operators need explicit and predictable back-pressure modes under high load.

Proposal:

Add policy enum for acquire semantics: `FailFast`, `BoundedWait`, `Adaptive`.

Expected benefits:

Easier SLO tuning and incident response without ad-hoc timeout tuning.

Costs:

More configuration matrix and docs complexity.

Risks:

Misconfigured adaptive policy can increase tail latency.

Compatibility impact:

Non-breaking if default behavior remains equivalent.

Status: Review

## P003: Classified Reload (In-place vs Destructive)

Type: Non-breaking

Motivation:

Current `reload_config` always swaps pool, which is heavier than needed for some changes.

Proposal:

Classify config fields into in-place safe updates vs destructive replacements.

Expected benefits:

Lower disruption and fewer cold-start penalties during operational tuning.

Costs:

Higher implementation complexity and stronger invariants needed.

Risks:

Incorrect classification can cause subtle runtime inconsistencies.

Compatibility impact:

Non-breaking if old behavior remains available as explicit mode.

Status: Draft

## P004: Resilience Bridge

Type: Non-breaking

Motivation:

Resource and resilience crates need explicit contract for circuit state and retry budget sharing.

Proposal:

Define adapter interface mapping `Error` variants to resilience policies and telemetry labels.

Expected benefits:

Consistent behavior across runtime modules and fewer duplicated wrappers.

Costs:

Coordination across crates and contract testing overhead.

Risks:

Tight coupling if adapter API leaks resilience internals.

Compatibility impact:

Non-breaking with additive traits.

Status: Review

## P005: Distributed Resource Coordination

Type: Breaking

Motivation:

Future multi-worker deployments may require global lease/placement for scarce resources.

Proposal:

Introduce optional distributed coordinator for selected resource classes.

Expected benefits:

Improved fairness and predictable global capacity control.

Costs:

Significant complexity, external dependencies, and operational burden.

Risks:

Availability and consistency trade-offs can degrade local reliability.

Compatibility impact:

Likely major for APIs exposing placement/lease semantics.

Status: Defer
