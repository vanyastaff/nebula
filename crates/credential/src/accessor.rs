//! Credential accessor implementations.
//!
//! The [`CredentialAccessor`] trait is defined in `nebula_core::accessor` and
//! re-exported here. This module provides concrete implementations:
//!
//! - [`NoopCredentialAccessor`] — stub for contexts without credential support.
//! - [`ScopedCredentialAccessor`] — wraps a real accessor with key-based access control.

use std::{fmt, future::Future, pin::Pin, sync::Arc};

use nebula_core::{CoreError, CredentialKey};

/// Type alias for dyn-safe async return (mirrors core's definition).
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// No-op credential accessor used when runtime does not inject credentials.
#[derive(Debug, Default)]
pub struct NoopCredentialAccessor;

impl nebula_core::accessor::CredentialAccessor for NoopCredentialAccessor {
    fn has(&self, _key: &CredentialKey) -> bool {
        false
    }

    fn resolve_any(
        &self,
        _key: &CredentialKey,
    ) -> BoxFuture<'_, Result<Box<dyn std::any::Any + Send + Sync>, CoreError>> {
        Box::pin(async {
            Err(CoreError::CredentialNotConfigured(
                "credential capability is not configured in context".to_owned(),
            ))
        })
    }

    fn try_resolve_any(
        &self,
        _key: &CredentialKey,
    ) -> BoxFuture<'_, Result<Option<Box<dyn std::any::Any + Send + Sync>>, CoreError>> {
        Box::pin(async { Ok(None) })
    }
}

/// Default credential accessor capability.
#[must_use]
pub fn default_credential_accessor() -> Arc<dyn nebula_core::accessor::CredentialAccessor> {
    Arc::new(NoopCredentialAccessor)
}

/// Credential accessor that enforces key-based access scoping.
///
/// Engine wraps the real accessor with this at context construction time.
/// The `allowed_keys` set is populated from the action's declared
/// credential key dependencies.
///
/// Any [`resolve_any()`] or [`has()`] call for a key not in the set is
/// rejected.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_core::CredentialKey;
/// use nebula_credential::ScopedCredentialAccessor;
///
/// let scoped = ScopedCredentialAccessor::new(
///     inner_accessor,
///     vec![CredentialKey::new("my_api_key").unwrap()],
///     "my_action",
/// );
/// ```
pub struct ScopedCredentialAccessor {
    inner: Arc<dyn nebula_core::accessor::CredentialAccessor>,
    allowed_keys: std::collections::HashSet<String>,
    action_id: String,
}

impl ScopedCredentialAccessor {
    /// Create from a real accessor and a list of allowed credential keys.
    #[must_use]
    pub fn new(
        inner: Arc<dyn nebula_core::accessor::CredentialAccessor>,
        allowed_keys: Vec<CredentialKey>,
        action_id: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            allowed_keys: allowed_keys.iter().map(|k| k.as_str().to_owned()).collect(),
            action_id: action_id.into(),
        }
    }
}

impl fmt::Debug for ScopedCredentialAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedCredentialAccessor")
            .field("allowed_keys_count", &self.allowed_keys.len())
            .finish_non_exhaustive()
    }
}

impl nebula_core::accessor::CredentialAccessor for ScopedCredentialAccessor {
    fn has(&self, key: &CredentialKey) -> bool {
        self.allowed_keys.contains(key.as_str()) && self.inner.has(key)
    }

