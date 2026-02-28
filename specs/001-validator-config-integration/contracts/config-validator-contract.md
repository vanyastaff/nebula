# Contract: Config <-> Validator Integration

## Contract Scope

This contract defines stable behavior for validation-gated activation in `nebula-config` when using `nebula-validator`.

## Stable Surface

- Config candidate validation before startup activation.
- Config candidate validation before reload activation.
- Atomic rejection semantics for invalid candidates.
- Last-known-good retention semantics after rejection.
- Stable validation outcome category mapping used in compatibility fixtures.
- Redacted diagnostics contract for validation failures.

## Behavioral Guarantees

1. A candidate MUST NOT become active if validation fails.
2. Reload rejection MUST preserve the previously active valid snapshot.
3. Validation category names used by fixtures MUST remain stable within minor releases.
4. Optional source failures MUST NOT bypass validator gate.
5. Diagnostics MUST include actionable path/source context without exposing sensitive values.

## Compatibility Policy

- Minor releases:
  - additive validator options and metadata fields only
  - no changes to existing category semantics
- Major releases:
  - required for behavior-significant changes in activation/rejection semantics
  - required for category semantic changes used by fixtures
- Migration:
  - breaking changes require explicit old->new mapping in migration documentation

## Required Contract Tests

- Valid candidate activation test.
- Invalid candidate rejection test.
- Last-known-good retention test.
- Category compatibility fixture test.
- Sensitive diagnostics redaction test.

## Non-Goals

- Consumer-specific business validation rules.
- Transport/API envelope formatting for non-config crates.
