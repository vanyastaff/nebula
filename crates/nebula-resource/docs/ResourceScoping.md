---
title:  ResourceScoping
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---

# Resource Scoping System

## Overview

Resource scoping in nebula-resource provides fine-grained control over resource lifecycle and visibility. Resources can be scoped at different levels: global (application-wide), workflow (shared within a workflow), or action (isolated to a single action).

## Scope Levels

### Scope Hierarchy

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceScope {
    /// Global scope - shared across entire application
    Global,
    
    /// Tenant scope - isolated per tenant
    Tenant { 
        tenant_id: TenantId 
    },
    
    /// Workflow scope - shared within workflow execution
    Workflow { 
        workflow_id: WorkflowId,
        tenant_id: Option<TenantId>,
    },
    
    /// Action scope - isolated to single action
    Action { 
        action_id: ActionId,
        workflow_id: Option<WorkflowId>,
        tenant_id: Option<TenantId>,
    },
    
    /// Custom scope with arbitrary hierarchy
    Custom {
        scope_type: String,
        scope_id: String,
        parent: Option<Box<ResourceScope>>,
    },
}

impl ResourceScope {
    /// Check if this scope is more specific than another
    pub fn is_more_specific_than(&self, other: &ResourceScope) -> bool {
        use ResourceScope::*;
        match (self, other) {
            (Global, _) => false,
            (_, Global) => true,
            (Tenant { .. }, Tenant { .. }) => false,
            (Tenant { .. }, _) => false,
            (_, Tenant { .. }) => true,
            (Workflow { .. }, Workflow { .. }) => false,
            (Workflow { .. }, _) => false,
            (_, Workflow { .. }) => true,
            (Action { .. }, Action { .. }) => false,
            _ => false,
        }
    }
    
    /// Check if this scope contains another scope
    pub fn contains(&self, other: &ResourceScope) -> bool {
        use ResourceScope::*;
        match (self, other) {
            (Global, _) => true,
            (Tenant { tenant_id: t1 }, Tenant { tenant_id: t2 }) => t1 == t2,
            (Tenant { tenant_id: t1 }, Workflow { tenant_id: Some(t2), .. }) => t1 == t2,
            (Tenant { tenant_id: t1 }, Action { tenant_id: Some(t2), .. }) => t1 == t2,
            (Workflow { workflow_id: w1, .. }, Action { workflow_id: Some(w2), .. }) => w1 == w2,
            _ => false,
        }
    }
}
```

## Implementation

### Scoped Resource Manager

```rust
use nebula_resource::prelude::*;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::{HashMap, HashSet};

pub struct ScopedResourceManager {
    /// Resources organized by scope
    resources: Arc<RwLock<ScopeTree>>,
    
    /// Scope resolution strategy
    resolution_strategy: ScopeResolutionStrategy,
    
    /// Access control
    access_control: Arc<dyn AccessControl>,
    
    /// Scope lifecycle hooks
    lifecycle_hooks: Vec<Box<dyn ScopeLifecycleHook>>,
}

struct ScopeTree {
    /// Global resources
    global: HashMap<ResourceId, Arc<dyn Resource>>,
    
    /// Tenant-scoped resources
    tenants: HashMap<TenantId, TenantScope>,
    
    /// Workflow-scoped resources
    workflows: HashMap<WorkflowId, WorkflowScope>,
    
    /// Action-scoped resources
    actions: HashMap<ActionId, ActionScope>,
    
    /// Custom scopes
    custom: HashMap<String, CustomScope>,
}

struct TenantScope {
    resources: HashMap<ResourceId, Arc<dyn Resource>>,
    workflows: HashSet<WorkflowId>,
    config: TenantConfig,
}

struct WorkflowScope {
    resources: HashMap<ResourceId, Arc<dyn Resource>>,
    actions: HashSet<ActionId>,
    parent_tenant: Option<TenantId>,
    created_at: Instant,
    expires_at: Option<Instant>,
}

