# Feature Specification: Validator Integration in Config Crate

**Feature Branch**: `001-validator-config-integration`  
**Created**: 2026-02-28  
**Status**: Draft  
**Input**: User description: "надо внедрить validator в config крейт @docs\crates"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Safe Activation Gate (Priority: P1)

As a service owner, I need configuration to be activated only after validator checks pass so that invalid configuration never becomes active in runtime services.

**Why this priority**: This directly protects runtime stability and prevents outage-causing config drift.

**Independent Test**: Submit one valid and one invalid configuration candidate and verify that only the valid candidate becomes active while invalid candidate is rejected.

**Acceptance Scenarios**:

1. **Given** a candidate configuration that passes validation, **When** configuration is loaded or reloaded, **Then** the candidate becomes active and is available to consumers.
2. **Given** a candidate configuration that fails validation, **When** configuration is loaded or reloaded, **Then** activation is blocked and previously active valid configuration remains active.

---

### User Story 2 - Cross-Crate Contract Consistency (Priority: P2)

As a platform engineer, I need config and validator crates to share a clear interaction contract so that error semantics and behavior remain stable across releases.

**Why this priority**: Stable cross-crate contracts reduce regression risk and speed up upgrades in dependent crates.

**Independent Test**: Run contract fixtures that assert stable categories for validation outcomes and verify governance rules for additive minor changes.

**Acceptance Scenarios**:

1. **Given** the documented contract baseline, **When** contract tests execute, **Then** expected validation categories and activation behavior are unchanged.
2. **Given** a behavior-significant proposal, **When** release readiness is evaluated, **Then** migration mapping is required before approval.

---

### User Story 3 - Operator-Ready Diagnostics and Runbooks (Priority: P3)

As an operator, I need actionable validation failure diagnostics and documented recovery steps so that reload incidents can be resolved quickly without unsafe workarounds.

**Why this priority**: Reliable operations depend on clear diagnostics and deterministic rollback/fallback instructions.

**Independent Test**: Trigger validation failures and verify operators can identify source/path context and complete documented remediation flow.

**Acceptance Scenarios**:

1. **Given** a reload failure caused by validation, **When** diagnostics are reviewed, **Then** source and failure context is available without exposing sensitive values.
2. **Given** repeated validation failures, **When** operators follow runbook steps, **Then** they can restore a valid active configuration deterministically.

---

### Edge Cases

- Multiple sources provide conflicting values while one source introduces invalid data.
- Validation rules change between releases without corresponding migration mapping.
- Optional sources are unavailable during reload while required validations still execute.
- Validation failure occurs repeatedly during high-frequency config updates.
- Diagnostics include values that must be redacted due to sensitive content.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The configuration workflow MUST enforce validation as a mandatory gate before any initial activation or reload activation.
- **FR-002**: The system MUST reject invalid configuration candidates atomically and preserve the last-known-good active configuration.
- **FR-003**: The integration contract between config and validator MUST define stable, documented validation outcome categories.
- **FR-004**: The system MUST expose failure diagnostics with actionable source/path context while redacting sensitive values.
- **FR-005**: Contract fixtures MUST verify stable behavior for valid activation, invalid rejection, and retention of active state.
- **FR-006**: Governance documentation MUST require additive-only minor evolution for config-validator contract behavior.
- **FR-007**: Any behavior-significant contract change MUST include explicit old-to-new migration mapping before release.
- **FR-008**: Documentation in `docs/crates` MUST describe operational runbooks for validation failures and reload recovery.
- **FR-009**: Downstream crates depending on config MUST be able to verify contract compliance through repeatable compatibility checks.
- **FR-010**: The integration scope MUST exclude unrelated business-domain validation rules owned by consuming crates.

### Key Entities *(include if feature involves data)*

- **Config Candidate**: A merged configuration payload prepared for activation; has states such as candidate, active, rejected.
- **Validation Outcome**: Structured result of evaluating a config candidate, including category and diagnostic context.
- **Contract Fixture**: Versioned scenario defining expected activation/rejection behavior and compatibility outcomes.
- **Migration Mapping**: Explicit record of old behavior to new behavior when contract semantics change.
- **Operational Runbook Entry**: Step-by-step operator guidance for validation failure investigation and recovery.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of invalid configuration candidates in contract scenarios are rejected without replacing active valid configuration.
- **SC-002**: 100% of compatibility fixtures for config-validator interaction pass across repeated executions in the same release line.
- **SC-003**: Operators can complete documented validation-failure triage and recovery flow in under 10 minutes for standard incident scenarios.
- **SC-004**: 0 high-severity regressions are observed in dependent crates attributable to undocumented config-validator contract changes during one release cycle.
