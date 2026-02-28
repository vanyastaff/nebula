# Phase 0 Research: Validator Integration in Config Crate

## Decision 1: Validation remains a hard activation gate for startup and reload

- Decision: `nebula-config` must validate merged candidate configuration through validator hooks before activation on both startup and reload.
- Rationale: This directly satisfies runtime safety requirements and prevents invalid live state.
- Alternatives considered:
  - Warning-only validation mode by default: rejected because it allows unsafe activation.
  - Post-activation validation: rejected because rollback timing is nondeterministic.

## Decision 2: Preserve last-known-good snapshot on any validation failure

- Decision: Reject candidate atomically and retain previous active snapshot if validation fails.
- Rationale: This guarantees continuity and deterministic failure handling for operators.
- Alternatives considered:
  - Partial apply of valid sections: rejected due to cross-key consistency risk.
  - Reset to empty/default-only state: rejected due to avoidable service disruption.

## Decision 3: Use stable contract categories for config-validator outcomes

- Decision: Treat validation outcomes (`validation_failed`, `missing_path`, `type_mismatch`, etc.) as compatibility-governed contract categories.
- Rationale: Consumer crates need a stable envelope to automate upgrade checks.
- Alternatives considered:
  - Free-form error messages only: rejected because messages are not stable contracts.
  - Consumer-specific category mappings: rejected due to fragmentation risk.

## Decision 4: Contract-first compatibility via versioned fixtures

- Decision: Maintain versioned fixtures for precedence/activation/path semantics under `crates/config/tests/fixtures/compat`.
- Rationale: Fixture-driven checks provide deterministic regression detection across minor releases.
- Alternatives considered:
  - Ad-hoc integration tests without fixtures: rejected due to weak repeatability.
  - Docs-only contract governance: rejected because it is not enforceable in CI.

## Decision 5: Governance requires explicit migration mapping for behavior-significant change

- Decision: Any contract-semantic change must include explicit old->new mapping in migration docs before release.
- Rationale: This is required for safe downstream adoption and aligns with additive minor policy.
- Alternatives considered:
  - Release notes only: rejected due to ambiguity and missed migration steps.
  - Major-only documentation without mapping tables: rejected due to poor operator usability.

## Decision 6: Diagnostics must include context but always protect secrets

- Decision: Validation diagnostics include source/path context and redacted value output for sensitive fields.
- Rationale: Operators need actionable data, but sensitive leakage is unacceptable.
- Alternatives considered:
  - Full payload logging: rejected for security reasons.
  - Minimal/no context logging: rejected because triage becomes too slow.
