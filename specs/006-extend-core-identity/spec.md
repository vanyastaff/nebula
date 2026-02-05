# Feature Specification: Extend nebula-core with Identity and Multi-Tenancy Types

**Feature Branch**: `006-extend-core-identity`  
**Created**: 2026-02-05  
**Status**: Draft  
**Input**: Extend nebula-core with identity and multi-tenancy types for identity management and multi-tenancy system support

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Developer Creates New Project with Proper ID Types (Priority: P1)

A Nebula developer needs to create a new project management feature and requires type-safe identifiers for projects, avoiding the risk of accidentally mixing project IDs with user IDs or other identifier types.

**Why this priority**: This is foundational infrastructure that all future identity features depend on. Without proper ID types in core, no other identity features can be implemented safely.

**Independent Test**: Can be fully tested by importing `ProjectId`, `RoleId`, `OrganizationId` from `nebula-core`, creating instances, and verifying type safety prevents mixing different ID types at compile time.

**Acceptance Scenarios**:

1. **Given** a developer imports `ProjectId` from `nebula_core`, **When** they create a new `ProjectId::new("project-123")`, **Then** the ID is created successfully and has type `ProjectId` (not `String`)
2. **Given** a developer has both `ProjectId` and `UserId`, **When** they attempt to pass a `ProjectId` where `UserId` is expected, **Then** the compiler rejects the code with a type error
3. **Given** a developer creates a `ProjectId`, **When** they serialize it to JSON, **Then** it serializes as a plain string value
4. **Given** a developer deserializes JSON containing a project ID, **When** they parse it into `ProjectId`, **Then** it successfully creates a typed `ProjectId` instance

---

### User Story 2 - Developer Implements Project-Scoped Resource Isolation (Priority: P1)

A Nebula developer needs to ensure that workflows, credentials, and other resources are properly isolated at the project level, preventing users in one project from accessing another project's resources.

**Why this priority**: Resource isolation is critical for multi-tenancy security. Without project-level scope support, the system cannot safely separate tenant data.

**Independent Test**: Can be fully tested by creating `ScopeLevel::Project(project_id)` instances, checking containment relationships, and verifying the scope hierarchy (Organization > Project > Workflow > Execution > Action) works correctly.

**Acceptance Scenarios**:

1. **Given** a developer creates `ScopeLevel::Project(project_a)`, **When** they check if it contains `ScopeLevel::Execution(exec_id)`, **Then** the containment check returns true (projects contain executions)
2. **Given** a developer creates two different `ScopeLevel::Project` instances, **When** they check containment between them, **Then** neither contains the other (projects are peers)
3. **Given** a developer creates `ScopeLevel::Global`, **When** they check if it contains any `ScopeLevel::Project`, **Then** it returns true (global contains all projects)
4. **Given** a developer creates `ScopeLevel::Organization(org_id)`, **When** they check if it contains `ScopeLevel::Project(project_id)`, **Then** it returns true (organizations contain projects)

---

### User Story 3 - Developer Defines RBAC Role Scopes (Priority: P2)

A Nebula developer implementing the RBAC system needs to specify whether a role applies globally, to a specific project, to credentials, or to workflows, using type-safe enums.

**Why this priority**: This enables the RBAC system to be built on top of core types, but the RBAC system itself is a separate milestone.

**Independent Test**: Can be fully tested by creating `RoleScope` enum instances (Global, Project, Credential, Workflow) and using them in pattern matching to determine role applicability.

**Acceptance Scenarios**:

1. **Given** a developer creates `RoleScope::Global`, **When** they pattern match on it, **Then** they can identify it as a global role
2. **Given** a developer creates `RoleScope::Project`, **When** they check if it's Copy, **Then** the type implements Copy trait (no heap allocation needed)
3. **Given** a developer serializes a `RoleScope::Credential` to JSON, **When** they deserialize it back, **Then** the value round-trips correctly

---

### User Story 4 - Developer Distinguishes Personal vs Team Projects (Priority: P3)

A Nebula developer implementing project management needs to differentiate between personal projects (one user) and team projects (multiple users), using a type-safe enum.

**Why this priority**: This is a convenience type that improves code clarity but doesn't affect core functionality. Personal projects can be simulated as team projects with one member.

