//! Resource context and execution environment

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::scoping::ResourceScope;

/// Comprehensive context information for resource operations
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResourceContext {
    /// Unique identifier for this context
    pub context_id: Uuid,

    /// Timestamp when the context was created
    pub created_at: DateTime<Utc>,

    /// Workflow information
    pub workflow: WorkflowContext,

    /// Execution information
    pub execution: ExecutionContext,

    /// Action information (if applicable)
    pub action: Option<ActionContext>,

    /// Tracing information
    pub tracing: TracingContext,

    /// User and tenant information
    pub identity: IdentityContext,

    /// Environment and deployment information
    pub environment: EnvironmentContext,

    /// Resource scope for this context
    pub scope: ResourceScope,

    /// Custom metadata
    pub metadata: HashMap<String, serde_json::Value>,

    /// Tags for resource categorization
    pub tags: HashMap<String, String>,
}

/// Workflow-specific context information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct WorkflowContext {
    /// Workflow identifier
    pub workflow_id: String,
    /// Workflow name
    pub workflow_name: String,
    /// Workflow version
    pub workflow_version: String,
    /// Workflow definition hash
    pub definition_hash: Option<String>,
}

/// Execution-specific context information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ExecutionContext {
    /// Execution identifier
    pub execution_id: String,
    /// Parent execution (for sub-workflows)
    pub parent_execution_id: Option<String>,
    /// Execution attempt number
    pub attempt_number: u32,
    /// Execution start time
    pub started_at: DateTime<Utc>,
    /// Execution timeout (if any)
    pub timeout_at: Option<DateTime<Utc>>,
}

/// Action-specific context information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ActionContext {
    /// Action identifier
    pub action_id: String,
    /// Action name
    pub action_name: String,
    /// Action path within the workflow
    pub action_path: Vec<String>,
    /// Action attempt number
    pub attempt_number: u32,
    /// Action start time
    pub started_at: DateTime<Utc>,
}

/// Distributed tracing context information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TracingContext {
    /// Trace identifier
    pub trace_id: String,
    /// Current span identifier
    pub span_id: String,
    /// Parent span identifier
    pub parent_span_id: Option<String>,
    /// Trace flags
    pub trace_flags: u8,
    /// Baggage items
    pub baggage: HashMap<String, String>,
}

/// Identity and authorization context
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct IdentityContext {
    /// User identifier (if known)
    pub user_id: Option<String>,
    /// Tenant identifier (for multi-tenant environments)
    pub tenant_id: Option<String>,
    /// Account identifier
    pub account_id: Option<String>,
    /// User roles and permissions
    pub roles: Vec<String>,
    /// Additional claims
    pub claims: HashMap<String, serde_json::Value>,
}

/// Environment and deployment context
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EnvironmentContext {
    /// Environment name (e.g., "development", "staging", "production")
    pub environment: String,
    /// Geographic region
    pub region: String,
    /// Availability zone
    pub availability_zone: Option<String>,
    /// Deployment tier (e.g., "free", "pro", "enterprise")
    pub deployment_tier: String,
    /// Service version
    pub service_version: String,
    /// Additional environment variables
    pub variables: HashMap<String, String>,
}

impl ResourceContext {
    /// Create a new resource context with minimal information
    pub fn new(
        workflow_id: String,
        workflow_name: String,
        execution_id: String,
        environment: String,
    ) -> Self {
        let now = Utc::now();

        Self {
            context_id: Uuid::new_v4(),
            created_at: now,
            workflow: WorkflowContext {
                workflow_id: workflow_id.clone(),
                workflow_name,
                workflow_version: "1.0.0".to_string(),
                definition_hash: None,
            },
            execution: ExecutionContext {
                execution_id: execution_id.clone(),
                parent_execution_id: None,
                attempt_number: 1,
                started_at: now,
                timeout_at: None,
            },
            action: None,
            tracing: TracingContext {
                trace_id: Uuid::new_v4().to_string(),
                span_id: Uuid::new_v4().to_string(),
                parent_span_id: None,
                trace_flags: 0,
                baggage: HashMap::new(),
            },
            identity: IdentityContext {
                user_id: None,
                tenant_id: None,
                account_id: None,
                roles: Vec::new(),
                claims: HashMap::new(),
            },
            environment: EnvironmentContext {
                environment,
                region: "us-east-1".to_string(),
                availability_zone: None,
                deployment_tier: "standard".to_string(),
                service_version: env!("CARGO_PKG_VERSION").to_string(),
                variables: HashMap::new(),
            },
            scope: ResourceScope::execution(execution_id),
            metadata: HashMap::new(),
            tags: HashMap::new(),
        }
    }

