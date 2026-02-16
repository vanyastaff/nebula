//! Resource scoping and visibility management

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Defines the scope and visibility of a resource
///
/// Each variant carries optional parent identifiers so that `contains()`
/// can verify the parent chain instead of unconditionally returning true.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub enum Scope {
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

/// Check if parent chain fields are consistent for transitivity.
///
/// Returns `false` if the parent specifies a value but the child does not
/// (deny-by-default). This ensures that a scope with known ancestry never
/// "contains" a scope with unknown (and therefore potentially different)
/// ancestry.
fn parents_consistent(parent: Option<&str>, child: Option<&str>) -> bool {
    match (parent, child) {
        (Some(p), Some(c)) => p == c,
        (Some(_), None) => false,
        // (None, Some(_)) or (None, None) — parent doesn't constrain
        _ => true,
    }
}

impl Scope {
    /// Create a tenant scope.
    ///
    /// # Panics
    /// Panics if `tenant_id` is empty.
    pub fn tenant<S: Into<String>>(tenant_id: S) -> Self {
        let tenant_id = tenant_id.into();
        assert!(!tenant_id.is_empty(), "tenant_id must not be empty");
        Self::Tenant { tenant_id }
    }

    /// Create a workflow scope without parent info.
    ///
    /// # Panics
    /// Panics if `workflow_id` is empty.
    pub fn workflow<S: Into<String>>(workflow_id: S) -> Self {
        let workflow_id = workflow_id.into();
        assert!(!workflow_id.is_empty(), "workflow_id must not be empty");
        Self::Workflow {
            workflow_id,
            tenant_id: None,
        }
    }

