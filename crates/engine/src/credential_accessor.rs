//! Engine-side [`CredentialAccessor`] implementation.
//!
//! [`EngineCredentialAccessor`] bridges the engine's credential resolution
//! infrastructure to the [`CredentialAccessor`] capability trait consumed by
//! actions. It enforces an allowlist of declared credential keys so that
//! actions can only access credentials they have explicitly declared as
//! dependencies.
//!
//! # Design
//!
//! [`CredentialResolver<S>`](nebula_credential::CredentialResolver) is generic
//! over a store type, which would infect the engine with an unbounded type
//! parameter. Instead, the resolution function is captured once as a
//! type-erased `Arc<dyn Fn + Send + Sync>` returning a pinned future. This
//! keeps `WorkflowEngine` concrete while still delegating to any store
//! implementation.
//!
//! # Allowlist semantics
//!
//! - An **empty** allowlist means **no credentials are permitted**. If an
//!   action has no declared credential dependencies, no credentials can be
//!   fetched.
//! - A **non-empty** allowlist only permits the keys in the set. Requests for
//!   undeclared keys are rejected with [`ActionError::SandboxViolation`].

use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_action::capability::CredentialAccessor;
use nebula_action::error::ActionError;
use nebula_credential::CredentialSnapshot;

/// Type alias for the boxed async credential-resolution function.
type ResolveFn = Arc<
    dyn Fn(&str) -> Pin<Box<dyn Future<Output = Result<CredentialSnapshot, ActionError>> + Send>>
        + Send
        + Sync,
>;

/// Engine-side implementation of [`CredentialAccessor`].
///
/// Validates that the requested credential key is in the declared allowlist
/// before delegating resolution to the underlying resolver function.
///
/// # Examples
///
/// ```rust,ignore
/// use std::collections::HashSet;
/// use std::sync::Arc;
/// use nebula_engine::credential_accessor::EngineCredentialAccessor;
/// use nebula_credential::{CredentialResolver, InMemoryStore};
///
/// let store = Arc::new(InMemoryStore::new());
/// let resolver = Arc::new(CredentialResolver::new(store));
///
/// let allowed = HashSet::from(["github_token".to_string()]);
/// let accessor = EngineCredentialAccessor::new(allowed, {
///     let resolver = Arc::clone(&resolver);
///     move |id: &str| {
///         let resolver = Arc::clone(&resolver);
///         let id = id.to_owned();
///         Box::pin(async move {
///             resolver.resolve_snapshot(&id).await
///                 .map_err(|e| ActionError::fatal(e.to_string()))
///         })
///     }
/// });
/// ```
pub struct EngineCredentialAccessor {
    /// Set of credential keys this accessor is permitted to resolve.
    ///
    /// An empty set means no credentials are accessible.
    allowed_keys: HashSet<String>,
    /// Type-erased async resolution function.
    resolve_fn: ResolveFn,
}

impl EngineCredentialAccessor {
    /// Creates a new accessor with the given allowlist and resolution function.
    ///
    /// # Parameters
    ///
    /// - `allowed_keys` — the set of credential IDs this accessor may resolve.
    ///   Pass an empty set to deny all credential access.
    /// - `resolve_fn` — async closure that resolves a credential ID to a
    ///   [`CredentialSnapshot`] or an [`ActionError`].
    pub fn new<F, Fut>(allowed_keys: HashSet<String>, resolve_fn: F) -> Self
    where
        F: Fn(&str) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<CredentialSnapshot, ActionError>> + Send + 'static,
    {
        Self {
            allowed_keys,
            resolve_fn: Arc::new(move |id: &str| {
                Box::pin(resolve_fn(id))
                    as Pin<Box<dyn Future<Output = Result<CredentialSnapshot, ActionError>> + Send>>
            }),
        }
    }

    /// Returns `true` if `id` is in the allowed set.
    fn is_allowed(&self, id: &str) -> bool {
        self.allowed_keys.contains(id)
    }
}

impl fmt::Debug for EngineCredentialAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineCredentialAccessor")
            .field("allowed_keys", &self.allowed_keys)
            .field("resolve_fn", &"<fn>")
            .finish()
    }
}

