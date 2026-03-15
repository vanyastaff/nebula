//! Resource scoping and access-control model.
//!
//! [`Scope`] expresses the *containment level* at which a resource is registered or
//! requested. The six levels form a hierarchy:
//!
//! ```text
//! Global  ⊇  Tenant  ⊇  Workflow  ⊇  Execution  ⊇  Action
//!                                                     Custom (orthogonal, level 1)
//! ```
//!
//! [`Strategy`] decides whether a pool registered at scope S can serve a caller at
//! scope C:
//!
//! - **`Strict`**: S must equal C exactly.
//! - **`Hierarchical`** (default): S must *contain* C, i.e. S is broader than or
//!   equal to C. A `Global` pool serves every caller; an `Execution`-scoped pool
//!   only serves callers within that specific execution.
//! - **`Fallback`**: exact match first, hierarchical if no exact match exists.
//!
//! Every constructor (`try_tenant`, `try_workflow_in_tenant`, …) is fallible and
//! returns `Err` when any ID string is empty, preventing the most common
//! misconfiguration at the call site.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Defines the scope and visibility of a resource
///
/// Each variant carries optional parent identifiers so that `contains()`
/// can verify the parent chain instead of unconditionally returning true.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
    /// Custom scope with a key-value pair.
    ///
    /// **Note:** Custom scopes are always *isolated* — they only contain
    /// themselves (exact key + value match). The [`Strategy::Hierarchical`]
    /// containment rules do **not** apply to `Custom` scopes because
    /// there is no defined parent-child relationship between arbitrary
    /// key-value pairs. If you need hierarchical scoping, use the
    /// built-in variants ([`Tenant`](Self::Tenant),
    /// [`Workflow`](Self::Workflow), etc.) instead.
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
    /// Try to create a tenant scope.
    pub fn try_tenant<S: Into<String>>(tenant_id: S) -> Result<Self, String> {
        let tenant_id = tenant_id.into();
        if tenant_id.is_empty() {
            return Err("tenant_id must not be empty".to_string());
        }
        Ok(Self::Tenant { tenant_id })
    }

    /// Try to create a workflow scope without parent info.
    pub fn try_workflow<S: Into<String>>(workflow_id: S) -> Result<Self, String> {
        let workflow_id = workflow_id.into();
        if workflow_id.is_empty() {
            return Err("workflow_id must not be empty".to_string());
        }
        Ok(Self::Workflow {
            workflow_id,
            tenant_id: None,
        })
    }

    /// Try to create a workflow scope with tenant parent.
    pub fn try_workflow_in_tenant(
        workflow_id: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> Result<Self, String> {
        let workflow_id = workflow_id.into();
        let tenant_id = tenant_id.into();
        if workflow_id.is_empty() {
            return Err("workflow_id must not be empty".to_string());
        }
        if tenant_id.is_empty() {
            return Err("tenant_id must not be empty".to_string());
        }
        Ok(Self::Workflow {
            workflow_id,
            tenant_id: Some(tenant_id),
        })
    }

    /// Try to create an execution scope without parent info.
    pub fn try_execution<S: Into<String>>(execution_id: S) -> Result<Self, String> {
        let execution_id = execution_id.into();
        if execution_id.is_empty() {
            return Err("execution_id must not be empty".to_string());
        }
        Ok(Self::Execution {
            execution_id,
            workflow_id: None,
            tenant_id: None,
        })
    }

    /// Try to create an execution scope with full parent chain.
    pub fn try_execution_in_workflow(
        execution_id: impl Into<String>,
        workflow_id: impl Into<String>,
        tenant_id: Option<String>,
    ) -> Result<Self, String> {
        let execution_id = execution_id.into();
        let workflow_id = workflow_id.into();
        if execution_id.is_empty() {
            return Err("execution_id must not be empty".to_string());
        }
        if workflow_id.is_empty() {
            return Err("workflow_id must not be empty".to_string());
        }
        Ok(Self::Execution {
            execution_id,
            workflow_id: Some(workflow_id),
            tenant_id,
        })
    }

    /// Try to create an action scope without parent info.
    pub fn try_action<S: Into<String>>(action_id: S) -> Result<Self, String> {
        let action_id = action_id.into();
        if action_id.is_empty() {
            return Err("action_id must not be empty".to_string());
        }
        Ok(Self::Action {
            action_id,
            execution_id: None,
            workflow_id: None,
            tenant_id: None,
        })
    }

    /// Try to create an action scope with full parent chain.
    pub fn try_action_in_execution(
        action_id: impl Into<String>,
        execution_id: impl Into<String>,
        workflow_id: Option<String>,
        tenant_id: Option<String>,
    ) -> Result<Self, String> {
        let action_id = action_id.into();
        let execution_id = execution_id.into();
        if action_id.is_empty() {
            return Err("action_id must not be empty".to_string());
        }
        if execution_id.is_empty() {
            return Err("execution_id must not be empty".to_string());
        }
        Ok(Self::Action {
            action_id,
            execution_id: Some(execution_id),
            workflow_id,
            tenant_id,
        })
    }

    /// Try to create a custom scope.
    pub fn try_custom(key: impl Into<String>, value: impl Into<String>) -> Result<Self, String> {
        let key = key.into();
        let value = value.into();
        if key.is_empty() {
            return Err("custom scope key must not be empty".to_string());
        }
        if value.is_empty() {
            return Err("custom scope value must not be empty".to_string());
        }
        Ok(Self::Custom { key, value })
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

            // Custom scopes only contain themselves (always isolated,
            // no hierarchical containment — see `Scope::Custom` docs).
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
    pub fn description(&self) -> Cow<'static, str> {
        match self {
            Self::Global => "Global scope (shared across all workflows and tenants)".into(),
            Self::Tenant { tenant_id } => {
                format!("Tenant scope (tenant: {tenant_id})").into()
            }
            Self::Workflow { workflow_id, .. } => {
                format!("Workflow scope (workflow: {workflow_id})").into()
            }
            Self::Execution { execution_id, .. } => {
                format!("Execution scope (execution: {execution_id})").into()
            }
            Self::Action { action_id, .. } => {
                format!("Action scope (action: {action_id})").into()
            }
            Self::Custom { key, value } => format!("Custom scope ({key}={value})").into(),
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.scope_key())
    }
}

