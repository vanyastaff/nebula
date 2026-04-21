//! Type-erased credential registry for runtime dispatch.

use std::{collections::HashMap, fmt, sync::Arc};

use nebula_credential::{Credential, CredentialState};

type ProjectFn =
    Arc<dyn Fn(&[u8]) -> Result<Box<dyn std::any::Any + Send + Sync>, RegistryError> + Send + Sync>;

/// Type-erased registry mapping `state_kind` to projection handlers.
pub struct CredentialRegistry {
    handlers: HashMap<String, ProjectFn>,
}

impl CredentialRegistry {
    /// Create an empty credential registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a credential type into the registry.
    pub fn register<C>(&mut self)
    where
        C: Credential,
        C::Scheme: 'static,
    {
        let kind = <C::State as CredentialState>::KIND.to_string();
        self.handlers.insert(
            kind,
            Arc::new(|bytes: &[u8]| {
                let state: C::State = serde_json::from_slice(bytes)
                    .map_err(|e| RegistryError::Deserialize(e.to_string()))?;
                let scheme = C::project(&state);
                Ok(Box::new(scheme) as Box<dyn std::any::Any + Send + Sync>)
            }),
        );
    }

    /// Project serialized state bytes for `state_kind` into a type-erased scheme.
    pub fn project(
        &self,
        state_kind: &str,
        data: &[u8],
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, RegistryError> {
        let handler = self
            .handlers
            .get(state_kind)
            .ok_or_else(|| RegistryError::UnknownKind(state_kind.to_string()))?;
        handler(data)
    }

    /// Returns `true` when a handler for `state_kind` is registered.
    #[must_use]
    pub fn contains(&self, state_kind: &str) -> bool {
        self.handlers.contains_key(state_kind)
    }

    /// Number of registered handlers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Returns `true` when no handlers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl fmt::Debug for CredentialRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialRegistry")
            .field(
                "registered_kinds",
                &self.handlers.keys().collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl Default for CredentialRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors produced by [`CredentialRegistry`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// No handler was registered for the requested state kind.
    #[error("unknown credential kind: {0}")]
    UnknownKind(String),
    /// Stored bytes failed to deserialize into the expected state type.
    #[error("deserialize failed: {0}")]
    Deserialize(String),
}
