---
title: Scope Violations and Permission Errors
description: Troubleshooting guide for scope violations, ACL errors, and permission debugging
tags: [troubleshooting, permissions, acl, scope, access-control, security]
related:
  - "[[Common-Errors]]"
  - "[[../Advanced/Access-Control]]"
  - "[[../Getting-Started/Core-Concepts]]"
  - "[[Debugging-Checklist]]"
status: published
version: 1.0.0
---

# Scope Violations and Permission Errors

Comprehensive troubleshooting for scope violations, ACL misconfigurations, and permission debugging in `nebula-credential`.

---

## Overview

**Access Control Model**:
- **Ownership**: Every credential has an owner with full permissions
- **ACL (Access Control List)**: Owners can grant permissions to other principals
- **Scopes**: Resource isolation (global, workflow-specific, node-specific)

**Permission Types**:
1. `Read` - Retrieve credential
2. `Write` - Modify credential
3. `Delete` - Remove credential
4. `Rotate` - Rotate credential
5. `Grant` - Grant permissions to others
6. `Execute` - Use credential for authentication

---

## Quick Diagnosis

| Error Message | Cause | Solution |
|---------------|-------|----------|
| `User 'alice' cannot rotate credential owned by 'bob'` | Not owner | Request permission from bob |
| `Read-only access, cannot delete credential` | Insufficient permission | Request Delete permission |
| `Scope 'workflow:123' does not match credential scope 'global'` | Wrong scope | Use correct scope_id |
| `Cannot grant higher privilege than own` | Grant escalation attempt | Can only grant owned permissions |
| `Principal 'charlie' not found in ACL` | No ACL entry | Owner must grant access |

---

## 1. Permission Denied Errors

### 1.1 Not the Owner

**Error**: `PermissionDenied("User 'alice' cannot rotate credential owned by 'bob'")`

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

