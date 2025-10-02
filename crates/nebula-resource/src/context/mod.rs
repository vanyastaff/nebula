//! Advanced context awareness and propagation for resource management
//!
//! This module provides sophisticated context tracking and propagation mechanisms
//! for the Nebula resource management system, enabling distributed tracing,
//! multi-tenancy, and execution context awareness.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

pub mod propagation;
pub mod tracing;

/// Represents the execution environment for resource operations
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Unique identifier for this execution context
    pub id: Uuid,
    /// The workflow that initiated this execution
    pub workflow_id: String,
    /// Specific execution instance within the workflow
    pub execution_id: String,
    /// Current action or step being executed
    pub action_id: String,
    /// User or system identity
    pub identity: IdentityContext,
    /// Tenant information for multi-tenancy
    pub tenant: TenantContext,
    /// Environment configuration
    pub environment: EnvironmentContext,
    /// Tracing and observability context
    pub tracing: TracingContext,
    /// Custom metadata for extensions
    pub metadata: HashMap<String, serde_json::Value>,
    /// Creation timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl ExecutionContext {
    /// Create a new execution context
    pub fn new(
        workflow_id: String,
        execution_id: String,
        action_id: String,
        identity: IdentityContext,
        tenant: TenantContext,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            workflow_id,
            execution_id,
            action_id,
            identity,
            tenant,
            environment: EnvironmentContext::default(),
            tracing: TracingContext::new(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Create a child context for nested operations
    pub fn create_child(&self, action_id: String) -> Self {
        let mut child = self.clone();
        child.id = Uuid::new_v4();
        child.action_id = action_id;
        child.created_at = chrono::Utc::now();
        child.tracing = self.tracing.create_child();
        child
    }

    /// Add custom metadata
    pub fn with_metadata(mut self, key: String, value: serde_json::Value) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Get metadata value
    pub fn get_metadata(&self, key: &str) -> Option<&serde_json::Value> {
        self.metadata.get(key)
    }

    /// Check if this context is for a specific tenant
    pub fn is_tenant(&self, tenant_id: &str) -> bool {
        self.tenant.id == tenant_id
    }

    /// Check if this context has a specific capability
    pub fn has_capability(&self, capability: &str) -> bool {
        self.identity.capabilities.contains(capability)
    }

    /// Get execution path (workflow -> execution -> action)
    pub fn execution_path(&self) -> String {
        format!("{}/{}/{}", self.workflow_id, self.execution_id, self.action_id)
    }
}

/// Identity context for authentication and authorization
#[derive(Debug, Clone)]
pub struct IdentityContext {
    /// User or service identifier
    pub user_id: String,
    /// Session or token identifier
    pub session_id: Option<String>,
    /// User roles
    pub roles: Vec<String>,
    /// Specific capabilities/permissions
    pub capabilities: Vec<String>,
    /// Authentication method used
    pub auth_method: AuthMethod,
    /// Authentication timestamp
    pub authenticated_at: chrono::DateTime<chrono::Utc>,
}

impl IdentityContext {
    /// Create a new identity context
    pub fn new(user_id: String, roles: Vec<String>, capabilities: Vec<String>) -> Self {
        Self {
            user_id,
            session_id: None,
            roles,
            capabilities,
            auth_method: AuthMethod::Internal,
            authenticated_at: chrono::Utc::now(),
        }
    }

    /// Create a system identity context
    pub fn system() -> Self {
        Self {
            user_id: "system".to_string(),
            session_id: None,
            roles: vec!["system".to_string()],
            capabilities: vec!["*".to_string()],
            auth_method: AuthMethod::Internal,
            authenticated_at: chrono::Utc::now(),
        }
    }

    /// Check if identity has a specific role
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.contains(&role.to_string()) || self.roles.contains(&"*".to_string())
    }

    /// Check if identity has a specific capability
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities.contains(&capability.to_string()) || self.capabilities.contains(&"*".to_string())
    }
}

/// Authentication method
#[derive(Debug, Clone, PartialEq)]
pub enum AuthMethod {
    /// Internal system authentication
    Internal,
    /// Token-based authentication
    Token,
    /// Certificate-based authentication
    Certificate,
    /// OAuth authentication
    OAuth,
    /// API key authentication
    ApiKey,
}

