//! Base traits for Nebula entities
//!
//! These traits provide common functionality that can be implemented
//! by various types throughout the system.

use super::id::{ExecutionId, NodeId, TenantId, UserId, WorkflowId};
use super::scope::ScopeLevel;

/// Trait for entities that have a scope
pub trait Scoped {
    /// Get the scope level for this entity
    fn scope(&self) -> &ScopeLevel;

    /// Check if this entity is in the given scope
    fn is_in_scope(&self, scope: &ScopeLevel) -> bool {
        self.scope().is_contained_in(scope)
    }

    /// Check if this entity is global
    fn is_global(&self) -> bool {
        self.scope().is_global()
    }

    /// Check if this entity is workflow-scoped
    fn is_workflow(&self) -> bool {
        self.scope().is_workflow()
    }

    /// Check if this entity is execution-scoped
    fn is_execution(&self) -> bool {
        self.scope().is_execution()
    }

    /// Check if this entity is action-scoped
    fn is_action(&self) -> bool {
        self.scope().is_action()
    }
}

/// Trait for entities that have execution context
pub trait HasContext {
    /// Get the execution ID if available
    fn execution_id(&self) -> Option<&ExecutionId>;

    /// Get the workflow ID if available
    fn workflow_id(&self) -> Option<&WorkflowId>;

    /// Get the node ID if available
    fn node_id(&self) -> Option<&NodeId>;

    /// Get the user ID if available
    fn user_id(&self) -> Option<&UserId>;

    /// Get the tenant ID if available
    fn tenant_id(&self) -> Option<&TenantId>;

    /// Check if this entity has execution context
    fn has_execution_context(&self) -> bool {
        self.execution_id().is_some()
    }

    /// Check if this entity has workflow context
    fn has_workflow_context(&self) -> bool {
        self.workflow_id().is_some()
    }

    /// Check if this entity has user context
    fn has_user_context(&self) -> bool {
        self.user_id().is_some()
    }

    /// Check if this entity has tenant context
    fn has_tenant_context(&self) -> bool {
        self.tenant_id().is_some()
    }
}

// NOTE: Additional generic utility traits (Identifiable, Validatable, Serializable,
// Cloneable, Comparable, Hashable, Displayable, Debuggable, StringConvertible,
// HasMetadata/EntityMetadata) were removed to keep core focused on scope and
// execution context. Crates should define more specific traits locally if needed.

#[cfg(test)]
mod tests {
    use super::super::id::{ExecutionId, NodeId, WorkflowId};
    use super::*;

    // Test implementation of Scoped
    #[derive(Debug)]
    struct TestScopedEntity {
        scope: ScopeLevel,
    }

    impl Scoped for TestScopedEntity {
        fn scope(&self) -> &ScopeLevel {
            &self.scope
        }
    }

    // Test implementation of HasContext
    #[derive(Debug)]
    struct TestContextEntity {
        execution_id: Option<ExecutionId>,
        workflow_id: Option<WorkflowId>,
        node_id: Option<NodeId>,
    }

    impl HasContext for TestContextEntity {
        fn execution_id(&self) -> Option<&ExecutionId> {
            self.execution_id.as_ref()
        }

        fn workflow_id(&self) -> Option<&WorkflowId> {
            self.workflow_id.as_ref()
        }

        fn node_id(&self) -> Option<&NodeId> {
            self.node_id.as_ref()
        }

        fn user_id(&self) -> Option<&UserId> {
            None
        }

        fn tenant_id(&self) -> Option<&TenantId> {
            None
        }
    }

    #[test]
    fn test_scoped_trait() {
        let execution_id = ExecutionId::new();
        let entity = TestScopedEntity {
            scope: ScopeLevel::Execution(execution_id),
        };

        assert!(entity.is_execution());
        assert!(!entity.is_global());
        assert!(!entity.is_workflow());
        assert!(!entity.is_action());
    }

    #[test]
    fn test_has_context_trait() {
        let execution_id = ExecutionId::new();
        let workflow_id = WorkflowId::new();
        let node_id = NodeId::new();

        let entity = TestContextEntity {
            execution_id: Some(execution_id),
            workflow_id: Some(workflow_id),
            node_id: Some(node_id),
        };

        assert!(entity.has_execution_context());
        assert!(entity.has_workflow_context());
        assert_eq!(entity.execution_id(), Some(&execution_id));
        assert_eq!(entity.workflow_id(), Some(&workflow_id));
        assert_eq!(entity.node_id(), Some(&node_id));
    }
}
