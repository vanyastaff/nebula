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
//! - An **empty** allowlist means **all credentials are permitted** (open/passthrough
//!   mode). This is the current default until per-node credential declarations are
//!   wired in from action dependency metadata.
//! - A **non-empty** allowlist only permits the keys in the set. Requests for
//!   undeclared keys are rejected with [`CredentialAccessError::AccessDenied`].

use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_credential::{CredentialAccessError, CredentialAccessor, CredentialSnapshot};

/// Type alias for the boxed async credential-resolution function.
type ResolveFn = Arc<
    dyn Fn(
            &str,
        ) -> Pin<
            Box<dyn Future<Output = Result<CredentialSnapshot, CredentialAccessError>> + Send>,
        > + Send
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
    /// An empty set means **all** credentials are accessible (open/passthrough mode).
    /// This is used when node-level credential declarations have not yet been populated
    /// from action dependency metadata (current default).
    allowed_keys: HashSet<String>,
    /// Type-erased async resolution function.
    resolve_fn: ResolveFn,
    /// Identity of the action this accessor is scoped to, for security attribution.
    action_id: String,
}

impl EngineCredentialAccessor {
    /// Creates a new accessor with the given allowlist, resolution function, and action identity.
    ///
    /// # Parameters
    ///
    /// - `allowed_keys` — the set of credential IDs this accessor may resolve.
    ///   Pass an empty set to allow all credential access (open/passthrough mode, used when
    ///   per-node declarations are not yet populated).
    /// - `resolve_fn` — async closure that resolves a credential ID to a
    ///   [`CredentialSnapshot`] or a [`CredentialAccessError`].
    /// - `action_id` — the action key or node identifier for security attribution in
    ///   [`CredentialAccessError::AccessDenied`] events.
    pub fn new<F, Fut>(allowed_keys: HashSet<String>, resolve_fn: F, action_id: String) -> Self
    where
        F: Fn(&str) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<CredentialSnapshot, CredentialAccessError>> + Send + 'static,
    {
        Self {
            allowed_keys,
            resolve_fn: Arc::new(move |id: &str| {
                Box::pin(resolve_fn(id))
                    as Pin<
                        Box<
                            dyn Future<Output = Result<CredentialSnapshot, CredentialAccessError>>
                                + Send,
                        >,
                    >
            }),
            action_id,
        }
    }

    /// Returns `true` if `id` is permitted by the allowlist.
    ///
    /// When the allowlist is **empty**, all keys are permitted (open/passthrough mode).
    /// When the allowlist is **non-empty**, only listed keys are permitted.
    fn is_allowed(&self, id: &str) -> bool {
        self.allowed_keys.is_empty() || self.allowed_keys.contains(id)
    }
}

impl fmt::Debug for EngineCredentialAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineCredentialAccessor")
            .field("allowed_keys", &self.allowed_keys)
            .field("resolve_fn", &"<fn>")
            .field("action_id", &self.action_id)
            .finish()
    }
}

#[async_trait]
impl CredentialAccessor for EngineCredentialAccessor {
    /// Retrieve a credential snapshot by id.
    ///
    /// # Errors
    ///
    /// - [`CredentialAccessError::AccessDenied`] — if `id` is not in the allowlist
    ///   (when the allowlist is non-empty and `id` is not listed).
    /// - Any error returned by the underlying resolver function.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel-safe. If the future is dropped before completion,
    /// no state is modified.
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, CredentialAccessError> {
        if !self.is_allowed(id) {
            return Err(CredentialAccessError::AccessDenied {
                capability: format!("credential:{id}"),
                action_id: self.action_id.clone(),
            });
        }
        (self.resolve_fn)(id).await
    }

