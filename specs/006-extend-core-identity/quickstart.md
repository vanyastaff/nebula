# Quickstart: Using Identity Types in nebula-core

**Feature**: 006-extend-core-identity  
**Updated**: 2026-02-10  
**Phase**: 1 (Design & Contracts)

## Overview

This guide shows how to use the identity and multi-tenancy types in `nebula-core`. All entity IDs use `domain-key` 0.2.1 `Uuid<D>` — typed UUID wrappers that are `Copy`, 16 bytes, and provide compile-time domain separation.

---

## 1. Creating Type-Safe Identifiers

### Basic Usage

```rust
use nebula_core::prelude::*;

fn main() {
    // Create random UUIDs
    let project_id = ProjectId::v4();
    let role_id = RoleId::v4();
    let org_id = OrganizationId::v4();

    println!("Project: {}", project_id);       // prints UUID string
    println!("Role: {}", role_id);
    println!("Organization: {}", org_id);

    // IDs are Copy — no .clone() needed
    let copy = project_id;
    assert_eq!(project_id, copy);  // both usable
}
```

### Parsing from Strings

```rust
use nebula_core::ProjectId;

fn main() {
    // Parse from UUID string
    let id = ProjectId::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
    println!("Parsed: {}", id);

    // TryFrom<&str> also works
    let id: ProjectId = "6ba7b810-9dad-11d1-80b4-00c04fd430c8".try_into().unwrap();

    // Invalid strings are rejected
    let err = ProjectId::parse("not-a-uuid");
    assert!(err.is_err());

    // Nil (zero) UUID for defaults
    let nil = ProjectId::nil();
    assert!(nil.is_nil());
}
```

### Type Safety in Action

```rust
use nebula_core::{ProjectId, UserId};

fn process_project(id: ProjectId) {
    println!("Processing project: {}", id);
}

fn process_user(id: UserId) {
    println!("Processing user: {}", id);
}

fn main() {
    let project_id = ProjectId::v4();
    let user_id = UserId::v4();

    process_project(project_id);  // OK
    process_user(user_id);        // OK

    // process_project(user_id);  // COMPILE ERROR: expected ProjectId, found UserId
}
```

### Accessing the Inner UUID

```rust
use nebula_core::ProjectId;

fn main() {
    let id = ProjectId::v4();

    // Get inner uuid::Uuid
    let uuid: uuid::Uuid = id.get();

    // Get bytes
    let bytes: &[u8; 16] = id.as_bytes();

    // Get domain name
    let domain: &str = id.domain();  // "ProjectId"

    // Convert to string
    let s: String = id.to_string();
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
    let org = ScopeLevel::Organization(OrganizationId::v4());
    let project = ScopeLevel::Project(ProjectId::v4());
    let execution = ScopeLevel::Execution(ExecutionId::v4());

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
    resource_scope.is_contained_in(user_scope)
}

fn main() {
    let org_scope = ScopeLevel::Organization(OrganizationId::v4());
    let project_scope = ScopeLevel::Project(ProjectId::v4());
    let global_scope = ScopeLevel::Global;

    // Project is contained in organization
    assert!(can_access_resource(&project_scope, &org_scope));

    // Both are contained in global
    assert!(can_access_resource(&project_scope, &global_scope));
    assert!(can_access_resource(&org_scope, &global_scope));

    // Organization is NOT contained in project
    assert!(!can_access_resource(&org_scope, &project_scope));
}
```

### Multi-Tenant Isolation Example

```rust
use nebula_core::{ScopeLevel, ProjectId};
use std::collections::HashMap;

struct ResourceManager {
    resources: HashMap<String, (ScopeLevel, String)>,
}

impl ResourceManager {
    fn new() -> Self {
        Self { resources: HashMap::new() }
    }

    fn store(&mut self, key: String, scope: ScopeLevel, data: String) {
        self.resources.insert(key, (scope, data));
    }

    fn get(&self, key: &str, access_scope: &ScopeLevel) -> Option<&String> {
        self.resources.get(key).and_then(|(resource_scope, data)| {
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

    let project_a = ScopeLevel::Project(ProjectId::v4());
    let project_b = ScopeLevel::Project(ProjectId::v4());
    let global = ScopeLevel::Global;

    manager.store("secret-a".into(), project_a.clone(), "data-a".into());
    manager.store("secret-b".into(), project_b.clone(), "data-b".into());

    // Access from project A context — can't see project B's data
    assert_eq!(manager.get("secret-a", &project_a), Some(&"data-a".to_string()));
    assert_eq!(manager.get("secret-b", &project_a), None);

    // Access from global context — sees everything
    assert_eq!(manager.get("secret-a", &global), Some(&"data-a".to_string()));
    assert_eq!(manager.get("secret-b", &global), Some(&"data-b".to_string()));
}
```

---

## 3. Scope Hierarchy Navigation

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
}

