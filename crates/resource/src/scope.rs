//! Resource scoping and visibility management

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Defines the scope and visibility of a resource
///
/// Each variant carries optional parent identifiers so that `contains()`
/// can verify the parent chain instead of unconditionally returning true.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub enum ResourceScope {
    /// Global scope - shared across all workflows and tenants
    #[default]
    Global,
    /// Tenant scope - isolated per tenant for multi-tenancy
    Tenant {
        /// The tenant identifier
        tenant_id: String,
    },
    /// Workflow scope - scoped to a specific workflow definition
    Workflow {
        /// The workflow identifier
        workflow_id: String,
        /// Owning tenant (if known)
        tenant_id: Option<String>,
    },
    /// Execution scope - scoped to a specific workflow execution
    Execution {
        /// The execution identifier
        execution_id: String,
        /// Owning workflow (if known)
        workflow_id: Option<String>,
        /// Owning tenant (if known)
        tenant_id: Option<String>,
    },
    /// Action scope - scoped to a specific action within a workflow
    Action {
        /// The action identifier
        action_id: String,
        /// Owning execution (if known)
        execution_id: Option<String>,
        /// Owning workflow (if known)
        workflow_id: Option<String>,
        /// Owning tenant (if known)
        tenant_id: Option<String>,
    },
    /// Custom scope with a key-value pair
    Custom {
        /// The scope key
        key: String,
        /// The scope value
        value: String,
    },
}

impl ResourceScope {
    /// Create a tenant scope
    pub fn tenant<S: Into<String>>(tenant_id: S) -> Self {
        Self::Tenant {
            tenant_id: tenant_id.into(),
        }
    }

    /// Create a workflow scope without parent info
    pub fn workflow<S: Into<String>>(workflow_id: S) -> Self {
        Self::Workflow {
            workflow_id: workflow_id.into(),
            tenant_id: None,
        }
    }

    /// Create a workflow scope with tenant parent
    pub fn workflow_in_tenant<S: Into<String>>(workflow_id: S, tenant_id: S) -> Self {
        Self::Workflow {
            workflow_id: workflow_id.into(),
            tenant_id: Some(tenant_id.into()),
        }
    }

    /// Create an execution scope without parent info
    pub fn execution<S: Into<String>>(execution_id: S) -> Self {
        Self::Execution {
            execution_id: execution_id.into(),
            workflow_id: None,
            tenant_id: None,
        }
    }

    /// Create an execution scope with full parent chain
    pub fn execution_in_workflow<S: Into<String>>(
        execution_id: S,
        workflow_id: S,
        tenant_id: Option<String>,
    ) -> Self {
        Self::Execution {
            execution_id: execution_id.into(),
            workflow_id: Some(workflow_id.into()),
            tenant_id,
        }
    }

    /// Create an action scope without parent info
    pub fn action<S: Into<String>>(action_id: S) -> Self {
        Self::Action {
            action_id: action_id.into(),
            execution_id: None,
            workflow_id: None,
            tenant_id: None,
        }
    }

    /// Create an action scope with full parent chain
    pub fn action_in_execution<S: Into<String>>(
        action_id: S,
        execution_id: S,
        workflow_id: Option<String>,
        tenant_id: Option<String>,
    ) -> Self {
        Self::Action {
            action_id: action_id.into(),
            execution_id: Some(execution_id.into()),
            workflow_id,
            tenant_id,
        }
    }

    /// Create a custom scope
    pub fn custom<S: Into<String>>(key: S, value: S) -> Self {
        Self::Custom {
            key: key.into(),
            value: value.into(),
        }
    }

    /// Get the scope hierarchy level (lower numbers = broader scope)
    #[must_use]
    pub fn hierarchy_level(&self) -> u8 {
        match self {
            Self::Global => 0,
            Self::Tenant { .. } => 1,
            Self::Workflow { .. } => 2,
            Self::Execution { .. } => 3,
            Self::Action { .. } => 4,
            Self::Custom { .. } => 5,
        }
    }

    /// Check if this scope is broader than another scope
    #[must_use]
    pub fn is_broader_than(&self, other: &ResourceScope) -> bool {
        self.hierarchy_level() < other.hierarchy_level()
    }

    /// Check if this scope is narrower than another scope
    #[must_use]
    pub fn is_narrower_than(&self, other: &ResourceScope) -> bool {
        self.hierarchy_level() > other.hierarchy_level()
    }

