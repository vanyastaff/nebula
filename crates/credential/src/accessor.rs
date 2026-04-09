//! Credential accessor traits and implementations.
//!
//! [`CredentialAccessor`] is the object-safe capability trait injected into
//! action/trigger contexts. [`ScopedCredentialAccessor`] wraps a real accessor
//! with type-based access control. [`NoopCredentialAccessor`] is a stub for
//! contexts without credential support.

use std::any::TypeId;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;

use crate::CredentialSnapshot;
use crate::access_error::CredentialAccessError;

/// Object-safe credential accessor injected into action/trigger contexts.
///
/// Implementations are provided by the engine/runtime. Actions receive this
/// trait object via [`Arc<dyn CredentialAccessor>`].
#[async_trait]
pub trait CredentialAccessor: Send + Sync {
    /// Retrieve a credential snapshot by id.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialAccessError::NotFound`] if the credential does not
    /// exist, or [`CredentialAccessError::NotConfigured`] if the accessor is
    /// a no-op stub.
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, CredentialAccessError>;

    /// Check whether a credential exists for the given id.
    async fn has(&self, id: &str) -> bool;

    /// Retrieve a credential snapshot by [`TypeId`] of the
    /// [`AuthScheme`](nebula_core::AuthScheme).
    ///
    /// Used by type-based credential access: `ctx.credential_by_type::<S>().await`.
    /// Default: returns error (implementations that support type-based access
    /// override this).
    async fn get_by_type(
        &self,
        type_id: TypeId,
        type_name: &str,
    ) -> Result<CredentialSnapshot, CredentialAccessError> {
        let _ = type_id;
        Err(CredentialAccessError::NotConfigured(format!(
            "type-based credential access not supported for `{type_name}`"
        )))
    }
}

/// No-op credential accessor used when runtime does not inject credentials.
#[derive(Debug, Default)]
pub struct NoopCredentialAccessor;

#[async_trait]
impl CredentialAccessor for NoopCredentialAccessor {
    async fn get(&self, _id: &str) -> Result<CredentialSnapshot, CredentialAccessError> {
        Err(CredentialAccessError::NotConfigured(
            "credential capability is not configured in context".to_owned(),
        ))
    }

    async fn has(&self, _id: &str) -> bool {
        false
    }
}

/// Default credential accessor capability.
#[must_use]
pub fn default_credential_accessor() -> Arc<dyn CredentialAccessor> {
    Arc::new(NoopCredentialAccessor)
}

/// Credential accessor that enforces type-based access scoping.
///
/// Engine wraps the real accessor with this at context construction time.
/// The `allowed_types` set is populated from the action's declared
/// credential type dependencies.
///
/// Any [`get_by_type()`](CredentialAccessor::get_by_type) call for a `TypeId`
/// not in the set returns [`CredentialAccessError::AccessDenied`].
///
/// # Errors
///
/// [`get_by_type()`](CredentialAccessor::get_by_type) returns
/// [`CredentialAccessError::AccessDenied`] when the requested credential
/// `TypeId` was not declared in the action's credential type dependencies.
/// String-based [`get()`](CredentialAccessor::get) delegates without type
/// checking and forwards any error from the inner accessor.
///
/// # Examples
///
/// ```rust,ignore
/// use std::any::TypeId;
/// use nebula_credential::ScopedCredentialAccessor;
///
/// let scoped = ScopedCredentialAccessor::new(
///     inner_accessor,
///     vec![TypeId::of::<MyCredential>()],
///     "my_action",
/// );
/// // get_by_type for MyCredential -> delegates to inner
/// // get_by_type for OtherCredential -> AccessDenied
/// ```
pub struct ScopedCredentialAccessor {
    inner: Arc<dyn CredentialAccessor>,
    allowed_types: HashSet<TypeId>,
    action_id: String,
}

impl ScopedCredentialAccessor {
    /// Create from a real accessor and a list of allowed credential `TypeId`s.
    #[must_use]
    pub fn new(
        inner: Arc<dyn CredentialAccessor>,
        allowed_types: Vec<TypeId>,
        action_id: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            allowed_types: allowed_types.into_iter().collect(),
            action_id: action_id.into(),
        }
    }
}

