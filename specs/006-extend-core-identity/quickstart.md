# Quickstart: Using Identity Types in nebula-core

**Feature**: 006-extend-core-identity  
**Date**: 2026-02-05  
**Phase**: 1 (Design & Contracts)

## Overview

This guide shows how to use the new identity and multi-tenancy types added to `nebula-core`. These examples demonstrate type-safe identifiers, scope-based isolation, and RBAC foundations.

---

## 1. Creating Type-Safe Identifiers

### Basic Usage

```rust
use nebula_core::prelude::*;

fn main() {
    // Create project identifier
    let project_id = ProjectId::new("acme-corp-prod");
    println!("Project: {}", project_id);  // "acme-corp-prod"
    
    // Create role identifier
    let role_id = RoleId::new("role:admin");
    println!("Role: {}", role_id);
    
    // Create organization identifier
    let org_id = OrganizationId::new("acme-corp");
    println!("Organization: {}", org_id);
}
```

### Type Safety in Action

```rust
use nebula_core::{ProjectId, UserId};

// Function requires specific ID type
fn process_project(id: ProjectId) {
    println!("Processing project: {}", id.as_str());
}

fn process_user(id: UserId) {
    println!("Processing user: {}", id.as_str());
}

fn main() {
    let project_id = ProjectId::new("proj-123");
    let user_id = UserId::new("user-456");
    
    // This works
    process_project(project_id);
    
    // This would be a compile error - type safety prevents mixing!
    // process_project(user_id);  // ERROR: expected ProjectId, found UserId
    
    // Use the right type
    process_user(user_id);
}
```

### Working with ID Strings

```rust
use nebula_core::ProjectId;

fn main() {
    // Create from string literal
    let id1 = ProjectId::new("my-project");
    
    // Create from String
    let id2 = ProjectId::new(String::from("another-project"));
    
    // Create using From trait
    let id3: ProjectId = "third-project".into();
    
    // Get string reference
    let name: &str = id1.as_str();
    println!("Project name: {}", name);
    
    // Consume and get owned string
    let owned: String = id1.into_string();
    println!("Owned string: {}", owned);
}
```

---

## 2. Scope-Based Resource Isolation

### Understanding Scope Hierarchy

```rust
use nebula_core::{ScopeLevel, ProjectId, OrganizationId, ExecutionId, NodeId};

fn main() {
    // Create scopes at different levels
    let global = ScopeLevel::Global;
    let org = ScopeLevel::Organization(OrganizationId::new("acme"));
    let project = ScopeLevel::Project(ProjectId::new("proj-123"));
    let execution = ScopeLevel::Execution(ExecutionId::new());
    
    // Check scope types
    assert!(global.is_global());
    assert!(org.is_organization());
    assert!(project.is_project());
    assert!(execution.is_execution());
}
```

### Containment Checks for Isolation

```rust
use nebula_core::{ScopeLevel, ProjectId, OrganizationId};

fn can_access_resource(resource_scope: &ScopeLevel, user_scope: &ScopeLevel) -> bool {
    // Resource is accessible if it's in a scope contained by user's scope
    resource_scope.is_contained_in(user_scope)
}

fn main() {
    let org_scope = ScopeLevel::Organization(OrganizationId::new("acme"));
    let project_scope = ScopeLevel::Project(ProjectId::new("proj-123"));
    let global_scope = ScopeLevel::Global;
    
    // Project is contained in organization
    assert!(can_access_resource(&project_scope, &org_scope));
    
    // Both are contained in global
    assert!(can_access_resource(&project_scope, &global_scope));
    assert!(can_access_resource(&org_scope, &global_scope));
    
    // Organization is NOT contained in project (hierarchy flows down)
    assert!(!can_access_resource(&org_scope, &project_scope));
}
```

### Multi-Tenant Isolation Example

