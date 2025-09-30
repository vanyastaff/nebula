//! Context propagation utilities and implementations

use std::collections::HashMap;
use async_trait::async_trait;
use super::{ExecutionContext, ContextPropagation, TracingContext, IdentityContext, TenantContext};

/// HTTP header-based context propagation
pub struct HttpContextPropagator {
    /// Custom header mappings
    header_mappings: HashMap<String, String>,
}

impl HttpContextPropagator {
    /// Create a new HTTP context propagator
    pub fn new() -> Self {
        let mut header_mappings = HashMap::new();
        header_mappings.insert("workflow-id".to_string(), "X-Nebula-Workflow-Id".to_string());
        header_mappings.insert("execution-id".to_string(), "X-Nebula-Execution-Id".to_string());
        header_mappings.insert("action-id".to_string(), "X-Nebula-Action-Id".to_string());
        header_mappings.insert("user-id".to_string(), "X-Nebula-User-Id".to_string());
        header_mappings.insert("tenant-id".to_string(), "X-Nebula-Tenant-Id".to_string());

        Self { header_mappings }
    }

    /// Create with custom header mappings
    pub fn with_mappings(header_mappings: HashMap<String, String>) -> Self {
        Self { header_mappings }
    }

    /// Get header name for context key
    fn get_header_name(&self, key: &str) -> String {
        self.header_mappings.get(key).cloned().unwrap_or_else(|| key.to_string())
    }
}

#[async_trait]
impl ContextPropagation for HttpContextPropagator {
    async fn extract_context(&self, headers: &HashMap<String, String>) -> crate::core::error::ResourceResult<ExecutionContext> {
        // Extract required fields
        let workflow_id = headers.get(&self.get_header_name("workflow-id"))
            .or_else(|| headers.get("workflow-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing workflow-id in headers"))?
            .clone();

        let execution_id = headers.get(&self.get_header_name("execution-id"))
            .or_else(|| headers.get("execution-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing execution-id in headers"))?
            .clone();

        let action_id = headers.get(&self.get_header_name("action-id"))
            .or_else(|| headers.get("action-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing action-id in headers"))?
            .clone();

        // Extract optional fields
        let user_id = headers.get(&self.get_header_name("user-id"))
            .or_else(|| headers.get("user-id"))
            .cloned()
            .unwrap_or_else(|| "anonymous".to_string());

        let tenant_id = headers.get(&self.get_header_name("tenant-id"))
            .or_else(|| headers.get("tenant-id"))
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        // Create identity and tenant contexts
        let identity = IdentityContext::new(user_id, vec![], vec![]);
        let tenant = TenantContext::new(tenant_id, "Default".to_string());

        let mut context = ExecutionContext::new(workflow_id, execution_id, action_id, identity, tenant);

        // Extract tracing context
        if let Some(trace_header) = headers.get("traceparent") {
            if let Ok(tracing) = super::parse_trace_header(trace_header) {
                context.tracing = tracing;
            }
        }

        // Extract baggage if present
        if let Some(baggage_header) = headers.get("baggage") {
            let baggage = parse_baggage_header(baggage_header);
            for (key, value) in baggage {
                context.tracing.add_baggage(key, value);
            }
        }

        Ok(context)
    }

    async fn inject_context(&self, context: &ExecutionContext) -> crate::core::error::ResourceResult<HashMap<String, String>> {
        let mut headers = HashMap::new();

        // Inject required fields
        headers.insert(self.get_header_name("workflow-id"), context.workflow_id.clone());
        headers.insert(self.get_header_name("execution-id"), context.execution_id.clone());
        headers.insert(self.get_header_name("action-id"), context.action_id.clone());
        headers.insert(self.get_header_name("user-id"), context.identity.user_id.clone());
        headers.insert(self.get_header_name("tenant-id"), context.tenant.id.clone());

        // Inject tracing context
        headers.insert("traceparent".to_string(), context.tracing.to_header_value());

        // Inject baggage if present
        if !context.tracing.baggage.is_empty() {
            let baggage_header = create_baggage_header(&context.tracing.baggage);
            headers.insert("baggage".to_string(), baggage_header);
        }

        Ok(headers)
    }
}

/// Message queue context propagation
pub struct MessageQueueContextPropagator {
    /// Message property prefix
    property_prefix: String,
}

impl MessageQueueContextPropagator {
    /// Create a new message queue context propagator
    pub fn new() -> Self {
        Self {
            property_prefix: "nebula_".to_string(),
        }
    }

    /// Create with custom property prefix
    pub fn with_prefix(prefix: String) -> Self {
        Self {
            property_prefix: prefix,
        }
    }

    /// Get property name for context key
    fn get_property_name(&self, key: &str) -> String {
        format!("{}{}", self.property_prefix, key.replace("-", "_"))
    }
}

