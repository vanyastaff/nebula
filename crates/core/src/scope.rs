//! Scope system for resource lifecycle management
//!
//! Resources in Nebula have different lifecycle scopes:
//! - Global: Application lifetime
//! - Workflow: Per workflow execution
//! - Execution: Per single execution
//! - Action: Per action invocation

use serde::{Deserialize, Serialize};
use std::fmt;

use super::id::{ExecutionId, NodeId, OrganizationId, ProjectId, WorkflowId};

/// Defines the scope level for a resource
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScopeLevel {
    /// Resource lives for the entire application lifetime
    Global,

    /// Resource is scoped to an organization
    Organization(OrganizationId),

    /// Resource is scoped to a project
    Project(ProjectId),

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

    /// Check if this scope is organization-scoped
    pub fn is_organization(&self) -> bool {
        matches!(self, ScopeLevel::Organization(_))
    }

    /// Check if this scope is project-scoped
    pub fn is_project(&self) -> bool {
        matches!(self, ScopeLevel::Project(_))
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

    /// Get the organization ID if this scope is organization-scoped
    pub fn organization_id(&self) -> Option<&OrganizationId> {
        match self {
            ScopeLevel::Organization(id) => Some(id),
            _ => None,
        }
    }

    /// Get the project ID if this scope is project-scoped
    pub fn project_id(&self) -> Option<&ProjectId> {
        match self {
            ScopeLevel::Project(id) => Some(id),
            _ => None,
        }
    }

    /// Get the workflow ID if this scope is workflow-scoped or lower
    pub fn workflow_id(&self) -> Option<&WorkflowId> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Organization(_) => None,
            ScopeLevel::Project(_) => None,
            ScopeLevel::Workflow(id) => Some(id),
            ScopeLevel::Execution(_) => None, // Execution doesn't directly know workflow
            ScopeLevel::Action(_, _) => None, // Action doesn't directly know workflow
        }
    }

    /// Get the execution ID if this scope is execution-scoped or lower
    pub fn execution_id(&self) -> Option<&ExecutionId> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Organization(_) => None,
            ScopeLevel::Project(_) => None,
            ScopeLevel::Workflow(_) => None,
            ScopeLevel::Execution(id) => Some(id),
            ScopeLevel::Action(id, _) => Some(id),
        }
    }

    /// Get the node ID if this scope is action-scoped
    pub fn node_id(&self) -> Option<&NodeId> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Organization(_) => None,
            ScopeLevel::Project(_) => None,
            ScopeLevel::Workflow(_) => None,
            ScopeLevel::Execution(_) => None,
            ScopeLevel::Action(_, node_id) => Some(node_id),
        }
    }

    /// Check if this scope is contained within another scope
    ///
    /// Hierarchy: Global > Organization > Project > Workflow > Execution > Action
    pub fn is_contained_in(&self, other: &ScopeLevel) -> bool {
        match (self, other) {
            // Everything is contained in itself
            (a, b) if a == b => true,

            // Global scope contains everything
            (_, ScopeLevel::Global) => true,

            // Organization contains project, workflow, execution, action
            (ScopeLevel::Project(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Workflow(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Execution(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Action(_, _), ScopeLevel::Organization(_)) => true,

            // Project contains workflow, execution, action
            (ScopeLevel::Workflow(_), ScopeLevel::Project(_)) => true,
            (ScopeLevel::Execution(_), ScopeLevel::Project(_)) => true,
            (ScopeLevel::Action(_, _), ScopeLevel::Project(_)) => true,

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
    ///
    /// Note: Organization and Project don't have direct parent relationships
    /// as they require specific IDs. Use Global as the conceptual parent.
    pub fn parent(&self) -> Option<ScopeLevel> {
        match self {
            ScopeLevel::Global => None,
            ScopeLevel::Organization(_) => Some(ScopeLevel::Global),
            ScopeLevel::Project(_) => None, // Project parent would be Organization, but we don't track it
            ScopeLevel::Workflow(_) => None, // Workflow parent would be Project, but we don't track it
            ScopeLevel::Execution(_) => None, // Execution doesn't have a direct parent
            ScopeLevel::Action(exec_id, _) => Some(ScopeLevel::Execution(*exec_id)),
        }
    }

    /// Create a child scope from this scope
    pub fn child(&self, child_type: ChildScopeType) -> Option<ScopeLevel> {
        match (self, child_type) {
            (ScopeLevel::Global, ChildScopeType::Organization(org_id)) => {
                Some(ScopeLevel::Organization(org_id))
            }
            (ScopeLevel::Organization(_), ChildScopeType::Project(project_id)) => {
                Some(ScopeLevel::Project(project_id))
            }
            (ScopeLevel::Project(_), ChildScopeType::Workflow(workflow_id)) => {
                Some(ScopeLevel::Workflow(workflow_id))
            }
            (ScopeLevel::Global, ChildScopeType::Workflow(workflow_id)) => {
                Some(ScopeLevel::Workflow(workflow_id))
            }
            (ScopeLevel::Workflow(_), ChildScopeType::Execution(exec_id)) => {
                Some(ScopeLevel::Execution(exec_id))
            }
            (ScopeLevel::Execution(exec_id), ChildScopeType::Action(node_id)) => {
                Some(ScopeLevel::Action(*exec_id, node_id))
            }
            _ => None,
        }
    }
}

impl fmt::Display for ScopeLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeLevel::Global => write!(f, "global"),
            ScopeLevel::Organization(id) => write!(f, "organization:{}", id),
            ScopeLevel::Project(id) => write!(f, "project:{}", id),
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
    Organization(OrganizationId),
    Project(ProjectId),
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
        let workflow_id = WorkflowId::v4();
        let execution_id = ExecutionId::v4();
        let node_id = NodeId::v4();

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);
        let action = ScopeLevel::Action(execution_id, node_id);

        assert!(global.is_global());
        assert!(workflow.is_workflow());
        assert!(execution.is_execution());
        assert!(action.is_action());
    }

    #[test]
    fn test_scope_containment() {
        let workflow_id = WorkflowId::v4();
        let execution_id = ExecutionId::v4();
        let node_id = NodeId::v4();

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);
        let action = ScopeLevel::Action(execution_id, node_id);

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
        let workflow_id = WorkflowId::v4();
        let execution_id = ExecutionId::v4();
        let node_id = NodeId::v4();

        let global_id = ScopedId::global("global-resource");
        let workflow_id_scoped = ScopedId::workflow(workflow_id, "workflow-resource");
        let execution_id_scoped = ScopedId::execution(execution_id, "execution-resource");
        let action_id_scoped = ScopedId::action(execution_id, node_id, "action-resource");

        assert_eq!(global_id.scope, ScopeLevel::Global);
        assert_eq!(workflow_id_scoped.scope, ScopeLevel::Workflow(workflow_id));
        assert_eq!(
            execution_id_scoped.scope,
            ScopeLevel::Execution(execution_id)
        );
        assert_eq!(
            action_id_scoped.scope,
            ScopeLevel::Action(execution_id, node_id)
        );
    }

    #[test]
    fn test_scope_display() {
        let workflow_id = WorkflowId::v4();
        let execution_id = ExecutionId::v4();
        let node_id = NodeId::v4();

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);
        let action = ScopeLevel::Action(execution_id, node_id);

        assert_eq!(global.to_string(), "global");
        assert!(workflow.to_string().starts_with("workflow:"));
        assert!(execution.to_string().starts_with("execution:"));
        assert!(action.to_string().starts_with("action:"));
    }

    #[test]
    fn test_organization_scope() {
        let org_id = OrganizationId::v4();
        let org_scope = ScopeLevel::Organization(org_id);

        assert!(org_scope.is_organization());
        assert!(!org_scope.is_project());
        assert!(!org_scope.is_global());
        assert_eq!(org_scope.organization_id(), Some(&org_id));
        assert!(org_scope.to_string().starts_with("organization:"));
    }

    #[test]
    fn test_project_scope() {
        let project_id = ProjectId::v4();
        let project_scope = ScopeLevel::Project(project_id);

        assert!(project_scope.is_project());
        assert!(!project_scope.is_organization());
        assert!(!project_scope.is_global());
        assert_eq!(project_scope.project_id(), Some(&project_id));
        assert!(project_scope.to_string().starts_with("project:"));
    }

    #[test]
    fn test_new_scope_containment() {
        let org_id = OrganizationId::v4();
        let project_id = ProjectId::v4();
        let workflow_id = WorkflowId::v4();
        let execution_id = ExecutionId::v4();
        let node_id = NodeId::v4();

        let global = ScopeLevel::Global;
        let org = ScopeLevel::Organization(org_id);
        let project = ScopeLevel::Project(project_id);
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);
        let action = ScopeLevel::Action(execution_id, node_id);

        // Test hierarchy: Global > Organization > Project > Workflow > Execution > Action

        // Organization is contained in Global
        assert!(org.is_contained_in(&global));
        assert!(org.is_contained_in(&org));

        // Project is contained in Organization and Global
        assert!(project.is_contained_in(&org));
        assert!(project.is_contained_in(&global));
        assert!(project.is_contained_in(&project));

        // Workflow is contained in Project, Organization, and Global
        assert!(workflow.is_contained_in(&project));
        assert!(workflow.is_contained_in(&org));
        assert!(workflow.is_contained_in(&global));

        // Execution is contained in Workflow, Project, Organization, and Global
        assert!(execution.is_contained_in(&workflow));
        assert!(execution.is_contained_in(&project));
        assert!(execution.is_contained_in(&org));
        assert!(execution.is_contained_in(&global));

        // Action is contained in all higher levels
        assert!(action.is_contained_in(&execution));
        assert!(action.is_contained_in(&workflow));
        assert!(action.is_contained_in(&project));
        assert!(action.is_contained_in(&org));
        assert!(action.is_contained_in(&global));
    }

    #[test]
    fn test_scope_parent() {
        let org_id = OrganizationId::v4();
        let org_scope = ScopeLevel::Organization(org_id);

        // Organization's parent is Global
        assert_eq!(org_scope.parent(), Some(ScopeLevel::Global));

        // Global has no parent
        assert_eq!(ScopeLevel::Global.parent(), None);
    }

    #[test]
    fn test_scope_child() {
        let org_id = OrganizationId::v4();
        let project_id = ProjectId::v4();
        let workflow_id = WorkflowId::v4();

        // Global can create Organization child
        let global = ScopeLevel::Global;
        let org_scope = global.child(ChildScopeType::Organization(org_id));
        assert_eq!(org_scope, Some(ScopeLevel::Organization(org_id)));

        // Organization can create Project child
        let org = ScopeLevel::Organization(org_id);
        let project_scope = org.child(ChildScopeType::Project(project_id));
        assert_eq!(project_scope, Some(ScopeLevel::Project(project_id)));

        // Project can create Workflow child
        let project = ScopeLevel::Project(project_id);
        let workflow_scope = project.child(ChildScopeType::Workflow(workflow_id));
        assert_eq!(workflow_scope, Some(ScopeLevel::Workflow(workflow_id)));
    }

    #[test]
    fn test_scope_id_getters() {
        let org_id = OrganizationId::v4();
        let project_id = ProjectId::v4();

        let org_scope = ScopeLevel::Organization(org_id);
        let project_scope = ScopeLevel::Project(project_id);

        // Test organization_id getter
        assert_eq!(org_scope.organization_id(), Some(&org_id));
        assert_eq!(project_scope.organization_id(), None);

        // Test project_id getter
        assert_eq!(project_scope.project_id(), Some(&project_id));
        assert_eq!(org_scope.project_id(), None);
    }
}
