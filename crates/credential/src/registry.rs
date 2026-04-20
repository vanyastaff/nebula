//! Type-erased credential registry for runtime dispatch.
//!
//! Maps `state_kind` strings to projection functions so the resolver can
//! deserialize and project stored credentials without knowing the concrete
//! [`Credential`](crate::credential::Credential) type at compile time.

use std::{collections::HashMap, fmt, sync::Arc};

use crate::{credential::Credential, state::CredentialState};

/// A function that projects stored bytes into a type-erased
/// [`AuthScheme`](nebula_core::AuthScheme).
///
/// Registered per `state_kind`, called by the resolver during
/// type-erased resolution.
type ProjectFn =
    Arc<dyn Fn(&[u8]) -> Result<Box<dyn std::any::Any + Send + Sync>, RegistryError> + Send + Sync>;

/// Registry of credential type handlers.
///
/// Maps `state_kind` strings to projection functions that deserialize
/// stored bytes and project them to their [`AuthScheme`](nebula_core::AuthScheme).
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::CredentialRegistry;
/// use nebula_credential::credentials::ApiKeyCredential;
///
/// let mut registry = CredentialRegistry::new();
/// registry.register::<ApiKeyCredential>();
///
/// // Later, project stored bytes without knowing the concrete type:
/// let scheme = registry.project("secret_token", &stored_bytes)?;
/// ```
pub struct CredentialRegistry {
    handlers: HashMap<String, ProjectFn>,
}

impl CredentialRegistry {
    /// Creates a new, empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Registers a credential type for runtime resolution.
    ///
    /// Captures the deserialize-then-project pipeline for `C` so that
    /// stored bytes with `state_kind == C::State::KIND` can be projected
    /// to `C::Scheme` without compile-time knowledge of `C`.
    ///
    /// # Panics
    ///
    /// Does not panic. Silently overwrites a previous registration for
    /// the same `state_kind`.
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

    /// Projects stored credential data to its type-erased [`AuthScheme`](nebula_core::AuthScheme).
    ///
    /// The returned `Box<dyn Any>` can be downcast to the concrete scheme
    /// type using [`downcast`](Box::downcast).
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::UnknownKind`] if no handler is registered
    /// for `state_kind`.
    ///
    /// Returns [`RegistryError::Deserialize`] if the stored bytes cannot
    /// be deserialized into the expected state type.
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

    /// Returns `true` if a handler is registered for the given `state_kind`.
    #[must_use]
    pub fn contains(&self, state_kind: &str) -> bool {
        self.handlers.contains_key(state_kind)
    }

    /// Returns the number of registered handlers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Returns `true` if no handlers are registered.
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

/// Error from registry operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryError {
    /// No handler registered for the given `state_kind`.
    #[error("unknown credential kind: {0}")]
    UnknownKind(String),
    /// Failed to deserialize stored bytes.
    #[error("deserialize failed: {0}")]
    Deserialize(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{credentials::ApiKeyCredential, scheme::SecretToken};

    #[test]
    fn register_and_project_secret_token() {
        let mut registry = CredentialRegistry::new();
        registry.register::<ApiKeyCredential>();

        assert!(registry.contains("secret_token"));
        assert_eq!(registry.len(), 1);

        // Construct raw JSON directly because SecretString serializes
        // as "[REDACTED]" — the real store would hold encrypted raw values.
        let data = br#"{"token":"test-key"}"#.to_vec();

        let any = registry.project("secret_token", &data).unwrap();
        let projected = any.downcast::<SecretToken>().unwrap();
        let value = projected.token().expose_secret(ToOwned::to_owned);
        assert_eq!(value, "test-key");
    }

    #[test]
    fn project_unknown_kind_returns_error() {
        let registry = CredentialRegistry::new();
        let result = registry.project("nonexistent", b"{}");
        assert!(matches!(result, Err(RegistryError::UnknownKind(_))));
    }

    #[test]
    fn project_invalid_data_returns_deserialize_error() {
        let mut registry = CredentialRegistry::new();
        registry.register::<ApiKeyCredential>();

        let result = registry.project("secret_token", b"not-json");
        assert!(matches!(result, Err(RegistryError::Deserialize(_))));
    }

    #[test]
    fn empty_registry() {
        let registry = CredentialRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
        assert!(!registry.contains("secret_token"));
    }
}
