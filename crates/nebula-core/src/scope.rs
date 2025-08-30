//! Scope system for resource lifecycle management
//! 
//! Resources in Nebula have different lifecycle scopes:
//! - Global: Application lifetime
//! - Workflow: Per workflow execution
//! - Execution: Per single execution
//! - Action: Per action invocation

use serde::{Deserialize, Serialize};
use std::fmt;

use super::id::{ExecutionId, WorkflowId, NodeId};

/// Defines the scope level for a resource
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScopeLevel {
    /// Resource lives for the entire application lifetime
    Global,
    
    /// Resource lives for the duration of a workflow execution
    Workflow(WorkflowId),
    
    /// Resource lives for the duration of a single execution
    Execution(ExecutionId),
    
    /// Resource lives for the duration of a single action invocation
    Action(ExecutionId, NodeId),
}

impl ScopeLevel {
    /// Check if this scope is global
    pub fn is_global(&self) -> bool {
        matches!(self, ScopeLevel::Global)
    }

    /// Check if this scope is workflow-scoped
    pub fn is_workflow(&self) -> bool {
        matches!(self, ScopeLevel::Workflow(_))
    }

    /// Check if this scope is execution-scoped
    pub fn is_execution(&self) -> bool {
        matches!(self, ScopeLevel::Execution(_))
    }

    /// Check if this scope is action-scoped
    pub fn is_action(&self) -> bool {
        matches!(self, ScopeLevel::Action(_, _))
    }

    /// Get the workflow ID if this scope is workflow-scoped or lower
    pub fn workflow_id(&self) -> Option<&WorkflowId> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Workflow(id) => Some(id),
            ScopeLevel::Execution(_) => None, // Execution doesn't directly know workflow
            ScopeLevel::Action(_, _) => None, // Action doesn't directly know workflow
        }
    }

    /// Get the execution ID if this scope is execution-scoped or lower
    pub fn execution_id(&self) -> Option<&ExecutionId> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Workflow(_) => None,
            ScopeLevel::Execution(id) => Some(id),
            ScopeLevel::Action(id, _) => Some(id),
        }
    }

    /// Get the node ID if this scope is action-scoped
    pub fn node_id(&self) -> Option<&NodeId> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Workflow(_) => None,
            ScopeLevel::Execution(_) => None,
            ScopeLevel::Action(_, node_id) => Some(node_id),
        }
    }

    /// Check if this scope is contained within another scope
    pub fn is_contained_in(&self, other: &ScopeLevel) -> bool {
        match (self, other) {
            // Global scope contains everything
            (_, ScopeLevel::Global) => true,
            
            // Workflow scope contains execution and action scopes for that workflow
            (ScopeLevel::Execution(_exec_id), ScopeLevel::Workflow(_)) => {
                // Note: This is a simplified check. In practice, we'd need to
                // verify that the execution belongs to the workflow
                true
            }
            (ScopeLevel::Action(_exec_id, _), ScopeLevel::Workflow(_)) => {
                // Note: This is a simplified check. In practice, we'd need to
                // verify that the execution belongs to the workflow
                true
            }
            
            // Execution scope contains action scopes for that execution
            (ScopeLevel::Action(exec_id, _), ScopeLevel::Execution(other_exec_id)) => {
                exec_id == other_exec_id
            }
            
            // Otherwise, no containment
            _ => false,
        }
    }

    /// Get the parent scope level
    pub fn parent(&self) -> Option<ScopeLevel> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Workflow(_) => Some(ScopeLevel::Global),
            ScopeLevel::Execution(_) => None, // Execution doesn't have a direct parent
            ScopeLevel::Action(exec_id, _) => Some(ScopeLevel::Execution(exec_id.clone())),
        }
    }

    /// Create a child scope from this scope
    pub fn child(&self, child_type: ChildScopeType) -> Option<ScopeLevel> {
        match (self, child_type) {
            (ScopeLevel::Global, ChildScopeType::Workflow(workflow_id)) => {
                Some(ScopeLevel::Workflow(workflow_id))
            }
            (ScopeLevel::Workflow(_), ChildScopeType::Execution(exec_id)) => {
                Some(ScopeLevel::Execution(exec_id))
            }
            (ScopeLevel::Execution(exec_id), ChildScopeType::Action(node_id)) => {
                Some(ScopeLevel::Action(exec_id.clone(), node_id))
            }
            _ => None,
        }
    }
}

