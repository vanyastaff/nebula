//! Scope system for resource lifecycle management.
//!
//! Resources in Nebula have different lifecycle scopes:
//! - Global: Application lifetime
//! - Organization: Per organization
//! - Workspace: Per workspace (formerly "project")
//! - Workflow: Per workflow definition
//! - Execution: Per single execution

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    NodeKey,
    id::{
        AttemptId, ExecutionId, InstanceId, OrgId, TriggerId, WorkflowId, WorkflowVersionId,
        WorkspaceId,
    },
};

/// Defines the scope level for a resource.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScopeLevel {
    /// Resource lives for the entire application lifetime.
    Global,

    /// Resource is scoped to an organization.
    Organization(OrgId),

    /// Resource is scoped to a workspace.
    Workspace(WorkspaceId),

    /// Resource lives for the duration of a workflow.
    Workflow(WorkflowId),

    /// Resource lives for the duration of a single execution.
    Execution(ExecutionId),
}

impl ScopeLevel {
    /// Check if this scope is global.
    pub fn is_global(&self) -> bool {
        matches!(self, ScopeLevel::Global)
    }

    /// Check if this scope is organization-scoped.
    pub fn is_organization(&self) -> bool {
        matches!(self, ScopeLevel::Organization(_))
    }

    /// Check if this scope is workspace-scoped.
    pub fn is_workspace(&self) -> bool {
        matches!(self, ScopeLevel::Workspace(_))
    }

    /// Check if this scope is workflow-scoped.
    pub fn is_workflow(&self) -> bool {
        matches!(self, ScopeLevel::Workflow(_))
    }

    /// Check if this scope is execution-scoped.
    pub fn is_execution(&self) -> bool {
        matches!(self, ScopeLevel::Execution(_))
    }

    /// Get the organization ID if this scope is organization-scoped.
    pub fn organization_id(&self) -> Option<&OrgId> {
        match self {
            ScopeLevel::Organization(id) => Some(id),
            _ => None,
        }
    }

    /// Get the workspace ID if this scope is workspace-scoped.
    pub fn workspace_id(&self) -> Option<&WorkspaceId> {
        match self {
            ScopeLevel::Workspace(id) => Some(id),
            _ => None,
        }
    }

    /// Get the workflow ID if this scope is workflow-scoped.
    pub fn workflow_id(&self) -> Option<&WorkflowId> {
        match self {
            ScopeLevel::Workflow(id) => Some(id),
            _ => None,
        }
    }

    /// Get the execution ID if this scope is execution-scoped.
    pub fn execution_id(&self) -> Option<&ExecutionId> {
        match self {
            ScopeLevel::Execution(id) => Some(id),
            _ => None,
        }
    }

