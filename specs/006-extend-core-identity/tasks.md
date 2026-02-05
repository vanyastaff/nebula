# Implementation Tasks: Extend nebula-core with Identity Types

**Feature**: 006-extend-core-identity  
**Branch**: `006-extend-core-identity`  
**Spec**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md)

## Overview

This document breaks down the implementation into independently testable, parallelizable tasks organized by user story. Each user story represents a complete, deployable increment following TDD principles.

**Total Tasks**: 19 tasks  
**Parallel Opportunities**: 12 parallelizable tasks  
**MVP Scope**: User Story 1 (T003-T008) - ID Types with type safety

---

## Implementation Strategy

### MVP-First Approach

**Minimum Viable Product**: User Story 1 only
- Delivers: Type-safe ID types (ProjectId, RoleId, OrganizationId)
- Value: Prevents ID type mixing at compile time
- Independent: Can ship without other stories
- Tests: 6 test functions verify type safety, serialization, conversions

**Incremental Delivery**:
1. **Ship MVP** (US1): ID types → blocks Milestone 1.1 (nebula-user)
2. **Ship US2**: Scope isolation → blocks Milestone 1.2 (nebula-project)
3. **Ship US3+US4**: RBAC support → blocks Milestone 2.2 (nebula-rbac)

Each story is independently testable and deployable.

### TDD Workflow

Per Constitution Principle III (Test-Driven Development):
1. Write test → verify it fails (RED)
2. Implement minimum code to pass (GREEN)
3. Refactor if needed
4. Run quality gates before marking complete

**Quality Gates** (run after each story):
```bash
cargo fmt --all
cargo clippy -p nebula-core -- -D warnings
cargo test -p nebula-core
cargo check --workspace  # Verify backward compatibility
cargo doc -p nebula-core
```

---

## Task Dependency Graph

```
Phase 1 (Setup)
└─> Phase 2 (Foundational)
     └─> Phase 3 (US1: ID Types) ──┬──> Phase 4 (US2: Scope Isolation)
                                     │
                                     └──> Phase 5 (US3: RBAC Scopes)
                                          └──> Phase 6 (US4: Project Types)
                                               └──> Phase 7 (Polish)
```

**Critical Path**: Setup → Foundational → US1 → US2 → Polish  
**Parallel Opportunities**: Within each user story phase, most tasks are parallelizable

---

## Phase 1: Setup

**Goal**: Prepare crate structure and verify baseline

### Tasks

- [ ] T001 Verify nebula-core compiles with existing tests (`cargo test -p nebula-core`)
- [ ] T002 Create feature branch and verify git status clean

**Completion Criteria**:
- ✅ Existing tests pass (baseline)
- ✅ Branch created and checked out
- ✅ No uncommitted changes

---

## Phase 2: Foundational Tasks

**Goal**: Add module structure and update exports (blocks all user stories)

### Tasks

- [ ] T003 Add module-level documentation to `crates/nebula-core/src/types.rs` for RBAC enums

**Completion Criteria**:
- ✅ Module file exists with proper documentation header
- ✅ Ready to accept ProjectType and RoleScope enums

**Note**: This task must complete before any user story tasks can begin, as it establishes the module structure.

---

## Phase 3: User Story 1 - Type-Safe ID Types (P1)

**Story Goal**: Enable developers to create type-safe identifiers that prevent mixing different ID types at compile time.

**Independent Test Criteria**: 
- Can import ProjectId, RoleId, OrganizationId from nebula_core
- Creating IDs with `new()` succeeds
- Compiler rejects passing wrong ID type to functions
- Serialization to/from JSON works correctly

**Value Delivered**: Foundational type safety for all future identity features

### Test Tasks

- [ ] T004 [P] [US1] Write test for ProjectId creation and methods in `crates/nebula-core/src/id.rs` (test module)
- [ ] T005 [P] [US1] Write test for RoleId creation and methods in `crates/nebula-core/src/id.rs` (test module)
- [ ] T006 [P] [US1] Write test for OrganizationId creation and methods in `crates/nebula-core/src/id.rs` (test module)

**Test Coverage**:
- ID creation: `ProjectId::new("test")`
- Accessor methods: `as_str()`, `into_string()`
- Conversions: `From<String>`, `From<&str>`
- Display trait: `format!("{}", id)`
- Serialization: JSON round-trip via serde_json
- Type safety: Compile-time verification (separate test file if needed)

### Implementation Tasks

- [ ] T007 [P] [US1] Implement ProjectId struct with traits in `crates/nebula-core/src/id.rs`
- [ ] T008 [P] [US1] Implement RoleId struct with traits in `crates/nebula-core/src/id.rs`
- [ ] T009 [P] [US1] Implement OrganizationId struct with traits in `crates/nebula-core/src/id.rs`