    /// Check whether a credential key is accessible and exists in the store.
    ///
    /// Returns `true` only if `id` is permitted by the allowlist **and** the
    /// underlying resolver can successfully resolve it. When the allowlist is
    /// empty (open/passthrough mode), all keys that resolve successfully are
    /// considered available.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel-safe. If the future is dropped before completion,
    /// no state is modified.
    async fn has(&self, id: &str) -> bool {
        self.is_allowed(id) && (self.resolve_fn)(id).await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an accessor with the given allowed keys and a stub resolver.
    ///
    /// The stub resolver always fails with "not implemented in stub". Use
    /// [`make_succeeding_accessor`] when resolver success is needed.
    fn make_accessor(allowed: impl IntoIterator<Item = &'static str>) -> EngineCredentialAccessor {
        let allowed_keys: HashSet<String> = allowed.into_iter().map(str::to_owned).collect();
        EngineCredentialAccessor::new(
            allowed_keys,
            |_id: &str| async {
                Err(CredentialAccessError::NotConfigured(
                    "not implemented in stub".to_owned(),
                ))
            },
            "test_action".to_owned(),
        )
    }

    /// Builds an accessor whose resolver always returns the given error.
    fn make_failing_accessor(
        allowed: impl IntoIterator<Item = &'static str>,
        err: CredentialAccessError,
    ) -> EngineCredentialAccessor {
        let allowed_keys: HashSet<String> = allowed.into_iter().map(str::to_owned).collect();
        EngineCredentialAccessor::new(
            allowed_keys,
            move |_id: &str| {
                let err = err.clone();
                async move { Err(err) }
            },
            "test_action".to_owned(),
        )
    }

    #[tokio::test]
    async fn rejects_undeclared_key() {
        let accessor = make_accessor(["declared_key"]);
        let result = accessor.get("undeclared_key").await;
        assert!(
            matches!(result, Err(CredentialAccessError::AccessDenied { .. })),
            "expected AccessDenied, got {result:?}"
        );
    }

    #[tokio::test]
    async fn allows_declared_key_and_delegates_to_resolver() {
        let allowed_keys: HashSet<String> =
            ["my_credential"].iter().map(|s| s.to_string()).collect();

        let accessor = EngineCredentialAccessor::new(
            allowed_keys,
            |_id: &str| async {
                Err(CredentialAccessError::NotFound(
                    "resolver reached".to_owned(),
                ))
            },
            "test_action".to_owned(),
        );

        let result = accessor.get("my_credential").await;
        // The resolver was called — we get NotFound, not AccessDenied.
        assert!(
            matches!(result, Err(CredentialAccessError::NotFound(_))),
            "expected NotFound from resolver, got {result:?}"
        );
    }

    #[tokio::test]
    async fn has_returns_false_for_allowed_key_not_in_store() {
        // Resolver fails -> key is in allowlist but not resolvable -> has() = false.
        let accessor = make_accessor(["allowed"]);
        assert!(!accessor.has("allowed").await);
    }

    #[tokio::test]
    async fn has_returns_false_for_undeclared_key() {
        // Key is not in (non-empty) allowlist -> rejected before resolver call.
        let accessor = make_accessor(["allowed"]);
        assert!(!accessor.has("not_allowed").await);
    }

    #[tokio::test]
    async fn has_returns_false_for_empty_allowlist_when_resolver_fails() {
        // Empty allowlist = allow all, but resolver still fails -> has() = false.
        let accessor = make_accessor([]);
        assert!(!accessor.has("anything").await);
    }

    #[tokio::test]
    async fn get_allows_any_key_when_allowlist_is_empty() {
        // Empty allowlist = open/passthrough mode: no AccessDenied, resolver is called.
        let accessor = make_accessor([]);
        let result = accessor.get("any_key").await;
        // Stub resolver returns NotConfigured, not AccessDenied — resolver was reached.
        assert!(
            matches!(result, Err(CredentialAccessError::NotConfigured(_))),
            "expected NotConfigured from resolver (not AccessDenied), got {result:?}"
        );
    }

    #[tokio::test]
    async fn access_denied_contains_credential_capability_name_and_action_id() {
        let accessor = make_accessor(["allowed"]);
        let err = accessor.get("secret_key").await.unwrap_err();
        match err {
            CredentialAccessError::AccessDenied {
                capability,
                action_id,
            } => {
                assert_eq!(capability, "credential:secret_key");
                assert_eq!(action_id, "test_action");
            }
            other => panic!("expected AccessDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolver_error_propagates_for_allowed_key() {
        let accessor = make_failing_accessor(
            ["my_key"],
            CredentialAccessError::NotFound("transient failure".to_owned()),
        );
        let result = accessor.get("my_key").await;
        assert!(
            matches!(result, Err(CredentialAccessError::NotFound(_))),
            "expected NotFound, got {result:?}"
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