```rust
use nebula_core::{ScopeLevel, ProjectId};
use std::collections::HashMap;

// Simplified resource manager with project-level isolation
struct ResourceManager {
    resources: HashMap<String, (ScopeLevel, String)>,  // (scope, data)
}

impl ResourceManager {
    fn new() -> Self {
        Self {
            resources: HashMap::new(),
        }
    }
    
    fn store(&mut self, key: String, scope: ScopeLevel, data: String) {
        self.resources.insert(key, (scope, data));
    }
    
    fn get(&self, key: &str, access_scope: &ScopeLevel) -> Option<&String> {
        self.resources.get(key).and_then(|(resource_scope, data)| {
            // Only return if resource scope is contained in access scope
            if resource_scope.is_contained_in(access_scope) {
                Some(data)
            } else {
                None
            }
        })
    }
}

fn main() {
    let mut manager = ResourceManager::new();
    
    let project_a = ScopeLevel::Project(ProjectId::new("project-a"));
    let project_b = ScopeLevel::Project(ProjectId::new("project-b"));
    let global = ScopeLevel::Global;
    
    // Store resources in different projects
    manager.store("secret-a".to_string(), project_a.clone(), "data-a".to_string());
    manager.store("secret-b".to_string(), project_b.clone(), "data-b".to_string());
    manager.store("global-config".to_string(), global.clone(), "config-data".to_string());
    
    // Access from project A context
    assert_eq!(manager.get("secret-a", &project_a), Some(&"data-a".to_string()));
    assert_eq!(manager.get("secret-b", &project_a), None);  // Can't access project B's data!
    assert_eq!(manager.get("global-config", &project_a), Some(&"config-data".to_string()));
    
    // Access from global context
    assert_eq!(manager.get("secret-a", &global), Some(&"data-a".to_string()));
    assert_eq!(manager.get("secret-b", &global), Some(&"data-b".to_string()));
    assert_eq!(manager.get("global-config", &global), Some(&"config-data".to_string()));
}
```

---

## 3. Working with Scope Hierarchy

### Extracting IDs from Scopes

```rust
use nebula_core::{ScopeLevel, ProjectId, OrganizationId};

fn print_scope_info(scope: &ScopeLevel) {
    if let Some(org_id) = scope.organization_id() {
        println!("Organization: {}", org_id);
    }
    
    if let Some(project_id) = scope.project_id() {
        println!("Project: {}", project_id);
    }
    
    if let Some(exec_id) = scope.execution_id() {
        println!("Execution: {}", exec_id);
    }
}

fn main() {
    let org_scope = ScopeLevel::Organization(OrganizationId::new("acme"));
    let project_scope = ScopeLevel::Project(ProjectId::new("proj-123"));
    
    print_scope_info(&org_scope);     // Prints: Organization: acme
    print_scope_info(&project_scope);  // Prints: Project: proj-123
}
```

### Creating Child Scopes

```rust
use nebula_core::{ScopeLevel, ChildScopeType, OrganizationId, ProjectId};

fn main() {
    // Start with global scope
    let global = ScopeLevel::Global;
    
    // Create organization under global
    let org = global.child(ChildScopeType::Organization(
        OrganizationId::new("acme")
    )).unwrap();
    assert!(org.is_organization());
    
    // Create project under organization
    let project = org.child(ChildScopeType::Project(
        ProjectId::new("proj-123")
    )).unwrap();
    assert!(project.is_project());
    
    // Invalid: can't create organization under project
    let invalid = project.child(ChildScopeType::Organization(
        OrganizationId::new("invalid")
    ));
    assert!(invalid.is_none());  // Returns None for invalid hierarchy
}
```

### Parent Scope Navigation

```rust
use nebula_core::{ScopeLevel, OrganizationId, ExecutionId, NodeId};

fn main() {
    // Organization parent is Global
    let org = ScopeLevel::Organization(OrganizationId::new("acme"));
    assert_eq!(org.parent(), Some(ScopeLevel::Global));
    
    // Global has no parent
    let global = ScopeLevel::Global;
    assert_eq!(global.parent(), None);
    
    // Action parent is Execution
    let exec_id = ExecutionId::new();
    let action = ScopeLevel::Action(exec_id.clone(), NodeId::new("node-1"));
    assert_eq!(action.parent(), Some(ScopeLevel::Execution(exec_id)));
}
```