/// Scoping strategy for resource allocation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
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

    fn tenant(id: impl Into<String>) -> Scope {
        Scope::try_tenant(id).expect("valid tenant scope")
    }

    fn workflow(id: impl Into<String>) -> Scope {
        Scope::try_workflow(id).expect("valid workflow scope")
    }

    fn workflow_in_tenant(workflow_id: impl Into<String>, tenant_id: impl Into<String>) -> Scope {
        Scope::try_workflow_in_tenant(workflow_id, tenant_id)
            .expect("valid workflow scope with tenant")
    }

    fn execution(id: impl Into<String>) -> Scope {
        Scope::try_execution(id).expect("valid execution scope")
    }

    fn execution_in_workflow(
        execution_id: impl Into<String>,
        workflow_id: impl Into<String>,
        tenant_id: Option<String>,
    ) -> Scope {
        Scope::try_execution_in_workflow(execution_id, workflow_id, tenant_id)
            .expect("valid execution scope with workflow")
    }

    fn action(id: impl Into<String>) -> Scope {
        Scope::try_action(id).expect("valid action scope")
    }

    fn action_in_execution(
        action_id: impl Into<String>,
        execution_id: impl Into<String>,
        workflow_id: Option<String>,
        tenant_id: Option<String>,
    ) -> Scope {
        Scope::try_action_in_execution(action_id, execution_id, workflow_id, tenant_id)
            .expect("valid action scope with execution")
    }

    fn custom(key: impl Into<String>, value: impl Into<String>) -> Scope {
        Scope::try_custom(key, value).expect("valid custom scope")
    }

    #[test]
    fn test_scope_hierarchy_levels() {
        assert_eq!(Scope::Global.hierarchy_level(), 0);
        assert_eq!(tenant("test").hierarchy_level(), 1);
        assert_eq!(workflow("wf").hierarchy_level(), 2);
        assert_eq!(execution("ex").hierarchy_level(), 3);
        assert_eq!(action("act").hierarchy_level(), 4);
    }

    #[test]
    fn test_scope_containment_global() {
        let global = Scope::Global;
        let tenant = tenant("tenant1");
        let workflow = workflow("wf1");

        assert!(global.contains(&tenant));
        assert!(global.contains(&workflow));
        assert!(!tenant.contains(&global));
        assert!(!workflow.contains(&global));
    }

    #[test]
    fn test_tenant_isolation() {
        let tenant_a = tenant("A");
        let tenant_b = tenant("B");

        // Same tenant
        assert!(tenant_a.contains(&tenant("A")));
        // Different tenant
        assert!(!tenant_a.contains(&tenant_b));

        // Workflow with known parent tenant
        let wf_in_a = workflow_in_tenant("wf1", "A");
        let wf_in_b = workflow_in_tenant("wf1", "B");
        assert!(tenant_a.contains(&wf_in_a));
        assert!(!tenant_a.contains(&wf_in_b));

        // Workflow without parent info: deny by default
        let wf_no_parent = workflow("wf1");
        assert!(!tenant_a.contains(&wf_no_parent));
    }

    #[test]
    fn test_workflow_containment() {
        let wf = workflow("wf1");

        // Execution with matching workflow parent
        let exec_in_wf1 = execution_in_workflow("ex1", "wf1", Some("A".to_string()));
        assert!(wf.contains(&exec_in_wf1));

        // Execution with different workflow parent
        let exec_in_wf2 = execution_in_workflow("ex1", "wf2", Some("A".to_string()));
        assert!(!wf.contains(&exec_in_wf2));

        // Execution without workflow info: deny
        let exec_no_parent = execution("ex1");
        assert!(!wf.contains(&exec_no_parent));
    }

    #[test]
    fn test_execution_containment() {
        let exec = execution("ex1");

        // Action with matching execution parent
        let action_in_ex1 = action_in_execution("a1", "ex1", None, None);
        assert!(exec.contains(&action_in_ex1));

        // Action with different execution parent
        let action_in_ex2 = action_in_execution("a1", "ex2", None, None);
        assert!(!exec.contains(&action_in_ex2));

        // Action without execution info: deny
        let action_no_parent = action("a1");
        assert!(!exec.contains(&action_no_parent));
    }

    #[test]
    fn test_scope_keys() {
        assert_eq!(Scope::Global.scope_key(), "global");
        assert_eq!(tenant("t1").scope_key(), "tenant:t1");
        assert_eq!(workflow("w1").scope_key(), "workflow:w1");
        assert_eq!(execution("e1").scope_key(), "execution:e1");
        assert_eq!(action("a1").scope_key(), "action:a1");

        let custom = custom("env", "prod");
        assert_eq!(custom.scope_key(), "custom:env=prod");
    }

    #[test]
    fn test_scoping_strategies() {
        let global = Scope::Global;
        let tenant = tenant("t1");

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
        let tenant_a = tenant("A");

        let wf_in_b = workflow_in_tenant("wf1", "B");
        let exec_in_b = execution_in_workflow("ex1", "wf1", Some("B".to_string()));
        let action_in_b =
            action_in_execution("a1", "ex1", Some("wf1".to_string()), Some("B".to_string()));

        assert!(!tenant_a.contains(&wf_in_b));
        assert!(!tenant_a.contains(&exec_in_b));
        assert!(!tenant_a.contains(&action_in_b));
    }

    #[test]
    fn test_scope_is_hashable() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Scope::Global);
        set.insert(tenant("A"));
        set.insert(tenant("A")); // duplicate
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_empty_tenant_id_errors() {
        let err = Scope::try_tenant("").expect_err("empty tenant id must fail");
        assert_eq!(err, "tenant_id must not be empty");
    }

    #[test]
    fn test_empty_workflow_id_errors() {
        let err = Scope::try_workflow("").expect_err("empty workflow id must fail");
        assert_eq!(err, "workflow_id must not be empty");
    }

    #[test]
    fn test_empty_execution_id_errors() {
        let err = Scope::try_execution("").expect_err("empty execution id must fail");
        assert_eq!(err, "execution_id must not be empty");
    }

    #[test]
    fn test_empty_action_id_errors() {
        let err = Scope::try_action("").expect_err("empty action id must fail");
        assert_eq!(err, "action_id must not be empty");
    }

    #[test]
    fn test_empty_custom_key_errors() {
        let err = Scope::try_custom("", "value").expect_err("empty custom key must fail");
        assert_eq!(err, "custom scope key must not be empty");
    }

    #[test]
    fn test_empty_custom_value_errors() {
        let err = Scope::try_custom("key", "").expect_err("empty custom value must fail");
        assert_eq!(err, "custom scope value must not be empty");
    }
}
