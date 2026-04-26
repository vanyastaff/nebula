//! Engine-side `CredentialAccessor` implementation.
//!
//! [`EngineCredentialAccessor`] bridges the engine's credential resolution
//! infrastructure to the `CredentialAccessor` capability trait consumed by
//! actions. It enforces an allowlist of declared credential keys so that
//! actions can only access credentials they have explicitly declared as
//! dependencies.
//!
//! # Design
//!
//! [`CredentialResolver<S>`](crate::credential::CredentialResolver) is generic
//! over a store type, which would infect the engine with an unbounded type
//! parameter. Instead, the resolution function is captured once as a
//! type-erased `Arc<dyn Fn + Send + Sync>` returning a pinned future. This
//! keeps `WorkflowEngine` concrete while still delegating to any store
//! implementation.
//!
//! # Allowlist semantics (deny-by-default)
//!
//! Per `PRODUCT_CANON` §4.5 (operational honesty) and §12.5 (secrets and auth):
//! an action may only acquire a credential it has **explicitly declared**.
//!
//! - An **empty** allowlist means **no credentials are permitted**. Every request is rejected with
//!   [`CoreError::CredentialAccessDenied`]. This is the deny-by-default baseline when the engine
//!   receives no declaration for the action being dispatched.
//! - A **non-empty** allowlist only permits the keys in the set. Requests for any key not in the
//!   set are rejected with [`CoreError::CredentialAccessDenied`].
//!
//! The engine populates the allowlist from per-action credential declarations
//! supplied through [`WorkflowEngine::with_action_credentials`]
//! (see [`crate::engine`]). An action that never had its credentials declared
//! to the engine therefore falls through to the deny baseline — there is no
//! "fail-open" escape hatch.
//!
//! [`WorkflowEngine::with_action_credentials`]: crate::WorkflowEngine::with_action_credentials

use std::{collections::HashSet, fmt, future::Future, pin::Pin, sync::Arc};

use nebula_core::{CoreError, CredentialKey};

