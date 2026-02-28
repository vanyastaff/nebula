# Proposals (Senior Review)

## P-001: Typed Value Layer Above Raw JSON (Potential Breaking)

Problem:
- `ParameterValues` stores raw `serde_json::Value`, which pushes type mismatch detection later.

Proposal:
- add typed runtime value enum (`ParameterRuntimeValue`) aligned to `ParameterKind`.

Impact:
- cleaner runtime contracts and better error locality; migration required for direct JSON access patterns.

Migration:
1. introduce typed API in parallel.
2. keep JSON API as compatibility layer.
3. phase out direct JSON in major release.

## P-002: Deterministic Error Path and Ordering Contract

Problem:
- nested validation error ordering can drift with internal iteration details.

Proposal:
- define stable traversal and ordering contract for error output.

Impact:
- non-breaking now, prevents future accidental behavioral breaks in API/UI.

## P-003: Compile-time-safe Parameter Keys (Potential Breaking)

Problem:
- keys are plain strings; typos are discovered late.

Proposal:
- introduce `ParameterKey` newtype and optional registry helpers.

Impact:
- signature changes possible, but improves reliability in large schemas.

## P-004: Schema Lint Pass

Problem:
- invalid/contradictory schema setups (duplicate keys, dead display rules) are not fully preflighted.

Proposal:
- add `ParameterCollection::lint()` returning warnings/errors before runtime usage.

Impact:
- non-breaking additive feature with strong DX benefits.

## P-005: ValidationRule Versioning

Problem:
- rule enum evolution may break persisted schemas if not managed explicitly.

Proposal:
- introduce schema/rule version metadata and migration utilities.

Impact:
- upfront complexity, but safer long-term compatibility.
