//! Dyn-safe future newtype for [`ExternalProvider`](super::ExternalProvider).
//!
//! Mirrors the `NowOrLater` pattern from `aws-credential-types` (smithy-rs
//! `aws/rust-runtime/aws-credential-types/src/provider/future.rs`): an
//! enum-shaped envelope around either an immediately-known value or a boxed
//! future. The ready variant lets synchronous providers (env-var, in-memory)
//! avoid the heap allocation of `Box::pin(async { … })` while keeping the
//! trait dyn-safe.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use super::{ProviderError, ProviderResolution};

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Future returned by [`ExternalProvider::resolve`](super::ExternalProvider::resolve).
///
/// Two-state envelope:
/// - [`ProviderFuture::ready`] — resolved synchronously, no allocation. Use for
///   env-var providers, in-memory test doubles, and any source that does not
///   perform I/O.
/// - [`ProviderFuture::new`] — wraps an `async` block. Use for Vault / AWS SM /
///   GCP SM / Azure KV and any HTTP- or socket-bound resolver.
///
/// The trait stays dyn-safe (`Arc<dyn ExternalProvider>` is supported) because
/// the return type is a concrete struct, not `impl Future` (which is not
/// object-safe).
pub struct ProviderFuture<'a> {
    inner: Inner<'a>,
}

enum Inner<'a> {
    Ready(Option<Result<ProviderResolution, ProviderError>>),
    Boxed(BoxFut<'a, Result<ProviderResolution, ProviderError>>),
}

impl<'a> ProviderFuture<'a> {
    /// Wrap an `async` block. The future is boxed once at construction.
    #[must_use]
    pub fn new<F>(fut: F) -> Self
    where
        F: Future<Output = Result<ProviderResolution, ProviderError>> + Send + 'a,
    {
        Self {
            inner: Inner::Boxed(Box::pin(fut)),
        }
    }

    /// Return a synchronously-known value without heap allocation.
    ///
    /// Use this for providers that do not perform I/O — e.g. an env-var
    /// provider whose `resolve` reads `std::env::var` (a sync, fast op).
    #[must_use]
    pub fn ready(value: Result<ProviderResolution, ProviderError>) -> Self {
        Self {
            inner: Inner::Ready(Some(value)),
        }
    }
}

// `ProviderFuture<'a>: Unpin` is auto-derived: both `Option<Result<…>>` and
// `Pin<Box<dyn Future + Send>>` are `Unpin` (the latter via the blanket
// `impl<P> Unpin for Pin<P>`), so the inner enum and the outer struct inherit
// `Unpin` without an explicit impl. This lets us project safely via
// `Pin::into_inner` in `poll`.

impl Future for ProviderFuture<'_> {
    type Output = Result<ProviderResolution, ProviderError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = Pin::into_inner(self);
        match &mut this.inner {
            Inner::Ready(slot) => {
                let value = slot
                    .take()
                    .expect("ProviderFuture::Ready polled after completion");
                Poll::Ready(value)
            },
            Inner::Boxed(fut) => fut.as_mut().poll(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretString;

    #[tokio::test]
    async fn ready_resolves_without_allocation() {
        let resolution = ProviderResolution::from_secret(SecretString::new("sk-test"));
        let fut = ProviderFuture::ready(Ok(resolution));
        let got = fut.await.unwrap();
        assert_eq!(got.secret.expose_secret(), "sk-test");
    }

    #[tokio::test]
    async fn boxed_resolves_via_async_block() {
        let fut = ProviderFuture::new(async {
            Ok(ProviderResolution::from_secret(SecretString::new(
                "from-async",
            )))
        });
        let got = fut.await.unwrap();
        assert_eq!(got.secret.expose_secret(), "from-async");
    }

    #[tokio::test]
    async fn ready_propagates_error() {
        let fut = ProviderFuture::ready(Err(ProviderError::NotFound {
            path: "missing".to_owned(),
        }));
        let err = fut.await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound { .. }));
    }
}