    /// Create a workflow scope with tenant parent.
    ///
    /// # Panics
    /// Panics if `workflow_id` or `tenant_id` is empty.
    pub fn workflow_in_tenant(
        workflow_id: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> Self {
        let workflow_id = workflow_id.into();
        let tenant_id = tenant_id.into();
        assert!(!workflow_id.is_empty(), "workflow_id must not be empty");
        assert!(!tenant_id.is_empty(), "tenant_id must not be empty");
        Self::Workflow {
            workflow_id,
            tenant_id: Some(tenant_id),
        }
    }

    /// Create an execution scope without parent info.
    ///
    /// # Panics
    /// Panics if `execution_id` is empty.
    pub fn execution<S: Into<String>>(execution_id: S) -> Self {
        let execution_id = execution_id.into();
        assert!(!execution_id.is_empty(), "execution_id must not be empty");
        Self::Execution {
            execution_id,
            workflow_id: None,
            tenant_id: None,
        }
    }

    /// Create an execution scope with full parent chain.
    ///
    /// # Panics
    /// Panics if `execution_id` or `workflow_id` is empty.
    pub fn execution_in_workflow(
        execution_id: impl Into<String>,
        workflow_id: impl Into<String>,
        tenant_id: Option<String>,
    ) -> Self {
        let execution_id = execution_id.into();
        let workflow_id = workflow_id.into();
        assert!(!execution_id.is_empty(), "execution_id must not be empty");
        assert!(!workflow_id.is_empty(), "workflow_id must not be empty");
        Self::Execution {
            execution_id,
            workflow_id: Some(workflow_id),
            tenant_id,
        }
    }

    /// Create an action scope without parent info.
    ///
    /// # Panics
    /// Panics if `action_id` is empty.
    pub fn action<S: Into<String>>(action_id: S) -> Self {
        let action_id = action_id.into();
        assert!(!action_id.is_empty(), "action_id must not be empty");
        Self::Action {
            action_id,
            execution_id: None,
            workflow_id: None,
            tenant_id: None,
        }
    }

    /// Create an action scope with full parent chain.
    ///
    /// # Panics
    /// Panics if `action_id` or `execution_id` is empty.
    pub fn action_in_execution(
        action_id: impl Into<String>,
        execution_id: impl Into<String>,
        workflow_id: Option<String>,
        tenant_id: Option<String>,
    ) -> Self {
        let action_id = action_id.into();
        let execution_id = execution_id.into();
        assert!(!action_id.is_empty(), "action_id must not be empty");
        assert!(!execution_id.is_empty(), "execution_id must not be empty");
        Self::Action {
            action_id,
            execution_id: Some(execution_id),
            workflow_id,
            tenant_id,
        }
    }

    /// Create a custom scope.
    ///
    /// # Panics
    /// Panics if `key` or `value` is empty.
    pub fn custom(key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = value.into();
        assert!(!key.is_empty(), "custom scope key must not be empty");
        assert!(!value.is_empty(), "custom scope value must not be empty");
        Self::Custom { key, value }
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
    pub fn is_broader_than(&self, other: &Scope) -> bool {
        self.hierarchy_level() < other.hierarchy_level()
    }

    /// Check if this scope is narrower than another scope
    #[must_use]
    pub fn is_narrower_than(&self, other: &Scope) -> bool {
        self.hierarchy_level() > other.hierarchy_level()
    }

    /// Check if this scope contains another scope.
    ///
    /// Containment requires the child to have matching parent identifiers
    /// throughout the entire chain. If the child's parent is unknown (`None`)
    /// but the parent scope specifies that field, containment is denied
    /// (deny-by-default for security). This guarantees transitivity:
    /// if A contains B and B contains C, then A contains C.
    #[must_use]
    pub fn contains(&self, other: &Scope) -> bool {
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

            // Workflow contains Workflow: all fields must match
            (Self::Workflow { .. }, Self::Workflow { .. }) => self == other,

            // Workflow contains Execution if workflow_id matches AND tenant consistent
            (
                Self::Workflow {
                    workflow_id: w1,
                    tenant_id: t1,
                },
                Self::Execution {
                    workflow_id: Some(w2),
                    tenant_id: t2,
                    ..
                },
            ) => w1 == w2 && parents_consistent(t1.as_deref(), t2.as_deref()),

            // Workflow contains Action if workflow_id matches AND tenant consistent
            (
                Self::Workflow {
                    workflow_id: w1,
                    tenant_id: t1,
                },
                Self::Action {
                    workflow_id: Some(w2),
                    tenant_id: t2,
                    ..
                },
            ) => w1 == w2 && parents_consistent(t1.as_deref(), t2.as_deref()),

            // Execution contains Execution: all fields must match
            (Self::Execution { .. }, Self::Execution { .. }) => self == other,

            // Execution contains Action if execution_id matches AND parents consistent
            (
                Self::Execution {
                    execution_id: e1,
                    workflow_id: w1,
                    tenant_id: t1,
                },
                Self::Action {
                    execution_id: Some(e2),
                    workflow_id: w2,
                    tenant_id: t2,
                    ..
                },
            ) => {
                e1 == e2
                    && parents_consistent(w1.as_deref(), w2.as_deref())
                    && parents_consistent(t1.as_deref(), t2.as_deref())
            }

            // Action only contains the exact same action (all fields must match)
            (Self::Action { .. }, Self::Action { .. }) => self == other,

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

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.scope_key())
    }
}

/// Scoping strategy for resource allocation
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default)]
pub enum Strategy {
    /// Strict scoping - only exact scope matches
    Strict,
    /// Hierarchical scoping - allows broader scopes to be used
    #[default]
    Hierarchical,
    /// Fallback scoping - tries exact match, then falls back to broader scopes
    Fallback,
}

impl Strategy {
    /// Check if a resource scope is compatible with a requested scope using this strategy
    #[must_use]
    pub fn is_compatible(&self, resource_scope: &Scope, requested_scope: &Scope) -> bool {
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
        assert_eq!(Scope::Global.hierarchy_level(), 0);
        assert_eq!(Scope::tenant("test").hierarchy_level(), 1);
        assert_eq!(Scope::workflow("wf").hierarchy_level(), 2);
        assert_eq!(Scope::execution("ex").hierarchy_level(), 3);
        assert_eq!(Scope::action("act").hierarchy_level(), 4);
    }

    #[test]
    fn test_scope_containment_global() {
        let global = Scope::Global;
        let tenant = Scope::tenant("tenant1");
        let workflow = Scope::workflow("wf1");

        assert!(global.contains(&tenant));
        assert!(global.contains(&workflow));
        assert!(!tenant.contains(&global));
        assert!(!workflow.contains(&global));
    }

    #[test]
    fn test_tenant_isolation() {
        let tenant_a = Scope::tenant("A");
        let tenant_b = Scope::tenant("B");

        // Same tenant
        assert!(tenant_a.contains(&Scope::tenant("A")));
        // Different tenant
        assert!(!tenant_a.contains(&tenant_b));

        // Workflow with known parent tenant
        let wf_in_a = Scope::workflow_in_tenant("wf1", "A");
        let wf_in_b = Scope::workflow_in_tenant("wf1", "B");
        assert!(tenant_a.contains(&wf_in_a));
        assert!(!tenant_a.contains(&wf_in_b));

        // Workflow without parent info: deny by default
        let wf_no_parent = Scope::workflow("wf1");
        assert!(!tenant_a.contains(&wf_no_parent));
    }