**Independent Test**: Can be fully tested by creating `ProjectType::Personal` and `ProjectType::Team` instances and verifying they serialize/deserialize correctly.

**Acceptance Scenarios**:

1. **Given** a developer creates `ProjectType::Personal`, **When** they pattern match on it, **Then** they can identify it as a personal project type
2. **Given** a developer creates `ProjectType::Team`, **When** they serialize it to JSON, **Then** it serializes as the string "team"

---

### Edge Cases

- What happens when a developer creates deeply nested scopes (Organization > Project > Workflow > Execution > Action)? **Answer**: The containment hierarchy should correctly identify that Global contains Organization, Organization contains Project, etc.
- How does the system handle comparing IDs of different types (ProjectId vs UserId)? **Answer**: The Rust type system prevents this at compile time due to newtype wrappers.
- What happens when serializing/deserializing IDs with special characters or empty strings? **Answer**: String-based IDs accept any valid UTF-8 string; validation is the responsibility of higher-level code.
- How does the system handle creating ScopeLevel parent/child relationships? **Answer**: The `parent()` and `child()` methods return `Option<ScopeLevel>` to handle cases where no parent/child exists.

## Requirements *(mandatory)*

### Functional Requirements

**ID Types (id.rs)**:

- **FR-001**: System MUST provide a `ProjectId` newtype wrapper around `String` for type-safe project identification
- **FR-002**: System MUST provide a `RoleId` newtype wrapper around `String` for type-safe role identification  
- **FR-003**: System MUST provide an `OrganizationId` newtype wrapper around `String` for type-safe organization identification
- **FR-004**: All ID types MUST implement `new(impl Into<String>)`, `as_str()`, and `into_string()` methods
- **FR-005**: All ID types MUST implement `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`, `Serialize`, `Deserialize`, `Display` traits
- **FR-006**: All ID types MUST support `From<String>` and `From<&str>` conversions
- **FR-007**: All ID types MUST display as their underlying string value when using the `Display` trait

**Scope System (scope.rs)**:

- **FR-008**: `ScopeLevel` enum MUST add `Organization(OrganizationId)` variant
- **FR-009**: `ScopeLevel` enum MUST add `Project(ProjectId)` variant
- **FR-010**: `ScopeLevel` MUST support hierarchy: `Global > Organization > Project > Workflow > Execution > Action`
- **FR-011**: System MUST provide `is_organization()` method returning `bool` for scope level checking
- **FR-012**: System MUST provide `is_project()` method returning `bool` for scope level checking
- **FR-013**: System MUST provide `organization_id()` method returning `Option<&OrganizationId>` to extract organization ID
- **FR-014**: System MUST provide `project_id()` method returning `Option<&ProjectId>` to extract project ID
- **FR-015**: `is_contained_in()` method MUST correctly handle Organization and Project variants in containment checks
- **FR-016**: `parent()` method MUST return correct parent scope for Organization and Project variants
- **FR-017**: `child()` method MUST support creating Organization and Project child scopes
- **FR-018**: `Display` implementation MUST format Organization scopes as "organization:{id}" and Project scopes as "project:{id}"
- **FR-019**: `ChildScopeType` enum MUST add `Organization(OrganizationId)` and `Project(ProjectId)` variants

**RBAC Types (types.rs)**:

- **FR-020**: System MUST provide `ProjectType` enum with variants: `Personal`, `Team`
- **FR-021**: System MUST provide `RoleScope` enum with variants: `Global`, `Project`, `Credential`, `Workflow`
- **FR-022**: `ProjectType` MUST implement `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Serialize`, `Deserialize` traits
- **FR-023**: `RoleScope` MUST implement `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Serialize`, `Deserialize` traits

**Module Exports (lib.rs)**:

- **FR-024**: System MUST export `ProjectId`, `RoleId`, `OrganizationId` from the prelude module
- **FR-025**: System MUST export `ProjectType`, `RoleScope` from the prelude module

**Backwards Compatibility**:

- **FR-026**: Existing `ScopeLevel` variants (`Global`, `Workflow`, `Execution`, `Action`) MUST continue to work unchanged
- **FR-027**: Existing ID types (`UserId`, `TenantId`, `ExecutionId`, etc.) MUST continue to work unchanged
- **FR-028**: Existing code using `nebula-core` MUST compile without modifications (only additive changes)

### Key Entities

- **ProjectId**: Represents a unique identifier for a project (personal or team workspace); string-based for flexibility with external systems
- **RoleId**: Represents a unique identifier for a role (global, project-level, or resource-level); string-based to support both built-in and custom roles
- **OrganizationId**: Represents a unique identifier for an organization (collection of projects); string-based for enterprise integration
- **ScopeLevel**: Represents resource lifecycle and isolation boundaries; now includes Organization and Project levels in the hierarchy
- **ProjectType**: Distinguishes between Personal (single-user) and Team (multi-user) projects
- **RoleScope**: Defines where a role applies (Global instance-wide, Project-specific, Credential access, Workflow access)

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All new ID types successfully prevent mixing different identifier types at compile time (verified by attempting to pass wrong ID type and receiving compiler error)
- **SC-002**: Scope containment checks correctly identify hierarchical relationships in 100% of test cases (Organization > Project > Workflow > Execution > Action)
- **SC-003**: All existing nebula-core tests continue to pass without modification (`cargo test -p nebula-core` exits with code 0)
- **SC-004**: All existing crates using nebula-core compile without changes (`cargo check --workspace` exits with code 0)
- **SC-005**: New types successfully serialize to and deserialize from JSON with 100% data fidelity
- **SC-006**: Code compiles with zero clippy warnings (`cargo clippy -p nebula-core -- -D warnings` exits with code 0)
- **SC-007**: All public types and methods have complete rustdoc documentation (`cargo doc -p nebula-core` generates docs without warnings)
- **SC-008**: Test coverage includes all new methods and edge cases (ID conversions, scope containment, parent/child relationships)

## Technical Constraints *(mandatory)*

- Must use Rust 2024 Edition (MSRV: 1.92)
- No new external dependencies (only existing: `serde`, `uuid`, `thiserror`)
- Must follow existing code patterns and style in nebula-core
- All ID types must use same pattern as existing types (newtype wrapper, same trait implementations)
- Enum variants must be non-exhaustive-friendly (tests should not break if new variants added)
- Changes must be purely additive (no breaking changes to existing API)

## Assumptions *(mandatory)*

- Project IDs will be string-based rather than UUID-based to support human-readable identifiers and external system integration
- Role IDs will be string-based to support both built-in roles ("global:admin") and custom role UUIDs
- Organization feature is optional for Phase 1 but types are added now to avoid future breaking changes
- Scope containment is based on hierarchical relationships, not runtime validation of actual project/organization membership
- Empty strings are valid ID values (validation is responsibility of higher-level code like nebula-project)
- The `parent()` method for Execution and Workflow scopes returns None because the relationship requires runtime context (execution belongs to workflow, which belongs to project)
- The existing codebase does not currently use Organization or Project scope levels, so adding them is purely additive

## Dependencies

**Depends On**:
- Existing nebula-core types and traits

**Blocks**:
- nebula-user (Milestone 1.1) - needs `RoleId` type
- nebula-project (Milestone 1.2) - needs `ProjectId`, `ProjectType` types
- nebula-rbac (Milestone 2.2) - needs `RoleId`, `RoleScope` types
- nebula-tenant (Milestone 3.1) - needs Project scope level
- nebula-organization (Milestone 4.3) - needs `OrganizationId` and Organization scope level

## Security Considerations *(if applicable)*

- ID types prevent accidental cross-contamination of identifiers (e.g., using a project ID where a user ID is expected)
- Scope hierarchy enforces proper isolation boundaries for multi-tenant architecture
- Type safety ensures that scope containment checks cannot be bypassed through string manipulation
- No sensitive data is stored in core types (IDs are opaque identifiers only)

## References

- Roadmap: `docs/roadmaps/identity-multi-tenancy-roadmap.md` (Milestone 1.0)
- Existing nebula-core implementation: `crates/nebula-core/src/`
- Architecture documentation: `docs/nebula-architecture-final.md`
- n8n multi-tenancy research (context for design decisions)