fn main() {
    let org_scope = ScopeLevel::Organization(OrganizationId::v4());
    let project_scope = ScopeLevel::Project(ProjectId::v4());

    print_scope_info(&org_scope);      // Organization: <uuid>
    print_scope_info(&project_scope);  // Project: <uuid>
}
```

### Creating Child Scopes

```rust
use nebula_core::{ScopeLevel, ChildScopeType, OrganizationId, ProjectId};

fn main() {
    let global = ScopeLevel::Global;

    // Create organization under global
    let org = global.child(ChildScopeType::Organization(OrganizationId::v4())).unwrap();
    assert!(org.is_organization());

    // Create project under organization
    let project = org.child(ChildScopeType::Project(ProjectId::v4())).unwrap();
    assert!(project.is_project());

    // Invalid: can't create organization under project
    let invalid = project.child(ChildScopeType::Organization(OrganizationId::v4()));
    assert!(invalid.is_none());
}
```

---

## 4. RBAC Enum Usage

### ProjectType

```rust
use nebula_core::ProjectType;

fn configure_project(project_type: ProjectType) {
    match project_type {
        ProjectType::Personal => println!("Single-user workspace"),
        ProjectType::Team => println!("Collaborative workspace"),
    }
}

fn main() {
    let personal = ProjectType::Personal;
    let team = ProjectType::Team;

    configure_project(personal);
    configure_project(team);

    // Copy — both still usable
    let same_team = team;
    assert_eq!(team, same_team);
}
```

### RoleScope

```rust
use nebula_core::RoleScope;

fn check_permission(role_scope: RoleScope, resource_type: &str) -> bool {
    match (role_scope, resource_type) {
        (RoleScope::Global, _) => true,
        (RoleScope::Project, "workflow") => true,
        (RoleScope::Workflow, "workflow") => true,
        (RoleScope::Credential, "credential") => true,
        _ => false,
    }
}

fn main() {
    assert!(check_permission(RoleScope::Global, "workflow"));
    assert!(check_permission(RoleScope::Global, "credential"));
    assert!(check_permission(RoleScope::Project, "workflow"));
    assert!(!check_permission(RoleScope::Project, "credential"));
}
```

---

## 5. Serialization

### JSON Round-Trip

```rust
use nebula_core::{ProjectId, ProjectType, RoleScope};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ID serializes as UUID string
    let project_id = ProjectId::v4();
    let json = serde_json::to_string(&project_id)?;
    // json = "\"550e8400-e29b-41d4-a716-446655440000\""

    let deserialized: ProjectId = serde_json::from_str(&json)?;
    assert_eq!(deserialized, project_id);

    // Enums serialize as snake_case strings
    let json = serde_json::to_string(&ProjectType::Team)?;
    assert_eq!(json, r#""team""#);

    let json = serde_json::to_string(&RoleScope::Project)?;
    assert_eq!(json, r#""project""#);

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
        id: ProjectId::v4(),
        name: "My Project".to_string(),
        project_type: ProjectType::Team,
        owner_role: RoleId::v4(),
    };

    let json = serde_json::to_string_pretty(&config)?;
    println!("{}", json);
    /*
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "My Project",
      "project_type": "team",
      "owner_role": "6ba7b810-9dad-11d1-80b4-00c04fd430c8"
    }
    */

    let deserialized: ProjectConfig = serde_json::from_str(&json)?;
    assert_eq!(deserialized.id, config.id);

    Ok(())
}
```

---

## 6. Integration with Existing Types

### Combining Scopes

```rust
use nebula_core::{ScopeLevel, ProjectId, WorkflowId, ExecutionId};

fn main() {
    let project = ScopeLevel::Project(ProjectId::v4());
    let workflow = ScopeLevel::Workflow(WorkflowId::v4());
    let execution = ScopeLevel::Execution(ExecutionId::v4());

    // Project scope contains workflow and execution
    assert!(workflow.is_contained_in(&project));
    assert!(execution.is_contained_in(&project));

    // All contained in global
    assert!(project.is_contained_in(&ScopeLevel::Global));
    assert!(workflow.is_contained_in(&ScopeLevel::Global));
}
```

### Display Formatting

```rust
use nebula_core::{ScopeLevel, ProjectId, OrganizationId};

fn main() {
    let scopes = vec![
        ScopeLevel::Global,
        ScopeLevel::Organization(OrganizationId::v4()),
        ScopeLevel::Project(ProjectId::v4()),
    ];

    for scope in scopes {
        println!("Scope: {}", scope);
    }
    // Scope: global
    // Scope: organization:550e8400-e29b-41d4-a716-446655440000
    // Scope: project:6ba7b810-9dad-11d1-80b4-00c04fd430c8
}
```

---

## References

- Specification: [spec.md](./spec.md)
- Data Model: [data-model.md](./data-model.md)
- Research: [research.md](./research.md)
- domain-key 0.2.1: https://crates.io/crates/domain-key
- nebula-core source: `crates/nebula-core/src/`
