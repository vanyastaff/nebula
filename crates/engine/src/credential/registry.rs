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
    ///
    /// Returns [`RegistryError::DuplicateKind`] if a handler for the
    /// same `<C::State as CredentialState>::KIND` is already registered.
    /// Fail-closed — silent `HashMap::insert` overwrite would hide
    /// namespace collisions including supply-chain plugin replacement
    /// (Tech Spec §15.6, N7 mitigation; active-dev policy:
    /// reject-second-registration).
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::DuplicateKind`] if the `state_kind` is
    /// already registered. Operators resolve the collision by renaming,
    /// namespacing via `#[plugin_credential]`, or removing the duplicate.
    pub fn register<C>(&mut self) -> Result<(), RegistryError>
    where
        C: Credential,
        C::Scheme: 'static,
    {
        let kind = <C::State as CredentialState>::KIND;
        if self.handlers.contains_key(kind) {
            return Err(RegistryError::DuplicateKind {
                kind: kind.to_string(),
            });
        }
        tracing::info!(
            credential.kind = %kind,
            "credential kind registered"
        );
        self.handlers.insert(
            kind.to_string(),
            Arc::new(|bytes: &[u8]| {
                let state: C::State = serde_json::from_slice(bytes)
                    .map_err(|e| RegistryError::Deserialize(e.to_string()))?;
                let scheme = C::project(&state);
                Ok(Box::new(scheme) as Box<dyn std::any::Any + Send + Sync>)
            }),
        );
        Ok(())
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
    /// Attempted to register a credential kind that was already registered.
    ///
    /// Active-dev policy: reject-second-registration. Silent
    /// `HashMap::insert` overwrite (prior behavior) hid namespace
    /// collisions including supply-chain plugin replacement.
    /// Operators resolve via renaming, `#[plugin_credential]`
    /// namespacing, or removing the duplicate registration.
    #[error(
        "duplicate credential kind: {kind} (active-dev policy: reject-second-registration; \
         resolve via rename, #[plugin_credential] namespace, or remove duplicate)"
    )]
    DuplicateKind {
        /// The `<C::State as CredentialState>::KIND` string whose second
        /// registration was rejected. Operator surfaces this value in logs
        /// to identify which credential type collision occurred.
        kind: String,
    },
}
