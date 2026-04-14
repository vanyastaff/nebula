use std::fmt;

/// Errors produced while building or validating the dependency graph.
#[derive(Debug)]
pub enum DependencyError {
    /// A required dependency was not registered.
    Missing {
        /// Name of the dependency that is missing.
        name: &'static str,
        /// Name of the component that declared the dependency.
        required_by: &'static str,
    },

    /// A cycle was detected in the dependency graph.
    Cycle {
        /// Component names participating in the cycle, in order.
        path: Vec<&'static str>,
    },

    /// Invariant in the backing registry was violated.
    ///
    /// Indicates a bug in the engine rather than user configuration.
    RegistryInvariant(&'static str),
}

impl fmt::Display for DependencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyError::Missing { name, required_by } => {
                write!(
                    f,
                    "missing dependency: `{required_by}` requires `{name}`, but it is not registered"
                )
            },
            DependencyError::Cycle { path } => {
                write!(f, "dependency cycle detected: {}", path.join(" -> "))
            },
            DependencyError::RegistryInvariant(msg) => {
                write!(f, "registry invariant violated: {msg}")
            },
        }
    }
}

impl std::error::Error for DependencyError {}
