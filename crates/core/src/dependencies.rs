//! Dependency declaration types (spec 23).

use std::any::TypeId;

use crate::{CredentialKey, ResourceKey};

/// Container for declared dependencies.
#[derive(Debug, Default)]
pub struct Dependencies {
    credentials: Vec<CredentialRequirement>,
    resources: Vec<ResourceRequirement>,
}

impl Dependencies {
    /// Create empty dependencies.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a credential requirement.
    pub fn credential(mut self, req: CredentialRequirement) -> Self {
        self.credentials.push(req);
        self
    }

    /// Add a resource requirement.
    pub fn resource(mut self, req: ResourceRequirement) -> Self {
        self.resources.push(req);
        self
    }

    /// Get credential requirements.
    pub fn credentials(&self) -> &[CredentialRequirement] {
        &self.credentials
    }

    /// Get resource requirements.
    pub fn resources(&self) -> &[ResourceRequirement] {
        &self.resources
    }
}

/// Single method dependency declaration trait.
pub trait DeclaresDependencies {
    /// Declare dependencies for this type.
    fn dependencies() -> Dependencies
    where
        Self: Sized,
    {
        Dependencies::new()
    }
}

/// A required or optional credential dependency.
#[derive(Debug, Clone)]
pub struct CredentialRequirement {
    /// The credential key.
    pub key: CredentialKey,
    /// The type ID of the expected credential type.
    pub type_id: TypeId,
    /// The type name (for diagnostics).
    pub type_name: &'static str,
    /// Whether this credential is required.
    pub required: bool,
    /// Purpose description.
    pub purpose: Option<&'static str>,
}

/// A required or optional resource dependency.
#[derive(Debug, Clone)]
pub struct ResourceRequirement {
    /// The resource key.
    pub key: ResourceKey,
    /// The type ID of the expected resource type.
    pub type_id: TypeId,
    /// The type name (for diagnostics).
    pub type_name: &'static str,
    /// Whether this resource is required.
    pub required: bool,
    /// Purpose description.
    pub purpose: Option<&'static str>,
}

/// Errors from dependency validation (registration-time).
///
/// Note: `CoreError::DependencyCycle` / `DependencyMissing` exist for the
/// public API boundary. This error type is used internally by registry
/// validation code. The overlap is intentional — `DependencyError` is
/// converted to `CoreError` at the API boundary via `From` impl.
#[derive(Debug, thiserror::Error)]
pub enum DependencyError {
    /// A required dependency was not registered.
    #[error("missing dependency: `{required_by}` requires `{name}`")]
    Missing {
        /// Name of the missing dependency.
        name: &'static str,
        /// Name of the component that declared the dependency.
        required_by: &'static str,
    },

    /// A cycle was detected in the dependency graph.
    #[error("dependency cycle: {}", path.join(" -> "))]
    Cycle {
        /// Component names participating in the cycle.
        path: Vec<&'static str>,
    },

    /// Invariant in the backing registry was violated.
    #[error("registry invariant violated: {0}")]
    RegistryInvariant(&'static str),
}

impl From<DependencyError> for crate::CoreError {
    fn from(err: DependencyError) -> Self {
        match err {
            DependencyError::Missing { name, required_by } => {
                crate::CoreError::DependencyMissing { name, required_by }
            },
            DependencyError::Cycle { path } => crate::CoreError::DependencyCycle { path },
            DependencyError::RegistryInvariant(msg) => crate::CoreError::DependencyMissing {
                name: msg,
                required_by: "registry",
            },
        }
    }
}