    /// Create a builder for more complex context construction
    pub fn builder() -> ResourceContextBuilder {
        ResourceContextBuilder::new()
    }

    /// Add or update metadata
    pub fn with_metadata<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<serde_json::Value>,
    {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Add or update a tag
    pub fn with_tag<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Set the resource scope
    pub fn with_scope(mut self, scope: ResourceScope) -> Self {
        self.scope = scope;
        self
    }

    /// Set action context
    pub fn with_action(mut self, action: ActionContext) -> Self {
        self.action = Some(action);
        self
    }

    /// Set user identity
    pub fn with_user<S: Into<String>>(mut self, user_id: S) -> Self {
        self.identity.user_id = Some(user_id.into());
        self
    }

    /// Set tenant identity
    pub fn with_tenant<S: Into<String>>(mut self, tenant_id: S) -> Self {
        self.identity.tenant_id = Some(tenant_id.into());
        self
    }

    /// Create a derived context for a child action
    pub fn derive_for_action(&self, action_id: String, action_name: String) -> Self {
        let mut derived = self.clone();
        derived.context_id = Uuid::new_v4();
        derived.action = Some(ActionContext {
            action_id: action_id.clone(),
            action_name,
            action_path: vec![action_id.clone()],
            attempt_number: 1,
            started_at: Utc::now(),
        });
        derived.scope = ResourceScope::action(action_id);
        derived
    }

    /// Get all context fields as a flat map for structured logging
    pub fn to_log_fields(&self) -> HashMap<String, serde_json::Value> {
        let mut fields = HashMap::new();

        // Basic context
        fields.insert("context_id".to_string(), self.context_id.to_string().into());
        fields.insert(
            "created_at".to_string(),
            self.created_at.to_rfc3339().into(),
        );

        // Workflow context
        fields.insert(
            "workflow_id".to_string(),
            self.workflow.workflow_id.clone().into(),
        );
        fields.insert(
            "workflow_name".to_string(),
            self.workflow.workflow_name.clone().into(),
        );
        fields.insert(
            "workflow_version".to_string(),
            self.workflow.workflow_version.clone().into(),
        );

        // Execution context
        fields.insert(
            "execution_id".to_string(),
            self.execution.execution_id.clone().into(),
        );
        fields.insert(
            "attempt_number".to_string(),
            self.execution.attempt_number.into(),
        );

        // Action context (if present)
        if let Some(ref action) = self.action {
            fields.insert("action_id".to_string(), action.action_id.clone().into());
            fields.insert("action_name".to_string(), action.action_name.clone().into());
        }

        // Tracing context
        fields.insert("trace_id".to_string(), self.tracing.trace_id.clone().into());
        fields.insert("span_id".to_string(), self.tracing.span_id.clone().into());

        // Identity context
        if let Some(ref user_id) = self.identity.user_id {
            fields.insert("user_id".to_string(), user_id.clone().into());
        }
        if let Some(ref tenant_id) = self.identity.tenant_id {
            fields.insert("tenant_id".to_string(), tenant_id.clone().into());
        }

        // Environment context
        fields.insert(
            "environment".to_string(),
            self.environment.environment.clone().into(),
        );
        fields.insert("region".to_string(), self.environment.region.clone().into());

        // Scope
        fields.insert("resource_scope".to_string(), self.scope.scope_key().into());

        // Include custom metadata
        for (key, value) in &self.metadata {
            fields.insert(format!("meta_{}", key), value.clone());
        }

        // Include tags
        for (key, value) in &self.tags {
            fields.insert(format!("tag_{}", key), value.clone().into());
        }

        fields
    }
}

/// Builder for constructing ResourceContext instances
pub struct ResourceContextBuilder {
    context: ResourceContext,
}

impl ResourceContextBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            context: ResourceContext::new(
                "default".to_string(),
                "Default Workflow".to_string(),
                "default-execution".to_string(),
                "development".to_string(),
            ),
        }
    }

    /// Set workflow information
    pub fn workflow(mut self, id: String, name: String, version: String) -> Self {
        self.context.workflow = WorkflowContext {
            workflow_id: id,
            workflow_name: name,
            workflow_version: version,
            definition_hash: None,
        };
        self
    }

    /// Set execution information
    pub fn execution(mut self, id: String, attempt: u32) -> Self {
        self.context.execution.execution_id = id;
        self.context.execution.attempt_number = attempt;
        self
    }

    /// Set environment information
    pub fn environment(mut self, env: String, region: String, tier: String) -> Self {
        self.context.environment.environment = env;
        self.context.environment.region = region;
        self.context.environment.deployment_tier = tier;
        self
    }

    /// Set tracing information
    pub fn tracing(mut self, trace_id: String, span_id: String) -> Self {
        self.context.tracing.trace_id = trace_id;
        self.context.tracing.span_id = span_id;
        self
    }

    /// Set identity information
    pub fn identity(mut self, user_id: Option<String>, tenant_id: Option<String>) -> Self {
        self.context.identity.user_id = user_id;
        self.context.identity.tenant_id = tenant_id;
        self
    }

    /// Set resource scope
    pub fn scope(mut self, scope: ResourceScope) -> Self {
        self.context.scope = scope;
        self
    }

    /// Add metadata
    pub fn metadata<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<serde_json::Value>,
    {
        self.context.metadata.insert(key.into(), value.into());
        self
    }

    /// Add tag
    pub fn tag<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.context.tags.insert(key.into(), value.into());
        self
    }

    /// Build the final context
    pub fn build(self) -> ResourceContext {
        self.context
    }
}

