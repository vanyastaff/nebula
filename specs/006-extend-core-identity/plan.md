# Implementation Plan: Extend nebula-core with Identity and Multi-Tenancy Types

**Branch**: `006-extend-core-identity` | **Date**: 2026-02-05 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/006-extend-core-identity/spec.md`

## Summary

Extend the existing `nebula-core` crate with new type-safe identifiers (`ProjectId`, `RoleId`, `OrganizationId`), enhanced scope hierarchy for multi-tenant isolation (Organization > Project > Workflow > Execution > Action), and RBAC foundational types (`ProjectType`, `RoleScope`). This is purely additive infrastructure that enables future identity and multi-tenancy features while maintaining backward compatibility with all existing code.

**Technical Approach**: Follow existing patterns in `nebula-core` for ID types (newtype wrappers with trait implementations), extend `ScopeLevel` enum with new variants, add Copy-able enums for RBAC, and update prelude exports. All changes are compile-time type system extensions with zero runtime overhead.

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)
**Primary Dependencies**: 
- `serde` v1.0 (already present) - for ID serialization
- `uuid` v1.0 (already present) - currently used by ExecutionId pattern
No new dependencies required.

**Storage**: N/A (this feature only adds types, no persistence logic)
**Testing**: `cargo test -p nebula-core`, unit tests for each new type, scope containment tests
**Target Platform**: Cross-platform (types are platform-agnostic)
**Project Type**: Extension to existing `nebula-core` crate (Core layer of 16-crate workspace)

**Performance Goals**: 
- Zero runtime overhead (all types are zero-cost abstractions)
- Copy-able enums for hot path usage (no allocations)
- String-based IDs incur heap allocation on creation (acceptable for identifiers)

**Constraints**: 
- Must maintain 100% backward compatibility (only additive changes)
- Must not break any existing tests in workspace
- Must follow existing patterns exactly (same traits, same structure)

**Scale/Scope**: 
- 3 new ID types (ProjectId, RoleId, OrganizationId)
- 2 new ScopeLevel variants (Organization, Project)
- 2 new enums (ProjectType, RoleScope)
- ~300 lines of code total (following existing patterns)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with `.specify/memory/constitution.md` principles:

- [x] **Type Safety First**: Feature is entirely about adding type-safe newtypes (ProjectId, RoleId, etc.) and enums (ProjectType, RoleScope). All domain identifiers use newtype patterns. No String-based identifiers exposed in public API.
- [x] **Isolated Error Handling**: No errors introduced (types don't have error conditions). Future error handling will use existing `CoreError` from nebula-core.
- [x] **Test-Driven Development**: Test strategy defined - write tests for each ID type (creation, conversion, Display), scope containment hierarchy, and serialization before implementing.
- [x] **Async Discipline**: N/A - no async code in this feature (pure type definitions)
- [x] **Modular Architecture**: Extends existing `nebula-core` crate (Core layer). No new dependencies, no layer violations. Future crates (nebula-user, nebula-project) will depend on these types.
- [x] **Observability**: N/A - no runtime behavior to observe (compile-time types only)
- [x] **Simplicity**: Feature adds necessary types for identity system without premature abstractions. Each type has clear, single purpose. No speculative features.
- [x] **Rust API Guidelines**: All types follow rustdoc conventions, standard naming (snake_case/PascalCase), include examples. Quality gates (fmt, clippy, doc) will run after implementation.

**Constitution Compliance**: ✅ PASS - No violations. Feature is pure type system extension following existing patterns.

## Project Structure

### Documentation (this feature)

```text
specs/006-extend-core-identity/
├── plan.md              # This file
├── research.md          # Phase 0: Pattern analysis, design decisions
├── data-model.md        # Phase 1: Type definitions, relationships
├── quickstart.md        # Phase 1: Usage examples for developers
└── tasks.md             # Phase 2: Implementation tasks (created by /speckit.tasks)
```

### Source Code (repository root)

```text
crates/
└── nebula-core/          # EXISTING crate - will be modified
    ├── src/
    │   ├── lib.rs        # Add exports to prelude
    │   ├── id.rs         # Add: ProjectId, RoleId, OrganizationId
    │   ├── scope.rs      # Extend: ScopeLevel enum with Organization, Project
    │   └── types.rs      # Add: ProjectType, RoleScope enums
    ├── tests/
    │   └── (existing)    # No new test files needed (use inline tests)
    └── Cargo.toml        # No changes (no new dependencies)