**Implementation Checklist** (per ID type):
- Struct definition: `pub struct ProjectId(String);`
- Methods: `new()`, `as_str()`, `into_string()`
- Traits: Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize
- Display impl: `write!(f, "{}", self.0)`
- From impls: `From<String>`, `From<&str>`
- Doc comments with examples (per Rust API Guidelines)

### Integration Tasks

- [ ] T010 [US1] Add ID type exports to prelude in `crates/nebula-core/src/lib.rs`

**Integration Checklist**:
- Add to prelude: `ProjectId`, `RoleId`, `OrganizationId`
- Update module docs if needed
- Verify exports with `cargo doc -p nebula-core`

### Story Completion

**Independent Test**: Run test demonstrating type safety
```rust
#[test]
fn test_id_type_safety() {
    let project_id = ProjectId::new("proj-123");
    let user_id = UserId::new("user-456");
    
    fn requires_project(id: ProjectId) {}
    requires_project(project_id);  // OK
    // requires_project(user_id);  // Compile error - type safety works!
}
```

**Verification**:
```bash
cargo test -p nebula-core -- id::tests
cargo clippy -p nebula-core -- -D warnings
cargo check --workspace  # Verify no breakage
```

**Deliverable**: Type-safe ID types ready for nebula-user (Milestone 1.1)

---

## Phase 4: User Story 2 - Project-Scoped Resource Isolation (P1)

**Story Goal**: Enable developers to implement multi-tenant resource isolation using project-level scopes.

**Independent Test Criteria**:
- Can create ScopeLevel::Project and ScopeLevel::Organization
- Containment checks work correctly (Organization > Project > Workflow)
- Parent/child relationships return correct scopes
- Display formatting includes new variants

**Value Delivered**: Security foundation for multi-tenancy

### Test Tasks

- [ ] T011 [P] [US2] Write tests for Organization and Project scope variants in `crates/nebula-core/src/scope.rs` (test module)
- [ ] T012 [P] [US2] Write containment matrix tests for new hierarchy in `crates/nebula-core/src/scope.rs` (test module)
- [ ] T013 [P] [US2] Write parent/child relationship tests in `crates/nebula-core/src/scope.rs` (test module)

**Test Coverage**:
- Variant creation: `ScopeLevel::Organization(org_id)`, `ScopeLevel::Project(project_id)`
- Type checks: `is_organization()`, `is_project()`
- ID extraction: `organization_id()`, `project_id()`
- Containment matrix: All 36 combinations (6 variants × 6 variants)
- Parent: Organization → Global, Project → None (runtime relationship)
- Child: Global → Organization, Organization → Project
- Display: "organization:acme", "project:proj-123"

### Implementation Tasks

- [ ] T014 [US2] Extend ScopeLevel enum with Organization and Project variants in `crates/nebula-core/src/scope.rs`
- [ ] T015 [US2] Implement type check methods (is_organization, is_project) in `crates/nebula-core/src/scope.rs`
- [ ] T016 [US2] Implement ID extraction methods (organization_id, project_id) in `crates/nebula-core/src/scope.rs`
- [ ] T017 [US2] Update is_contained_in() with new hierarchy logic in `crates/nebula-core/src/scope.rs`
- [ ] T018 [US2] Update parent() and child() methods for new variants in `crates/nebula-core/src/scope.rs`
- [ ] T019 [US2] Update Display impl to format new variants in `crates/nebula-core/src/scope.rs`
- [ ] T020 [US2] Extend ChildScopeType enum with Organization and Project in `crates/nebula-core/src/scope.rs`

**Implementation Notes**:
- Containment hierarchy: Global > Organization > Project > Workflow > Execution > Action
- Static containment (no runtime ID matching for Org/Project)
- Parent returns None for Project (runtime relationship unknown at this layer)

### Integration Tasks

- [ ] T021 [US2] Update prelude exports with ScopeLevel changes in `crates/nebula-core/src/lib.rs`

### Story Completion

**Independent Test**: Run test demonstrating isolation
```rust
#[test]
fn test_project_isolation() {
    let project_a = ScopeLevel::Project(ProjectId::new("proj-a"));
    let project_b = ScopeLevel::Project(ProjectId::new("proj-b"));
    let execution = ScopeLevel::Execution(ExecutionId::new());
    
    // Projects contain executions
    assert!(execution.is_contained_in(&project_a));
    
    // Projects don't contain each other (isolation)
    assert!(!project_a.is_contained_in(&project_b));
    assert!(!project_b.is_contained_in(&project_a));
}
```