struct ActionScope {
    resources: HashMap<ResourceId, Arc<dyn Resource>>,
    parent_workflow: Option<WorkflowId>,
    parent_tenant: Option<TenantId>,
    created_at: Instant,
    auto_cleanup: bool,
}
```

### Resource Registration with Scope

```rust
impl ScopedResourceManager {
    /// Register a resource with specific scope
    pub async fn register_resource<R: Resource + 'static>(
        &self,
        resource: R,
        scope: ResourceScope,
    ) -> Result<ResourceHandle<R>> {
        // Check permissions
        self.access_control.check_permission(
            &scope,
            Permission::CreateResource,
        ).await?;
        
        let resource_arc = Arc::new(resource);
        let resource_id = resource_arc.id();
        
        // Add to appropriate scope
        let mut tree = self.resources.write().await;
        match scope {
            ResourceScope::Global => {
                tree.global.insert(resource_id.clone(), resource_arc.clone());
            }
            ResourceScope::Tenant { tenant_id } => {
                tree.tenants
                    .entry(tenant_id)
                    .or_insert_with(|| TenantScope::default())
                    .resources
                    .insert(resource_id.clone(), resource_arc.clone());
            }
            ResourceScope::Workflow { workflow_id, tenant_id } => {
                let workflow_scope = tree.workflows
                    .entry(workflow_id)
                    .or_insert_with(|| WorkflowScope {
                        resources: HashMap::new(),
                        actions: HashSet::new(),
                        parent_tenant: tenant_id,
                        created_at: Instant::now(),
                        expires_at: None,
                    });
                
                workflow_scope.resources.insert(resource_id.clone(), resource_arc.clone());
                
                // Also register workflow in tenant scope
                if let Some(tid) = tenant_id {
                    tree.tenants
                        .entry(tid)
                        .or_insert_with(|| TenantScope::default())
                        .workflows
                        .insert(workflow_id);
                }
            }
            ResourceScope::Action { action_id, workflow_id, tenant_id } => {
                let action_scope = tree.actions
                    .entry(action_id)
                    .or_insert_with(|| ActionScope {
                        resources: HashMap::new(),
                        parent_workflow: workflow_id,
                        parent_tenant: tenant_id,
                        created_at: Instant::now(),
                        auto_cleanup: true,
                    });
                
                action_scope.resources.insert(resource_id.clone(), resource_arc.clone());
                
                // Register in parent scopes
                if let Some(wid) = workflow_id {
                    tree.workflows
                        .entry(wid)
                        .or_insert_with(|| WorkflowScope::default())
                        .actions
                        .insert(action_id);
                }
            }
            ResourceScope::Custom { .. } => {
                // Handle custom scopes
                todo!()
            }
        }
        
        // Notify lifecycle hooks
        for hook in &self.lifecycle_hooks {
            hook.on_resource_registered(&resource_id, &scope).await;
        }
        
        Ok(ResourceHandle::new(resource_arc, scope))
    }
}
```

### Resource Resolution

```rust
#[derive(Debug, Clone)]
pub enum ScopeResolutionStrategy {
    /// Most specific scope wins
    MostSpecific,
    
    /// Least specific scope wins
    LeastSpecific,
    
    /// First found in hierarchy
    FirstFound,
    
    /// Merge resources from all matching scopes
    Merge,
    
    /// Custom resolution logic
    Custom(Arc<dyn Fn(&[ResourceScope]) -> Option<ResourceScope> + Send + Sync>),
}

