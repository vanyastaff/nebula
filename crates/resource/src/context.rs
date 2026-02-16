//! Flat resource context with cancellation support

use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

use crate::scope::Scope;

/// Context for resource operations.
///
/// Carries scope, identifiers, cancellation, and arbitrary metadata.
#[derive(Debug, Clone)]
pub struct Context {
    pub scope: Scope,
    pub execution_id: String,
    pub workflow_id: String,
    pub tenant_id: Option<String>,
    pub cancellation: CancellationToken,
    pub metadata: HashMap<String, String>,
}

impl Context {
    pub fn new(
        scope: Scope,
        workflow_id: impl Into<String>,
        execution_id: impl Into<String>,
    ) -> Self {
        Self {
            scope,
            execution_id: execution_id.into(),
            workflow_id: workflow_id.into(),
            tenant_id: None,
            cancellation: CancellationToken::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_tenant(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1");
        assert_eq!(ctx.workflow_id, "wf-1");
        assert_eq!(ctx.execution_id, "ex-1");
        assert!(ctx.tenant_id.is_none());
        assert!(ctx.metadata.is_empty());
    }

    #[test]
    fn test_context_with_tenant() {
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1").with_tenant("tenant-a");
        assert_eq!(ctx.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn test_context_with_metadata() {
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1")
            .with_metadata("env", "prod")
            .with_metadata("region", "us-east-1");
        assert_eq!(ctx.metadata.get("env").unwrap(), "prod");
        assert_eq!(ctx.metadata.get("region").unwrap(), "us-east-1");
    }

    #[test]
    fn test_context_with_cancellation() {
        let token = CancellationToken::new();
        let child = token.child_token();
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1").with_cancellation(child);
        assert!(!ctx.cancellation.is_cancelled());
        token.cancel();
        assert!(ctx.cancellation.is_cancelled());
    }
}
