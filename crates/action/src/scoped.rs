//! Scoped credential accessor — enforces type-based access control.
//!
//! Wraps a real [`CredentialAccessor`] and restricts access to only the
//! credential types declared in the action's [`ActionDependencies`](crate::ActionDependencies).

use std::any::TypeId;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_credential::CredentialSnapshot;

use crate::capability::CredentialAccessor;
use crate::error::ActionError;

/// Credential accessor that enforces type-based access scoping.
///
/// Engine wraps the real accessor with this at [`ActionContext`](crate::ActionContext)
/// construction time. The `allowed_types` set is populated from
/// [`ActionDependencies::credential_types()`](crate::ActionDependencies::credential_types).
///
/// Any [`get_by_type()`](CredentialAccessor::get_by_type) call for a `TypeId` not in
/// the set returns [`ActionError::SandboxViolation`].
///
/// # Errors
///
/// [`get_by_type()`](CredentialAccessor::get_by_type) returns
/// [`ActionError::SandboxViolation`] when the requested credential `TypeId`
/// was not declared in the action's
/// [`ActionDependencies::credential_types()`](crate::ActionDependencies::credential_types).
/// String-based [`get()`](CredentialAccessor::get) delegates without type checking
/// and forwards any error from the inner accessor.
///
/// # Examples
///
/// ```rust,ignore
/// use std::any::TypeId;
/// use nebula_action::ScopedCredentialAccessor;
///
/// let scoped = ScopedCredentialAccessor::new(
///     inner_accessor,
///     vec![TypeId::of::<MyCredential>()],
///     "my_action",
/// );
/// // get_by_type for MyCredential → delegates to inner
/// // get_by_type for OtherCredential → SandboxViolation
/// ```
pub struct ScopedCredentialAccessor {
    inner: Arc<dyn CredentialAccessor>,
    allowed_types: HashSet<TypeId>,
    action_id: String,
}

impl ScopedCredentialAccessor {
    /// Create from a real accessor and a list of allowed credential `TypeId`s.
    ///
    /// The `allowed_types` are typically gathered from
    /// [`ActionDependencies::credential_types()`](crate::ActionDependencies::credential_types)
    /// at action registration time.
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
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, ActionError> {
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
    ) -> Result<CredentialSnapshot, ActionError> {
        if !self.allowed_types.contains(&type_id) {
            return Err(ActionError::SandboxViolation {
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
    use nebula_credential::{CredentialMetadata, CredentialSnapshot, SecretToken};

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
        async fn get(&self, id: &str) -> Result<CredentialSnapshot, ActionError> {
            self.by_id
                .get(id)
                .cloned()
                .ok_or_else(|| ActionError::fatal(format!("credential `{id}` not found")))
        }

        async fn has(&self, id: &str) -> bool {
            self.by_id.contains_key(id)
        }

        async fn get_by_type(
            &self,
            type_id: TypeId,
            type_name: &str,
        ) -> Result<CredentialSnapshot, ActionError> {
            self.by_type.get(&type_id).cloned().ok_or_else(|| {
                ActionError::fatal(format!("credential type `{type_name}` not found"))
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
    async fn disallowed_type_returns_sandbox_violation() {
        let allowed_id = TypeId::of::<AllowedCred>();
        let disallowed_id = TypeId::of::<DisallowedCred>();

        let inner = Arc::new(MockAccessor::new());
        let scoped = ScopedCredentialAccessor::new(inner, vec![allowed_id], "test_action");

        let result = scoped.get_by_type(disallowed_id, "DisallowedCred").await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(&err, ActionError::SandboxViolation { capability, .. }
                if capability.contains("DisallowedCred")
            ),
            "expected SandboxViolation, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn string_based_get_delegates_without_type_check() {
        let snapshot = test_snapshot("legacy");

        let inner = Arc::new(MockAccessor::new().with_id("my_cred", snapshot.clone()));
        // No allowed types at all — string access should still work.
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
}