impl ScopedResourceManager {
    /// Get resource with scope resolution
    pub async fn get_resource(
        &self,
        resource_id: &ResourceId,
        context: &ExecutionContext,
    ) -> Result<Arc<dyn Resource>> {
        // Build scope hierarchy for context
        let scopes = self.build_scope_hierarchy(context);
        
        // Find resource in scopes
        let tree = self.resources.read().await;
        let mut found_resources = Vec::new();
        
        for scope in &scopes {
            let resource = match scope {
                ResourceScope::Global => {
                    tree.global.get(resource_id).cloned()
                }
                ResourceScope::Tenant { tenant_id } => {
                    tree.tenants
                        .get(tenant_id)
                        .and_then(|t| t.resources.get(resource_id))
                        .cloned()
                }
                ResourceScope::Workflow { workflow_id, .. } => {
                    tree.workflows
                        .get(workflow_id)
                        .and_then(|w| w.resources.get(resource_id))
                        .cloned()
                }
                ResourceScope::Action { action_id, .. } => {
                    tree.actions
                        .get(action_id)
                        .and_then(|a| a.resources.get(resource_id))
                        .cloned()
                }
                _ => None,
            };
            
            if let Some(r) = resource {
                found_resources.push((scope.clone(), r));
            }
        }
        
        // Apply resolution strategy
        match self.resolution_strategy {
            ScopeResolutionStrategy::MostSpecific => {
                found_resources.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                found_resources.into_iter().next().map(|(_, r)| r)
            }
            ScopeResolutionStrategy::LeastSpecific => {
                found_resources.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                found_resources.into_iter().next().map(|(_, r)| r)
            }
            ScopeResolutionStrategy::FirstFound => {
                found_resources.into_iter().next().map(|(_, r)| r)
            }
            _ => {
                found_resources.into_iter().next().map(|(_, r)| r)
            }
        }
        .ok_or_else(|| Error::ResourceNotFound(resource_id.clone()))
    }
    
    /// Build scope hierarchy for execution context
    fn build_scope_hierarchy(&self, context: &ExecutionContext) -> Vec<ResourceScope> {
        let mut scopes = Vec::new();
        
        // Most specific first
        if let Some(action_id) = context.action_id() {
            scopes.push(ResourceScope::Action {
                action_id: action_id.clone(),
                workflow_id: context.workflow_id().cloned(),
                tenant_id: context.tenant_id().cloned(),
            });
        }
        
        if let Some(workflow_id) = context.workflow_id() {
            scopes.push(ResourceScope::Workflow {
                workflow_id: workflow_id.clone(),
                tenant_id: context.tenant_id().cloned(),
            });
        }
        
        if let Some(tenant_id) = context.tenant_id() {
            scopes.push(ResourceScope::Tenant {
                tenant_id: tenant_id.clone(),
            });
        }
        
        // Global is always last
        scopes.push(ResourceScope::Global);
        
        scopes
    }
}
```

### Scope Lifecycle Management

```rust
#[async_trait]
pub trait ScopeLifecycleHook: Send + Sync {
    /// Called when resource is registered
    async fn on_resource_registered(&self, resource_id: &ResourceId, scope: &ResourceScope);
    
    /// Called when scope is created
    async fn on_scope_created(&self, scope: &ResourceScope);
    
    /// Called when scope is destroyed
    async fn on_scope_destroyed(&self, scope: &ResourceScope);
    
    /// Called when resource moves between scopes
    async fn on_resource_moved(&self, resource_id: &ResourceId, from: &ResourceScope, to: &ResourceScope);
}

impl ScopedResourceManager {
    /// Clean up action scope when action completes
    pub async fn cleanup_action_scope(&self, action_id: ActionId) -> Result<()> {
        let mut tree = self.resources.write().await;
        
        if let Some(action_scope) = tree.actions.remove(&action_id) {
            // Clean up resources if auto_cleanup is enabled
            if action_scope.auto_cleanup {
                for (_, resource) in action_scope.resources {
                    resource.cleanup().await?;
                }
            }
            
            // Remove from parent workflow
            if let Some(workflow_id) = action_scope.parent_workflow {
                if let Some(workflow) = tree.workflows.get_mut(&workflow_id) {
                    workflow.actions.remove(&action_id);
                }
            }
            
            // Notify hooks
            for hook in &self.lifecycle_hooks {
                hook.on_scope_destroyed(&ResourceScope::Action {
                    action_id,
                    workflow_id: action_scope.parent_workflow,
                    tenant_id: action_scope.parent_tenant,
                }).await;
            }
        }
        
        Ok(())
    }
    