/// Tenant context for multi-tenancy
#[derive(Debug, Clone)]
pub struct TenantContext {
    /// Tenant identifier
    pub id: String,
    /// Tenant display name
    pub name: String,
    /// Tenant configuration
    pub config: HashMap<String, serde_json::Value>,
    /// Resource quotas and limits
    pub quotas: ResourceQuotas,
    /// Tenant status
    pub status: TenantStatus,
}

impl TenantContext {
    /// Create a new tenant context
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            config: HashMap::new(),
            quotas: ResourceQuotas::default(),
            status: TenantStatus::Active,
        }
    }

    /// Create a default tenant context
    pub fn default_tenant() -> Self {
        Self::new("default".to_string(), "Default Tenant".to_string())
    }

    /// Check if tenant can allocate resources
    pub fn can_allocate(&self, resource_type: &str, count: u32) -> bool {
        match self.status {
            TenantStatus::Active => self.quotas.can_allocate(resource_type, count),
            _ => false,
        }
    }

    /// Get tenant configuration value
    pub fn get_config(&self, key: &str) -> Option<&serde_json::Value> {
        self.config.get(key)
    }
}

/// Tenant status
#[derive(Debug, Clone, PartialEq)]
pub enum TenantStatus {
    /// Tenant is active and can use resources
    Active,
    /// Tenant is suspended (read-only access)
    Suspended,
    /// Tenant is disabled (no access)
    Disabled,
    /// Tenant is being migrated
    Migrating,
}

/// Resource quotas for tenants
#[derive(Debug, Clone)]
pub struct ResourceQuotas {
    /// Maximum number of resources by type
    pub max_resources: HashMap<String, u32>,
    /// Currently allocated resources
    pub allocated: HashMap<String, u32>,
    /// Memory limits in bytes
    pub max_memory: u64,
    /// CPU limits in millicores
    pub max_cpu: u32,
    /// Storage limits in bytes
    pub max_storage: u64,
}

impl Default for ResourceQuotas {
    fn default() -> Self {
        Self {
            max_resources: HashMap::new(),
            allocated: HashMap::new(),
            max_memory: 1024 * 1024 * 1024, // 1GB
            max_cpu: 1000,                   // 1 CPU core
            max_storage: 10 * 1024 * 1024 * 1024, // 10GB
        }
    }
}

impl ResourceQuotas {
    /// Check if more resources can be allocated
    pub fn can_allocate(&self, resource_type: &str, count: u32) -> bool {
        let max = self.max_resources.get(resource_type).copied().unwrap_or(u32::MAX);
        let current = self.allocated.get(resource_type).copied().unwrap_or(0);
        current + count <= max
    }

    /// Allocate resources
    pub fn allocate(&mut self, resource_type: &str, count: u32) -> Result<(), String> {
        if !self.can_allocate(resource_type, count) {
            return Err(format!("Cannot allocate {} resources of type {}", count, resource_type));
        }

        *self.allocated.entry(resource_type.to_string()).or_insert(0) += count;
        Ok(())
    }

    /// Deallocate resources
    pub fn deallocate(&mut self, resource_type: &str, count: u32) {
        if let Some(allocated) = self.allocated.get_mut(resource_type) {
            *allocated = allocated.saturating_sub(count);
        }
    }
}

/// Environment context
#[derive(Debug, Clone)]
pub struct EnvironmentContext {
    /// Environment name (dev, staging, prod, etc.)
    pub name: String,
    /// Environment configuration
    pub config: HashMap<String, String>,
    /// Environment variables
    pub variables: HashMap<String, String>,
    /// Feature flags
    pub features: HashMap<String, bool>,
    /// Environment tags
    pub tags: Vec<String>,
}

impl Default for EnvironmentContext {
    fn default() -> Self {
        Self {
            name: "development".to_string(),
            config: HashMap::new(),
            variables: HashMap::new(),
            features: HashMap::new(),
            tags: Vec::new(),
        }
    }
}

impl EnvironmentContext {
    /// Create a new environment context
    pub fn new(name: String) -> Self {
        Self {
            name,
            config: HashMap::new(),
            variables: HashMap::new(),
            features: HashMap::new(),
            tags: Vec::new(),
        }
    }