#[async_trait]
impl ContextPropagation for MessageQueueContextPropagator {
    async fn extract_context(&self, properties: &HashMap<String, String>) -> crate::core::error::ResourceResult<ExecutionContext> {
        // Extract required fields with prefix
        let workflow_id = properties.get(&self.get_property_name("workflow-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing workflow-id in message properties"))?
            .clone();

        let execution_id = properties.get(&self.get_property_name("execution-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing execution-id in message properties"))?
            .clone();

        let action_id = properties.get(&self.get_property_name("action-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing action-id in message properties"))?
            .clone();

        // Extract optional fields
        let user_id = properties.get(&self.get_property_name("user-id"))
            .cloned()
            .unwrap_or_else(|| "system".to_string());

        let tenant_id = properties.get(&self.get_property_name("tenant-id"))
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        // Create contexts
        let identity = IdentityContext::new(user_id, vec![], vec![]);
        let tenant = TenantContext::new(tenant_id, "Default".to_string());

        let mut context = ExecutionContext::new(workflow_id, execution_id, action_id, identity, tenant);

        // Extract tracing context
        if let Some(trace_id) = properties.get(&self.get_property_name("trace-id")) {
            if let Some(span_id) = properties.get(&self.get_property_name("span-id")) {
                context.tracing.trace_id = trace_id.clone();
                context.tracing.span_id = span_id.clone();

                if let Some(parent_span) = properties.get(&self.get_property_name("parent-span-id")) {
                    context.tracing.parent_span_id = Some(parent_span.clone());
                }
            }
        }

        Ok(context)
    }

    async fn inject_context(&self, context: &ExecutionContext) -> crate::core::error::ResourceResult<HashMap<String, String>> {
        let mut properties = HashMap::new();

        // Inject required fields
        properties.insert(self.get_property_name("workflow-id"), context.workflow_id.clone());
        properties.insert(self.get_property_name("execution-id"), context.execution_id.clone());
        properties.insert(self.get_property_name("action-id"), context.action_id.clone());
        properties.insert(self.get_property_name("user-id"), context.identity.user_id.clone());
        properties.insert(self.get_property_name("tenant-id"), context.tenant.id.clone());

        // Inject tracing context
        properties.insert(self.get_property_name("trace-id"), context.tracing.trace_id.clone());
        properties.insert(self.get_property_name("span-id"), context.tracing.span_id.clone());

        if let Some(parent_span) = &context.tracing.parent_span_id {
            properties.insert(self.get_property_name("parent-span-id"), parent_span.clone());
        }

        Ok(properties)
    }
}

/// Environment variable context propagation
pub struct EnvironmentContextPropagator {
    /// Environment variable prefix
    var_prefix: String,
}

impl EnvironmentContextPropagator {
    /// Create a new environment context propagator
    pub fn new() -> Self {
        Self {
            var_prefix: "NEBULA_".to_string(),
        }
    }

    /// Create with custom variable prefix
    pub fn with_prefix(prefix: String) -> Self {
        Self {
            var_prefix: prefix,
        }
    }

    /// Get environment variable name for context key
    fn get_var_name(&self, key: &str) -> String {
        format!("{}{}", self.var_prefix, key.replace("-", "_").to_uppercase())
    }
}

#[async_trait]
impl ContextPropagation for EnvironmentContextPropagator {
    async fn extract_context(&self, env_vars: &HashMap<String, String>) -> crate::core::error::ResourceResult<ExecutionContext> {
        // Extract required fields from environment variables
        let workflow_id = env_vars.get(&self.get_var_name("workflow-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing NEBULA_WORKFLOW_ID environment variable"))?
            .clone();

        let execution_id = env_vars.get(&self.get_var_name("execution-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing NEBULA_EXECUTION_ID environment variable"))?
            .clone();

        let action_id = env_vars.get(&self.get_var_name("action-id"))
            .ok_or_else(|| crate::core::error::ResourceError::configuration("Missing NEBULA_ACTION_ID environment variable"))?
            .clone();

        // Extract optional fields
        let user_id = env_vars.get(&self.get_var_name("user-id"))
            .cloned()
            .unwrap_or_else(|| "system".to_string());

        let tenant_id = env_vars.get(&self.get_var_name("tenant-id"))
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        // Create contexts
        let identity = IdentityContext::new(user_id, vec![], vec![]);
        let tenant = TenantContext::new(tenant_id, "Default".to_string());

        let context = ExecutionContext::new(workflow_id, execution_id, action_id, identity, tenant);

        Ok(context)
    }

    async fn inject_context(&self, context: &ExecutionContext) -> crate::core::error::ResourceResult<HashMap<String, String>> {
        let mut env_vars = HashMap::new();

        env_vars.insert(self.get_var_name("workflow-id"), context.workflow_id.clone());
        env_vars.insert(self.get_var_name("execution-id"), context.execution_id.clone());
        env_vars.insert(self.get_var_name("action-id"), context.action_id.clone());
        env_vars.insert(self.get_var_name("user-id"), context.identity.user_id.clone());
        env_vars.insert(self.get_var_name("tenant-id"), context.tenant.id.clone());
        env_vars.insert(self.get_var_name("trace-id"), context.tracing.trace_id.clone());
        env_vars.insert(self.get_var_name("span-id"), context.tracing.span_id.clone());

        Ok(env_vars)
    }
}

