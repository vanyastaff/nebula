//! Scope-enforcing [`ResumeTokenStore`] decorator.
//!
//! `revoke_on_terminal` carries a caller-supplied `&Scope` — the decorator
//! substitutes the bound scope to prevent a confused caller from revoking
//! another tenant's tokens.
//!
//! `consume` has NO `&Scope` parameter by design (scope comes FROM the
//! returned row; possession of the 256-bit hash is the only authorisation).
//! The decorator delegates it unchanged.

use std::sync::Arc;

use nebula_storage_port::Scope;
use nebula_storage_port::StorageError;
use nebula_storage_port::dto::resume_token::{ResumeTokenRow, TokenHash};
use nebula_storage_port::store::ResumeTokenStore;

/// Wraps a [`ResumeTokenStore`] and forces `revoke_on_terminal` into a
/// single bound [`Scope`].  The caller-supplied `scope` argument on
/// `revoke_on_terminal` is **ignored** — an engine runner cannot revoke
/// another tenant's parked tokens even if it passes a forged scope.
///
/// `consume` is delegated without scope substitution because it has no
/// `&Scope` parameter: hash possession is the only key.
#[derive(Clone)]
pub struct ScopedResumeTokenStore {
    inner: Arc<dyn ResumeTokenStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedResumeTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedResumeTokenStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedResumeTokenStore {
    /// Bind `inner` to `scope`. Constructed at the composition root from
    /// the request principal via a `ScopeResolver`.
    #[must_use]
    pub fn new(inner: Arc<dyn ResumeTokenStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl ResumeTokenStore for ScopedResumeTokenStore {
    async fn consume(
        &self,
        token_hash: &TokenHash,
    ) -> Result<Option<ResumeTokenRow>, StorageError> {
        // No scope substitution: `consume` carries no `&Scope` argument by
        // design — the scope is read FROM the returned row.
        self.inner.consume(token_hash).await
    }

    async fn revoke_on_terminal(
        &self,
        _scope: &Scope,
        execution_id: &str,
    ) -> Result<u64, StorageError> {
        // Substitute the bound scope; caller-supplied scope is ignored.
        self.inner
            .revoke_on_terminal(&self.bound, execution_id)
            .await
    }
}