#[async_trait]
impl CredentialAccessor for EngineCredentialAccessor {
    /// Retrieve a credential snapshot by id.
    ///
    /// # Errors
    ///
    /// - [`ActionError::SandboxViolation`] — if `id` is not in the allowlist.
    /// - Any error returned by the underlying resolver function.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel-safe. If the future is dropped before completion,
    /// no state is modified.
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, ActionError> {
        if !self.is_allowed(id) {
            return Err(ActionError::SandboxViolation {
                capability: format!("credential:{id}"),
                action_id: String::new(),
            });
        }
        (self.resolve_fn)(id).await
    }

    /// Check whether a credential key is in the allowed set.
    ///
    /// Returns `true` only if `id` was declared as an allowed credential key.
    /// This does **not** verify that the credential exists in the store.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel-safe — it performs no async I/O.
    async fn has(&self, id: &str) -> bool {
        self.is_allowed(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an accessor with the given allowed keys and a stub resolver.
    ///
    /// The stub resolver always succeeds if called (it returns a test snapshot
    /// via `nebula_credential::CredentialSnapshot::new`).
    fn make_accessor(allowed: impl IntoIterator<Item = &'static str>) -> EngineCredentialAccessor {
        let allowed_keys: HashSet<String> = allowed.into_iter().map(str::to_owned).collect();
        EngineCredentialAccessor::new(allowed_keys, |_id: &str| async {
            Err(ActionError::fatal("not implemented in stub"))
        })
    }

    /// Builds an accessor whose resolver always returns the given error.
    fn make_failing_accessor(
        allowed: impl IntoIterator<Item = &'static str>,
        err: ActionError,
    ) -> EngineCredentialAccessor {
        let allowed_keys: HashSet<String> = allowed.into_iter().map(str::to_owned).collect();
        EngineCredentialAccessor::new(allowed_keys, move |_id: &str| {
            let err = err.clone();
            async move { Err(err) }
        })
    }

    #[tokio::test]
    async fn rejects_undeclared_key() {
        let accessor = make_accessor(["declared_key"]);
        let result = accessor.get("undeclared_key").await;
        assert!(
            matches!(result, Err(ActionError::SandboxViolation { .. })),
            "expected SandboxViolation, got {result:?}"
        );
    }

    #[tokio::test]
    async fn allows_declared_key_and_delegates_to_resolver() {
        let allowed_keys: HashSet<String> =
            ["my_credential"].iter().map(|s| s.to_string()).collect();

        let accessor = EngineCredentialAccessor::new(allowed_keys, |_id: &str| async {
            Err(ActionError::fatal("resolver reached"))
        });

        let result = accessor.get("my_credential").await;
        // The resolver was called — we get a fatal error, not a sandbox violation.
        assert!(
            matches!(result, Err(ActionError::Fatal { .. })),
            "expected Fatal from resolver, got {result:?}"
        );
    }

    #[tokio::test]
    async fn has_returns_true_for_allowed_key() {
        let accessor = make_accessor(["allowed"]);
        assert!(accessor.has("allowed").await);
    }

    #[tokio::test]
    async fn has_returns_false_for_undeclared_key() {
        let accessor = make_accessor(["allowed"]);
        assert!(!accessor.has("not_allowed").await);
    }

    #[tokio::test]
    async fn has_returns_false_for_empty_allowlist() {
        let accessor = make_accessor([]);
        assert!(!accessor.has("anything").await);
    }

    #[tokio::test]
    async fn get_rejects_all_keys_when_allowlist_is_empty() {
        let accessor = make_accessor([]);
        let result = accessor.get("any_key").await;
        assert!(
            matches!(result, Err(ActionError::SandboxViolation { .. })),
            "expected SandboxViolation, got {result:?}"
        );
    }

    #[tokio::test]
    async fn sandbox_violation_contains_credential_capability_name() {
        let accessor = make_accessor(["allowed"]);
        let err = accessor.get("secret_key").await.unwrap_err();
        match err {
            ActionError::SandboxViolation { capability, .. } => {
                assert_eq!(capability, "credential:secret_key");
            }
            other => panic!("expected SandboxViolation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolver_error_propagates_for_allowed_key() {
        let accessor =
            make_failing_accessor(["my_key"], ActionError::retryable("transient failure"));
        let result = accessor.get("my_key").await;
        assert!(
            matches!(result, Err(ActionError::Retryable { .. })),
            "expected Retryable, got {result:?}"
        );
    }

    #[test]
    fn debug_redacts_resolve_fn() {
        let accessor = make_accessor(["k"]);
        let debug = format!("{accessor:?}");
        assert!(debug.contains("<fn>"));
        assert!(!debug.contains("resolve_fn: Arc"));
    }
}
