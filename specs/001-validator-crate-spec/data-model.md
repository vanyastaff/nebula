# Data Model: Validator Contract Hardening

## Entity: ValidationRule

- Description: Typed or composable predicate validating an input domain.
- Fields:
  - `rule_id` (string): Stable identifier for docs/tests.
  - `input_type` (string): Rust type domain (for example `String`, `serde_json::Value`).
  - `category` (enum): `length`, `pattern`, `range`, `content`, `network`, `temporal`, `nullable`, `boolean`, `custom`.
  - `is_combinator` (bool): Whether this rule composes child rules.
  - `version_introduced` (semver string): First public version.
  - `deprecated_in` (optional semver string): Deprecation start.
- Relationships:
  - Can be composed into many `CombinatorChain` nodes.

## Entity: CombinatorChain

- Description: Ordered validation graph preserving deterministic semantics.
- Fields:
  - `chain_id` (string): Stable fixture identifier.
  - `operators` (ordered list): Operators such as `and`, `or`, `not`, `when`, `unless`, `optional`, `field`, `json_field`, `each`.
  - `evaluation_mode` (enum): `fail_fast` or `collect_all` (policy-defined; default behavior documented).
  - `short_circuit_enabled` (bool): Whether early termination semantics apply.
- Relationships:
  - Contains one or more `ValidationRule`.
  - Emits one `ValidationErrorEnvelope` on failure.

## Entity: ValidationErrorEnvelope

- Description: Machine-readable contract payload representing deterministic validation failure.
- Fields:
  - `code` (string): Stable error code contract.
  - `message` (string): Human-readable safe diagnostic text.
  - `field_path` (optional string): Consumer-mappable field path.
  - `params` (object map): Optional structured details.
  - `severity` (optional enum): `error`, `warning`, `info`.
  - `nested` (array of `ValidationErrorEnvelope`): Child failures.
  - `help` (optional string): Safe remediation guidance.
- Validation rules:
  - `code` MUST be stable across minor releases.
  - `message` MUST NOT leak sensitive values.
  - `nested` MUST be bounded by policy in integration points.

## Entity: CompatibilityFixture

- Description: Versioned fixture asserting behavior and error-schema compatibility.
- Fields:
  - `fixture_id` (string)
  - `input` (serialized test payload)
  - `validator_chain_ref` (string)
  - `expected_outcome` (enum): `pass` or `fail`
  - `expected_error` (optional `ValidationErrorEnvelope`)
  - `version_range` (string): Supported compatibility window.
- State transitions:
  - `draft` -> `approved` -> `locked` -> `deprecated`.

## Entity: GovernancePolicy

- Description: Rules for introducing, deprecating, and breaking validator behavior.
- Fields:
  - `policy_version` (string)
  - `minor_change_rules` (list): Additive-only constraints.
  - `major_change_rules` (list): Breaking-change requirements.
  - `deprecation_window_minor_releases` (integer, minimum `1`)
  - `migration_required` (bool)
- Relationships:
  - Governs `ValidationRule` and `ValidationErrorEnvelope` evolution.
