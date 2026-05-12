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

// `ProviderFuture<'a>: Unpin` is auto-derived. The chain of reasoning:
// - `Box<T>: Unpin` for any `T` (heap pointers are freely movable regardless
//   of pointee pinning state).
// - `Pin<P>: Unpin` when `P: Unpin` (via `impl<P: Unpin> Unpin for Pin<P>`),
//   so `Pin<Box<dyn Future + Send>>: Unpin`.
// - `Option<T>: Unpin` when `T: Unpin`, so `Option<Result<…>>: Unpin`.
// - Both `Inner` variants therefore satisfy `Unpin`, and so does the outer
//   struct via auto-derive.
//
// This lets us project safely via `Pin::into_inner` in `poll` without `unsafe`.

impl Future for ProviderFuture<'_> {
    type Output = Result<ProviderResolution, ProviderError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = Pin::into_inner(self);
        match &mut this.inner {
            Inner::Ready(slot) => match slot.take() {
                Some(value) => Poll::Ready(value),
                // Double-poll: the `Future` contract permits us to return any
                // `Poll::Ready` here (further polls are undefined per contract).
                // Returning a `Backend` error rather than panicking complies
                // with the no-`panic!`/`expect` rule for library code (see
                // AGENTS.md → Agent Rules). Hitting this path indicates a
                // caller bug — a properly-driven future stops being polled
                // after `Poll::Ready`.
                None => Poll::Ready(Err(ProviderError::Backend(
                    "ProviderFuture::Ready polled after completion (caller bug)"
                        .to_owned()
                        .into(),
                ))),
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

    #[test]
    fn double_poll_returns_backend_error_not_panic() {
        // Drive the future manually so we can poll it twice — the second
        // poll must produce a `Backend` error rather than panicking,
        // honouring the no-`panic!`/`expect` rule for library code.
        use std::{
            future::Future as _,
            task::{Context, Poll, Waker},
        };

        let mut fut = std::pin::pin!(ProviderFuture::ready(Ok(ProviderResolution::from_secret(
            SecretString::new("once")
        ))));

        // `Waker::noop` is stable since Rust 1.85; no extra crate needed.
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        // First poll: ready with the original value.
        let first = fut.as_mut().poll(&mut cx);
        assert!(matches!(first, Poll::Ready(Ok(_))));

        // Second poll: ready with a Backend error, NOT a panic.
        let second = fut.as_mut().poll(&mut cx);
        match second {
            Poll::Ready(Err(ProviderError::Backend(_))) => {},
            other => panic!("expected Ready(Err(Backend(_))), got {other:?}"),
        }
    }
}
