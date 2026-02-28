# Data Model: Config Contract Hardening

## Entity: ConfigurationSource

- Description: A prioritized origin of configuration data used in merged resolution.
- Fields:
  - `source_id` (string): Stable source identifier.
  - `source_kind` (enum): `defaults`, `file`, `env`, `inline`, `composite`, `remote`, `database`, `key_value`.
  - `priority` (integer): Higher value wins on conflict.
  - `required` (bool): Whether source failure blocks activation.
  - `metadata` (map): Path, provider, timestamp, diagnostics context.
  - `load_status` (enum): `loaded`, `failed`, `skipped`.
- Relationships:
  - Many `ConfigurationSource` entries feed one `MergedConfigurationSnapshot`.

## Entity: MergedConfigurationSnapshot

- Description: Fully resolved candidate or active configuration state.
- Fields:
  - `snapshot_id` (string)
  - `values` (object tree): Resolved key/value payload.
  - `created_at` (timestamp)
  - `source_set_hash` (string): Deterministic identity of input source set.
  - `state` (enum): `candidate`, `active`, `rejected`.
- Validation rules:
  - Same source set and ordering must produce same resolved snapshot.
  - Candidate becomes active only when validation succeeds.

## Entity: ValidationGateResult

- Description: Outcome of config validation before activation.
- Fields:
  - `result_id` (string)
  - `snapshot_id` (string)
  - `passed` (bool)
  - `errors` (list of `ConfigErrorEnvelope`)
  - `validated_at` (timestamp)
- Relationships:
  - One `ValidationGateResult` belongs to one `MergedConfigurationSnapshot`.

## Entity: ReloadAttempt

- Description: A lifecycle event processing config updates.
- Fields:
  - `attempt_id` (string)
  - `trigger` (enum): `watch_event`, `manual`, `startup`, `scheduled`.
  - `started_at` (timestamp)
  - `ended_at` (timestamp)
  - `candidate_snapshot_id` (string)
  - `activation_outcome` (enum): `activated`, `rejected`, `no_change`.
  - `fallback_used` (bool): Indicates last-known-good retention.
- State transitions:
  - `started` -> `loaded` -> `validated` -> (`activated` | `rejected`).

## Entity: PathAccessContract

- Description: Consumer-visible retrieval conventions for key paths and typed access.
- Fields:
  - `path` (string): Dot-style path used by consumers.
  - `expected_type` (string)
  - `required` (bool)
  - `error_category_on_failure` (enum): `missing_path`, `type_mismatch`, `invalid_value`.
  - `version_range` (string)
- Relationships:
  - Referenced by `CompatibilityFixture` entries.

## Entity: CompatibilityFixture

- Description: Versioned scenario asserting stable behavior across releases.
- Fields:
  - `fixture_id` (string)
  - `sources` (list of `ConfigurationSource` payloads)
  - `expected_snapshot_fragment` (object)
  - `path_contract_checks` (list of `PathAccessContract` checks)
  - `reload_expectation` (enum): `activate`, `reject_keep_previous`.
  - `version_scope` (string)
- State transitions:
  - `draft` -> `approved` -> `locked` -> `deprecated`.

## Entity: MigrationRuleSet

- Description: Governance rule group for classifying compatibility impact.
- Fields:
  - `rule_set_version` (string)
  - `minor_additive_rules` (list)
  - `major_break_rules` (list)
  - `requires_mapping` (bool)
  - `deprecation_window_minor_releases` (integer, minimum `1`)
- Relationships:
  - Governs changes to precedence, path contracts, and validation semantics.
