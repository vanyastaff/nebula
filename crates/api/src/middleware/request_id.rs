//! Request ID Middleware
//!
//! Adds unique request ID to each request for tracing.

use std::task::{Context, Poll};

use axum::{
    extract::Request,
    response::{IntoResponse, Response},
};
use futures::future::BoxFuture;
use tower::{Layer, Service};
use uuid::Uuid;

/// Request ID header name
pub const X_REQUEST_ID: &str = "x-request-id";

/// Layer that adds request ID
#[derive(Clone)]
pub struct RequestIdLayer;

impl<S> Layer<S> for RequestIdLayer {
    type Service = RequestIdService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestIdService { inner }
    }
}

/// Service that adds request ID
#[derive(Clone)]
pub struct RequestIdService<S> {
    inner: S,
}

impl<S> Service<Request> for RequestIdService<S>
where
    S: Service<Request> + Send + 'static,
    S::Response: IntoResponse + Send + 'static,
    S::Error: Into<std::convert::Infallible> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request) -> Self::Future {
        // Generate or extract request ID
        let request_id = request
            .headers()
            .get(X_REQUEST_ID)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // Insert into extensions for handlers
        request
            .extensions_mut()
            .insert(RequestId(request_id.clone()));

        let future = self.inner.call(request);

        // Add request ID to response headers
        Box::pin(async move {
            let mut response = future.await?.into_response();
            if let Ok(value) = request_id.parse() {
                response.headers_mut().insert(X_REQUEST_ID, value);
            }
            Ok(response)
        })
    }
}

/// Request ID extension
#[derive(Debug, Clone)]
pub struct RequestId(pub String);
