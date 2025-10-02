//! Resource scoping and visibility management

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Defines the scope and visibility of a resource
///
/// Note: Hash is not derived because the Custom variant contains HashMap
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ResourceScope {
    /// Global scope - shared across all workflows and tenants
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
    },
    /// Execution scope - scoped to a specific workflow execution
    Execution {
        /// The execution identifier
        execution_id: String,
    },
    /// Action scope - scoped to a specific action within a workflow
    Action {
        /// The action identifier
        action_id: String,
    },
    /// Custom scope with arbitrary key-value pairs
    Custom {
        /// The scope name
        name: String,
        /// Custom scope attributes
        attributes: std::collections::HashMap<String, String>,
    },
}

impl ResourceScope {
    /// Create a tenant scope
    pub fn tenant<S: Into<String>>(tenant_id: S) -> Self {
        Self::Tenant {
            tenant_id: tenant_id.into(),
        }
    }

    /// Create a workflow scope
    pub fn workflow<S: Into<String>>(workflow_id: S) -> Self {
        Self::Workflow {
            workflow_id: workflow_id.into(),
        }
    }

    /// Create an execution scope
    pub fn execution<S: Into<String>>(execution_id: S) -> Self {
        Self::Execution {
            execution_id: execution_id.into(),
        }
    }

    /// Create an action scope
    pub fn action<S: Into<String>>(action_id: S) -> Self {
        Self::Action {
            action_id: action_id.into(),
        }
    }

    /// Create a custom scope
    pub fn custom<S: Into<String>>(
        name: S,
        attributes: std::collections::HashMap<String, String>,
    ) -> Self {
        Self::Custom {
            name: name.into(),
            attributes,
        }
    }

    /// Get the scope hierarchy level (lower numbers = broader scope)
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
    pub fn is_broader_than(&self, other: &ResourceScope) -> bool {
        self.hierarchy_level() < other.hierarchy_level()
    }

    /// Check if this scope is narrower than another scope
    pub fn is_narrower_than(&self, other: &ResourceScope) -> bool {
        self.hierarchy_level() > other.hierarchy_level()
    }

    /// Check if this scope contains another scope
    pub fn contains(&self, other: &ResourceScope) -> bool {
        match (self, other) {
            // Global contains everything
            (Self::Global, _) => true,

            // Tenant contains workflow, execution, and action in same tenant
            (Self::Tenant { tenant_id: t1 }, Self::Workflow { .. })
            | (Self::Tenant { tenant_id: t1 }, Self::Execution { .. })
            | (Self::Tenant { tenant_id: t1 }, Self::Action { .. }) => {
                // Note: This is simplified - in reality we'd need context to check tenant ownership
                true
            }
            (Self::Tenant { tenant_id: t1 }, Self::Tenant { tenant_id: t2 }) => t1 == t2,

            // Workflow contains execution and action in same workflow
            (Self::Workflow { workflow_id: w1 }, Self::Execution { .. })
            | (Self::Workflow { workflow_id: w1 }, Self::Action { .. }) => {
                // Note: This is simplified - in reality we'd need context to check workflow ownership
                true
            }
            (Self::Workflow { workflow_id: w1 }, Self::Workflow { workflow_id: w2 }) => w1 == w2,

            // Execution contains action in same execution
            (Self::Execution { execution_id: e1 }, Self::Action { .. }) => {
                // Note: This is simplified - in reality we'd need context to check execution ownership
                true
            }
            (Self::Execution { execution_id: e1 }, Self::Execution { execution_id: e2 }) => {
                e1 == e2
            }

            // Action only contains itself
            (Self::Action { action_id: a1 }, Self::Action { action_id: a2 }) => a1 == a2,

            // Custom scopes only contain themselves
            (
                Self::Custom {
                    name: n1,
                    attributes: a1,
                },
                Self::Custom {
                    name: n2,
                    attributes: a2,
                },
            ) => n1 == n2 && a1 == a2,

            // All other combinations
            _ => false,
        }
    }

    /// Generate a scope key for storage/lookup
    pub fn scope_key(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Tenant { tenant_id } => format!("tenant:{}", tenant_id),
            Self::Workflow { workflow_id } => format!("workflow:{}", workflow_id),
            Self::Execution { execution_id } => format!("execution:{}", execution_id),
            Self::Action { action_id } => format!("action:{}", action_id),
            Self::Custom { name, attributes } => {
                let mut key = format!("custom:{}", name);
                for (k, v) in attributes {
                    key.push_str(&format!(":{}={}", k, v));
                }
                key
            }
        }
    }

    /// Get a human-readable description of the scope
    pub fn description(&self) -> String {
        match self {
            Self::Global => "Global scope (shared across all workflows and tenants)".to_string(),
            Self::Tenant { tenant_id } => format!("Tenant scope (tenant: {})", tenant_id),
            Self::Workflow { workflow_id } => format!("Workflow scope (workflow: {})", workflow_id),
            Self::Execution { execution_id } => {
                format!("Execution scope (execution: {})", execution_id)
            }
            Self::Action { action_id } => format!("Action scope (action: {})", action_id),
            Self::Custom { name, attributes } => {
                format!(
                    "Custom scope '{}' with {} attributes",
                    name,
                    attributes.len()
                )
            }
        }
    }
}

impl fmt::Display for ResourceScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.scope_key())
    }
}

impl Default for ResourceScope {
    fn default() -> Self {
        Self::Global
    }
}

/// Scoping strategy for resource allocation
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ScopingStrategy {
    /// Strict scoping - only exact scope matches
    Strict,
    /// Hierarchical scoping - allows broader scopes to be used
    Hierarchical,
    /// Fallback scoping - tries exact match, then falls back to broader scopes
    Fallback,
}

impl ScopingStrategy {
    /// Check if a resource scope is compatible with a requested scope using this strategy
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

impl Default for ScopingStrategy {
    fn default() -> Self {
        Self::Hierarchical
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_scope_hierarchy_levels() {
        assert_eq!(ResourceScope::Global.hierarchy_level(), 0);
        assert_eq!(ResourceScope::tenant("test").hierarchy_level(), 1);
        assert_eq!(ResourceScope::workflow("wf").hierarchy_level(), 2);
        assert_eq!(ResourceScope::execution("ex").hierarchy_level(), 3);
        assert_eq!(ResourceScope::action("act").hierarchy_level(), 4);
    }

    #[test]
    fn test_scope_containment() {
        let global = ResourceScope::Global;
        let tenant = ResourceScope::tenant("tenant1");
        let workflow = ResourceScope::workflow("wf1");

        assert!(global.contains(&tenant));
        assert!(global.contains(&workflow));
        assert!(!tenant.contains(&global));
        assert!(!workflow.contains(&global));
    }

    #[test]
    fn test_scope_keys() {
        assert_eq!(ResourceScope::Global.scope_key(), "global");
        assert_eq!(ResourceScope::tenant("t1").scope_key(), "tenant:t1");
        assert_eq!(ResourceScope::workflow("w1").scope_key(), "workflow:w1");

        let mut attrs = HashMap::new();
        attrs.insert("env".to_string(), "prod".to_string());
        let custom = ResourceScope::custom("test", attrs);
        assert!(custom.scope_key().starts_with("custom:test:"));
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
}
