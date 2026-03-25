//! Backward-compatibility types for dependents still using v1 API.
//!
//! These types will be removed once all consumers migrate to v2.
//! **Do not use in new code.**

use std::collections::HashMap;

use nebula_core::{ExecutionId, WorkflowId};
use tokio_util::sync::CancellationToken;

/// Legacy scope enum for backward compatibility.
#[deprecated(since = "0.1.0", note = "use nebula_resource::ScopeLevel instead")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Global scope.
    Global,
}

/// Legacy execution context for backward compatibility.
///
/// Migrating consumers should use [`crate::BasicCtx`] instead.
#[deprecated(since = "0.1.0", note = "use nebula_resource::BasicCtx instead")]
#[derive(Clone, Debug)]
pub struct Context {
    /// The scope of this context.
    #[allow(deprecated)]
    pub scope: Scope,
    /// The workflow ID.
    pub workflow_id: WorkflowId,
    /// The execution ID.
    pub execution_id: ExecutionId,
    /// Optional cancellation token.
    pub cancellation: Option<CancellationToken>,
    /// Key-value metadata.
    pub metadata: HashMap<String, String>,
}

#[allow(deprecated)]
impl Context {
    /// Creates a new context.
    pub fn new(scope: Scope, workflow_id: WorkflowId, execution_id: ExecutionId) -> Self {
        Self {
            scope,
            workflow_id,
            execution_id,
            cancellation: Some(CancellationToken::new()),
            metadata: HashMap::new(),
        }
    }

    /// Adds a metadata key-value pair.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Returns the tenant ID from metadata, if present.
    pub fn tenant_id(&self) -> Option<&str> {
        self.metadata.get("tenant_id").map(String::as_str)
    }
}