    /// Check if this scope is contained within another scope.
    ///
    /// Hierarchy: Global > Organization > Workspace > Workflow > Execution
    pub fn is_contained_in(&self, other: &ScopeLevel) -> bool {
        match (self, other) {
            (a, b) if a == b => true,
            (_, ScopeLevel::Global) => true,
            (ScopeLevel::Workspace(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Workflow(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Execution(_), ScopeLevel::Organization(_)) => true,
            (ScopeLevel::Workflow(_), ScopeLevel::Workspace(_)) => true,
            (ScopeLevel::Execution(_), ScopeLevel::Workspace(_)) => true,
            (ScopeLevel::Execution(_), ScopeLevel::Workflow(_)) => true,
            _ => false,
        }
    }

    /// Strict containment check that verifies ID ownership via a resolver.
    pub fn is_contained_in_strict<R: ScopeResolver>(
        &self,
        other: &ScopeLevel,
        resolver: &R,
    ) -> bool {
        match (self, other) {
            (a, b) if a == b => true,
            (_, ScopeLevel::Global) => true,

            (ScopeLevel::Workspace(this), ScopeLevel::Organization(other_org)) => {
                resolver.organization_for_workspace(this).as_ref() == Some(other_org)
            },
            (ScopeLevel::Workflow(this), ScopeLevel::Organization(other_org)) => {
                resolver
                    .workspace_for_workflow(this)
                    .and_then(|p| resolver.organization_for_workspace(&p))
                    .as_ref()
                    == Some(other_org)
            },
            (ScopeLevel::Execution(this), ScopeLevel::Organization(other_org)) => {
                resolver
                    .workflow_for_execution(this)
                    .and_then(|w| resolver.workspace_for_workflow(&w))
                    .and_then(|p| resolver.organization_for_workspace(&p))
                    .as_ref()
                    == Some(other_org)
            },

            (ScopeLevel::Workflow(this), ScopeLevel::Workspace(other_ws)) => {
                resolver.workspace_for_workflow(this).as_ref() == Some(other_ws)
            },
            (ScopeLevel::Execution(this), ScopeLevel::Workspace(other_ws)) => {
                resolver
                    .workflow_for_execution(this)
                    .and_then(|w| resolver.workspace_for_workflow(&w))
                    .as_ref()
                    == Some(other_ws)
            },

            (ScopeLevel::Execution(this), ScopeLevel::Workflow(other_wf)) => {
                resolver.workflow_for_execution(this).as_ref() == Some(other_wf)
            },

            _ => false,
        }
    }
}

/// Resolver for scope ownership.
pub trait ScopeResolver {
    /// Resolve the workflow that owns this execution.
    fn workflow_for_execution(&self, exec_id: &ExecutionId) -> Option<WorkflowId>;

    /// Resolve the workspace that owns this workflow.
    fn workspace_for_workflow(&self, workflow_id: &WorkflowId) -> Option<WorkspaceId>;

    /// Resolve the organization that owns this workspace.
    fn organization_for_workspace(&self, workspace_id: &WorkspaceId) -> Option<OrgId>;
}

impl fmt::Display for ScopeLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeLevel::Global => write!(f, "global"),
            ScopeLevel::Organization(id) => write!(f, "organization:{id}"),
            ScopeLevel::Workspace(id) => write!(f, "workspace:{id}"),
            ScopeLevel::Workflow(id) => write!(f, "workflow:{id}"),
            ScopeLevel::Execution(id) => write!(f, "execution:{id}"),
        }
    }
}

/// Bag of optional IDs representing the full scope path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// Organization ID, if known.
    pub org_id: Option<OrgId>,
    /// Workspace ID, if known.
    pub workspace_id: Option<WorkspaceId>,
    /// Workflow ID, if known.
    pub workflow_id: Option<WorkflowId>,
    /// Workflow version ID, if known.
    pub workflow_version_id: Option<WorkflowVersionId>,
    /// Execution ID, if known.
    pub execution_id: Option<ExecutionId>,
    /// Author-defined node key, if known.
    pub node_key: Option<NodeKey>,
    /// Attempt ID, if known.
    pub attempt_id: Option<AttemptId>,
    /// Trigger ID, if known.
    pub trigger_id: Option<TriggerId>,
    /// Instance ID, if known.
    pub instance_id: Option<InstanceId>,
}

impl Scope {
    /// Check whether this scope can access a resource registered at the given level.
    ///
    /// Strict containment: resource scope must be broader-or-equal to caller scope.
    pub fn can_access(&self, registered: &ScopeLevel) -> bool {
        match registered {
            ScopeLevel::Global => true,
            ScopeLevel::Organization(o) => self.org_id == Some(*o),
            ScopeLevel::Workspace(w) => self.workspace_id == Some(*w),
            ScopeLevel::Workflow(w) => self.workflow_id == Some(*w),
            ScopeLevel::Execution(e) => self.execution_id == Some(*e),
        }
    }
}