```

**Structure Decision**: 
- **Modifying existing crate**: `nebula-core` (Core layer)
- **Rationale**: These are foundational types that multiple future crates will depend on (nebula-user, nebula-project, nebula-rbac, nebula-auth). Placing them in `nebula-core` prevents circular dependencies and follows the existing pattern where UserId, TenantId already live in core.
- **Layer justification**: Core layer is the foundation for all other crates. Adding identity-related ID types here is consistent with existing ExecutionId, WorkflowId, UserId, TenantId.
- **No new crates**: Following Principle V (Modular Architecture), we extend existing core rather than creating a new crate, as these types have no independent existence - they're consumed by higher layers.

## Complexity Tracking

> **No constitution violations** - this section intentionally empty.

All principles followed without exceptions. Feature adds necessary type safety infrastructure using established patterns.

---

## Phase 0: Research & Resolution

**Goal**: Resolve all design decisions and establish patterns to follow.

### Research Tasks

1. **Pattern Analysis**: Document existing ID type patterns in `nebula-core/src/id.rs`
   - Extract: struct definition pattern, trait implementations, method signatures
   - Output: Template for new ID types (ProjectId, RoleId, OrganizationId)

2. **Scope Hierarchy Design**: Analyze `nebula-core/src/scope.rs` containment logic
   - Current hierarchy: Global > Workflow > Execution > Action
   - Design new hierarchy: Global > Organization > Project > Workflow > Execution > Action
   - Key decision: How to handle Workflow/Execution relationships with Project (runtime vs static)
   - Output: Containment matrix showing all variant relationships

3. **Enum Pattern Study**: Review Rust Copy enums in codebase
   - Best practices for simple, Copy-able enums
   - Serialization format (string vs integer)
   - Output: Pattern for ProjectType and RoleScope

4. **Naming Convention Verification**: Check against Rust API Guidelines
   - Verify: `ProjectId::new()`, `as_str()`, `into_string()` match existing patterns
   - Verify: enum variant names follow PascalCase
   - Output: Naming checklist

**Outputs**: 
- `research.md` with all patterns documented
- Design decisions for scope containment logic
- Justification for string-based IDs vs UUID-based

---

## Phase 1: Design & Contracts

**Goal**: Define exact type signatures, relationships, and usage examples.

### Data Model

**Outputs**: `data-model.md` with:

1. **ID Type Definitions**:
```rust
// Template (following ExecutionId, WorkflowId patterns)
pub struct ProjectId(String);
pub struct RoleId(String);  
pub struct OrganizationId(String);

// Methods (same for all three):
impl ProjectId {
    pub fn new(id: impl Into<String>) -> Self;
    pub fn as_str(&self) -> &str;
    pub fn into_string(self) -> String;
}

// Traits: Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display
// From<String>, From<&str>
```

2. **Scope Hierarchy Extension**:
```rust
pub enum ScopeLevel {
    Global,
    Organization(OrganizationId),  // NEW
    Project(ProjectId),              // NEW
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(ExecutionId, NodeId),
}

// New methods:
impl ScopeLevel {
    pub fn is_organization(&self) -> bool;
    pub fn is_project(&self) -> bool;
    pub fn organization_id(&self) -> Option<&OrganizationId>;
    pub fn project_id(&self) -> Option<&ProjectId>;
}
```

3. **RBAC Enums**:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectType {
    Personal,
    Team,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoleScope {
    Global,
    Project,
    Credential,
    Workflow,
}
```

4. **Containment Matrix**: Document which scopes contain which
   - Global contains all
   - Organization contains Project, Workflow, Execution, Action
   - Project contains Workflow, Execution, Action (runtime relationship)
   - Workflow contains Execution, Action (runtime)
   - Execution contains Action
   - Action contains nothing

### Quickstart Guide

**Outputs**: `quickstart.md` with usage examples:

```rust
// Example 1: Creating type-safe IDs
use nebula_core::{ProjectId, RoleId, OrganizationId};

let project_id = ProjectId::new("project-123");
let role_id = RoleId::new("role:admin");
let org_id = OrganizationId::new("acme-corp");

// Type safety prevents mixing:
fn process_project(id: ProjectId) { /* ... */ }
// process_project(role_id); // Compile error!

// Example 2: Scope-based isolation
use nebula_core::{ScopeLevel, ProjectId};

let project_scope = ScopeLevel::Project(ProjectId::new("proj-a"));
let execution_scope = ScopeLevel::Execution(ExecutionId::new());

// Check containment for resource isolation
if project_scope.is_contained_in(&ScopeLevel::Global) {
    println!("Project is within global scope");
}

// Example 3: RBAC types
use nebula_core::{ProjectType, RoleScope};

let project_type = ProjectType::Team;
let role_scope = RoleScope::Project;

match role_scope {
    RoleScope::Global => { /* instance-wide permissions */ }
    RoleScope::Project => { /* project-specific permissions */ }
    _ => {}
}
```

### Agent Context Update

After Phase 1 design completion:

```bash
.specify/scripts/bash/update-agent-context.sh claude
```

This will update `CLAUDE.md` with:
- New types added to nebula-core: ProjectId, RoleId, OrganizationId
- Updated Scope system hierarchy
- RBAC enums (ProjectType, RoleScope)

---

## Phase 2: Task Breakdown

**Note**: This phase is executed by `/speckit.tasks`, not `/speckit.plan`.

Tasks will be generated based on:
- Add 3 ID types to `id.rs` (following ExecutionId pattern)
- Extend `ScopeLevel` enum in `scope.rs` (2 variants + 6 methods)
- Add 2 enums to `types.rs`
- Update `lib.rs` prelude exports (5 new types)
- Write tests for each new type (~10 test functions)
- Update module documentation

**Estimated tasks**: 8-10 independent tasks suitable for parallel implementation.

---

## Quality Gates

### Phase 0 Completion
- [x] Constitution Check passed
- [ ] All NEEDS CLARIFICATION resolved in research.md
- [ ] Patterns documented with examples
- [ ] Design decisions justified

### Phase 1 Completion
- [ ] data-model.md complete with all type signatures
- [ ] quickstart.md has runnable examples
- [ ] Agent context updated
- [ ] No breaking changes to existing API

### Phase 2 Completion (verified by /speckit.tasks)
- [ ] All code follows existing patterns exactly
- [ ] `cargo fmt --all` passes
- [ ] `cargo clippy -p nebula-core -- -D warnings` passes
- [ ] `cargo test -p nebula-core` passes
- [ ] `cargo check --workspace` passes (backward compatibility)
- [ ] `cargo doc -p nebula-core` builds without warnings
- [ ] No phase markers or task references in public documentation

---

## Risk Assessment

| Risk | Impact | Mitigation | Status |
|------|--------|------------|--------|
| Breaking existing code | HIGH | Only additive changes, enum is non-exhaustive friendly | ✅ Mitigated |
| Scope containment bugs | MEDIUM | Comprehensive test matrix for all combinations | 📋 Planned |
| Inconsistent patterns | MEDIUM | Strict adherence to existing ID type template | 📋 Planned |
| Missing exports in prelude | LOW | Verify with quickstart examples | 📋 Planned |
| Circular dependencies | LOW | All types in core layer, no new crate dependencies | ✅ Mitigated |

---

## Dependencies

**This feature blocks**:
- Milestone 1.1: nebula-user (needs RoleId)
- Milestone 1.2: nebula-project (needs ProjectId, ProjectType)
- Milestone 2.2: nebula-rbac (needs RoleId, RoleScope)
- Milestone 3.1: nebula-tenant (needs Project scope level)
- Milestone 4.3: nebula-organization (needs OrganizationId, Organization scope)

**This feature depends on**: None (extends existing nebula-core only)

---

## Success Criteria

1. ✅ All ID types compile with same trait set as existing types
2. ✅ Scope hierarchy tests cover all containment relationships
3. ✅ Existing nebula-core tests pass unchanged
4. ✅ Workspace compiles without errors (`cargo check --workspace`)
5. ✅ Zero clippy warnings
6. ✅ Documentation builds and includes examples
7. ✅ Quickstart examples compile and run
8. ✅ No breaking changes to public API

---

## Next Steps

1. Run `/speckit.plan` completion tasks (research, data-model, quickstart)
2. Review and commit Phase 0-1 artifacts
3. Run `/speckit.tasks` to generate implementation tasks
4. Execute tasks following TDD (tests first, then implementation)
5. Run quality gates before marking complete
