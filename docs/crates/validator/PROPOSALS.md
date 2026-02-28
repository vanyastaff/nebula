# Proposals

## P001: Explicit fail-fast vs collect-all policy

Type: Breaking (behavioral)

Motivation:
- different consumers need different error accumulation semantics.

Proposal:
- add explicit policy API for combinator chains and collection validators.

Expected benefits:
- deterministic behavior by intent, fewer surprises across crates.

Costs:
- API expansion and migration work for existing compositions.

Risks:
- accidental behavior shifts if defaults change silently.

Compatibility impact:
- major-version candidate if default semantics change.

Status: Review

## P002: Error code registry with compatibility checks

Type: Non-breaking (initially)

Motivation:
- downstream API/UI layers depend on stable codes.

Proposal:
- maintain canonical registry and tests to block accidental code drift.

Expected benefits:
- predictable integration and safer upgrades.

Costs:
- governance overhead and test maintenance.

Risks:
- false positives if registry process is too rigid.

Compatibility impact:
- low if introduced as additive enforcement.

Status: Review

## P003: Typed FieldPath

Type: Breaking (potential)

Motivation:
- string paths are error-prone and inconsistent.

Proposal:
- introduce `FieldPath` with validated segments + display conversion.

Expected benefits:
- safer nested/object validation tooling.

Costs:
- migration for APIs currently expecting `String` paths.

Risks:
- friction for simple consumers if adapter layer is poor.

Compatibility impact:
- likely major-version change.

Status: Draft

## P004: Schema bridge layer for plugin ecosystem

Type: Non-breaking (if additive)

Motivation:
- plugin and UI ecosystems may need declarative schema exchange.

Proposal:
- provide optional schema bridge over existing typed validators.

Expected benefits:
- better interoperability without giving up typed core model.

Costs:
- additional maintenance surface.

Risks:
- dual source of truth if not tightly coupled to typed validators.

Compatibility impact:
- low if strictly additive and clearly scoped.

Status: Draft