    /// Check if this scope contains another scope.
    ///
    /// Containment requires the child to have a matching parent identifier.
    /// If the child's parent is unknown (`None`), containment is denied
    /// (deny-by-default for security).
    #[must_use]
    pub fn contains(&self, other: &ResourceScope) -> bool {
        match (self, other) {
            // Global contains everything
            (Self::Global, _) => true,

            // Tenant == Tenant: same tenant_id
            (Self::Tenant { tenant_id: t1 }, Self::Tenant { tenant_id: t2 }) => t1 == t2,

            // Tenant contains Workflow if tenant_id matches
            (
                Self::Tenant { tenant_id: t1 },
                Self::Workflow {
                    tenant_id: Some(t2),
                    ..
                },
            ) => t1 == t2,

            // Tenant contains Execution if tenant_id matches
            (
                Self::Tenant { tenant_id: t1 },
                Self::Execution {
                    tenant_id: Some(t2),
                    ..
                },
            ) => t1 == t2,

            // Tenant contains Action if tenant_id matches
            (
                Self::Tenant { tenant_id: t1 },
                Self::Action {
                    tenant_id: Some(t2),
                    ..
                },
            ) => t1 == t2,

            // Workflow == Workflow: same workflow_id
            (
                Self::Workflow {
                    workflow_id: w1, ..
                },
                Self::Workflow {
                    workflow_id: w2, ..
                },
            ) => w1 == w2,

            // Workflow contains Execution if workflow_id matches
            (
                Self::Workflow {
                    workflow_id: w1, ..
                },
                Self::Execution {
                    workflow_id: Some(w2),
                    ..
                },
            ) => w1 == w2,

            // Workflow contains Action if workflow_id matches
            (
                Self::Workflow {
                    workflow_id: w1, ..
                },
                Self::Action {
                    workflow_id: Some(w2),
                    ..
                },
            ) => w1 == w2,

            // Execution == Execution: same execution_id
            (
                Self::Execution {
                    execution_id: e1, ..
                },
                Self::Execution {
                    execution_id: e2, ..
                },
            ) => e1 == e2,

            // Execution contains Action if execution_id matches
            (
                Self::Execution {
                    execution_id: e1, ..
                },
                Self::Action {
                    execution_id: Some(e2),
                    ..
                },
            ) => e1 == e2,

            // Action only contains itself
            (Self::Action { action_id: a1, .. }, Self::Action { action_id: a2, .. }) => a1 == a2,

            // Custom scopes only contain themselves
            (
                Self::Custom {
                    key: k1, value: v1, ..
                },
                Self::Custom {
                    key: k2, value: v2, ..
                },
            ) => k1 == k2 && v1 == v2,

            // Everything else: deny by default
            _ => false,
        }
    }

    /// Generate a scope key for storage/lookup
    #[must_use]
    pub fn scope_key(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Tenant { tenant_id } => format!("tenant:{tenant_id}"),
            Self::Workflow { workflow_id, .. } => format!("workflow:{workflow_id}"),
            Self::Execution { execution_id, .. } => format!("execution:{execution_id}"),
            Self::Action { action_id, .. } => format!("action:{action_id}"),
            Self::Custom { key, value } => format!("custom:{key}={value}"),
        }
    }

    /// Get a human-readable description of the scope
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::Global => "Global scope (shared across all workflows and tenants)".to_string(),
            Self::Tenant { tenant_id } => format!("Tenant scope (tenant: {tenant_id})"),
            Self::Workflow { workflow_id, .. } => {
                format!("Workflow scope (workflow: {workflow_id})")
            }
            Self::Execution { execution_id, .. } => {
                format!("Execution scope (execution: {execution_id})")
            }
            Self::Action { action_id, .. } => format!("Action scope (action: {action_id})"),
            Self::Custom { key, value } => {
                format!("Custom scope ({key}={value})")
            }
        }
    }
}

impl fmt::Display for ResourceScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.scope_key())
    }
}

/// Scoping strategy for resource allocation
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub enum ScopingStrategy {
    /// Strict scoping - only exact scope matches
    Strict,
    /// Hierarchical scoping - allows broader scopes to be used
    #[default]
    Hierarchical,
    /// Fallback scoping - tries exact match, then falls back to broader scopes
    Fallback,
}