impl Default for ResourceContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let context = ResourceContext::new(
            "wf-123".to_string(),
            "Test Workflow".to_string(),
            "ex-456".to_string(),
            "test".to_string(),
        );

        assert_eq!(context.workflow.workflow_id, "wf-123");
        assert_eq!(context.workflow.workflow_name, "Test Workflow");
        assert_eq!(context.execution.execution_id, "ex-456");
        assert_eq!(context.environment.environment, "test");
    }

    #[test]
    fn test_context_builder() {
        let context = ResourceContext::builder()
            .workflow("wf-1".to_string(), "Test".to_string(), "2.0".to_string())
            .execution("ex-1".to_string(), 2)
            .environment(
                "prod".to_string(),
                "us-west-2".to_string(),
                "enterprise".to_string(),
            )
            .metadata("test_key", "test_value")
            .tag("env", "production")
            .build();

        assert_eq!(context.workflow.workflow_version, "2.0");
        assert_eq!(context.execution.attempt_number, 2);
        assert_eq!(context.environment.region, "us-west-2");
        assert_eq!(context.metadata.get("test_key").unwrap(), "test_value");
        assert_eq!(context.tags.get("env").unwrap(), "production");
    }

    #[test]
    fn test_derive_for_action() {
        let base_context = ResourceContext::new(
            "wf-123".to_string(),
            "Test Workflow".to_string(),
            "ex-456".to_string(),
            "test".to_string(),
        );

        let action_context =
            base_context.derive_for_action("action-789".to_string(), "Test Action".to_string());

        assert_eq!(action_context.execution.execution_id, "ex-456");
        assert!(action_context.action.is_some());
        assert_eq!(action_context.action.unwrap().action_id, "action-789");
        assert_ne!(action_context.context_id, base_context.context_id);
    }

    #[test]
    fn test_log_fields() {
        let context = ResourceContext::new(
            "wf-123".to_string(),
            "Test Workflow".to_string(),
            "ex-456".to_string(),
            "test".to_string(),
        )
        .with_metadata("custom", "value")
        .with_tag("team", "backend");

        let fields = context.to_log_fields();

        assert!(fields.contains_key("workflow_id"));
        assert!(fields.contains_key("execution_id"));
        assert!(fields.contains_key("trace_id"));
        assert!(fields.contains_key("meta_custom"));
        assert!(fields.contains_key("tag_team"));
    }
}
