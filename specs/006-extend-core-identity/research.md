# Research: Extend nebula-core with Identity Types

**Feature**: 006-extend-core-identity  
**Date**: 2026-02-05  
**Updated**: 2026-02-10  
**Phase**: 0 (Research & Resolution)

## Overview

This document analyzes existing patterns in `nebula-core` and establishes design decisions for adding identity and multi-tenancy types.

---

## 1. ID Type Pattern Analysis

### Evolution: domain-key 0.1.1 → 0.2.1

**Previous approach (domain-key 0.1.1)** — all IDs used `Key<D>` (SmartString-based):
```rust
define_domain!(UserIdDomain, "user-id");
key_type!(UserId, UserIdDomain);  // type UserId = Key<UserIdDomain>

let id = UserId::new(uuid.to_string()).unwrap();  // UUID → String → SmartString
id.as_str()  // returns &str
```

Problems with this approach:
- **Not Copy**: requires `.clone()` for passing around
- **~24+ bytes**: SmartString overhead vs 16 bytes for raw UUID
- **String round-trip**: `Uuid → String → SmartString` on creation, `SmartString → String → Uuid` on extraction
- **No UUID validation**: accepts any string, not just valid UUIDs

**New approach (domain-key 0.2.1)** — IDs use `Uuid<D>` (native uuid::Uuid wrapper):
```rust
define_uuid!(ProjectIdDomain => ProjectId);  // type ProjectId = Uuid<ProjectIdDomain>

let id = ProjectId::v4();       // random UUID, no string allocation
let id = ProjectId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
id.get()    // returns uuid::Uuid
```

Benefits:
- **Copy**: 16 bytes, stack-allocated, zero-cost pass-by-value
- **Native UUID**: no string conversion overhead
- **Validated**: only valid UUIDs accepted at parse time
- **Rich trait set**: `Copy`, `Clone`, `Eq`, `Ord`, `Hash`, `Serialize`, `Deserialize`, `Display`, `FromStr`
- **Domain separation**: `ProjectId` and `UserId` are incompatible types at compile time

### Decision: UUID-based IDs via domain-key 0.2.1

**Chosen**: All entity IDs use `define_uuid!` macro (UUID-based, `Copy`, 16 bytes)

**Rationale**:
1. **Copy semantics**: IDs are passed around constantly — `Copy` eliminates `.clone()` noise
2. **Compact**: 16 bytes vs 24+ bytes (SmartString) — better cache locality
3. **Security**: Random UUIDs prevent enumeration attacks on resource endpoints
4. **Consistency**: All ID types share the same `Uuid<D>` base — uniform API
5. **Validation**: Only valid UUIDs accepted — no "empty string" or "invalid format" bugs
6. **Performance**: No string allocation on creation (`v4()` directly), no parsing overhead

**Alternatives Considered**:
- **String-based IDs** (`Key<D>` / `key_type!`): Rejected — unnecessary heap allocation for what are always UUIDs; human-readable names belong in separate fields
- **`Id<D>` (NonZeroU64)**: Rejected — 64-bit range is insufficient for distributed systems, no standard encoding
- **Manual newtype wrappers**: Rejected — domain-key provides all needed traits + validation for free

**String keys remain for non-ID use cases**:
```rust
// keys.rs — these stay on key_type! (SmartString-based)
define_domain!(ParameterDomain, "parameter");
key_type!(ParameterKey, ParameterDomain);   // human-readable parameter names

define_domain!(CredentialDomain, "credential");
key_type!(CredentialKey, CredentialDomain);  // human-readable credential keys
```

### Migration scope

All existing ID types in `id.rs` must be migrated from `key_type!` to `define_uuid!`:

| Type | Before (0.1.1) | After (0.2.1) |
|------|----------------|---------------|
| `UserId` | `Key<UserIdDomain>` (SmartString) | `Uuid<UserIdDomain>` (uuid::Uuid, Copy) |
| `TenantId` | `Key<TenantIdDomain>` | `Uuid<TenantIdDomain>` |
| `ExecutionId` | `Key<ExecutionIdDomain>` | `Uuid<ExecutionIdDomain>` |
| `WorkflowId` | `Key<WorkflowIdDomain>` | `Uuid<WorkflowIdDomain>` |
| `NodeId` | `Key<NodeIdDomain>` | `Uuid<NodeIdDomain>` |
| `ActionId` | `Key<ActionIdDomain>` | `Uuid<ActionIdDomain>` |
| `ResourceId` | `Key<ResourceIdDomain>` | `Uuid<ResourceIdDomain>` |
| `CredentialId` | `Key<CredentialIdDomain>` | `Uuid<CredentialIdDomain>` |
| **`ProjectId`** | *(new)* | `Uuid<ProjectIdDomain>` |
| **`RoleId`** | *(new)* | `Uuid<RoleIdDomain>` |
| **`OrganizationId`** | *(new)* | `Uuid<OrganizationIdDomain>` |

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

**ID Types**: Serialize as UUID strings (delegated to `uuid` crate by `domain-key`):
```json
{
  "project_id": "550e8400-e29b-41d4-a716-446655440000",
  "role_id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  "organization_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479"
}
```

**Enums**: Serialize as snake_case strings
```json
{
  "project_type": "team",
  "role_scope": "project"
}
```