**Verification**:
```bash
cargo test -p nebula-core -- scope::tests
cargo test -p nebula-core  # Ensure existing tests still pass
```

**Deliverable**: Scope-based isolation ready for nebula-project (Milestone 1.2)

---

## Phase 5: User Story 3 - RBAC Role Scopes (P2)

**Story Goal**: Enable RBAC developers to specify role applicability (global, project, credential, workflow) using type-safe enums.

**Independent Test Criteria**:
- Can create RoleScope enum instances
- Pattern matching identifies scope type
- Copy trait works (no heap allocation)
- JSON serialization uses snake_case strings

**Value Delivered**: RBAC foundation for permission system

### Test Tasks

- [ ] T022 [P] [US3] Write tests for RoleScope enum in `crates/nebula-core/src/types.rs` (test module)

**Test Coverage**:
- Variant creation: All 4 variants (Global, Project, Credential, Workflow)
- Pattern matching works
- Copy behavior: `let copy = scope;` then use both
- Serialization: Round-trip to/from JSON
- Format: Verify "global", "project", "credential", "workflow" strings

### Implementation Tasks

- [ ] T023 [US3] Implement RoleScope enum with derives in `crates/nebula-core/src/types.rs`

**Implementation Checklist**:
- Enum definition with 4 variants
- Derives: Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize
- Serde rename: `#[serde(rename_all = "snake_case")]`
- Doc comments explaining each variant's purpose

### Integration Tasks

- [ ] T024 [US3] Add RoleScope to prelude exports in `crates/nebula-core/src/lib.rs`

### Story Completion

**Independent Test**:
```rust
#[test]
fn test_role_scope_copy_and_matching() {
    let scope = RoleScope::Project;
    let copy = scope;  // Copy, not move
    
    match scope {
        RoleScope::Global => panic!("wrong variant"),
        RoleScope::Project => {}, // Correct
        _ => panic!("wrong variant"),
    }
    
    // Both still usable (Copy trait)
    assert_eq!(scope, copy);
}
```

**Verification**:
```bash
cargo test -p nebula-core -- types::tests::role_scope
```

**Deliverable**: RoleScope ready for nebula-rbac (Milestone 2.2)

---

## Phase 6: User Story 4 - Personal vs Team Project Types (P3)

**Story Goal**: Enable project management code to distinguish personal from team projects for UI/feature decisions.

**Independent Test Criteria**:
- Can create ProjectType::Personal and ProjectType::Team
- Pattern matching works
- JSON serialization uses snake_case

**Value Delivered**: Code clarity for project management features

### Test Tasks

- [ ] T025 [P] [US4] Write tests for ProjectType enum in `crates/nebula-core/src/types.rs` (test module)

**Test Coverage**:
- Both variants create successfully
- Pattern matching identifies type
- Serialization: "personal" and "team" strings
- Copy behavior works

### Implementation Tasks

- [ ] T026 [US4] Implement ProjectType enum with derives in `crates/nebula-core/src/types.rs`

**Implementation Checklist**:
- Enum with Personal and Team variants
- Same derives as RoleScope
- Doc comments explaining use case

### Integration Tasks

- [ ] T027 [US4] Add ProjectType to prelude exports in `crates/nebula-core/src/lib.rs`

### Story Completion

**Independent Test**:
```rust
#[test]
fn test_project_type_distinction() {
    let personal = ProjectType::Personal;
    let team = ProjectType::Team;
    
    assert_ne!(personal, team);
    
    // Serialization check
    let json = serde_json::to_string(&team).unwrap();
    assert_eq!(json, r#""team""#);
}
```

**Deliverable**: ProjectType ready for nebula-project (Milestone 1.2)

---

## Phase 7: Polish & Cross-Cutting Concerns

**Goal**: Finalize documentation, run quality gates, verify backward compatibility

### Tasks

- [ ] T028 [P] Run cargo fmt on entire workspace (`cargo fmt --all`)
- [ ] T029 [P] Run clippy with strict warnings (`cargo clippy -p nebula-core -- -D warnings`)
- [ ] T030 Run comprehensive test suite (`cargo test --workspace`)
- [ ] T031 Build documentation (`cargo doc -p nebula-core --no-deps`)
- [ ] T032 [P] Verify quickstart examples compile (copy examples from quickstart.md to temp file and test)
- [ ] T033 Update CLAUDE.md if needed with final technology notes

**Completion Criteria**:
- ✅ Zero clippy warnings
- ✅ All tests pass (new + existing)
- ✅ Documentation builds without errors
- ✅ Workspace compiles (backward compatibility verified)
- ✅ Examples in quickstart.md are valid Rust code