match manager.rotate_credential(&id, &policy, &ctx).await {
    Err(CredentialError::PermissionDenied(msg)) => {
        eprintln!("Permission denied: {msg}");
        
        // Check ownership
        let metadata = manager.get_metadata(&id).await?;
        eprintln!("\nOwnership:");
        eprintln!("  Owner: {}", metadata.owner_id);
        eprintln!("  Requester: {}", ctx.owner_id);
        
        if metadata.owner_id != ctx.owner_id {
            eprintln!("\n⚠️  You are not the owner of this credential");
            eprintln!("   Options:");
            eprintln!("   1. Request permission from owner");
            eprintln!("   2. Use owner's context");
            eprintln!("   3. Transfer ownership (requires owner approval)");
        }
    }
    Ok(_) => println!("Rotation successful"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

**Solutions**:

1. **Request permission from owner**:

```rust
use nebula_credential::prelude::*;

pub async fn request_permission(
    manager: &CredentialManager,
    id: &CredentialId,
    requester: &OwnerId,
    permission: Permission,
) -> Result<(), CredentialError> {
    let metadata = manager.get_metadata(id).await?;
    let owner_id = metadata.owner_id;
    
    eprintln!("Requesting {} permission for credential {id}", permission_name(&permission));
    eprintln!("From owner: {owner_id}");
    eprintln!("For principal: {requester}");
    
    // Send notification to owner (implementation-specific)
    send_permission_request_notification(&owner_id, requester, id, permission).await?;
    
    eprintln!("✓ Permission request sent");
    eprintln!("  Awaiting owner approval");
    
    Ok(())
}

fn permission_name(permission: &Permission) -> &'static str {
    match permission {
        Permission::Read => "Read",
        Permission::Write => "Write",
        Permission::Delete => "Delete",
        Permission::Rotate => "Rotate",
        Permission::Grant => "Grant",
        Permission::Execute => "Execute",
    }
}
```

2. **Owner grants permission**:

```rust
use nebula_credential::prelude::*;

pub async fn grant_permission_as_owner(
    manager: &CredentialManager,
    id: &CredentialId,
    grantee: &OwnerId,
    permissions: PermissionSet,
    owner_ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    // Verify caller is owner
    let metadata = manager.get_metadata(id).await?;
    if metadata.owner_id != owner_ctx.owner_id {
        return Err(CredentialError::PermissionDenied(
            "Only owner can grant permissions".to_string()
        ));
    }
    
    // Get current ACL
    let mut acl = manager.get_acl(id).await?;
    
    eprintln!("Granting permissions to {grantee}:");
    if permissions.can_read { eprintln!("  ✓ Read"); }
    if permissions.can_write { eprintln!("  ✓ Write"); }
    if permissions.can_delete { eprintln!("  ✓ Delete"); }
    if permissions.can_rotate { eprintln!("  ✓ Rotate"); }
    if permissions.can_test { eprintln!("  ✓ Test"); }
    if permissions.can_share { eprintln!("  ✓ Share"); }
    
    // Grant access
    acl.grant_access(
        grantee.as_str().to_string(),
        PrincipalType::User,
        permissions,
        owner_ctx.owner_id.as_str().to_string(),
    );
    
    // Save ACL
    manager.update_acl(id, &acl).await?;
    
    eprintln!("\n✓ Permissions granted");
    
    Ok(())
}
```

3. **Transfer ownership**:

```rust
use nebula_credential::prelude::*;

pub async fn transfer_ownership(
    manager: &CredentialManager,
    id: &CredentialId,
    new_owner: &OwnerId,
    current_owner_ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    // Verify caller is current owner
    let metadata = manager.get_metadata(id).await?;
    if metadata.owner_id != current_owner_ctx.owner_id {
        return Err(CredentialError::PermissionDenied(
            "Only owner can transfer ownership".to_string()
        ));
    }
    
    eprintln!("Transferring ownership:");
    eprintln!("  From: {}", metadata.owner_id);
    eprintln!("  To: {new_owner}");
    
    // Update metadata
    let mut new_metadata = metadata.clone();
    new_metadata.owner_id = new_owner.clone();
    new_metadata.updated_at = Utc::now();
    
    manager.update_metadata(id, &new_metadata).await?;
    
    // Optional: Grant previous owner some permissions
    let mut acl = manager.get_acl(id).await?;
    acl.grant_access(
        metadata.owner_id.as_str().to_string(),
        PrincipalType::User,
        PermissionSet::read_only(),
        new_owner.as_str().to_string(),
    );
    manager.update_acl(id, &acl).await?;
    
    eprintln!("✓ Ownership transferred");
    eprintln!("  Previous owner retains read-only access");
    
    Ok(())
}
```

---

### 1.2 Insufficient Permission Level

**Error**: `PermissionDenied("Read-only access, cannot delete credential")`

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

pub async fn check_permissions(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    let acl = manager.get_acl(id).await?;
    let principal_id = ctx.owner_id.as_str();
    
    eprintln!("Permission check for {principal_id}:");
    eprintln!("  Read:   {}", if acl.has_permission(principal_id, Permission::Read) { "✓" } else { "✗" });
    eprintln!("  Write:  {}", if acl.has_permission(principal_id, Permission::Write) { "✓" } else { "✗" });
    eprintln!("  Delete: {}", if acl.has_permission(principal_id, Permission::Delete) { "✓" } else { "✗" });
    eprintln!("  Rotate: {}", if acl.has_permission(principal_id, Permission::Rotate) { "✓" } else { "✗" });
    eprintln!("  Test:   {}", if acl.has_permission(principal_id, Permission::Test) { "✓" } else { "✗" });
    eprintln!("  Grant:  {}", if acl.has_permission(principal_id, Permission::Grant) { "✓" } else { "✗" });
    
    Ok(())
}
```

**Solution**: Request elevated permission from owner

```rust
use nebula_credential::prelude::*;

// Request specific permission
request_permission(
    &manager,
    &id,
    &ctx.owner_id,
    Permission::Delete,
).await?;
```

---

### 1.3 Grant Permission Escalation

**Error**: `PermissionDenied("Cannot grant higher privilege than own")`

**Cause**: User trying to grant permission they don't have

**Example**:

```rust
use nebula_credential::prelude::*;

// Alice has Read + Write permissions
let alice_permissions = PermissionSet {
    can_read: true,
    can_write: true,
    can_delete: false,  // Alice cannot delete
    can_rotate: false,
    can_test: true,
    can_share: false,
};

// Alice tries to grant Delete permission to Bob (invalid!)
acl.grant_access(
    "bob".to_string(),
    PrincipalType::User,
    PermissionSet {
        can_delete: true,  // ✗ Alice cannot grant this
        ..PermissionSet::read_only()
    },
    "alice".to_string(),
)?;  // Error: Cannot grant higher privilege than own
```

**Solution**: Only grant permissions you have

```rust
use nebula_credential::prelude::*;

pub fn grant_safe(
    acl: &mut AccessControlList,
    granter: &str,
    grantee: &str,
    requested_permissions: PermissionSet,
) -> Result<(), CredentialError> {
    // Get granter's current permissions
    let granter_perms = acl.get_permissions(granter)
        .ok_or_else(|| CredentialError::PermissionDenied(
            format!("{granter} not found in ACL")
        ))?;
    
    // Validate each permission
    let mut safe_permissions = PermissionSet {
        can_read: false,
        can_write: false,
        can_delete: false,
        can_rotate: false,
        can_test: false,
        can_share: false,
    };
    
    if requested_permissions.can_read && granter_perms.can_read {
        safe_permissions.can_read = true;
    }
    if requested_permissions.can_write && granter_perms.can_write {
        safe_permissions.can_write = true;
    }
    if requested_permissions.can_delete && granter_perms.can_delete {
        safe_permissions.can_delete = true;
    }
    if requested_permissions.can_rotate && granter_perms.can_rotate {
        safe_permissions.can_rotate = true;
    }
    if requested_permissions.can_test && granter_perms.can_test {
        safe_permissions.can_test = true;
    }
    if requested_permissions.can_share && granter_perms.can_share {
        safe_permissions.can_share = true;
    }
    
    // Check if granter has Grant permission
    if !granter_perms.can_share {
        return Err(CredentialError::PermissionDenied(
            format!("{granter} lacks Grant permission")
        ));
    }
    
    // Grant validated permissions
    acl.grant_access(
        grantee.to_string(),
        PrincipalType::User,
        safe_permissions,
        granter.to_string(),
    );
    
    Ok(())
}
```

---

## 2. Scope Violations

### 2.1 Scope Mismatch

**Error**: `PermissionDenied("Scope 'workflow:123' does not match credential scope 'global'")`

**Cause**: Credential accessed with wrong scope context

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let metadata = manager.get_metadata(&id).await?;

eprintln!("Scope mismatch:");
eprintln!("  Credential scope: {:?}", metadata.scope_id);
eprintln!("  Request scope: {:?}", ctx.scope_id);

if metadata.scope_id != ctx.scope_id {
    eprintln!("\n⚠️  Scope mismatch detected");
    
    match (&metadata.scope_id, &ctx.scope_id) {
        (Some(cred_scope), None) => {
            eprintln!("  Credential is scoped to: {cred_scope}");
            eprintln!("  Request has no scope (global)");
            eprintln!("  Solution: Add scope to context");
        }
        (None, Some(req_scope)) => {
            eprintln!("  Credential is global");
            eprintln!("  Request scoped to: {req_scope}");
            eprintln!("  Solution: Remove scope from context or use global credential");
        }
        (Some(cred_scope), Some(req_scope)) => {
            eprintln!("  Credential scope: {cred_scope}");
            eprintln!("  Request scope: {req_scope}");
            eprintln!("  Solution: Use correct scope in context");
        }
        (None, None) => {
            eprintln!("  Both are global (no mismatch?)");
        }
    }
}
```

**Solution**: Use correct scope

```rust
use nebula_credential::prelude::*;

// Create context with correct scope
let ctx = CredentialContext::new(owner_id)
    .with_scope(ScopeId::workflow("workflow-123"));

// Retrieve with scope
let credential = manager.retrieve_credential(&id, &ctx).await?;
```

---

### 2.2 List Credentials by Scope

**Usage**: Find credentials in specific scope

```rust
use nebula_credential::prelude::*;

pub async fn list_credentials_by_scope(
    manager: &CredentialManager,
    scope: &ScopeId,
    ctx: &CredentialContext,
) -> Result<Vec<CredentialMetadata>, CredentialError> {
    let filter = CredentialFilter::new()
        .owner(ctx.owner_id.clone())
        .scope(scope.clone());
    
    let credentials = manager.list_credentials(Some(&filter)).await?;
    
    eprintln!("Credentials in scope {scope}:");
    for metadata in &credentials {
        eprintln!("  - {} ({})", metadata.id, metadata.credential_type);
    }
    
    Ok(credentials)
}

// Usage
let workflow_scope = ScopeId::workflow("workflow-123");
let creds = list_credentials_by_scope(&manager, &workflow_scope, &ctx).await?;
```

---

### 2.3 Scope Hierarchies

**Pattern**: Workflow > Node > Operation

```rust
use nebula_credential::prelude::*;

pub enum ScopeHierarchy {
    Global,
    Workflow(String),
    Node(String, String),  // (workflow_id, node_id)
    Operation(String, String, String),  // (workflow_id, node_id, operation_id)
}

impl ScopeHierarchy {
    pub fn to_scope_id(&self) -> ScopeId {
        match self {
            Self::Global => ScopeId::global(),
            Self::Workflow(wf) => ScopeId::new(format!("workflow:{wf}")),
            Self::Node(wf, node) => ScopeId::new(format!("workflow:{wf}:node:{node}")),
            Self::Operation(wf, node, op) => ScopeId::new(format!("workflow:{wf}:node:{node}:op:{op}")),
        }
    }
    
    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Global, _) => true,
            (Self::Workflow(wf1), Self::Node(wf2, _)) if wf1 == wf2 => true,
            (Self::Workflow(wf1), Self::Operation(wf2, _, _)) if wf1 == wf2 => true,
            (Self::Node(wf1, n1), Self::Operation(wf2, n2, _)) if wf1 == wf2 && n1 == n2 => true,
            _ => false,
        }
    }
}

// Example
let workflow_scope = ScopeHierarchy::Workflow("wf-123".to_string());
let node_scope = ScopeHierarchy::Node("wf-123".to_string(), "node-456".to_string());

assert!(workflow_scope.is_ancestor_of(&node_scope));
```

---

## 3. ACL Debugging

### 3.1 Inspect ACL

**Tool**: Comprehensive ACL inspector

```rust
use nebula_credential::prelude::*;

pub async fn inspect_acl(
    manager: &CredentialManager,
    id: &CredentialId,
) -> Result<(), CredentialError> {
    let metadata = manager.get_metadata(id).await?;
    let acl = manager.get_acl(id).await?;
    
    eprintln!("=== ACL Inspection for {} ===\n", id);
    
    eprintln!("Owner: {}", acl.owner_id);
    eprintln!("  Permissions: FULL (owner has all permissions)\n");
    
    if acl.entries.is_empty() {
        eprintln!("No additional ACL entries (owner-only access)\n");
    } else {
        eprintln!("ACL Entries ({} total):", acl.entries.len());
        
        for (idx, entry) in acl.entries.iter().enumerate() {
            eprintln!("\n  Entry {}: {}", idx + 1, entry.principal_id);
            eprintln!("    Type: {:?}", entry.principal_type);
            eprintln!("    Permissions:");
            eprintln!("      Read:   {}", if entry.permissions.can_read { "✓" } else { "✗" });
            eprintln!("      Write:  {}", if entry.permissions.can_write { "✓" } else { "✗" });
            eprintln!("      Delete: {}", if entry.permissions.can_delete { "✓" } else { "✗" });
            eprintln!("      Rotate: {}", if entry.permissions.can_rotate { "✓" } else { "✗" });
            eprintln!("      Test:   {}", if entry.permissions.can_test { "✓" } else { "✗" });
            eprintln!("      Grant:  {}", if entry.permissions.can_share { "✓" } else { "✗" });
            eprintln!("    Granted at: {}", entry.granted_at);
            eprintln!("    Granted by: {}", entry.granted_by);
        }
    }
    
    eprintln!("\n=== End ACL Inspection ===");
    
    Ok(())
}

// Usage
inspect_acl(&manager, &credential_id).await?;
```

**Output Example**:

```
=== ACL Inspection for cred-12345 ===

Owner: bob
  Permissions: FULL (owner has all permissions)

ACL Entries (2 total):

  Entry 1: alice
    Type: User
    Permissions:
      Read:   ✓
      Write:  ✓
      Delete: ✗
      Rotate: ✗
      Test:   ✓
      Grant:  ✗
    Granted at: 2026-02-03 10:30:00 UTC
    Granted by: bob

  Entry 2: service-account-1
    Type: Service
    Permissions:
      Read:   ✓
      Write:  ✗
      Delete: ✗
      Rotate: ✗
      Test:   ✓
      Grant:  ✗
    Granted at: 2026-02-03 11:15:00 UTC
    Granted by: bob

=== End ACL Inspection ===
```

---

### 3.2 Audit Permission Changes

**Tool**: Track ACL modifications

```rust
use nebula_credential::prelude::*;

pub struct AclAuditLog {
    pub credential_id: CredentialId,
    pub timestamp: DateTime<Utc>,
    pub action: AclAction,
    pub principal: String,
    pub performed_by: String,
}

pub enum AclAction {
    PermissionGranted(PermissionSet),
    PermissionRevoked,
    OwnershipTransferred { from: OwnerId, to: OwnerId },
}

pub async fn audit_acl_changes(
    manager: &CredentialManager,
    id: &CredentialId,
) -> Result<Vec<AclAuditLog>, CredentialError> {
    let logs = manager.get_acl_audit_log(id).await?;
    
    eprintln!("ACL Audit Log for {id}:");
    for log in &logs {
        eprintln!("\n[{}] {}", log.timestamp, log.performed_by);
        match &log.action {
            AclAction::PermissionGranted(perms) => {
                eprintln!("  Granted permissions to: {}", log.principal);
                if perms.can_read { eprintln!("    + Read"); }
                if perms.can_write { eprintln!("    + Write"); }
                if perms.can_delete { eprintln!("    + Delete"); }
                if perms.can_rotate { eprintln!("    + Rotate"); }
                if perms.can_test { eprintln!("    + Test"); }
                if perms.can_share { eprintln!("    + Grant"); }
            }
            AclAction::PermissionRevoked => {
                eprintln!("  Revoked all permissions from: {}", log.principal);
            }
            AclAction::OwnershipTransferred { from, to } => {
                eprintln!("  Transferred ownership: {from} → {to}");
            }
        }
    }
    
    Ok(logs)
}
```

---

### 3.3 Common ACL Misconfigurations

**Issue 1**: Missing ACL entry

```rust
// Check if principal has ANY access
let acl = manager.get_acl(&id).await?;

if !acl.has_any_permission("alice") {
    eprintln!("⚠️  Principal 'alice' has no permissions");
    eprintln!("   Owner must grant access");
}
```

**Issue 2**: Over-permissive ACL

```rust
// Audit for excessive permissions
for entry in &acl.entries {
    let perm_count = [
        entry.permissions.can_read,
        entry.permissions.can_write,
        entry.permissions.can_delete,
        entry.permissions.can_rotate,
        entry.permissions.can_test,
        entry.permissions.can_share,
    ].iter().filter(|&&p| p).count();
    
    if perm_count > 4 {
        eprintln!("⚠️  {} has {} permissions (potentially excessive)",
            entry.principal_id, perm_count);
    }
}
```

**Issue 3**: Orphaned ACL entries

```rust
// Check for inactive principals
for entry in &acl.entries {
    if !is_principal_active(&entry.principal_id).await? {
        eprintln!("⚠️  Orphaned ACL entry for inactive principal: {}",
            entry.principal_id);
        eprintln!("   Recommendation: Revoke access");
    }
}
```

---

## 4. Multi-Tenancy and Isolation

### 4.1 Verify Tenant Isolation

```rust
use nebula_credential::prelude::*;

pub async fn verify_tenant_isolation(
    manager: &CredentialManager,
    tenant_a: &OwnerId,
    tenant_b: &OwnerId,
) -> Result<(), CredentialError> {
    eprintln!("=== Tenant Isolation Verification ===\n");
    
    // List tenant A's credentials
    let filter_a = CredentialFilter::new().owner(tenant_a.clone());
    let creds_a = manager.list_credentials(Some(&filter_a)).await?;
    
    eprintln!("Tenant A ({}) credentials: {}", tenant_a, creds_a.len());
    
    // List tenant B's credentials
    let filter_b = CredentialFilter::new().owner(tenant_b.clone());
    let creds_b = manager.list_credentials(Some(&filter_b)).await?;
    
    eprintln!("Tenant B ({}) credentials: {}", tenant_b, creds_b.len());
    
    // Verify tenant B cannot access tenant A's credentials
    let ctx_b = CredentialContext::new(tenant_b.clone());
    
    for metadata_a in &creds_a {
        match manager.retrieve_credential(&metadata_a.id, &ctx_b).await {
            Err(CredentialError::PermissionDenied(_)) => {
                eprintln!("✓ Tenant B correctly denied access to {}", metadata_a.id);
            }
            Ok(_) => {
                eprintln!("✗ SECURITY VIOLATION: Tenant B can access {}", metadata_a.id);
                return Err(CredentialError::PermissionDenied(
                    "Tenant isolation violated".to_string()
                ));
            }
            Err(e) => {
                eprintln!("? Unexpected error for {}: {e}", metadata_a.id);
            }
        }
    }
    
    eprintln!("\n✓ Tenant isolation verified");
    
    Ok(())
}
```

---

## Related Documentation

- [[Common-Errors]] - All error types
- [[../Advanced/Access-Control]] - Access control model
- [[../Getting-Started/Core-Concepts]] - Scopes and ownership
- [[Debugging-Checklist]] - Systematic debugging

---

## Summary

This guide covers:

✅ **Permission denied** errors and solutions  
✅ **Scope violations** and mismatch resolution  
✅ **ACL debugging** tools and inspectors  
✅ **Grant permission** escalation prevention  
✅ **Multi-tenancy** isolation verification  
✅ **Ownership transfer** procedures  
✅ **Audit logging** for permission changes  

Use the ACL inspection tools to diagnose permission issues, and always verify scopes match between credentials and requests.