/// Actor identity within the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Principal {
    /// Human user.
    User(crate::id::UserId),
    /// Automated service account.
    ServiceAccount(crate::id::ServiceAccountId),
    /// Workflow acting on its own behalf, optionally triggered by a specific trigger.
    Workflow {
        /// The workflow performing the action.
        workflow_id: WorkflowId,
        /// The trigger that initiated the workflow, if any.
        trigger_id: Option<TriggerId>,
    },
    /// System-internal operation.
    System,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_level_creation() {
        let workflow_id = WorkflowId::new();
        let execution_id = ExecutionId::new();

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);

        assert!(global.is_global());
        assert!(workflow.is_workflow());
        assert!(execution.is_execution());
    }

    #[test]
    fn test_scope_containment() {
        let workflow_id = WorkflowId::new();
        let execution_id = ExecutionId::new();

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);

        assert!(global.is_contained_in(&global));
        assert!(workflow.is_contained_in(&global));
        assert!(execution.is_contained_in(&global));
        assert!(execution.is_contained_in(&workflow));
    }

    #[test]
    fn test_scope_display() {
        let workflow_id = WorkflowId::new();
        let execution_id = ExecutionId::new();

        let global = ScopeLevel::Global;
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);

        assert_eq!(global.to_string(), "global");
        assert!(workflow.to_string().starts_with("workflow:"));
        assert!(execution.to_string().starts_with("execution:"));
    }

    #[test]
    fn test_organization_scope() {
        let org_id = OrgId::new();
        let org_scope = ScopeLevel::Organization(org_id);

        assert!(org_scope.is_organization());
        assert!(!org_scope.is_workspace());
        assert!(!org_scope.is_global());
        assert_eq!(org_scope.organization_id(), Some(&org_id));
        assert!(org_scope.to_string().starts_with("organization:"));
    }

    #[test]
    fn test_workspace_scope() {
        let ws_id = WorkspaceId::new();
        let ws_scope = ScopeLevel::Workspace(ws_id);

        assert!(ws_scope.is_workspace());
        assert!(!ws_scope.is_organization());
        assert!(!ws_scope.is_global());
        assert_eq!(ws_scope.workspace_id(), Some(&ws_id));
        assert!(ws_scope.to_string().starts_with("workspace:"));
    }

    #[test]
    fn test_new_scope_containment() {
        let org_id = OrgId::new();
        let ws_id = WorkspaceId::new();
        let workflow_id = WorkflowId::new();
        let execution_id = ExecutionId::new();

        let global = ScopeLevel::Global;
        let org = ScopeLevel::Organization(org_id);
        let workspace = ScopeLevel::Workspace(ws_id);
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);

        // Organization is contained in Global
        assert!(org.is_contained_in(&global));
        assert!(org.is_contained_in(&org));

        // Workspace is contained in Organization and Global
        assert!(workspace.is_contained_in(&org));
        assert!(workspace.is_contained_in(&global));

        // Workflow is contained in Workspace, Organization, and Global
        assert!(workflow.is_contained_in(&workspace));
        assert!(workflow.is_contained_in(&org));
        assert!(workflow.is_contained_in(&global));

        // Execution is contained in all higher levels
        assert!(execution.is_contained_in(&workflow));
        assert!(execution.is_contained_in(&workspace));
        assert!(execution.is_contained_in(&org));
        assert!(execution.is_contained_in(&global));
    }

    #[test]
    fn test_is_contained_in_strict_with_resolver() {
        let org_id = OrgId::new();
        let ws_id = WorkspaceId::new();
        let workflow_id = WorkflowId::new();
        let execution_id = ExecutionId::new();

        struct MockResolver {
            org_id: OrgId,
            ws_id: WorkspaceId,
            workflow_id: WorkflowId,
            execution_id: ExecutionId,
        }
        impl ScopeResolver for MockResolver {
            fn workflow_for_execution(&self, exec_id: &ExecutionId) -> Option<WorkflowId> {
                if exec_id == &self.execution_id {
                    Some(self.workflow_id)
                } else {
                    None
                }
            }
            fn workspace_for_workflow(&self, wf_id: &WorkflowId) -> Option<WorkspaceId> {
                if wf_id == &self.workflow_id {
                    Some(self.ws_id)
                } else {
                    None
                }
            }
            fn organization_for_workspace(&self, ws_id: &WorkspaceId) -> Option<OrgId> {
                if ws_id == &self.ws_id {
                    Some(self.org_id)
                } else {
                    None
                }
            }
        }
        let resolver = MockResolver {
            org_id,
            ws_id,
            workflow_id,
            execution_id,
        };

        let org = ScopeLevel::Organization(org_id);
        let workspace = ScopeLevel::Workspace(ws_id);
        let workflow = ScopeLevel::Workflow(workflow_id);
        let execution = ScopeLevel::Execution(execution_id);

        assert!(execution.is_contained_in_strict(&workflow, &resolver));
        assert!(execution.is_contained_in_strict(&workspace, &resolver));
        assert!(execution.is_contained_in_strict(&org, &resolver));

        // Wrong execution_id would not resolve
        let other_exec_id = ExecutionId::new();
        let other_execution = ScopeLevel::Execution(other_exec_id);
        assert!(!other_execution.is_contained_in_strict(&workflow, &resolver));
    }
}