    /// Check if a feature is enabled
    pub fn is_feature_enabled(&self, feature: &str) -> bool {
        self.features.get(feature).copied().unwrap_or(false)
    }

    /// Get environment variable
    pub fn get_variable(&self, key: &str) -> Option<&String> {
        self.variables.get(key)
    }

    /// Check if environment has a specific tag
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains(&tag.to_string())
    }
}

/// Tracing context for distributed tracing
#[derive(Debug, Clone)]
pub struct TracingContext {
    /// Trace identifier
    pub trace_id: String,
    /// Span identifier
    pub span_id: String,
    /// Parent span identifier
    pub parent_span_id: Option<String>,
    /// Trace flags
    pub flags: u8,
    /// Baggage data
    pub baggage: HashMap<String, String>,
    /// Sampling decision
    pub sampled: bool,
}

impl TracingContext {
    /// Create a new tracing context
    pub fn new() -> Self {
        Self {
            trace_id: format!("{:032x}", rand::random::<u128>()),
            span_id: format!("{:016x}", rand::random::<u64>()),
            parent_span_id: None,
            flags: 0,
            baggage: HashMap::new(),
            sampled: true,
        }
    }

    /// Create a child tracing context
    pub fn create_child(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            span_id: format!("{:016x}", rand::random::<u64>()),
            parent_span_id: Some(self.span_id.clone()),
            flags: self.flags,
            baggage: self.baggage.clone(),
            sampled: self.sampled,
        }
    }

    /// Add baggage item
    pub fn add_baggage(&mut self, key: String, value: String) {
        self.baggage.insert(key, value);
    }

    /// Get baggage item
    pub fn get_baggage(&self, key: &str) -> Option<&String> {
        self.baggage.get(key)
    }

    /// Generate trace context header value
    pub fn to_header_value(&self) -> String {
        format!("00-{}-{}-{:02x}", self.trace_id, self.span_id, self.flags)
    }
}

/// Context propagation trait for carrying context across boundaries
#[async_trait]
pub trait ContextPropagation {
    /// Extract context from headers or environment
    async fn extract_context(&self, data: &HashMap<String, String>) -> crate::core::error::ResourceResult<ExecutionContext>;

    /// Inject context into headers or environment
    async fn inject_context(&self, context: &ExecutionContext) -> crate::core::error::ResourceResult<HashMap<String, String>>;
}

/// Default context propagator
pub struct DefaultContextPropagator;

#[async_trait]
impl ContextPropagation for DefaultContextPropagator {
    async fn extract_context(&self, data: &HashMap<String, String>) -> crate::core::error::ResourceResult<ExecutionContext> {
        let workflow_id = data.get("workflow-id")
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing workflow-id in context"))?
            .clone();

        let execution_id = data.get("execution-id")
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing execution-id in context"))?
            .clone();

        let action_id = data.get("action-id")
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing action-id in context"))?
            .clone();

        let user_id = data.get("user-id").cloned().unwrap_or_else(|| "anonymous".to_string());
        let tenant_id = data.get("tenant-id").cloned().unwrap_or_else(|| "default".to_string());

        let identity = IdentityContext::new(user_id, vec![], vec![]);
        let tenant = TenantContext::new(tenant_id, "Default".to_string());

        let mut context = ExecutionContext::new(workflow_id, execution_id, action_id, identity, tenant);

        // Extract tracing context if available
        if let Some(trace_header) = data.get("traceparent") {
            if let Ok(tracing) = parse_trace_header(trace_header) {
                context.tracing = tracing;
            }
        }

        Ok(context)
    }

    async fn inject_context(&self, context: &ExecutionContext) -> crate::core::error::ResourceResult<HashMap<String, String>> {
        let mut data = HashMap::new();

        data.insert("workflow-id".to_string(), context.workflow_id.clone());
        data.insert("execution-id".to_string(), context.execution_id.clone());
        data.insert("action-id".to_string(), context.action_id.clone());
        data.insert("user-id".to_string(), context.identity.user_id.clone());
        data.insert("tenant-id".to_string(), context.tenant.id.clone());
        data.insert("traceparent".to_string(), context.tracing.to_header_value());

        Ok(data)
    }
}