impl fmt::Display for ScopeLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeLevel::Global => write!(f, "global"),
            ScopeLevel::Workflow(id) => write!(f, "workflow:{}", id),
            ScopeLevel::Execution(id) => write!(f, "execution:{}", id),
            ScopeLevel::Action(exec_id, node_id) => {
                write!(f, "action:{}:{}", exec_id, node_id)
            }
        }
    }
}

/// Types of child scopes that can be created
#[derive(Debug, Clone)]
pub enum ChildScopeType {
    Workflow(WorkflowId),
    Execution(ExecutionId),
    Action(NodeId),
}

/// Scope-aware resource identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScopedId {
    /// The scope level for this resource
    pub scope: ScopeLevel,
    
    /// The resource identifier within the scope
    pub id: String,
}

impl ScopedId {
    /// Create a new scoped ID
    pub fn new(scope: ScopeLevel, id: impl Into<String>) -> Self {
        Self {
            scope,
            id: id.into(),
        }
    }

    /// Create a global scoped ID
    pub fn global(id: impl Into<String>) -> Self {
        Self::new(ScopeLevel::Global, id)
    }

    /// Create a workflow-scoped ID
    pub fn workflow(workflow_id: WorkflowId, id: impl Into<String>) -> Self {
        Self::new(ScopeLevel::Workflow(workflow_id), id)
    }

    /// Create an execution-scoped ID
    pub fn execution(execution_id: ExecutionId, id: impl Into<String>) -> Self {
        Self::new(ScopeLevel::Execution(execution_id), id)
    }

    /// Create an action-scoped ID
    pub fn action(execution_id: ExecutionId, node_id: NodeId, id: impl Into<String>) -> Self {
        Self::new(ScopeLevel::Action(execution_id, node_id), id)
    }

    /// Check if this ID is in the given scope
    pub fn is_in_scope(&self, scope: &ScopeLevel) -> bool {
        self.scope.is_contained_in(scope)
    }

    /// Get the full string representation
    pub fn to_string(&self) -> String {
        format!("{}:{}", self.scope, self.id)
    }
}

impl fmt::Display for ScopedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.scope, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_level_creation() {
        let workflow_id = WorkflowId::new("test-workflow");
        let execution_id = ExecutionId::new();
        let node_id = NodeId::new("test-node");

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id.clone());
        let execution = ScopeLevel::Execution(execution_id.clone());
        let action = ScopeLevel::Action(execution_id.clone(), node_id.clone());

        assert!(global.is_global());
        assert!(workflow.is_workflow());
        assert!(execution.is_execution());
        assert!(action.is_action());
    }

    #[test]
    fn test_scope_containment() {
        let workflow_id = WorkflowId::new("test-workflow");
        let execution_id = ExecutionId::new();
        let node_id = NodeId::new("test-node");

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id.clone());
        let execution = ScopeLevel::Execution(execution_id.clone());
        let action = ScopeLevel::Action(execution_id.clone(), node_id.clone());

        // Global contains everything
        assert!(global.is_contained_in(&global));
        assert!(workflow.is_contained_in(&global));
        assert!(execution.is_contained_in(&global));
        assert!(action.is_contained_in(&global));

        // Action is contained in execution
        assert!(action.is_contained_in(&execution));
    }

    #[test]
    fn test_scoped_id_creation() {
        let workflow_id = WorkflowId::new("test-workflow");
        let execution_id = ExecutionId::new();
        let node_id = NodeId::new("test-node");

        let global_id = ScopedId::global("global-resource");
        let workflow_id_scoped = ScopedId::workflow(workflow_id.clone(), "workflow-resource");
        let execution_id_scoped = ScopedId::execution(execution_id.clone(), "execution-resource");
        let action_id_scoped = ScopedId::action(execution_id.clone(), node_id.clone(), "action-resource");

        assert_eq!(global_id.scope, ScopeLevel::Global);
        assert_eq!(workflow_id_scoped.scope, ScopeLevel::Workflow(workflow_id));
        assert_eq!(execution_id_scoped.scope, ScopeLevel::Execution(execution_id.clone()));
        assert_eq!(action_id_scoped.scope, ScopeLevel::Action(execution_id, node_id));
    }

    #[test]
    fn test_scope_display() {
        let workflow_id = WorkflowId::new("test-workflow");
        let execution_id = ExecutionId::new();
        let node_id = NodeId::new("test-node");

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id.clone());
        let action = ScopeLevel::Action(execution_id, node_id);

        assert_eq!(global.to_string(), "global");
        assert_eq!(workflow.to_string(), "workflow:test-workflow");
        assert!(execution.to_string().starts_with("execution:"));
        assert!(action.to_string().starts_with("action:"));
    }
}