---

## Parallel Execution Examples

### Within User Story 1 (ID Types)

**Parallel Group 1** - Test Writing:
```bash
# Can be done simultaneously by different developers/agents
Task T004: Write ProjectId tests
Task T005: Write RoleId tests  
Task T006: Write OrganizationId tests
```

**Parallel Group 2** - Implementation:
```bash
# After tests written, implement in parallel
Task T007: Implement ProjectId
Task T008: Implement RoleId
Task T009: Implement OrganizationId
```

**Sequential**: T010 (prelude exports) must wait for T007-T009

### Within User Story 2 (Scope Isolation)

**Parallel Group 1** - Test Writing:
```bash
Task T011: Write scope variant tests
Task T012: Write containment tests
Task T013: Write parent/child tests
```

**Sequential**: Implementation tasks (T014-T020) have dependencies:
- T014 (extend enum) must complete first
- T015-T016 can run in parallel after T014
- T017-T020 can run in parallel after T015-T016

### Across User Stories

**Cannot Parallelize**:
- US2 depends on US1 (needs ID types in scope variants)
- US3/US4 are independent of US2 but typically done after for logical flow

**Can Parallelize** (if US1 complete):
- US3 (RoleScope) and US4 (ProjectType) can be done simultaneously

---

## Task Summary

### By Phase

| Phase | Tasks | Parallelizable | Description |
|-------|-------|----------------|-------------|
| Setup | 2 | 0 | Baseline verification |
| Foundational | 1 | 0 | Module structure |
| US1 (P1) | 7 | 6 | Type-safe ID types |
| US2 (P1) | 11 | 3 | Scope isolation |
| US3 (P2) | 3 | 1 | RBAC scopes |
| US4 (P3) | 3 | 1 | Project types |
| Polish | 6 | 3 | Quality gates |
| **Total** | **33** | **14** | **All tasks** |

### By User Story

| Story | Priority | Tasks | Value | Blocks Milestone |
|-------|----------|-------|-------|------------------|
| US1   | P1       | 7     | Type safety | 1.1 (nebula-user) |
| US2   | P1       | 11    | Multi-tenant isolation | 1.2 (nebula-project) |
| US3   | P2       | 3     | RBAC foundation | 2.2 (nebula-rbac) |
| US4   | P3       | 3     | Project distinction | 1.2 (nebula-project) |

---

## Definition of Done

### Per Task
- [ ] Tests written and failing (RED)
- [ ] Implementation passes tests (GREEN)
- [ ] Code follows existing patterns exactly
- [ ] Rustdoc comments added with examples
- [ ] No clippy warnings introduced

### Per User Story
- [ ] Independent test criteria verified
- [ ] All story tasks complete
- [ ] Story-specific tests pass
- [ ] Quality gates pass:
  - `cargo test -p nebula-core`
  - `cargo clippy -p nebula-core -- -D warnings`
  - `cargo check --workspace`
  - `cargo doc -p nebula-core`

### Feature Complete
- [ ] All 4 user stories delivered
- [ ] Workspace compiles without changes to other crates
- [ ] All existing tests still pass
- [ ] New types exported in prelude
- [ ] Documentation complete and builds
- [ ] Backward compatibility verified

---

## Risk Mitigation

| Risk | Mitigation | Verification |
|------|------------|--------------|
| Breaking existing code | Only additive changes, enum non-exhaustive friendly | `cargo check --workspace` |
| Scope containment bugs | Comprehensive test matrix (36 combinations) | Test coverage report |
| Missing exports | Verify with quickstart examples | Manual example compilation |
| Pattern inconsistency | Follow existing ID type template exactly | Code review checklist |

---

## Next Actions

1. **Start with MVP** (US1):
   ```bash
   # Implement US1 tasks (T003-T010)
   cargo test -p nebula-core -- id::tests
   cargo clippy -p nebula-core
   ```

2. **Ship MVP** before continuing (optional stopping point)

3. **Continue with US2** for full feature:
   ```bash
   # Implement US2 tasks (T011-T021)
   cargo test -p nebula-core -- scope::tests
   ```

4. **Complete remaining stories** (US3, US4)

5. **Run final polish** (Phase 7)

6. **Merge to main** and unblock dependent milestones

---

## References

- Specification: [spec.md](./spec.md) - User stories and acceptance criteria
- Plan: [plan.md](./plan.md) - Technical approach and decisions
- Data Model: [data-model.md](./data-model.md) - Complete type signatures
- Quickstart: [quickstart.md](./quickstart.md) - Usage examples to verify
- Research: [research.md](./research.md) - Pattern analysis and design decisions