**Scope**: Serialize as formatted strings (Display impl)
```json
{
  "scope": "project:550e8400-e29b-41d4-a716-446655440000"
}
```

**Rationale**:
- UUID strings are standardized (RFC 4122) and universally parseable
- `domain-key` delegates serialization to `uuid` crate for efficiency
- Easy to debug — standard UUID format is widely recognized
- Consistent with REST API best practices for resource identifiers

---

## 6. Testing Strategy

### Test Categories

1. **ID Type Tests** (per type: ProjectId, RoleId, OrganizationId + migrated existing types):
   - Creation: `v4()`, `nil()`, `parse()`, `from_bytes()`
   - Accessors: `get()`, `as_bytes()`, `domain()`, `is_nil()`
   - Copy semantics: verify both copies usable after assignment
   - Equality: PartialEq, Ord, Hash
   - Display: `format!("{}", id)` outputs UUID string
   - Serialization: JSON round-trip (UUID string format)
   - Domain separation: verify `ProjectId` != `UserId` at type level

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

- 100% of public methods on new types
- All containment relationships in scope matrix
- Edge cases: nil UUIDs, parsing invalid strings
- Negative cases: type mismatches (compile-time verification)
- Migration verification: all existing ID types work with new `Uuid<D>` API

---

## 7. Breaking Change Analysis

### Changes Made

1. **domain-key upgrade**: 0.1.1 → 0.2.1 (breaking: `Key<D>` API changes, `Display` format changed)
2. **All ID types migrated**: `key_type!` → `define_uuid!` (breaking: API surface changes)
3. **ScopeLevel enum**: 2 new variants added (additive)
4. **New ID types**: 3 new types: ProjectId, RoleId, OrganizationId (additive)
5. **New enums**: 2 new types: ProjectType, RoleScope (additive)
6. **Prelude exports**: 5+ new exports (additive)

### Breaking Changes (acceptable in pre-1.0)

**ID type API changes**:

| Before (key_type!) | After (define_uuid!) | Migration |
|---------------------|----------------------|-----------|
| `UserId::new("...").unwrap()` | `UserId::v4()` or `UserId::parse("...").unwrap()` | Update call sites |
| `id.as_str()` | `id.get()` (returns `uuid::Uuid`) or `id.to_string()` | Update accessors |
| `id.clone()` | just `id` (Copy) | Remove `.clone()` calls |
| `From<String>` | `TryFrom<String>` | Handle parse errors |
| Not Copy | Copy | Simplifies code |

**Affected crates**: Any crate using `nebula-core` ID types must update usage patterns. Currently:
- `nebula-credential` — uses its own `CredentialId` (independent, not affected)
- Other crates — check `cargo check --workspace` after migration

### Mitigation

- Pre-1.0 project — breaking changes are acceptable
- All changes are mechanical (find/replace patterns)
- `cargo check --workspace` catches all breakage at compile time
- Migration simplifies code (Copy eliminates clone noise)

---

## 8. Summary of Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| ID type base | `domain-key` 0.2.1 `Uuid<D>` | Copy, 16 bytes, validated, uniform API across all IDs |
| domain-key version | 0.2.1 with `uuid-v4` feature | Provides `define_uuid!` macro, `v4()` generation |
| Existing ID migration | All `key_type!` → `define_uuid!` | Consistency — all entity IDs are UUID-based |
| String keys | Keep `key_type!` for ParameterKey, CredentialKey | Human-readable names, not entity identifiers |
| Scope hierarchy | Static containment | Simple, testable, authorization is separate concern |
| Enum Copy trait | Yes | Small enums, hot path performance |
| Serialization | UUID strings (IDs), snake_case strings (enums) | RFC 4122 standard, debuggable |
| Breaking changes | Accepted (pre-1.0) | Mechanical migration, simplifies code |
| Test strategy | Inline tests + matrix | Fast feedback, comprehensive coverage |

---

## 9. Open Questions

### Resolved

✅ **Q1**: Should ProjectId be UUID or String?  
**A**: UUID via `domain-key` 0.2.1 `Uuid<D>` — provides Copy semantics, 16-byte compact representation, validated format, and uniform API. Human-readable names are separate fields in higher-level structs.

✅ **Q2**: How to handle Workflow-to-Project relationships in scope?  
**A**: Static containment - runtime relationships managed by higher layers

✅ **Q3**: Should enums be Copy?  
**A**: Yes - small enums benefit from Copy in hot paths

✅ **Q4**: Should existing ID types be migrated to `define_uuid!`?  
**A**: Yes — consistency across all entity IDs. Pre-1.0, breaking changes are acceptable. Migration is mechanical.

✅ **Q5**: What about ParameterKey and CredentialKey?  
**A**: Stay on `key_type!` (SmartString-based). These are human-readable keys, not entity identifiers.

### No Open Questions

All design decisions documented and justified.

---

## References

- Existing patterns: `crates/nebula-core/src/id.rs`, `crates/nebula-core/src/scope.rs`
- Constitution: `.specify/memory/constitution.md` (Principles I, V, VII, VIII)
- Rust API Guidelines: [https://rust-lang.github.io/api-guidelines/](https://rust-lang.github.io/api-guidelines/)
- n8n Multi-tenancy: Research context from specification phase