/// Parse W3C trace context header
fn parse_trace_header(header: &str) -> Result<TracingContext, String> {
    let parts: Vec<&str> = header.split('-').collect();
    if parts.len() != 4 {
        return Err("Invalid trace header format".to_string());
    }

    let trace_id = parts[1].to_string();
    let span_id = parts[2].to_string();
    let flags = u8::from_str_radix(parts[3], 16).map_err(|_| "Invalid flags")?;

    Ok(TracingContext {
        trace_id,
        span_id,
        parent_span_id: None,
        flags,
        baggage: HashMap::new(),
        sampled: flags & 1 == 1,
    })
}

/// Context middleware for automatic context injection
pub struct ContextMiddleware {
    propagator: Arc<dyn ContextPropagation + Send + Sync>,
}

impl ContextMiddleware {
    /// Create new context middleware
    pub fn new(propagator: Arc<dyn ContextPropagation + Send + Sync>) -> Self {
        Self { propagator }
    }

    /// Process incoming context
    pub async fn process_incoming(&self, headers: HashMap<String, String>) -> crate::core::error::ResourceResult<ExecutionContext> {
        self.propagator.extract_context(&headers).await
    }

    /// Process outgoing context
    pub async fn process_outgoing(&self, context: &ExecutionContext) -> crate::core::error::ResourceResult<HashMap<String, String>> {
        self.propagator.inject_context(context).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_context_creation() {
        let identity = IdentityContext::new(
            "user123".to_string(),
            vec!["admin".to_string()],
            vec!["read".to_string(), "write".to_string()],
        );
        let tenant = TenantContext::new("tenant1".to_string(), "Test Tenant".to_string());

        let context = ExecutionContext::new(
            "workflow1".to_string(),
            "exec1".to_string(),
            "action1".to_string(),
            identity,
            tenant,
        );

        assert_eq!(context.workflow_id, "workflow1");
        assert_eq!(context.execution_id, "exec1");
        assert_eq!(context.action_id, "action1");
        assert_eq!(context.identity.user_id, "user123");
        assert_eq!(context.tenant.id, "tenant1");
    }

    #[test]
    fn test_child_context() {
        let identity = IdentityContext::system();
        let tenant = TenantContext::default_tenant();

        let parent = ExecutionContext::new(
            "workflow1".to_string(),
            "exec1".to_string(),
            "action1".to_string(),
            identity,
            tenant,
        );

        let child = parent.create_child("action2".to_string());

        assert_eq!(child.workflow_id, parent.workflow_id);
        assert_eq!(child.execution_id, parent.execution_id);
        assert_eq!(child.action_id, "action2");
        assert_ne!(child.id, parent.id);
        assert_eq!(child.tracing.trace_id, parent.tracing.trace_id);
        assert_ne!(child.tracing.span_id, parent.tracing.span_id);
    }

    #[test]
    fn test_resource_quotas() {
        let mut quotas = ResourceQuotas::default();
        quotas.max_resources.insert("database".to_string(), 5);

        assert!(quotas.can_allocate("database", 3));
        assert!(quotas.allocate("database", 3).is_ok());
        assert!(!quotas.can_allocate("database", 3));
        assert!(quotas.can_allocate("database", 2));

        quotas.deallocate("database", 1);
        assert!(quotas.can_allocate("database", 3));
    }

    #[tokio::test]
    async fn test_context_propagation() {
        let propagator = DefaultContextPropagator;
        let mut headers = HashMap::new();
        headers.insert("workflow-id".to_string(), "wf1".to_string());
        headers.insert("execution-id".to_string(), "exec1".to_string());
        headers.insert("action-id".to_string(), "action1".to_string());
        headers.insert("user-id".to_string(), "user1".to_string());
        headers.insert("tenant-id".to_string(), "tenant1".to_string());

        let context = propagator.extract_context(&headers).await.unwrap();
        assert_eq!(context.workflow_id, "wf1");
        assert_eq!(context.identity.user_id, "user1");
        assert_eq!(context.tenant.id, "tenant1");

        let injected = propagator.inject_context(&context).await.unwrap();
        assert_eq!(injected.get("workflow-id").unwrap(), "wf1");
        assert_eq!(injected.get("user-id").unwrap(), "user1");
    }
}