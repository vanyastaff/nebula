# Data Model: Nebula Log Production Hardening

## Entity: LoggingProfile

- **Purpose**: Represents runtime logging behavior for a deployment context.
- **Fields**:
  - `profile_name` (development, test, production, env-derived)
  - `severity_level`
  - `output_format` (human-readable or machine-ingestable)
  - `enrichment_fields` (service/env/custom metadata)
  - `reloadable` (boolean)
- **Validation Rules**:
  - Must have one effective format and level.
  - Must resolve to deterministic final values after precedence rules.

## Entity: ConfigPrecedenceRule

- **Purpose**: Defines deterministic resolution order between config sources.
- **Fields**:
  - `explicit_config_priority`
  - `environment_config_priority`
  - `preset_default_priority`
  - `conflict_resolution_notes`
- **Validation Rules**:
  - Resolution order must be total (no ambiguous tie state).
  - Conflicting inputs must yield one documented effective result.

## Entity: DestinationSet

- **Purpose**: Represents all configured sinks for event delivery.
- **Fields**:
  - `destinations[]` (stdout/stderr/file/other)
  - `fanout_enabled` (boolean)
  - `failure_policy` (FailFast, BestEffort, PrimaryWithFallback)
  - `rolling_policy` (none/time/size)
- **Validation Rules**:
  - Multi-destination mode requires fanout semantics.
  - Size rolling requires positive threshold and explicit retention behavior.

## Entity: HookPolicy

- **Purpose**: Governs extension hook execution safety.
- **Fields**:
  - `execution_mode` (Inline, BoundedAsync)
  - `max_budget_per_hook`
  - `over_budget_behavior` (drop/shed/defer)
  - `panic_isolation_enabled` (boolean)
- **Validation Rules**:
  - Panic isolation must always remain enabled.
  - Bounded mode must enforce finite budget.

## Entity: ExecutionContextEnvelope

- **Purpose**: Carries request/user/session/workflow context through emission path.
- **Fields**:
  - `request_id`
  - `user_id`
  - `session_id`
  - `workflow_id`
  - `execution_id`
- **Validation Rules**:
  - Context must survive async boundaries when async mode is enabled.
  - Missing context must not crash emission; empty fields remain explicit.

## Entity: CompatibilityContract

- **Purpose**: Defines upgrade and migration guarantees for consumers.
- **Fields**:
  - `schema_version`
  - `deprecation_window`
  - `minor_compatibility_guarantee`
  - `migration_guidance_reference`
- **Validation Rules**:
  - Minor releases must preserve supported schema semantics.
  - Breaking removals require deprecation notice and migration path.

## Relationships

- `LoggingProfile` is resolved by `ConfigPrecedenceRule`.
- `LoggingProfile` configures one `DestinationSet`.
- `DestinationSet` applies one `HookPolicy` for extension execution behavior.
- `ExecutionContextEnvelope` enriches events delivered via `DestinationSet`.
- `CompatibilityContract` governs lifecycle changes for all entities.

## State Transitions

1. **Init Requested**: config sources collected.
2. **Profile Resolved**: precedence produces one `LoggingProfile`.
3. **Pipeline Active**: destination fanout + hook policy engaged.
4. **Degraded Mode**: one or more destinations/telemetry backends unavailable, core logging continues.
5. **Shutdown**: hooks flushed under bounded policy and system exits predictably.