impl fmt::Debug for ScopedCredentialAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopedCredentialAccessor")
            .field("allowed_types_count", &self.allowed_types.len())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CredentialAccessor for ScopedCredentialAccessor {
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, CredentialAccessError> {
        // String-based access delegates without type check (legacy path).
        self.inner.get(id).await
    }

    async fn has(&self, id: &str) -> bool {
        self.inner.has(id).await
    }

    async fn get_by_type(
        &self,
        type_id: TypeId,
        type_name: &str,
    ) -> Result<CredentialSnapshot, CredentialAccessError> {
        if !self.allowed_types.contains(&type_id) {
            return Err(CredentialAccessError::AccessDenied {
                capability: format!("credential type `{type_name}`"),
                action_id: self.action_id.clone(),
            });
        }
        self.inner.get_by_type(type_id, type_name).await
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use nebula_core::SecretString;

    use crate::{CredentialMetadata, CredentialSnapshot, SecretToken};

    use super::*;

    /// Test accessor that supports both string-based and type-based access.
    struct MockAccessor {
        by_id: HashMap<String, CredentialSnapshot>,
        by_type: HashMap<TypeId, CredentialSnapshot>,
    }

    impl MockAccessor {
        fn new() -> Self {
            Self {
                by_id: HashMap::new(),
                by_type: HashMap::new(),
            }
        }

        fn with_id(mut self, id: &str, snapshot: CredentialSnapshot) -> Self {
            self.by_id.insert(id.to_owned(), snapshot);
            self
        }

        fn with_type(mut self, type_id: TypeId, snapshot: CredentialSnapshot) -> Self {
            self.by_type.insert(type_id, snapshot);
            self
        }
    }

    #[async_trait]
    impl CredentialAccessor for MockAccessor {
        async fn get(&self, id: &str) -> Result<CredentialSnapshot, CredentialAccessError> {
            self.by_id.get(id).cloned().ok_or_else(|| {
                CredentialAccessError::NotFound(format!("credential `{id}` not found"))
            })
        }

        async fn has(&self, id: &str) -> bool {
            self.by_id.contains_key(id)
        }

        async fn get_by_type(
            &self,
            type_id: TypeId,
            type_name: &str,
        ) -> Result<CredentialSnapshot, CredentialAccessError> {
            self.by_type.get(&type_id).cloned().ok_or_else(|| {
                CredentialAccessError::NotFound(format!("credential type `{type_name}` not found"))
            })
        }
    }

    fn test_snapshot(name: &str) -> CredentialSnapshot {
        CredentialSnapshot::new(
            name,
            CredentialMetadata::new(),
            SecretToken::new(SecretString::new("test-value")),
        )
    }

    // Dummy types for TypeId discrimination.
    struct AllowedCred;
    struct DisallowedCred;

    #[tokio::test]
    async fn allowed_type_passes_through() {
        let allowed_id = TypeId::of::<AllowedCred>();
        let snapshot = test_snapshot("allowed");

        let inner = Arc::new(MockAccessor::new().with_type(allowed_id, snapshot.clone()));
        let scoped = ScopedCredentialAccessor::new(inner, vec![allowed_id], "test_action");

        let result = scoped.get_by_type(allowed_id, "AllowedCred").await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().scheme_pattern(), snapshot.scheme_pattern());
    }

    #[tokio::test]
    async fn disallowed_type_returns_access_denied() {
        let allowed_id = TypeId::of::<AllowedCred>();
        let disallowed_id = TypeId::of::<DisallowedCred>();

        let inner = Arc::new(MockAccessor::new());
        let scoped = ScopedCredentialAccessor::new(inner, vec![allowed_id], "test_action");

        let result = scoped.get_by_type(disallowed_id, "DisallowedCred").await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(
                &err,
                CredentialAccessError::AccessDenied { capability, .. }
                    if capability.contains("DisallowedCred")
            ),
            "expected AccessDenied, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn string_based_get_delegates_without_type_check() {
        let snapshot = test_snapshot("legacy");

        let inner = Arc::new(MockAccessor::new().with_id("my_cred", snapshot.clone()));
        // No allowed types at all - string access should still work.
        let scoped = ScopedCredentialAccessor::new(inner, vec![], "test_action");

        let result = scoped.get("my_cred").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().scheme_pattern(), snapshot.scheme_pattern());
    }

    #[tokio::test]
    async fn has_delegates_to_inner() {
        let inner = Arc::new(MockAccessor::new().with_id("exists", test_snapshot("exists")));
        let scoped = ScopedCredentialAccessor::new(inner, vec![], "test_action");

        assert!(scoped.has("exists").await);
        assert!(!scoped.has("missing").await);
    }

    #[test]
    fn debug_does_not_leak_inner_details() {
        let inner = Arc::new(MockAccessor::new());
        let scoped = ScopedCredentialAccessor::new(inner, vec![], "test_action");

        let debug = format!("{scoped:?}");
        assert!(debug.contains("ScopedCredentialAccessor"));
        assert!(debug.contains("allowed_types_count"));
    }

    #[tokio::test]
    async fn noop_accessor_returns_not_configured() {
        let noop = NoopCredentialAccessor;
        let result = noop.get("anything").await;
        assert!(
            matches!(result, Err(CredentialAccessError::NotConfigured(_))),
            "expected NotConfigured, got: {result:?}"
        );
        assert!(!noop.has("anything").await);
    }
}
