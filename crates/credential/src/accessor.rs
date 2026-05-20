//! Default credential accessor stub.
//!
//! The [`CredentialAccessor`](nebula_core::accessor::CredentialAccessor) trait
//! is defined in `nebula_core::accessor` and re-exported at the crate root.
//! This module ships only the no-op stub used by
//! [`CredentialContext`](crate::CredentialContext) when the runtime does not
//! inject a real accessor.
//!
//! The engine-runtime accessor that enforces per-action allowlists lives in
//! `nebula_engine::credential::ScopedCredentialAccessor` (engine depends on
//! `nebula-credential`, not the other way around).

use std::{future::Future, pin::Pin, sync::Arc};

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

#[cfg(test)]
mod tests {
    use nebula_core::{CoreError, accessor::CredentialAccessor, credential_key};

    use super::*;

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
}