    /// Clean up workflow scope when workflow completes
    pub async fn cleanup_workflow_scope(&self, workflow_id: WorkflowId) -> Result<()> {
        let mut tree = self.resources.write().await;
        
        if let Some(workflow_scope) = tree.workflows.remove(&workflow_id) {
            // Clean up all action scopes
            for action_id in workflow_scope.actions {
                if let Some(action_scope) = tree.actions.remove(&action_id) {
                    if action_scope.auto_cleanup {
                        for (_, resource) in action_scope.resources {
                            resource.cleanup().await?;
                        }
                    }
                }
            }
            
            // Clean up workflow resources
            for (_, resource) in workflow_scope.resources {
                resource.cleanup().await?;
            }
            
            // Remove from parent tenant
            if let Some(tenant_id) = workflow_scope.parent_tenant {
                if let Some(tenant) = tree.tenants.get_mut(&tenant_id) {
                    tenant.workflows.remove(&workflow_id);
                }
            }
            
            // Notify hooks
            for hook in &self.lifecycle_hooks {
                hook.on_scope_destroyed(&ResourceScope::Workflow {
                    workflow_id,
                    tenant_id: workflow_scope.parent_tenant,
                }).await;
            }
        }
        
        Ok(())
    }
}
```

### Scope Isolation and Sharing

```rust
/// Resource sharing policy between scopes
#[derive(Debug, Clone)]
pub enum SharingPolicy {
    /// No sharing between scopes
    Isolated,
    
    /// Read-only sharing
    ReadOnly,
    
    /// Full sharing
    Shared,
    
    /// Custom policy
    Custom(Arc<dyn Fn(&ResourceScope, &ResourceScope) -> bool + Send + Sync>),
}

/// Resource with scope-aware access control
pub struct ScopedResource<R: Resource> {
    inner: Arc<R>,
    scope: ResourceScope,
    sharing_policy: SharingPolicy,
}

impl<R: Resource> ScopedResource<R> {
    /// Check if resource can be accessed from given scope
    pub fn can_access_from(&self, from_scope: &ResourceScope) -> bool {
        match &self.sharing_policy {
            SharingPolicy::Isolated => &self.scope == from_scope,
            SharingPolicy::ReadOnly | SharingPolicy::Shared => {
                self.scope.contains(from_scope) || from_scope.contains(&self.scope)
            }
            SharingPolicy::Custom(policy) => policy(&self.scope, from_scope),
        }
    }
    
    /// Get resource with access check
    pub fn get_with_access_check(&self, from_scope: &ResourceScope) -> Result<&R> {
        if self.can_access_from(from_scope) {
            Ok(&self.inner)
        } else {
            Err(Error::AccessDenied {
                resource: self.inner.id(),
                from_scope: from_scope.clone(),
                resource_scope: self.scope.clone(),
            })
        }
    }
}
```

### Resource Migration Between Scopes

```rust
impl ScopedResourceManager {
    /// Move resource to different scope
    pub async fn move_resource(
        &self,
        resource_id: &ResourceId,
        from_scope: ResourceScope,
        to_scope: ResourceScope,
    ) -> Result<()> {
        // Check permissions for both scopes
        self.access_control.check_permission(
            &from_scope,
            Permission::RemoveResource,
        ).await?;
        
        self.access_control.check_permission(
            &to_scope,
            Permission::CreateResource,
        ).await?;
        
        let mut tree = self.resources.write().await;
        
        // Remove from old scope
        let resource = self.remove_from_scope(&mut tree, resource_id, &from_scope)?;
        
        // Add to new scope
        self.add_to_scope(&mut tree, resource_id, resource.clone(), &to_scope)?;
        
        // Notify lifecycle hooks
        for hook in &self.lifecycle_hooks {
            hook.on_resource_moved(resource_id, &from_scope, &to_scope).await;
        }
        
        Ok(())
    }
    