/// Composite context propagator that tries multiple propagators
pub struct CompositeContextPropagator {
    propagators: Vec<Box<dyn ContextPropagation + Send + Sync>>,
}

impl CompositeContextPropagator {
    /// Create a new composite propagator
    pub fn new() -> Self {
        Self {
            propagators: Vec::new(),
        }
    }

    /// Add a propagator to the composite
    pub fn add_propagator(mut self, propagator: Box<dyn ContextPropagation + Send + Sync>) -> Self {
        self.propagators.push(propagator);
        self
    }
}

#[async_trait]
impl ContextPropagation for CompositeContextPropagator {
    async fn extract_context(&self, data: &HashMap<String, String>) -> crate::core::error::ResourceResult<ExecutionContext> {
        let mut last_error = None;

        for propagator in &self.propagators {
            match propagator.extract_context(data).await {
                Ok(context) => return Ok(context),
                Err(e) => last_error = Some(e),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            crate::core::error::ResourceError::configuration("No propagators available")
        }))
    }

    async fn inject_context(&self, context: &ExecutionContext) -> crate::core::error::ResourceResult<HashMap<String, String>> {
        let mut combined_data = HashMap::new();

        for propagator in &self.propagators {
            if let Ok(data) = propagator.inject_context(context).await {
                combined_data.extend(data);
            }
        }

        Ok(combined_data)
    }
}

/// Parse W3C baggage header
fn parse_baggage_header(header: &str) -> HashMap<String, String> {
    let mut baggage = HashMap::new();

    for item in header.split(',') {
        let item = item.trim();
        if let Some(eq_pos) = item.find('=') {
            let key = item[..eq_pos].trim().to_string();
            let value = item[eq_pos + 1..].trim().to_string();
            baggage.insert(key, value);
        }
    }

    baggage
}

/// Create W3C baggage header
fn create_baggage_header(baggage: &HashMap<String, String>) -> String {
    baggage.iter()
        .map(|(key, value)| format!("{}={}", key, value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_http_context_propagator() {
        let propagator = HttpContextPropagator::new();
        let mut headers = HashMap::new();
        headers.insert("X-Nebula-Workflow-Id".to_string(), "wf1".to_string());
        headers.insert("X-Nebula-Execution-Id".to_string(), "exec1".to_string());
        headers.insert("X-Nebula-Action-Id".to_string(), "action1".to_string());
        headers.insert("X-Nebula-User-Id".to_string(), "user1".to_string());
        headers.insert("X-Nebula-Tenant-Id".to_string(), "tenant1".to_string());

        let context = propagator.extract_context(&headers).await.unwrap();
        assert_eq!(context.workflow_id, "wf1");
        assert_eq!(context.identity.user_id, "user1");

        let injected = propagator.inject_context(&context).await.unwrap();
        assert!(injected.contains_key("X-Nebula-Workflow-Id"));
        assert_eq!(injected.get("X-Nebula-Workflow-Id").unwrap(), "wf1");
    }

    #[tokio::test]
    async fn test_message_queue_propagator() {
        let propagator = MessageQueueContextPropagator::new();
        let mut properties = HashMap::new();
        properties.insert("nebula_workflow_id".to_string(), "wf1".to_string());
        properties.insert("nebula_execution_id".to_string(), "exec1".to_string());
        properties.insert("nebula_action_id".to_string(), "action1".to_string());

        let context = propagator.extract_context(&properties).await.unwrap();
        assert_eq!(context.workflow_id, "wf1");

        let injected = propagator.inject_context(&context).await.unwrap();
        assert!(injected.contains_key("nebula_workflow_id"));
    }

    #[tokio::test]
    async fn test_environment_propagator() {
        let propagator = EnvironmentContextPropagator::new();
        let mut env_vars = HashMap::new();
        env_vars.insert("NEBULA_WORKFLOW_ID".to_string(), "wf1".to_string());
        env_vars.insert("NEBULA_EXECUTION_ID".to_string(), "exec1".to_string());
        env_vars.insert("NEBULA_ACTION_ID".to_string(), "action1".to_string());

        let context = propagator.extract_context(&env_vars).await.unwrap();
        assert_eq!(context.workflow_id, "wf1");

        let injected = propagator.inject_context(&context).await.unwrap();
        assert!(injected.contains_key("NEBULA_WORKFLOW_ID"));
    }

    #[test]
    fn test_baggage_parsing() {
        let header = "key1=value1, key2=value2, key3=value3";
        let baggage = parse_baggage_header(header);

        assert_eq!(baggage.len(), 3);
        assert_eq!(baggage.get("key1"), Some(&"value1".to_string()));
        assert_eq!(baggage.get("key2"), Some(&"value2".to_string()));
        assert_eq!(baggage.get("key3"), Some(&"value3".to_string()));

        let recreated = create_baggage_header(&baggage);
        assert!(recreated.contains("key1=value1"));
        assert!(recreated.contains("key2=value2"));
        assert!(recreated.contains("key3=value3"));
    }
}