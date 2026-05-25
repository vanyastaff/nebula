//! Probe: `CredentialContext` must surface a `CancellationToken` and a
//! child token must be derivable for in-flight credential capability
//! calls. Cancellation observation goes through the borrowed token —
//! no proxied `is_cancelled` wrapper on the context.

use std::{any::Any, future::Future, pin::Pin, sync::Arc};

use nebula_core::{BaseContext, CoreError, ResourceKey, accessor::ResourceAccessor};
use nebula_credential::{CredentialContextBuilder, NoopCredentialAccessor};
use tokio_util::sync::CancellationToken;

struct NoopResourceAccessor;

impl ResourceAccessor for NoopResourceAccessor {
    fn has(&self, _: &ResourceKey) -> bool {
        false
    }

    fn acquire_any(
        &self,
        _: &ResourceKey,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, CoreError>> + Send + '_>>
    {
        Box::pin(async {
            Err(CoreError::CredentialNotConfigured(
                "noop resource accessor".into(),
            ))
        })
    }

    fn try_acquire_any(
        &self,
        _: &ResourceKey,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<Box<dyn Any + Send + Sync>>, CoreError>> + Send + '_>,
    > {
        Box::pin(async { Ok(None) })
    }
}

#[test]
fn child_token_derivable_and_cascades_from_parent() {
    let parent = CancellationToken::new();
    let ctx = CredentialContextBuilder::new(
        BaseContext::builder().build(),
        Arc::new(NoopCredentialAccessor),
        Arc::new(NoopResourceAccessor),
    )
    .with_cancel(parent.clone())
    .build();
    // Caller observes cancellation on the borrowed token directly —
    // no duplicated `is_cancelled` API on the context.
    let child = ctx.cancel_token().child_token();
    parent.cancel();
    assert!(
        child.is_cancelled(),
        "child token must cascade from parent cancel"
    );
}