    /// Promote resource to broader scope
    pub async fn promote_resource(
        &self,
        resource_id: &ResourceId,
        current_scope: ResourceScope,
    ) -> Result<ResourceScope> {
        let promoted_scope = match current_scope {
            ResourceScope::Action { workflow_id, tenant_id, .. } => {
                if let Some(wid) = workflow_id {
                    ResourceScope::Workflow {
                        workflow_id: wid,
                        tenant_id,
                    }
                } else if let Some(tid) = tenant_id {
                    ResourceScope::Tenant { tenant_id: tid }
                } else {
                    ResourceScope::Global
                }
            }
            ResourceScope::Workflow { tenant_id, .. } => {
                if let Some(tid) = tenant_id {
                    ResourceScope::Tenant { tenant_id: tid }
                } else {
                    ResourceScope::Global
                }
            }
            ResourceScope::Tenant { .. } => ResourceScope::Global,
            ResourceScope::Global => return Err(Error::AlreadyGlobalScope),
            _ => return Err(Error::UnsupportedScopePromotion),
        };
        
        self.move_resource(resource_id, current_scope, promoted_scope.clone()).await?;
        Ok(promoted_scope)
    }
}
```

## Advanced Features

### Scope Templates

```rust
/// Template for creating scoped resources
pub struct ScopeTemplate {
    pub name: String,
    pub scope_type: ResourceScope,
    pub resources: Vec<ResourceTemplate>,
    pub sharing_policy: SharingPolicy,
    pub lifecycle: ScopeLifecycle,
}

pub struct ResourceTemplate {
    pub resource_type: String,
    pub config: serde_json::Value,
    pub dependencies: Vec<String>,
}

pub struct ScopeLifecycle {
    pub auto_create: bool,
    pub auto_cleanup: bool,
    pub ttl: Option<Duration>,
    pub max_idle_time: Option<Duration>,
}

impl ScopedResourceManager {
    /// Create resources from template
    pub async fn create_from_template(
        &self,
        template: &ScopeTemplate,
        context: &ExecutionContext,
    ) -> Result<Vec<ResourceId>> {
        let mut created_resources = Vec::new();
        
        // Create scope
        let scope = self.instantiate_scope(&template.scope_type, context);
        
        // Create resources in dependency order
        for resource_template in &template.resources {
            let resource = self.create_resource_from_template(resource_template).await?;
            let handle = self.register_resource(resource, scope.clone()).await?;
            created_resources.push(handle.id());
        }
        
        Ok(created_resources)
    }
}
```

### Scope Metrics

```rust
#[derive(Debug, Clone)]
pub struct ScopeMetrics {
    pub scope: ResourceScope,
    pub resource_count: usize,
    pub total_memory: usize,
    pub active_operations: usize,
    pub created_at: Instant,
    pub last_accessed: Instant,
    pub access_count: u64,
}