impl ScopingStrategy {
    /// Check if a resource scope is compatible with a requested scope using this strategy
    #[must_use]
    pub fn is_compatible(
        &self,
        resource_scope: &ResourceScope,
        requested_scope: &ResourceScope,
    ) -> bool {
        match self {
            Self::Strict => resource_scope == requested_scope,
            Self::Hierarchical => resource_scope.contains(requested_scope),
            Self::Fallback => {
                resource_scope == requested_scope || resource_scope.contains(requested_scope)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_hierarchy_levels() {
        assert_eq!(ResourceScope::Global.hierarchy_level(), 0);
        assert_eq!(ResourceScope::tenant("test").hierarchy_level(), 1);
        assert_eq!(ResourceScope::workflow("wf").hierarchy_level(), 2);
        assert_eq!(ResourceScope::execution("ex").hierarchy_level(), 3);
        assert_eq!(ResourceScope::action("act").hierarchy_level(), 4);
    }

    #[test]
    fn test_scope_containment_global() {
        let global = ResourceScope::Global;
        let tenant = ResourceScope::tenant("tenant1");
        let workflow = ResourceScope::workflow("wf1");

        assert!(global.contains(&tenant));
        assert!(global.contains(&workflow));
        assert!(!tenant.contains(&global));
        assert!(!workflow.contains(&global));
    }

    #[test]
    fn test_tenant_isolation() {
        let tenant_a = ResourceScope::tenant("A");
        let tenant_b = ResourceScope::tenant("B");

        // Same tenant
        assert!(tenant_a.contains(&ResourceScope::tenant("A")));
        // Different tenant
        assert!(!tenant_a.contains(&tenant_b));

        // Workflow with known parent tenant
        let wf_in_a = ResourceScope::workflow_in_tenant("wf1", "A");
        let wf_in_b = ResourceScope::workflow_in_tenant("wf1", "B");
        assert!(tenant_a.contains(&wf_in_a));
        assert!(!tenant_a.contains(&wf_in_b));

        // Workflow without parent info: deny by default
        let wf_no_parent = ResourceScope::workflow("wf1");
        assert!(!tenant_a.contains(&wf_no_parent));
    }

    #[test]
    fn test_workflow_containment() {
        let wf = ResourceScope::workflow("wf1");

        // Execution with matching workflow parent
        let exec_in_wf1 = ResourceScope::execution_in_workflow("ex1", "wf1", Some("A".to_string()));
        assert!(wf.contains(&exec_in_wf1));

        // Execution with different workflow parent
        let exec_in_wf2 = ResourceScope::execution_in_workflow("ex1", "wf2", Some("A".to_string()));
        assert!(!wf.contains(&exec_in_wf2));

        // Execution without workflow info: deny
        let exec_no_parent = ResourceScope::execution("ex1");
        assert!(!wf.contains(&exec_no_parent));
    }

    #[test]
    fn test_execution_containment() {
        let exec = ResourceScope::execution("ex1");

        // Action with matching execution parent
        let action_in_ex1 = ResourceScope::action_in_execution("a1", "ex1", None, None);
        assert!(exec.contains(&action_in_ex1));

        // Action with different execution parent
        let action_in_ex2 = ResourceScope::action_in_execution("a1", "ex2", None, None);
        assert!(!exec.contains(&action_in_ex2));

        // Action without execution info: deny
        let action_no_parent = ResourceScope::action("a1");
        assert!(!exec.contains(&action_no_parent));
    }

    #[test]
    fn test_scope_keys() {
        assert_eq!(ResourceScope::Global.scope_key(), "global");
        assert_eq!(ResourceScope::tenant("t1").scope_key(), "tenant:t1");
        assert_eq!(ResourceScope::workflow("w1").scope_key(), "workflow:w1");
        assert_eq!(ResourceScope::execution("e1").scope_key(), "execution:e1");
        assert_eq!(ResourceScope::action("a1").scope_key(), "action:a1");

        let custom = ResourceScope::custom("env", "prod");
        assert_eq!(custom.scope_key(), "custom:env=prod");
    }

    #[test]
    fn test_scoping_strategies() {
        let global = ResourceScope::Global;
        let tenant = ResourceScope::tenant("t1");

        assert!(ScopingStrategy::Strict.is_compatible(&global, &global));
        assert!(!ScopingStrategy::Strict.is_compatible(&global, &tenant));

        assert!(ScopingStrategy::Hierarchical.is_compatible(&global, &tenant));
        assert!(!ScopingStrategy::Hierarchical.is_compatible(&tenant, &global));

        assert!(ScopingStrategy::Fallback.is_compatible(&global, &tenant));
        assert!(ScopingStrategy::Fallback.is_compatible(&tenant, &tenant));
    }

    #[test]
    fn test_cross_tenant_denial() {
        // This is the security fix: Tenant A must NOT be able to access Tenant B's resources
        let tenant_a = ResourceScope::tenant("A");

        let wf_in_b = ResourceScope::workflow_in_tenant("wf1", "B");
        let exec_in_b = ResourceScope::execution_in_workflow("ex1", "wf1", Some("B".to_string()));
        let action_in_b = ResourceScope::action_in_execution(
            "a1",
            "ex1",
            Some("wf1".to_string()),
            Some("B".to_string()),
        );

        assert!(!tenant_a.contains(&wf_in_b));
        assert!(!tenant_a.contains(&exec_in_b));
        assert!(!tenant_a.contains(&action_in_b));
    }
}
