# Data Model: Extend nebula-core with Identity Types

**Feature**: 006-extend-core-identity  
**Date**: 2026-02-05  
**Phase**: 1 (Design & Contracts)

## Overview

This document defines the exact type signatures, trait implementations, and relationships for all new types being added to `nebula-core`.

---

## 1. ID Type Definitions

### 1.1 ProjectId

**Purpose**: Type-safe identifier for projects (personal or team workspaces).

**Definition**:
```rust
/// Unique identifier for a project (personal or team workspace)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(String);
```

**Methods**:
```rust
impl ProjectId {
    /// Create a new project ID from a string
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::ProjectId;
    ///
    /// let id = ProjectId::new("acme-corp-prod");
    /// assert_eq!(id.as_str(), "acme-corp-prod");
    /// ```
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    
    /// Get the underlying string as a slice
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::ProjectId;
    ///
    /// let id = ProjectId::new("test-project");
    /// assert_eq!(id.as_str(), "test-project");
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }
    
    /// Convert into the owned string
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::ProjectId;
    ///
    /// let id = ProjectId::new("test");
    /// let string = id.into_string();
    /// assert_eq!(string, "test");
    /// ```
    pub fn into_string(self) -> String {
        self.0
    }
}
```

**Trait Implementations**:
```rust
impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ProjectId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ProjectId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}
```

---

### 1.2 RoleId

**Purpose**: Type-safe identifier for roles (global, project-level, resource-level).

**Definition**:
```rust
/// Unique identifier for a role
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoleId(String);
```

**Methods**: Same as ProjectId
```rust
impl RoleId {
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
    pub fn into_string(self) -> String { self.0 }
}
```

**Trait Implementations**: Same as ProjectId (Display, From<String>, From<&str>)

---

### 1.3 OrganizationId

**Purpose**: Type-safe identifier for organizations (collection of projects).

**Definition**:
```rust
/// Unique identifier for an organization
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrganizationId(String);
```

**Methods**: Same as ProjectId
```rust
impl OrganizationId {
    pub fn new(id: impl Into<String>) -> Self { Self(id.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
    pub fn into_string(self) -> String { self.0 }
}
```

**Trait Implementations**: Same as ProjectId (Display, From<String>, From<&str>)

---

## 2. Scope Level Extension

### 2.1 Updated ScopeLevel Enum

**Definition**:
```rust
/// Defines the scope level for a resource with multi-tenant hierarchy
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScopeLevel {
    /// Resource lives for the entire application lifetime
    Global,

    /// Resource lives within an organization scope
    Organization(OrganizationId),

    /// Resource lives within a project scope (personal or team)
    Project(ProjectId),

    /// Resource lives for the duration of a workflow execution
    Workflow(WorkflowId),

    /// Resource lives for the duration of a single execution
    Execution(ExecutionId),

    /// Resource lives for the duration of a single action invocation
    Action(ExecutionId, NodeId),
}
```

### 2.2 New Methods

```rust
impl ScopeLevel {
    /// Check if this scope is organization-scoped
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::{ScopeLevel, OrganizationId};
    ///
    /// let scope = ScopeLevel::Organization(OrganizationId::new("acme"));
    /// assert!(scope.is_organization());
    /// assert!(!scope.is_project());
    /// ```
    pub fn is_organization(&self) -> bool {
        matches!(self, ScopeLevel::Organization(_))
    }

    /// Check if this scope is project-scoped
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::{ScopeLevel, ProjectId};
    ///
    /// let scope = ScopeLevel::Project(ProjectId::new("proj-123"));
    /// assert!(scope.is_project());
    /// assert!(!scope.is_organization());
    /// ```
    pub fn is_project(&self) -> bool {
        matches!(self, ScopeLevel::Project(_))
    }

    /// Get the organization ID if this scope is organization-scoped
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::{ScopeLevel, OrganizationId};
    ///
    /// let org_id = OrganizationId::new("acme");
    /// let scope = ScopeLevel::Organization(org_id.clone());
    /// assert_eq!(scope.organization_id(), Some(&org_id));
    ///
    /// let global = ScopeLevel::Global;
    /// assert_eq!(global.organization_id(), None);
    /// ```
    pub fn organization_id(&self) -> Option<&OrganizationId> {
        match self {
            ScopeLevel::Organization(id) => Some(id),
            _ => None,
        }
    }

    /// Get the project ID if this scope is project-scoped
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::{ScopeLevel, ProjectId};
    ///
    /// let project_id = ProjectId::new("proj-123");
    /// let scope = ScopeLevel::Project(project_id.clone());
    /// assert_eq!(scope.project_id(), Some(&project_id));
    ///
    /// let global = ScopeLevel::Global;
    /// assert_eq!(global.project_id(), None);
    /// ```
    pub fn project_id(&self) -> Option<&ProjectId> {
        match self {
            ScopeLevel::Project(id) => Some(id),
            _ => None,
        }
    }
}
```

### 2.3 Updated Containment Logic

```rust
impl ScopeLevel {
    /// Check if this scope is contained within another scope
    ///
    /// Hierarchy: Global > Organization > Project > Workflow > Execution > Action
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::{ScopeLevel, ProjectId, OrganizationId};
    ///
    /// let org_scope = ScopeLevel::Organization(OrganizationId::new("acme"));
    /// let project_scope = ScopeLevel::Project(ProjectId::new("proj-123"));
    ///
    /// // Project is contained in Organization
    /// assert!(project_scope.is_contained_in(&org_scope));
    ///
    /// // Both are contained in Global
    /// assert!(project_scope.is_contained_in(&ScopeLevel::Global));
    /// assert!(org_scope.is_contained_in(&ScopeLevel::Global));
    /// ```
    pub fn is_contained_in(&self, other: &ScopeLevel) -> bool {
        match (self, other) {
            // Global contains everything
            (_, ScopeLevel::Global) => true,

            // Organization contains Project and everything below
            (ScopeLevel::Project(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Workflow(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Execution(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Action(_, _), ScopeLevel::Organization(_)) => true,

            // Project contains Workflow and everything below
            (ScopeLevel::Workflow(_), ScopeLevel::Project(_)) => true,
            (ScopeLevel::Execution(_), ScopeLevel::Project(_)) => true,
            (ScopeLevel::Action(_, _), ScopeLevel::Project(_)) => true,

            // Existing logic: Workflow contains Execution and Action
            (ScopeLevel::Execution(_), ScopeLevel::Workflow(_)) => true,
            (ScopeLevel::Action(_, _), ScopeLevel::Workflow(_)) => true,

            // Existing logic: Execution contains matching Action
            (ScopeLevel::Action(exec_id, _), ScopeLevel::Execution(other_exec_id)) => {
                exec_id == other_exec_id
            }

            // No other containment relationships
            _ => false,
        }
    }
}
```

### 2.4 Updated Parent/Child Methods

```rust
impl ScopeLevel {
    /// Get the parent scope level
    ///
    /// Returns None for Global scope or scopes without static parents
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::{ScopeLevel, OrganizationId, ProjectId};
    ///
    /// let org = ScopeLevel::Organization(OrganizationId::new("acme"));
    /// assert_eq!(org.parent(), Some(ScopeLevel::Global));
    ///
    /// let global = ScopeLevel::Global;
    /// assert_eq!(global.parent(), None);
    /// ```
    pub fn parent(&self) -> Option<ScopeLevel> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Organization(_) => Some(ScopeLevel::Global),
            // Note: Project parent is Organization, but we don't track which org
            // This relationship is managed by nebula-project at runtime
            ScopeLevel::Project(_) => None,
            // Similar for Workflow -> Project relationship
            ScopeLevel::Workflow(_) => None,
            ScopeLevel::Execution(_) => None,
            ScopeLevel::Action(exec_id, _) => Some(ScopeLevel::Execution(exec_id.clone())),
        }
    }
}
```

**Updated ChildScopeType**:
```rust
/// Types of child scopes that can be created
#[derive(Debug, Clone)]
pub enum ChildScopeType {
    Organization(OrganizationId),
    Project(ProjectId),
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(NodeId),
}
```

```rust
impl ScopeLevel {
    /// Create a child scope from this scope
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_core::{ScopeLevel, OrganizationId, ChildScopeType};
    ///
    /// let global = ScopeLevel::Global;
    /// let org_child = global.child(ChildScopeType::Organization(
    ///     OrganizationId::new("acme")
    /// ));
    /// assert!(org_child.is_some());
    /// assert!(org_child.unwrap().is_organization());
    /// ```
    pub fn child(&self, child_type: ChildScopeType) -> Option<ScopeLevel> {
        match (self, child_type) {
            (ScopeLevel::Global, ChildScopeType::Organization(org_id)) => {
                Some(ScopeLevel::Organization(org_id))
            }
            (ScopeLevel::Organization(_), ChildScopeType::Project(project_id)) => {
                Some(ScopeLevel::Project(project_id))
            }
            (ScopeLevel::Project(_), ChildScopeType::Workflow(workflow_id)) => {
                Some(ScopeLevel::Workflow(workflow_id))
            }
            (ScopeLevel::Workflow(_), ChildScopeType::Execution(exec_id)) => {
                Some(ScopeLevel::Execution(exec_id))
            }
            (ScopeLevel::Execution(exec_id), ChildScopeType::Action(node_id)) => {
                Some(ScopeLevel::Action(exec_id.clone(), node_id))
            }
            _ => None,
        }
    }
}
```

### 2.5 Updated Display Implementation

```rust
impl fmt::Display for ScopeLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeLevel::Global => write!(f, "global"),
            ScopeLevel::Organization(id) => write!(f, "organization:{}", id),
            ScopeLevel::Project(id) => write!(f, "project:{}", id),
            ScopeLevel::Workflow(id) => write!(f, "workflow:{}", id),
            ScopeLevel::Execution(id) => write!(f, "execution:{}", id),
            ScopeLevel::Action(exec_id, node_id) => {
                write!(f, "action:{}:{}", exec_id, node_id)
            }
        }
    }
}
```

---

## 3. RBAC Enums

### 3.1 ProjectType

**Purpose**: Distinguish between personal (single-user) and team (multi-user) projects.

**Definition**:
```rust
/// Type of project: personal (single-user) or team (multi-user)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    /// Personal project (single-user workspace)
    Personal,
    
    /// Team project (multi-user collaborative workspace)
    Team,
}
```

**Usage**:
```rust
let project_type = ProjectType::Team;
match project_type {
    ProjectType::Personal => {
        // Single user workflow isolation
    }
    ProjectType::Team => {
        // Multi-user collaboration features
    }
}
```

**Serialization**:
```json
{
  "project_type": "personal"  // or "team"
}
```

---

### 3.2 RoleScope

**Purpose**: Define where a role applies (global instance, specific project, credential access, workflow access).

**Definition**:
```rust
/// Scope where a role applies
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleScope {
    /// Role applies across the entire instance
    Global,
    
    /// Role applies within a specific project
    Project,
    
    /// Role applies to credential access
    Credential,
    
    /// Role applies to workflow access
    Workflow,
}
```

**Usage**:
```rust
let role_scope = RoleScope::Project;
if role_scope == RoleScope::Global {
    // Instance-wide permissions
} else if role_scope == RoleScope::Project {
    // Project-specific permissions
}
```

**Serialization**:
```json
{
  "role_scope": "project"  // or "global", "credential", "workflow"
}
```

---

## 4. Module Structure

### File Organization

```
crates/nebula-core/src/
├── lib.rs           # Module declarations, prelude updates
├── id.rs            # Add: ProjectId, RoleId, OrganizationId
├── scope.rs         # Extend: ScopeLevel, ChildScopeType
└── types.rs         # Add: ProjectType, RoleScope
```

### Prelude Updates

```rust
// nebula-core/src/lib.rs
pub mod prelude {
    pub use super::{
        // Existing exports
        CoreError, CredentialId, ExecutionId, HasContext, Identifiable, NodeId, Result, 
        ScopeLevel, Scoped, TenantId, UserId, WorkflowId,
        
        // New exports for identity system
        OrganizationId,
        ProjectId,
        ProjectType,
        RoleId,
        RoleScope,
    };