impl ScopedResourceManager {
    /// Get metrics for a scope
    pub async fn get_scope_metrics(&self, scope: &ResourceScope) -> Result<ScopeMetrics> {
        let tree = self.resources.read().await;
        
        let resources = match scope {
            ResourceScope::Global => &tree.global,
            ResourceScope::Tenant { tenant_id } => {
                &tree.tenants.get(tenant_id)
                    .ok_or_else(|| Error::ScopeNotFound)?
                    .resources
            }
            ResourceScope::Workflow { workflow_id, .. } => {
                &tree.workflows.get(workflow_id)
                    .ok_or_else(|| Error::ScopeNotFound)?
                    .resources
            }
            ResourceScope::Action { action_id, .. } => {
                &tree.actions.get(action_id)
                    .ok_or_else(|| Error::ScopeNotFound)?
                    .resources
            }
            _ => return Err(Error::UnsupportedScope),
        };
        
        let mut total_memory = 0;
        let mut active_operations = 0;
        
        for resource in resources.values() {
            if let Some(metrics) = resource.metrics().await {
                total_memory += metrics.memory_usage;
                active_operations += metrics.active_operations;
            }
        }
        
        Ok(ScopeMetrics {
            scope: scope.clone(),
            resource_count: resources.len(),
            total_memory,
            active_operations,
            created_at: Instant::now(), // Would track this properly
            last_accessed: Instant::now(),
            access_count: 0, // Would track this properly
        })
    }
}
```

## Configuration Example

```yaml
scoping:
  # Default scope for new resources
  default_scope: workflow
  
  # Scope resolution strategy
  resolution_strategy: most_specific
  
  # Scope lifecycle settings
  lifecycle:
    action:
      auto_cleanup: true
      max_lifetime: 5m
      max_idle: 1m
    
    workflow:
      auto_cleanup: true
      max_lifetime: 1h
      max_idle: 10m
    
    tenant:
      auto_cleanup: false
      max_lifetime: null
      max_idle: null
  
  # Sharing policies
  sharing:
    default: isolated
    
    database_connections:
      policy: shared
      scopes: [workflow, tenant]
    
    credentials:
      policy: read_only
      scopes: [action]
  
  # Scope templates
  templates:
    web_request:
      scope: action
      resources:
        - type: http_client
          config:
            timeout: 30s
            max_retries: 3
        - type: rate_limiter
          config:
            requests_per_second: 10
    
    data_pipeline:
      scope: workflow
      resources:
        - type: database_pool
          config:
            max_connections: 10
        - type: cache
          config:
            max_size: 100MB
        - type: metrics_collector
          config:
            interval: 10s
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_scope_hierarchy() {
        let action_scope = ResourceScope::Action {
            action_id: ActionId::new(),
            workflow_id: Some(WorkflowId::new()),
            tenant_id: Some(TenantId::new()),
        };
        
        let workflow_scope = ResourceScope::Workflow {
            workflow_id: WorkflowId::new(),
            tenant_id: Some(TenantId::new()),
        };
        
        assert!(action_scope.is_more_specific_than(&workflow_scope));
        assert!(workflow_scope.is_more_specific_than(&ResourceScope::Global));
    }
    
    #[tokio::test]
    async fn test_resource_resolution() {
        let manager = ScopedResourceManager::new(ScopeResolutionStrategy::MostSpecific);
        
        // Register same resource ID in different scopes
        let global_resource = MockResource::new("global");
        let workflow_resource = MockResource::new("workflow");
        let action_resource = MockResource::new("action");
        
        let resource_id = ResourceId::new();
        
        manager.register_resource(
            global_resource,
            ResourceScope::Global,
        ).await.unwrap();
        
        manager.register_resource(
            workflow_resource,
            ResourceScope::Workflow {
                workflow_id: WorkflowId::new(),
                tenant_id: None,
            },
        ).await.unwrap();
        
        manager.register_resource(
            action_resource,
            ResourceScope::Action {
                action_id: ActionId::new(),
                workflow_id: Some(WorkflowId::new()),
                tenant_id: None,
            },
        ).await.unwrap();
        
        // Should get action-scoped resource (most specific)
        let context = ExecutionContext::new()
            .with_action_id(ActionId::new())
            .with_workflow_id(WorkflowId::new());
        
        let resource = manager.get_resource(&resource_id, &context).await.unwrap();
        assert_eq!(resource.name(), "action");
    }
    
    #[tokio::test]
    async fn test_scope_cleanup() {
        let manager = ScopedResourceManager::new(ScopeResolutionStrategy::FirstFound);
        
        let action_id = ActionId::new();
        let resource = MockResource::new("test");
        
        manager.register_resource(
            resource,
            ResourceScope::Action {
                action_id: action_id.clone(),
                workflow_id: None,
                tenant_id: None,
            },
        ).await.unwrap();
        
        // Resource should exist
        assert!(manager.has_resource(&resource.id()).await);
        
        // Clean up action scope
        manager.cleanup_action_scope(action_id).await.unwrap();
        
        // Resource should be gone
        assert!(!manager.has_resource(&resource.id()).await);
    }
}
```

## Best Practices

1. **Choose appropriate scope** - Don't make resources global unless necessary
2. **Clean up scopes** - Prevent resource leaks with automatic cleanup
3. **Use scope templates** - Standardize resource creation patterns
4. **Monitor scope metrics** - Track resource usage per scope
5. **Implement access control** - Enforce scope boundaries
6. **Test scope transitions** - Ensure resources move correctly
7. **Document scope policies** - Make sharing rules clear
8. **Use scope hierarchies** - Leverage parent-child relationships
9. **Handle scope failures** - Gracefully handle missing scopes
10. **Version scope schemas** - Support evolution of scope structures