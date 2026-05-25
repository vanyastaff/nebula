//! Type-erased credential operation dispatch keyed by `Credential::KEY`.
//! Mirrors `nebula_engine::credential::StateProjectionRegistry`: a
//! runtime string key drives `Credential::resolve` /
//! `Refreshable::refresh` / `Testable::test` / `Revocable::revoke`
//! without reflection. Capability is encoded by closure *presence*
//! (`is_*` flags here; the operation closures themselves are populated
//! by the service in the `<B, PS>` generic context — see
//! `crate::service`). Structurally impossible to advertise a capability
//! the type lacks: only the capability-bounded `register_*` methods set
//! the corresponding flag.

use std::collections::HashMap;

use nebula_credential::Credential;

/// Registration-time failure (fail-closed on duplicate KEY, Tech Spec §15.6).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DispatchError {
    /// Two registrations shared a `Credential::KEY`. First wins; second
    /// rejected; table unchanged.
    #[error("duplicate credential dispatch key '{key}'")]
    DuplicateKey {
        /// The colliding key.
        key: &'static str,
    },

    /// A capability registrar (`register_testable_ops` /
    /// `register_refreshable_ops` / `register_revocable_ops` /
    /// `register_interactive_ops`) ran before the base ops for `key` were
    /// registered. Capability closures attach onto an existing base
    /// entry, so the base `register_runtime_ops` must run first.
    #[error("base credential ops absent for key '{key}'; register the base ops first")]
    BaseOpsMissing {
        /// The key whose base entry was missing.
        key: &'static str,
    },
}

/// One credential type's capability surface. The actual erased
/// operation closures are attached by the service layer where the
/// store/pending generics are in scope; this table owns the
/// key→capability bookkeeping mirrored from `StateProjectionRegistry`.
#[derive(Debug, Clone, Copy)]
struct DispatchEntry {
    refreshable: bool,
    testable: bool,
    revocable: bool,
}

/// Key → capability surface. Built alongside `register_builtins`.
#[derive(Debug, Default)]
pub struct CredentialDispatch {
    entries: HashMap<&'static str, DispatchEntry>,
}

impl CredentialDispatch {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Register a credential type. Fail-closed on duplicate KEY.
    ///
    /// # Errors
    ///
    /// [`DispatchError::DuplicateKey`] if `C::KEY` is already
    /// registered; the table is left unchanged for the rejected entry.
    pub fn register<C: Credential>(&mut self) -> Result<(), DispatchError> {
        let key: &'static str = C::KEY;
        if self.entries.contains_key(key) {
            return Err(DispatchError::DuplicateKey { key });
        }
        self.entries.insert(
            key,
            DispatchEntry {
                refreshable: false,
                testable: false,
                revocable: false,
            },
        );
        tracing::info!(credential.key = key, "credential dispatch registered");
        Ok(())
    }

    /// Mark the type at `C::KEY` refreshable. Callable only for
    /// `C: Refreshable`, so the flag cannot be set for a type that
    /// lacks the capability (structural, mirrors
    /// `plugin_capability_report`).
    ///
    /// # Errors
    ///
    /// [`DispatchError::DuplicateKey`] is never returned here; returns
    /// `Ok(())` after setting the flag. Unknown key is a no-op.
    pub fn mark_refreshable<C>(&mut self)
    where
        C: nebula_credential::Refreshable,
    {
        if let Some(e) = self.entries.get_mut(<C as Credential>::KEY) {
            e.refreshable = true;
        }
    }

    /// Mark the type at `C::KEY` testable. Callable only for
    /// `C: Testable`.
    pub fn mark_testable<C>(&mut self)
    where
        C: nebula_credential::Testable,
    {
        if let Some(e) = self.entries.get_mut(<C as Credential>::KEY) {
            e.testable = true;
        }
    }

    /// Mark the type at `C::KEY` revocable. Callable only for
    /// `C: Revocable`.
    pub fn mark_revocable<C>(&mut self)
    where
        C: nebula_credential::Revocable,
    {
        if let Some(e) = self.entries.get_mut(<C as Credential>::KEY) {
            e.revocable = true;
        }
    }

    /// Number of registered types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no types are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when `key` is registered.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Iterate registered credential keys (`Credential::KEY` strings).
    ///
    /// Used by the registry-sync invariant probe to assert that
    /// `CredentialDispatch` and `CredentialRegistry` hold the same key set
    /// after plugin init.
    pub fn iter_keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.keys().copied()
    }

    /// Whether the type at `key` is refreshable.
    #[must_use]
    pub fn is_refreshable(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|e| e.refreshable)
    }

    /// Whether the type at `key` is testable.
    #[must_use]
    pub fn is_testable(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|e| e.testable)
    }

    /// Whether the type at `key` is revocable.
    #[must_use]
    pub fn is_revocable(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|e| e.revocable)
    }
}

#[cfg(test)]
mod tests {
    use super::CredentialDispatch;
    use nebula_credential_builtin::BearerTokenCredential;

    #[test]
    fn register_and_lookup() {
        let mut d = CredentialDispatch::new();
        d.register::<BearerTokenCredential>().expect("register ok");
        assert!(d.contains("bearer_token"));
        assert_eq!(d.len(), 1);
        // bearer_token is static -> no capabilities.
        assert!(!d.is_refreshable("bearer_token"));
        assert!(!d.is_testable("bearer_token"));
        assert!(!d.is_revocable("bearer_token"));
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let mut d = CredentialDispatch::new();
        d.register::<BearerTokenCredential>().unwrap();
        let err = d.register::<BearerTokenCredential>().unwrap_err();
        assert!(matches!(err, super::DispatchError::DuplicateKey { .. }));
        assert_eq!(d.len(), 1);
    }
}
