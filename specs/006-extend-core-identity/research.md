# Research: Extend nebula-core with Identity Types

**Feature**: 006-extend-core-identity  
**Date**: 2026-02-05  
**Phase**: 0 (Research & Resolution)

## Overview

This document analyzes existing patterns in `nebula-core` and establishes design decisions for adding identity and multi-tenancy types.

---

## 1. ID Type Pattern Analysis

### Existing Pattern (from `crates/nebula-core/src/id.rs`)

**String-based IDs** (WorkflowId, NodeId, UserId, TenantId, ActionId, ResourceId):
```rust
pub struct WorkflowId(String);

impl WorkflowId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
    
    pub fn into_string(self) -> String {
        self.0
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new("") // Note: Empty default for some types
    }
}

// Trait implementations:
// - Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize
// - Display (formats as underlying string)
// - From<String>, From<&str>
```

**UUID-based IDs** (ExecutionId, CredentialId):
```rust
pub struct ExecutionId(Uuid);

impl ExecutionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
    
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl From<Uuid> for ExecutionId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}
```

### Decision: String-based IDs for Identity Types

**Chosen**: ProjectId, RoleId, OrganizationId will be String-based

**Rationale**:
1. **Human readability**: Project IDs like "acme-corp-prod" are more debuggable than UUIDs
2. **External system integration**: Many enterprise systems use string identifiers
3. **Consistency**: UserId and TenantId are already string-based
4. **Flexibility**: Supports both human-readable slugs and generated UUIDs as strings

**Alternatives Considered**:
- **UUID-based IDs**: Rejected - less readable, harder to type in CLI/UI
- **u64-based IDs**: Rejected - limited by range, harder to migrate from external systems
- **Composite IDs** (e.g., OrganizationId + ProjectId): Rejected - too complex, violates YAGNI

**Implementation Template**:
```rust
pub struct ProjectId(String);
pub struct RoleId(String);
pub struct OrganizationId(String);

// Same methods as WorkflowId: new(), as_str(), into_string()
// Same traits: Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display
// Same From impls: From<String>, From<&str>
```

---

## 2. Scope Hierarchy Design

### Current Scope System (from `crates/nebula-core/src/scope.rs`)

**Existing Hierarchy**:
```
Global
  └─ Workflow(WorkflowId)
       └─ Execution(ExecutionId)
            └─ Action(ExecutionId, NodeId)
```

**Containment Rules**:
- Global contains everything
- Workflow contains Execution and Action (simplified - no runtime check)
- Execution contains Action (by ExecutionId match)

**Key Code**:
```rust
pub fn is_contained_in(&self, other: &ScopeLevel) -> bool {
    match (self, other) {
        (_, ScopeLevel::Global) => true,
        
        (ScopeLevel::Execution(_), ScopeLevel::Workflow(_)) => true,
        (ScopeLevel::Action(_,_), ScopeLevel::Workflow(_)) => true,
        
        (ScopeLevel::Action(exec_id, _), ScopeLevel::Execution(other_exec_id)) => {
            exec_id == other_exec_id
        },
        
        _ => false,
    }
}
```

### New Hierarchy Design

**Target Hierarchy**:
```
Global
  └─ Organization(OrganizationId)
       └─ Project(ProjectId)
            └─ Workflow(WorkflowId)
                 └─ Execution(ExecutionId)
                      └─ Action(ExecutionId, NodeId)
```

**Decision: Static Containment with Runtime Flexibility**

**Approach**: 
- Organization and Project levels use static containment (no ID matching)
- Rationale: Actual membership (which project belongs to which org) is runtime data stored in higher layers (nebula-project)
- Scope system provides isolation boundaries, not authorization logic

**Containment Matrix**:

| Self ↓ / Other → | Global | Org | Project | Workflow | Execution | Action |
|------------------|--------|-----|---------|----------|-----------|--------|
| Global           | ✅     | ❌  | ❌      | ❌       | ❌        | ❌     |
| Organization     | ✅     | ❌* | ❌      | ❌       | ❌        | ❌     |
| Project          | ✅     | ✅  | ❌*     | ❌       | ❌        | ❌     |
| Workflow         | ✅     | ✅  | ✅      | ❌*      | ❌        | ❌     |
| Execution        | ✅     | ✅  | ✅      | ✅       | ❌*       | ❌     |
| Action           | ✅     | ✅  | ✅      | ✅       | ✅ (if match) | ❌* |