---

## 4. RBAC Enum Usage

### ProjectType

```rust
use nebula_core::ProjectType;

fn configure_project(project_type: ProjectType) {
    match project_type {
        ProjectType::Personal => {
            println!("Setting up single-user workspace");
            // Enable personal workflow features
            // Disable collaboration features
        }
        ProjectType::Team => {
            println!("Setting up collaborative workspace");
            // Enable sharing, permissions, notifications
        }
    }
}

fn main() {
    let personal = ProjectType::Personal;
    let team = ProjectType::Team;
    
    configure_project(personal);
    configure_project(team);
    
    // Enums are Copy, so no ownership issues
    let same_team = team;
    assert_eq!(team, same_team);
}
```

### RoleScope

```rust
use nebula_core::RoleScope;

fn check_permission(role_scope: RoleScope, resource_type: &str) -> bool {
    match (role_scope, resource_type) {
        (RoleScope::Global, _) => {
            // Global roles have access to everything
            true
        }
        (RoleScope::Project, "workflow") => {
            // Project roles can access workflows
            true
        }
        (RoleScope::Workflow, "workflow") => {
            // Workflow-specific roles can access workflows
            true
        }
        (RoleScope::Credential, "credential") => {
            // Credential roles can access credentials
            true
        }
        _ => false,
    }
}

fn main() {
    let global_role = RoleScope::Global;
    let project_role = RoleScope::Project;
    
    assert!(check_permission(global_role, "workflow"));
    assert!(check_permission(global_role, "credential"));
    
    assert!(check_permission(project_role, "workflow"));
    assert!(!check_permission(project_role, "credential"));
}
```

---

## 5. Serialization Examples

### JSON Round-Trip

```rust
use nebula_core::{ProjectId, ProjectType, RoleScope};
use serde_json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Serialize ID types
    let project_id = ProjectId::new("proj-123");
    let json = serde_json::to_string(&project_id)?;
    assert_eq!(json, r#""proj-123""#);  // Serializes as plain string
    
    // Deserialize
    let deserialized: ProjectId = serde_json::from_str(&json)?;
    assert_eq!(deserialized, project_id);
    
    // Serialize enums
    let project_type = ProjectType::Team;
    let json = serde_json::to_string(&project_type)?;
    assert_eq!(json, r#""team""#);  // snake_case string
    
    let role_scope = RoleScope::Project;
    let json = serde_json::to_string(&role_scope)?;
    assert_eq!(json, r#""project""#);  // snake_case string
    
    Ok(())
}
```

### Struct with Identity Types

```rust
use nebula_core::{ProjectId, RoleId, ProjectType};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct ProjectConfig {
    id: ProjectId,
    name: String,
    project_type: ProjectType,
    owner_role: RoleId,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ProjectConfig {
        id: ProjectId::new("proj-123"),
        name: "My Project".to_string(),
        project_type: ProjectType::Team,
        owner_role: RoleId::new("role:owner"),
    };
    
    // Serialize to JSON
    let json = serde_json::to_string_pretty(&config)?;
    println!("{}", json);
    /*
    {
      "id": "proj-123",
      "name": "My Project",
      "project_type": "team",
      "owner_role": "role:owner"
    }
    */
    
    // Deserialize back
    let deserialized: ProjectConfig = serde_json::from_str(&json)?;
    assert_eq!(deserialized.id, config.id);
    
    Ok(())
}
```

---

## 6. Integration with Existing Types

### Combining Old and New Scopes