    #[test]
    fn test_workflow_containment() {
        let wf = Scope::workflow("wf1");

        // Execution with matching workflow parent
        let exec_in_wf1 = Scope::execution_in_workflow("ex1", "wf1", Some("A".to_string()));
        assert!(wf.contains(&exec_in_wf1));

        // Execution with different workflow parent
        let exec_in_wf2 = Scope::execution_in_workflow("ex1", "wf2", Some("A".to_string()));
        assert!(!wf.contains(&exec_in_wf2));

        // Execution without workflow info: deny
        let exec_no_parent = Scope::execution("ex1");
        assert!(!wf.contains(&exec_no_parent));
    }

    #[test]
    fn test_execution_containment() {
        let exec = Scope::execution("ex1");

        // Action with matching execution parent
        let action_in_ex1 = Scope::action_in_execution("a1", "ex1", None, None);
        assert!(exec.contains(&action_in_ex1));

        // Action with different execution parent
        let action_in_ex2 = Scope::action_in_execution("a1", "ex2", None, None);
        assert!(!exec.contains(&action_in_ex2));

        // Action without execution info: deny
        let action_no_parent = Scope::action("a1");
        assert!(!exec.contains(&action_no_parent));
    }

    #[test]
    fn test_scope_keys() {
        assert_eq!(Scope::Global.scope_key(), "global");
        assert_eq!(Scope::tenant("t1").scope_key(), "tenant:t1");
        assert_eq!(Scope::workflow("w1").scope_key(), "workflow:w1");
        assert_eq!(Scope::execution("e1").scope_key(), "execution:e1");
        assert_eq!(Scope::action("a1").scope_key(), "action:a1");

        let custom = Scope::custom("env", "prod");
        assert_eq!(custom.scope_key(), "custom:env=prod");
    }

    #[test]
    fn test_scoping_strategies() {
        let global = Scope::Global;
        let tenant = Scope::tenant("t1");

        assert!(Strategy::Strict.is_compatible(&global, &global));
        assert!(!Strategy::Strict.is_compatible(&global, &tenant));

        assert!(Strategy::Hierarchical.is_compatible(&global, &tenant));
        assert!(!Strategy::Hierarchical.is_compatible(&tenant, &global));

        assert!(Strategy::Fallback.is_compatible(&global, &tenant));
        assert!(Strategy::Fallback.is_compatible(&tenant, &tenant));
    }

    #[test]
    fn test_cross_tenant_denial() {
        // This is the security fix: Tenant A must NOT be able to access Tenant B's resources
        let tenant_a = Scope::tenant("A");

        let wf_in_b = Scope::workflow_in_tenant("wf1", "B");
        let exec_in_b = Scope::execution_in_workflow("ex1", "wf1", Some("B".to_string()));
        let action_in_b =
            Scope::action_in_execution("a1", "ex1", Some("wf1".to_string()), Some("B".to_string()));

        assert!(!tenant_a.contains(&wf_in_b));
        assert!(!tenant_a.contains(&exec_in_b));
        assert!(!tenant_a.contains(&action_in_b));
    }

    #[test]
    fn test_scope_is_hashable() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Scope::Global);
        set.insert(Scope::tenant("A"));
        set.insert(Scope::tenant("A")); // duplicate
        assert_eq!(set.len(), 2);
    }

    #[test]
    #[should_panic(expected = "tenant_id must not be empty")]
    fn test_empty_tenant_id_panics() {
        Scope::tenant("");
    }

    #[test]
    #[should_panic(expected = "workflow_id must not be empty")]
    fn test_empty_workflow_id_panics() {
        Scope::workflow("");
    }

    #[test]
    #[should_panic(expected = "execution_id must not be empty")]
    fn test_empty_execution_id_panics() {
        Scope::execution("");
    }

    #[test]
    #[should_panic(expected = "action_id must not be empty")]
    fn test_empty_action_id_panics() {
        Scope::action("");
    }

    #[test]
    #[should_panic(expected = "custom scope key must not be empty")]
    fn test_empty_custom_key_panics() {
        Scope::custom("", "value");
    }

    #[test]
    #[should_panic(expected = "custom scope value must not be empty")]
    fn test_empty_custom_value_panics() {
        Scope::custom("key", "");
    }
}