/// Type alias for dyn-safe async return.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Type alias for the boxed async credential-resolution function.
///
/// The function takes a credential key string and returns a boxed
/// `CredentialSnapshot` (as `Box<dyn Any>`) or a `CoreError`.
type ResolveFn = Arc<
    dyn Fn(
            &str,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<Box<dyn std::any::Any + Send + Sync>, CoreError>> + Send,
            >,
        > + Send
        + Sync,
>;

/// Engine-side implementation of [`CredentialAccessor`](nebula_core::accessor::CredentialAccessor).
///
/// Validates that the requested credential key is in the declared allowlist
/// before delegating resolution to the underlying resolver function.
///
/// # Examples
///
/// ```rust,ignore
/// use std::collections::HashSet;
/// use std::sync::Arc;
/// use nebula_engine::credential::CredentialResolver;
/// use nebula_engine::credential_accessor::EngineCredentialAccessor;
/// use nebula_storage::credential::InMemoryStore;
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
///                 .map_err(|e| CoreError::CredentialNotFound { key: e.to_string() })
///         })
///     }
/// });
/// ```
pub struct EngineCredentialAccessor {
    /// Set of credential keys this accessor is permitted to resolve.
    ///
    /// An empty set means **no** credentials are accessible (deny-by-default).
    /// Populated from per-action credential declarations supplied through
    /// [`WorkflowEngine::with_action_credentials`](crate::WorkflowEngine::with_action_credentials).
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
    /// - `allowed_keys` — the set of credential IDs this accessor may resolve. An **empty** set
    ///   denies every request (deny-by-default, per `PRODUCT_CANON` §4.5 / §12.5). A non-empty set
    ///   permits only the listed keys.
    /// - `resolve_fn` — async closure that resolves a credential ID to a boxed `Any` (typically a
    ///   `CredentialSnapshot`) or a `CoreError`.
    /// - `action_id` — the action key or node identifier for security attribution in
    ///   `CoreError::CredentialAccessDenied` events.
    pub fn new<F, Fut>(allowed_keys: HashSet<String>, resolve_fn: F, action_id: String) -> Self
    where
        F: Fn(&str) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Box<dyn std::any::Any + Send + Sync>, CoreError>>
            + Send
            + 'static,
    {
        Self {
            allowed_keys,
            resolve_fn: Arc::new(move |id: &str| {
                Box::pin(resolve_fn(id))
                    as Pin<
                        Box<
                            dyn Future<
                                    Output = Result<
                                        Box<dyn std::any::Any + Send + Sync>,
                                        CoreError,
                                    >,
                                > + Send,
                        >,
                    >
            }),
            action_id,
        }
    }

    /// Returns `true` if `id` is permitted by the allowlist.
    ///
    /// Deny-by-default: an **empty** allowlist permits nothing.
    /// A **non-empty** allowlist permits only listed keys.
    fn is_allowed(&self, id: &str) -> bool {
        self.allowed_keys.contains(id)
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

impl nebula_core::accessor::CredentialAccessor for EngineCredentialAccessor {
    /// Check whether a credential key is accessible.
    ///
    /// Returns `true` only if `key` is permitted by the allowlist.
    /// Deny-by-default: an empty allowlist always yields `false`.
    ///
    /// Note: this is a synchronous check against the allowlist only.
    /// It does not verify the credential exists in the store.
    fn has(&self, key: &CredentialKey) -> bool {
        self.is_allowed(key.as_str())
    }

    /// Resolve a credential by key.
    ///
    /// # Errors
    ///
    /// - [`CoreError::CredentialAccessDenied`] — if `key` is not in the allowlist (including the
    ///   deny-by-default case of an empty allowlist).
    /// - Any error returned by the underlying resolver function.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel-safe. If the future is dropped before completion,
    /// no state is modified.
    fn resolve_any(
        &self,
        key: &CredentialKey,
    ) -> BoxFuture<'_, Result<Box<dyn std::any::Any + Send + Sync>, CoreError>> {
        let key_str = key.as_str();
        if !self.is_allowed(key_str) {
            let capability = format!("credential:{key_str}");
            let action_id = self.action_id.clone();
            return Box::pin(async move {
                Err(CoreError::CredentialAccessDenied {
                    capability,
                    action_id,
                })
            });
        }
        let key_owned = key_str.to_owned();
        let resolve_fn = Arc::clone(&self.resolve_fn);
        Box::pin(async move { (resolve_fn)(&key_owned).await })
    }

    /// Try to resolve a credential by key, returning `None` if not in allowlist.
    ///
    /// Unlike `resolve_any`, this does not error on disallowed keys — it
    /// returns `Ok(None)` instead, matching the "optional dependency" pattern.
    fn try_resolve_any(
        &self,
        key: &CredentialKey,
    ) -> BoxFuture<'_, Result<Option<Box<dyn std::any::Any + Send + Sync>>, CoreError>> {
        let key_str = key.as_str();
        if !self.is_allowed(key_str) {
            return Box::pin(async { Ok(None) });
        }
        let key_owned = key_str.to_owned();
        let resolve_fn = Arc::clone(&self.resolve_fn);
        Box::pin(async move { (resolve_fn)(&key_owned).await.map(Some) })
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{accessor::CredentialAccessor, credential_key};

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
                Err(CoreError::CredentialNotConfigured(
                    "not implemented in stub".to_owned(),
                ))
            },
            "test_action".to_owned(),
        )
    }

    /// Builds an accessor whose resolver always returns the given error.
    fn make_failing_accessor(
        allowed: impl IntoIterator<Item = &'static str>,
        err: CoreError,
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

    #[test]
    fn has_returns_true_for_allowed_key() {
        let accessor = make_accessor(["declared_key"]);
        assert!(accessor.has(&credential_key!("declared_key")));
    }

    #[test]
    fn has_returns_false_for_undeclared_key() {
        let accessor = make_accessor(["declared_key"]);
        assert!(!accessor.has(&credential_key!("undeclared_key")));
    }

    #[test]
    fn has_returns_false_for_empty_allowlist() {
        let accessor = make_accessor([]);
        assert!(!accessor.has(&credential_key!("anything")));
    }

    #[tokio::test]
    async fn rejects_undeclared_key() {
        let accessor = make_accessor(["declared_key"]);
        let result = accessor
            .resolve_any(&credential_key!("undeclared_key"))
            .await;
        assert!(
            matches!(result, Err(CoreError::CredentialAccessDenied { .. })),
            "expected CredentialAccessDenied, got {result:?}"
        );
    }

    #[tokio::test]
    async fn allows_declared_key_and_delegates_to_resolver() {
        let allowed_keys: HashSet<String> =
            ["my_credential"].iter().map(ToString::to_string).collect();

        let accessor = EngineCredentialAccessor::new(
            allowed_keys,
            |_id: &str| async {
                Err(CoreError::CredentialNotFound {
                    key: "resolver reached".to_owned(),
                })
            },
            "test_action".to_owned(),
        );

        let result = accessor
            .resolve_any(&credential_key!("my_credential"))
            .await;
        // The resolver was called — we get CredentialNotFound, not AccessDenied.
        assert!(
            matches!(result, Err(CoreError::CredentialNotFound { .. })),
            "expected CredentialNotFound from resolver, got {result:?}"
        );
    }

    #[tokio::test]
    async fn resolve_any_denies_every_key_when_allowlist_is_empty() {
        let accessor = make_accessor([]);
        let result = accessor.resolve_any(&credential_key!("any_key")).await;
        assert!(
            matches!(result, Err(CoreError::CredentialAccessDenied { .. })),
            "empty allowlist must deny; got {result:?}"
        );
    }

    #[tokio::test]
    async fn access_denied_contains_credential_capability_name_and_action_id() {
        let accessor = make_accessor(["allowed"]);
        let err = accessor
            .resolve_any(&credential_key!("secret_key"))
            .await
            .unwrap_err();
        match err {
            CoreError::CredentialAccessDenied {
                capability,
                action_id,
            } => {
                assert_eq!(capability, "credential:secret_key");
                assert_eq!(action_id, "test_action");
            },
            other => panic!("expected CredentialAccessDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolver_error_propagates_for_allowed_key() {
        let accessor = make_failing_accessor(
            ["my_key"],
            CoreError::CredentialNotFound {
                key: "transient failure".to_owned(),
            },
        );
        let result = accessor.resolve_any(&credential_key!("my_key")).await;
        assert!(
            matches!(result, Err(CoreError::CredentialNotFound { .. })),
            "expected CredentialNotFound, got {result:?}"
        );
    }

    #[test]
    fn debug_redacts_resolve_fn() {
        let accessor = make_accessor(["k"]);
        let debug = format!("{accessor:?}");
        assert!(debug.contains("<fn>"));
        assert!(!debug.contains("resolve_fn: Arc"));
    }

    #[tokio::test]
    async fn denied_request_never_invokes_resolver() {
        use std::sync::atomic::{AtomicU32, Ordering as AOrdering};
        let calls = Arc::new(AtomicU32::new(0));
        let calls_inner = calls.clone();
        let accessor = EngineCredentialAccessor::new(
            HashSet::new(), // deny-by-default
            move |_id: &str| {
                let calls = calls_inner.clone();
                async move {
                    calls.fetch_add(1, AOrdering::Relaxed);
                    Err(CoreError::CredentialNotFound {
                        key: "resolver should not be called".to_owned(),
                    })
                }
            },
            "test_action".to_owned(),
        );

        let _ = accessor.resolve_any(&credential_key!("anything")).await;
        assert_eq!(
            calls.load(AOrdering::Relaxed),
            0,
            "resolver must not run when denied"
        );
    }
}