```rust
use nebula_core::{ScopeLevel, ProjectId, ExecutionId, WorkflowId};

fn main() {
    let project = ScopeLevel::Project(ProjectId::new("proj-123"));
    let workflow = ScopeLevel::Workflow(WorkflowId::new("wf-456"));
    let execution = ScopeLevel::Execution(ExecutionId::new());
    
    // New project scope contains existing workflow scope
    assert!(workflow.is_contained_in(&project));
    
    // Existing execution scope works with new project scope
    assert!(execution.is_contained_in(&project));
    
    // Both old and new scopes contained in global
    assert!(project.is_contained_in(&ScopeLevel::Global));
    assert!(workflow.is_contained_in(&ScopeLevel::Global));
}
```

### Mixed ID Types

```rust
use nebula_core::{ProjectId, UserId, ExecutionId};
use std::collections::HashMap;

struct Context {
    project: ProjectId,
    user: UserId,
    execution: ExecutionId,
}

fn main() {
    // Type safety ensures we can't mix these up
    let mut contexts = HashMap::new();
    
    let ctx = Context {
        project: ProjectId::new("proj-123"),
        user: UserId::new("user-456"),
        execution: ExecutionId::new(),
    };
    
    contexts.insert("session-1".to_string(), ctx);
    
    // Each ID type maintains its own type safety
    let retrieved = contexts.get("session-1").unwrap();
    println!("Project: {}", retrieved.project);
    println!("User: {}", retrieved.user);
    println!("Execution: {}", retrieved.execution);
}
```

---

## 7. Common Patterns

### Resource Lookup by Scope

```rust
use nebula_core::{ScopeLevel, ProjectId};
use std::collections::HashMap;

struct ResourceStore {
    data: HashMap<String, ScopeLevel>,
}

impl ResourceStore {
    fn new() -> Self {
        Self { data: HashMap::new() }
    }
    
    fn register(&mut self, resource_id: String, scope: ScopeLevel) {
        self.data.insert(resource_id, scope);
    }
    
    fn find_by_scope(&self, search_scope: &ScopeLevel) -> Vec<&String> {
        self.data
            .iter()
            .filter(|(_, scope)| scope.is_contained_in(search_scope))
            .map(|(id, _)| id)
            .collect()
    }
}

fn main() {
    let mut store = ResourceStore::new();
    
    let project_a = ScopeLevel::Project(ProjectId::new("proj-a"));
    let project_b = ScopeLevel::Project(ProjectId::new("proj-b"));
    
    store.register("resource-1".to_string(), project_a.clone());
    store.register("resource-2".to_string(), project_a.clone());
    store.register("resource-3".to_string(), project_b.clone());
    
    // Find all resources in project A
    let resources = store.find_by_scope(&project_a);
    assert_eq!(resources.len(), 2);
    
    // Find all resources globally
    let all_resources = store.find_by_scope(&ScopeLevel::Global);
    assert_eq!(all_resources.len(), 3);
}
```

### Scope Display Formatting

```rust
use nebula_core::{ScopeLevel, ProjectId, OrganizationId, ExecutionId, NodeId};

fn main() {
    let scopes = vec![
        ScopeLevel::Global,
        ScopeLevel::Organization(OrganizationId::new("acme")),
        ScopeLevel::Project(ProjectId::new("proj-123")),
    ];
    
    for scope in scopes {
        println!("Scope: {}", scope);
    }
    /*
    Scope: global
    Scope: organization:acme
    Scope: project:proj-123
    */
}
```

---

## Next Steps

After implementing these types in `nebula-core`, they will be used by:

1. **nebula-user** (Milestone 1.1): Uses `RoleId` for user roles
2. **nebula-project** (Milestone 1.2): Uses `ProjectId`, `ProjectType` for project management
3. **nebula-rbac** (Milestone 2.2): Uses `RoleId`, `RoleScope` for permission system
4. **nebula-tenant** (Milestone 3.1): Uses `Project` scope level for tenant isolation
5. **nebula-organization** (Milestone 4.3): Uses `OrganizationId`, `Organization` scope level

---

## References

- Specification: [spec.md](./spec.md) - Requirements and acceptance criteria
- Data Model: [data-model.md](./data-model.md) - Complete type definitions
- Research: [research.md](./research.md) - Design decisions and patterns
- nebula-core source: `crates/nebula-core/src/`
