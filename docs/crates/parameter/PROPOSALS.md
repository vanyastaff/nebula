# Proposals

Use this for non-accepted ideas before they become decisions.

## P001: Typed Value Layer Above Raw JSON

Type: Breaking

Motivation: `ParameterValues` stores raw `serde_json::Value`; type mismatch detection is late. A typed runtime value enum aligned to `ParameterKind` would improve contracts and error locality.

Proposal: Introduce `ParameterRuntimeValue` enum; add typed API in parallel; keep JSON API as compatibility layer; phase out direct JSON in major release.

Expected benefits: Cleaner runtime contracts; better error locality; compile-time-like guarantees at value boundary.

Costs: Migration for direct JSON access patterns; increased API surface.

Risks: Breaking changes for consumers using `ParameterValues::get`/`set` with raw Value.

Compatibility impact: Major version bump; deprecation window 6+ months.

Status: Draft

---

## P002: Deterministic Error Path and Ordering Contract

Type: Non-breaking

Motivation: Nested validation error ordering can drift with internal iteration details; API/UI may rely on stable ordering.

Proposal: Define stable traversal and ordering contract for error output (e.g., depth-first, key order).

Expected benefits: Prevents future accidental behavioral breaks; predictable error presentation.

Costs: May constrain internal refactoring.

Risks: Low.

Compatibility impact: None if additive; document contract.

Status: Draft

---

## P003: Compile-time-safe Parameter Keys

Type: Breaking

Motivation: Keys are plain strings; typos discovered late in large schemas.

Proposal: Introduce `ParameterKey` newtype and optional registry helpers.

Expected benefits: Improved reliability; earlier typo detection.

Costs: Signature changes for lookup APIs; migration for key-heavy code.

Risks: Breaking changes for `get`, `get_by_key`, `contains`, etc.

Compatibility impact: Major version bump.

Status: Draft

---

## P004: Schema Lint Pass

Type: Non-breaking

Motivation: Invalid/contradictory schema setups (duplicate keys, dead display rules, cycles) are not fully preflighted.

Proposal: Add `ParameterCollection::lint()` returning warnings/errors before runtime usage.

Expected benefits: Strong DX; catch schema bugs early.

Costs: Implementation effort; cycle detection complexity.

Risks: May surface issues in existing schemas.

Compatibility impact: Additive; non-breaking.

Status: Draft

---

## P005: ValidationRule Versioning

Type: Non-breaking

Motivation: Rule enum evolution may break persisted schemas if not managed explicitly.

Proposal: Introduce schema/rule version metadata and migration utilities.

Expected benefits: Safer long-term compatibility; explicit upgrade path.

Costs: Upfront complexity; version field in persisted data.

Risks: Low.

Compatibility impact: Additive if optional; migration for existing schemas.

Status: Draft