    fn resolve_any(
        &self,
        key: &CredentialKey,
    ) -> BoxFuture<'_, Result<Box<dyn std::any::Any + Send + Sync>, CoreError>> {
        if !self.allowed_keys.contains(key.as_str()) {
            let capability = format!("credential:{}", key.as_str());
            let action_id = self.action_id.clone();
            return Box::pin(async move {
                Err(CoreError::CredentialAccessDenied {
                    capability,
                    action_id,
                })
            });
        }
        self.inner.resolve_any(key)
    }

    fn try_resolve_any(
        &self,
        key: &CredentialKey,
    ) -> BoxFuture<'_, Result<Option<Box<dyn std::any::Any + Send + Sync>>, CoreError>> {
        if !self.allowed_keys.contains(key.as_str()) {
            return Box::pin(async { Ok(None) });
        }
        self.inner.try_resolve_any(key)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_core::{CoreError, CredentialKey, accessor::CredentialAccessor, credential_key};

    use super::*;

    /// Test accessor that supports key-based access.
    struct MockAccessor {
        available_keys: std::collections::HashSet<String>,
    }

    impl MockAccessor {
        fn new() -> Self {
            Self {
                available_keys: std::collections::HashSet::new(),
            }
        }

        fn with_key(mut self, key: &str) -> Self {
            self.available_keys.insert(key.to_owned());
            self
        }
    }

    impl CredentialAccessor for MockAccessor {
        fn has(&self, key: &CredentialKey) -> bool {
            self.available_keys.contains(key.as_str())
        }

        fn resolve_any(
            &self,
            key: &CredentialKey,
        ) -> BoxFuture<'_, Result<Box<dyn std::any::Any + Send + Sync>, CoreError>> {
            if self.available_keys.contains(key.as_str()) {
                let val: Box<dyn std::any::Any + Send + Sync> =
                    Box::new(format!("resolved:{}", key.as_str()));
                Box::pin(async move { Ok(val) })
            } else {
                let key_str = key.as_str().to_owned();
                Box::pin(async move { Err(CoreError::CredentialNotFound { key: key_str }) })
            }
        }

        fn try_resolve_any(
            &self,
            key: &CredentialKey,
        ) -> BoxFuture<'_, Result<Option<Box<dyn std::any::Any + Send + Sync>>, CoreError>>
        {
            if self.available_keys.contains(key.as_str()) {
                let val: Box<dyn std::any::Any + Send + Sync> =
                    Box::new(format!("resolved:{}", key.as_str()));
                Box::pin(async move { Ok(Some(val)) })
            } else {
                Box::pin(async { Ok(None) })
            }
        }
    }

    #[test]
    fn noop_has_returns_false() {
        let noop = NoopCredentialAccessor;
        assert!(!noop.has(&credential_key!("anything")));
    }

    #[tokio::test]
    async fn noop_resolve_any_returns_not_configured() {
        let noop = NoopCredentialAccessor;
        let result = noop.resolve_any(&credential_key!("anything")).await;
        assert!(
            matches!(result, Err(CoreError::CredentialNotConfigured(_))),
            "expected CredentialNotConfigured, got: {result:?}"
        );
    }

    #[test]
    fn scoped_has_allowed_key() {
        let inner = Arc::new(MockAccessor::new().with_key("my_key"));
        let scoped =
            ScopedCredentialAccessor::new(inner, vec![credential_key!("my_key")], "test_action");
        assert!(scoped.has(&credential_key!("my_key")));
    }

    #[test]
    fn scoped_has_disallowed_key() {
        let inner = Arc::new(MockAccessor::new().with_key("other_key"));
        let scoped =
            ScopedCredentialAccessor::new(inner, vec![credential_key!("my_key")], "test_action");
        assert!(!scoped.has(&credential_key!("other_key")));
    }

    #[tokio::test]
    async fn scoped_resolve_allowed_key() {
        let inner = Arc::new(MockAccessor::new().with_key("my_key"));
        let scoped =
            ScopedCredentialAccessor::new(inner, vec![credential_key!("my_key")], "test_action");
        let result = scoped.resolve_any(&credential_key!("my_key")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn scoped_resolve_disallowed_key_returns_access_denied() {
        let inner = Arc::new(MockAccessor::new());
        let scoped =
            ScopedCredentialAccessor::new(inner, vec![credential_key!("my_key")], "test_action");
        let result = scoped.resolve_any(&credential_key!("other")).await;
        assert!(
            matches!(result, Err(CoreError::CredentialAccessDenied { .. })),
            "expected CredentialAccessDenied, got: {result:?}"
        );
    }

    #[test]
    fn debug_does_not_leak_inner_details() {
        let inner = Arc::new(MockAccessor::new());
        let scoped = ScopedCredentialAccessor::new(inner, vec![], "test_action");
        let debug = format!("{scoped:?}");
        assert!(debug.contains("ScopedCredentialAccessor"));
        assert!(debug.contains("allowed_keys_count"));
    }
}