*Self-containment returns false (scope is not contained in itself)

**Implementation Strategy**:
```rust
pub fn is_contained_in(&self, other: &ScopeLevel) -> bool {
    match (self, other) {
        // Global contains all
        (_, ScopeLevel::Global) => true,
        
        // Organization contains Project and below
        (ScopeLevel::Project(_), ScopeLevel::Organization(_)) => true,
        (ScopeLevel::Workflow(_), ScopeLevel::Organization(_)) => true,
        (ScopeLevel::Execution(_), ScopeLevel::Organization(_)) => true,
        (ScopeLevel::Action(_, _), ScopeLevel::Organization(_)) => true,
        
        // Project contains Workflow and below
        (ScopeLevel::Workflow(_), ScopeLevel::Project(_)) => true,
        (ScopeLevel::Execution(_), ScopeLevel::Project(_)) => true,
        (ScopeLevel::Action(_, _), ScopeLevel::Project(_)) => true,
        
        // Existing Workflow/Execution/Action logic unchanged
        (ScopeLevel::Execution(_), ScopeLevel::Workflow(_)) => true,
        (ScopeLevel::Action(_, _), ScopeLevel::Workflow(_)) => true,
        
        (ScopeLevel::Action(exec_id, _), ScopeLevel::Execution(other_exec_id)) => {
            exec_id == other_exec_id
        },
        
        _ => false,
    }
}
```

**Parent/Child Relationships**:
- `parent()`:
  - Organization → Global
  - Project → Organization (no ID link - simplified)
  - Workflow → Project (no ID link - simplified)
  - Execution → Workflow (no ID link - requires runtime context)
  - Action → Execution (by ExecutionId)
  
- `child()`:
  - Global → Organization (via ChildScopeType)
  - Organization → Project (via ChildScopeType)
  - Project → Workflow (via ChildScopeType)
  - Workflow → Execution (via ChildScopeType)
  - Execution → Action (via ChildScopeType)

**Rationale**:
- Simple static hierarchy provides isolation boundaries
- Runtime authorization (checking if workflow X actually belongs to project Y) is responsibility of nebula-project/nebula-rbac
- Scope system remains lightweight and testable

**Alternatives Considered**:
- **Runtime ID matching at all levels**: Rejected - requires scope system to know about relationships stored in other crates (circular dependency)
- **Separate "Authorization Scope" type**: Rejected - YAGNI, adds complexity
- **Flat scope with manual checks**: Rejected - loses hierarchical benefits

---

## 3. RBAC Enum Design

### ProjectType Enum

**Purpose**: Distinguish personal (single-user) from team (multi-user) projects.

**Design**:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Personal,
    Team,
}
```

**Decision: Copy-able enum**

**Rationale**:
- Small enum (2 variants) should be Copy for performance
- Serializes as strings: "personal", "team" (using serde rename_all)
- No complex data, no heap allocation

**Usage**:
```rust
let project_type = ProjectType::Team;
match project_type {
    ProjectType::Personal => { /* single user logic */ },
    ProjectType::Team => { /* multi-user logic */ },
}
```

### RoleScope Enum

**Purpose**: Define where a role applies (global, project-specific, credential access, workflow access).

**Design**:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleScope {
    Global,
    Project,
    Credential,
    Workflow,
}
```

**Decision: Copy-able enum with string serialization**

**Rationale**:
- Copy for hot path usage in RBAC checks
- String serialization for human-readable configs
- 4 variants cover all current needs (no premature variants)

**Usage**:
```rust
let role_scope = RoleScope::Project;
if role_scope == RoleScope::Global {
    // Instance-wide permissions
}
```

**Alternatives Considered**:
- **Bitflags**: Rejected - no need for combining scopes, simplicity wins
- **Nested enum**: Rejected - YAGNI, adds complexity

---

## 4. Naming Convention Verification

### Rust API Guidelines Compliance

**Checked against**:
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Existing nebula-core patterns
- Constitution Principle VIII

**Verified**:

| Item | Convention | Status |
|------|------------|--------|
| ID struct names | PascalCase | ✅ ProjectId, RoleId, OrganizationId |
| Constructor | `new()` | ✅ Follows existing pattern |
| Accessors | `as_*()` for cheap ref | ✅ `as_str()` |
| Conversion | `into_*()` for consuming | ✅ `into_string()` |
| Enum variants | PascalCase | ✅ Personal, Team, Global, etc. |
| Methods | snake_case | ✅ `is_project()`, `project_id()` |
| Constants | SCREAMING_SNAKE_CASE | N/A (no constants added) |

**Documentation Requirements**:
- Module-level docs explaining purpose
- Struct-level docs with examples
- Method-level docs for non-obvious behavior
- Examples in doc comments for main types

---

## 5. Serialization Strategy

### JSON Format

**ID Types**: Serialize as plain strings
```json
{
  "project_id": "acme-corp-prod",
  "role_id": "role:admin",
  "organization_id": "acme-corp"
}
```

**Enums**: Serialize as snake_case strings
```json
{
  "project_type": "team",
  "role_scope": "project"
}
```

**Scope**: Serialize as formatted strings
```json
{
  "scope": "project:acme-corp-prod"
}
```

**Rationale**:
- Human-readable
- Easy to debug
- Consistent with existing patterns
- Matches n8n's approach (reviewed in context)

---

## 6. Testing Strategy

### Test Categories

1. **ID Type Tests** (per type: ProjectId, RoleId, OrganizationId):
   - Creation: `new()`, `from()` conversions
   - Accessors: `as_str()`, `into_string()`
   - Equality: PartialEq, Hash
   - Display: format!("{}", id)
   - Serialization: JSON round-trip

2. **Scope Hierarchy Tests**:
   - Containment matrix verification (30+ combinations)
   - Parent/child relationships
   - Display formatting
   - New variant isolation (no breakage of existing tests)

3. **Enum Tests** (ProjectType, RoleScope):
   - Pattern matching
   - Serialization format
   - Copy behavior

4. **Integration Tests**:
   - Mixing new and existing types
   - Workspace compilation (`cargo check --workspace`)
   - Backward compatibility

### Test Coverage Goals

- 100% of public methods
- All containment relationships
- Edge cases: empty strings, special characters
- Negative cases: type mismatches (compile-time verification)

---

## 7. Breaking Change Analysis

### Changes Made

1. **ScopeLevel enum**: 2 new variants added
2. **New ID types**: 3 new types (no changes to existing)
3. **New enums**: 2 new types (no changes to existing)
4. **Prelude exports**: 5 new exports (no changes to existing)

### Backward Compatibility Verification

**Existing code patterns that MUST work**:
```rust
// Existing code using old scope variants
let scope = ScopeLevel::Global;
let workflow_scope = ScopeLevel::Workflow(WorkflowId::new("test"));

// Existing ID types unchanged
let execution_id = ExecutionId::new();
let user_id = UserId::new("user-123");

// Existing containment checks
assert!(workflow_scope.is_contained_in(&ScopeLevel::Global));
```

**Mitigation**:
- Enum is non-exhaustive friendly (match with catch-all)
- No method signature changes
- Only additive changes
- Existing tests run unchanged

---

## 8. Summary of Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| ID type base | String | Human-readable, flexible, consistent with UserId/TenantId |
| Scope hierarchy | Static containment | Simple, testable, authorization is separate concern |
| Enum Copy trait | Yes | Small enums, hot path performance |
| Serialization | snake_case strings | Human-readable, debuggable |
| Breaking changes | None | Pure additive changes |
| Test strategy | Inline tests + matrix | Fast feedback, comprehensive coverage |

---

## 9. Open Questions

### Resolved

✅ **Q1**: Should ProjectId be UUID or String?  
**A**: String - provides flexibility for human-readable IDs and external system integration

✅ **Q2**: How to handle Workflow-to-Project relationships in scope?  
**A**: Static containment - runtime relationships managed by higher layers

✅ **Q3**: Should enums be Copy?  
**A**: Yes - small enums benefit from Copy in hot paths

### No Open Questions

All design decisions documented and justified.

---

## References

- Existing patterns: `crates/nebula-core/src/id.rs`, `crates/nebula-core/src/scope.rs`
- Constitution: `.specify/memory/constitution.md` (Principles I, V, VII, VIII)
- Rust API Guidelines: [https://rust-lang.github.io/api-guidelines/](https://rust-lang.github.io/api-guidelines/)
- n8n Multi-tenancy: Research context from specification phase
