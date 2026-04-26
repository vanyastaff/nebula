//! Type-erased state-projection registry for runtime credential dispatch.
//!
//! Distinct from the KEY-keyed [`CredentialRegistry`] in
//! `nebula_credential::contract::registry` (Tech Spec §3.1, §15.6),
//! which stores `Box<dyn AnyCredential>` instances keyed by
//! `Credential::KEY`. This engine-side registry maps
//! `<C::State as CredentialState>::KIND` to a deserialization +
//! projection function: given persisted state bytes, produce the
//! type-erased `C::Scheme` an action consumer receives.
//!
//! Both registries fail-closed on duplicate registration per Tech
//! Spec §15.6 (closes security-lead N7); the previous `HashMap::insert`
//! overwrite path is gone in both. They serve distinct dispatch lookups
//! and are populated from the same plugin init phase.
//!
//! [`CredentialRegistry`]: nebula_credential::CredentialRegistry

use std::{collections::HashMap, fmt, sync::Arc};

use nebula_credential::{Credential, CredentialState};

type ProjectFn = Arc<
    dyn Fn(&[u8]) -> Result<Box<dyn std::any::Any + Send + Sync>, StateProjectionError>
        + Send
        + Sync,
>;

/// Type-erased registry mapping `state_kind` to projection handlers.
///
/// Renamed from `CredentialRegistry` per Tech Spec §15.6 — the
/// canonical [`CredentialRegistry`](nebula_credential::CredentialRegistry)
/// now lives on the contract side and stores credential instances keyed
/// by `Credential::KEY`. This engine-side registry is the runtime
/// state-projection dispatcher (deserialize stored bytes → project to
/// `Scheme`), retained alongside the contract registry — both serve
/// distinct lookups during a single resolve.
pub struct StateProjectionRegistry {
    handlers: HashMap<String, ProjectFn>,
}

impl StateProjectionRegistry {
    /// Create an empty state-projection registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a credential type into the registry.
    ///
    /// Returns [`StateProjectionError::DuplicateKind`] if a handler for
    /// the same `<C::State as CredentialState>::KIND` is already
    /// registered. Fail-closed — silent `HashMap::insert` overwrite
    /// would hide namespace collisions including supply-chain plugin
    /// replacement (Tech Spec §15.6, N7 mitigation; active-dev policy:
    /// reject-second-registration).
    ///
    /// # Errors
    ///
    /// Returns [`StateProjectionError::DuplicateKind`] if the
    /// `state_kind` is already registered. Operators resolve the
    /// collision by renaming, namespacing via `#[plugin_credential]`,
    /// or removing the duplicate.
    pub fn register<C>(&mut self) -> Result<(), StateProjectionError>
    where
        C: Credential,
        C::Scheme: 'static,
    {
        let kind = <C::State as CredentialState>::KIND;
        if self.handlers.contains_key(kind) {
            return Err(StateProjectionError::DuplicateKind {
                kind: kind.to_string(),
            });
        }
        tracing::info!(
            credential.kind = %kind,
            "credential state-projection kind registered"
        );
        self.handlers.insert(
            kind.to_string(),
            Arc::new(|bytes: &[u8]| {
                let state: C::State = serde_json::from_slice(bytes)
                    .map_err(|e| StateProjectionError::Deserialize(e.to_string()))?;
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
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, StateProjectionError> {
        let handler = self
            .handlers
            .get(state_kind)
            .ok_or_else(|| StateProjectionError::UnknownKind(state_kind.to_string()))?;
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

impl fmt::Debug for StateProjectionRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateProjectionRegistry")
            .field(
                "registered_kinds",
                &self.handlers.keys().collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl Default for StateProjectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors produced by [`StateProjectionRegistry`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StateProjectionError {
    /// No handler was registered for the requested state kind.
    #[error("unknown state kind: {0}")]
    UnknownKind(String),
    /// Stored bytes failed to deserialize into the expected state type.
    #[error("deserialize failed: {0}")]
    Deserialize(String),
    /// Attempted to register a state kind that was already registered.
    ///
    /// Active-dev policy: reject-second-registration. Silent
    /// `HashMap::insert` overwrite (prior behavior) hid namespace
    /// collisions including supply-chain plugin replacement.
    /// Operators resolve via renaming, `#[plugin_credential]`
    /// namespacing, or removing the duplicate registration.
    #[error(
        "duplicate state kind: {kind} (active-dev policy: reject-second-registration; resolve \
         via rename, #[plugin_credential] namespace, or remove duplicate)"
    )]
    DuplicateKind {
        /// The `<C::State as CredentialState>::KIND` string whose second
        /// registration was rejected. Operator surfaces this value in logs
        /// to identify which credential state-type collision occurred.
        kind: String,
    },
}