    pub use crate::keys::*;
}
```

---

## 5. Type Relationships

### Dependency Graph

```
ProjectId ──────────┐
RoleId ─────────────┼──> ScopeLevel (uses in enum variants)
OrganizationId ─────┘

ProjectType ───> (independent, used by nebula-project)
RoleScope ─────> (independent, used by nebula-rbac)
```

### Usage Flow

```
1. Developer imports from nebula_core::prelude
2. Creates type-safe IDs: ProjectId, RoleId, OrganizationId
3. Uses in ScopeLevel for resource isolation: ScopeLevel::Project(project_id)
4. Higher-layer crates (nebula-project, nebula-rbac) use these types
```

---

## 6. Validation Rules

### ID Types
- ✅ Accept any non-empty string (validation is responsibility of higher layers)
- ✅ Support empty string (for default/placeholder cases)
- ✅ UTF-8 encoded (Rust String guarantee)
- ❌ No format restrictions at this layer

### Scope Hierarchy
- ✅ Static containment relationships (no runtime authorization)
- ✅ Hierarchical: Global > Organization > Project > Workflow > Execution > Action
- ✅ Transitive containment (if A contains B and B contains C, then A contains C)

### Enums
- ✅ Exhaustive pattern matching supported
- ✅ Copy-able (no heap allocation)
- ✅ Serialize as snake_case strings

---

## 7. Backward Compatibility

### Unchanged Types
- ExecutionId
- WorkflowId
- NodeId
- UserId
- TenantId
- ActionId
- ResourceId
- CredentialId
- All traits (Scoped, HasContext, Identifiable)

### Unchanged Behavior
- Existing ScopeLevel variants work identically
- Existing containment checks unchanged
- Existing tests pass without modification

### Additive Changes Only
- New enum variants in ScopeLevel (non-breaking)
- New ID types (no conflicts)
- New methods on ScopeLevel (non-breaking)
- New prelude exports (non-breaking)

---

## References

- Research: [research.md](./research.md) - Pattern analysis and design decisions
- Specification: [spec.md](./spec.md) - Requirements and success criteria
- Existing types: `crates/nebula-core/src/id.rs`, `crates/nebula-core/src/scope.rs`
