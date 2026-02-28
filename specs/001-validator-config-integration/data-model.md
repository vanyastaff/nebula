# Data Model: Validator Integration in Config Crate

## Entity: ConfigCandidate

- Description: Merged configuration payload awaiting activation decision.
- Fields:
  - `candidate_id` (string): Unique identifier for a load/reload attempt.
  - `values` (object): Fully merged config tree.
  - `source_fingerprint` (string): Deterministic identifier of source set and ordering.
  - `state` (enum): `candidate`, `active`, `rejected`.
  - `created_at` (timestamp): Candidate creation time.
- Validation rules:
  - Must pass validator gate before moving to `active`.
  - If rejected, active state must remain unchanged.

## Entity: ValidationOutcome

- Description: Structured validator result for a `ConfigCandidate`.
- Fields:
  - `outcome_id` (string)
  - `candidate_id` (string)
  - `passed` (bool)
  - `category` (enum): `validation_failed`, `missing_path`, `type_mismatch`, `invalid_value`, `source_load_failed`.
  - `message` (string)
  - `path` (optional string)
  - `source_id` (optional string)
  - `details` (object)
  - `redacted` (bool): Indicates sensitive content protection in diagnostics.
- Relationships:
  - One `ValidationOutcome` belongs to one `ConfigCandidate`.

## Entity: ActiveConfigSnapshot

- Description: Last-known-good active configuration exposed to consumers.
- Fields:
  - `snapshot_id` (string)
  - `values` (object)
  - `activated_at` (timestamp)
  - `origin_candidate_id` (string)
- Validation rules:
  - Can only be replaced by a candidate with `ValidationOutcome.passed = true`.
  - Must persist across failed reload attempts.

## Entity: ContractFixture

- Description: Versioned scenario that enforces config-validator compatibility behavior.
- Fields:
  - `fixture_id` (string)
  - `version_scope` (string): e.g., `v1`.
  - `input_sources` (array)
  - `expected_activation` (enum): `activate`, `reject_keep_previous`.
  - `expected_category` (optional enum)
  - `expected_snapshot_fragment` (object)
- State transitions:
  - `draft` -> `approved` -> `locked`.

## Entity: MigrationMapping

- Description: Explicit old-to-new contract behavior mapping for breaking changes.
- Fields:
  - `mapping_id` (string)
  - `surface` (enum): `precedence`, `validation_gate`, `error_category`, `path_contract`.
  - `old_behavior` (string)
  - `new_behavior` (string)
  - `consumer_impact` (string)
  - `mitigation` (string)
  - `effective_release` (string)
- Validation rules:
  - Required for behavior-significant contract changes.
  - Must be published before release cut.
