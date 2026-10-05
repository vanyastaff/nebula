//! Pass-through response frames, with bounded JSON observation at end-of-stream.

use std::{
    fs,
    io::Write,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{MatchedPath, Request},
    middleware::{self, Next},
    response::Response,
};
use http_body::{Body as HttpBody, Frame, SizeHint};
use serde_json::Value;
use zeroize::Zeroize;

use super::{
    ConformanceError,
    validation::{
        Finding, Observation, Operation, inventory, unconsumed_response,
        validate_response_with_length,
    },
};

const BODY_CAP: usize = 8 * 1024 * 1024;

struct Observer {
    spec: Value,
    directory: PathBuf,
    operations: Vec<Operation>,
}

#[derive(Clone, Copy)]
struct ObservedResponse;

fn fail_closed() -> ! {
    // This module exists only under unsupported test-util and only enters
    // this path after explicit producer activation. A lost observation must
    // fail its test process even when another fixture covers the same branch.
    tracing::error!("OpenAPI conformance evidence could not be retained");
    std::process::exit(86)
}

pub(crate) fn observe_if_enabled(router: Router, spec: &utoipa::openapi::OpenApi) -> Router {
    let Some(directory) = std::env::var_os("NEBULA_OPENAPI_OBSERVATIONS") else {
        return router;
    };
    match Observer::new(PathBuf::from(directory), spec) {
        Ok(observer) => router.layer(middleware::from_fn(move |request: Request, next: Next| {
            let observer = Arc::clone(&observer);
            async move { observer.respond(request, next).await }
        })),
        Err(_) => fail_closed(),
    }
}

impl Observer {
    fn new(
        directory: PathBuf,
        spec: &utoipa::openapi::OpenApi,
    ) -> Result<Arc<Self>, ConformanceError> {
        let spec = serde_json::to_value(spec).map_err(|_| ConformanceError::InvalidSpec)?;
        let operations = inventory(&spec)?.into_keys().collect();
        fs::create_dir_all(&directory).map_err(|_| ConformanceError::ObservationWrite)?;
        // Per-router inventories remain independent: concurrent fixtures never
        // race a shared mutable file or overwrite another fixture's evidence.
        let name = format!("spec-{}.json", uuid::Uuid::new_v4());
        let bytes = serde_json::to_vec(&spec).map_err(|_| ConformanceError::InvalidSpec)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))
            .map_err(|_| ConformanceError::ObservationWrite)?;
        file.write_all(&bytes)
            .map_err(|_| ConformanceError::ObservationWrite)?;
        Ok(Arc::new(Self {
            spec,
            directory,
            operations,
        }))
    }

    async fn respond(self: Arc<Self>, request: Request, next: Next) -> Response {
        let method = request.method().as_str().to_ascii_lowercase();
        let operation = request
            .extensions()
            .get::<MatchedPath>()
            .and_then(|matched| {
                self.operations.iter().find(|operation| {
                    operation.method == method && operation.path == matched.as_str()
                })
            })
            .cloned();
        let mut response = next.run(request).await;
        if response.extensions().get::<ObservedResponse>().is_some() {
            return response;
        }
        let Some(operation) = operation else {
            return response;
        };
        response.extensions_mut().insert(ObservedResponse);
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let (parts, body) = response.into_parts();
        let mut observed = ObservedBody {
            inner: Box::pin(body),
            observer: self,
            operation,
            status,
            content_type,
            bytes: Vec::new(),
            body_length: 0,
            oversized: false,
            finished: false,
        };
        if observed.inner.is_end_stream() {
            observed.finish(false);
        }
        Response::from_parts(parts, Body::new(observed))
    }

    fn retain(&self, observation: &Observation) -> Result<(), ConformanceError> {
        let bytes =
            serde_json::to_vec(observation).map_err(|_| ConformanceError::InvalidObservation)?;
        let name = format!("case-{}.json", uuid::Uuid::new_v4());
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.directory.join(name))
            .map_err(|_| ConformanceError::ObservationWrite)?;
        file.write_all(&bytes)
            .map_err(|_| ConformanceError::ObservationWrite)
    }
}

struct ObservedBody {
    inner: Pin<Box<Body>>,
    observer: Arc<Observer>,
    operation: Operation,
    status: u16,
    content_type: Option<String>,
    bytes: Vec<u8>,
    body_length: usize,
    oversized: bool,
    finished: bool,
}

impl ObservedBody {
    /// Count one data frame; retain it only for a bounded JSON body. Text and
    /// binary bodies contribute their length but never their contents.
    fn record(&mut self, data: &Bytes) {
        self.body_length = self.body_length.saturating_add(data.len());
        let json = self.content_type.as_deref().is_some_and(|media| {
            media
                .split(';')
                .next()
                .is_some_and(|media| media.trim().ends_with("json"))
        });
        if !json || self.oversized {
            return;
        }
        if data.len() > BODY_CAP.saturating_sub(self.bytes.len()) {
            self.oversized = true;
            self.bytes.zeroize();
        } else {
            self.bytes.extend_from_slice(data);
        }
    }

    fn finish(&mut self, failed: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        let observation = if self.oversized || failed {
            Observation {
                operation: self.operation.clone(),
                status: self.status,
                body_complete: true,
                findings: vec![Finding {
                    operation: self.operation.clone(),
                    status: self.status,
                    kind: if failed {
                        "response-stream-failure"
                    } else {
                        "response-body-over-cap"
                    }
                    .to_owned(),
                    schema_path: String::new(),
                }],
            }
        } else {
            validate_response_with_length(
                &self.observer.spec,
                &self.operation,
                self.status,
                self.content_type.as_deref(),
                &self.bytes,
                self.body_length,
            )
        };
        self.bytes.zeroize();
        if self.observer.retain(&observation).is_err() {
            fail_closed();
        }
    }
}

impl Drop for ObservedBody {
    fn drop(&mut self) {
        // An unconsumed response proves no payload branch. It never contributes
        // to coverage; the denominator check must find another completed case.
        self.bytes.zeroize();
        if !self.finished {
            let observation = unconsumed_response(
                &self.observer.spec,
                &self.operation,
                self.status,
                self.content_type.as_deref(),
                self.body_length,
            );
            if self.observer.retain(&observation).is_err() {
                fail_closed();
            }
        }
    }
}

impl HttpBody for ObservedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match self.inner.as_mut().poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    self.record(data);
                }
                if self.inner.is_end_stream() {
                    self.finish(false);
                }
                Poll::Ready(Some(Ok(frame)))
            },
            Poll::Ready(Some(Err(error))) => {
                self.finish(true);
                Poll::Ready(Some(Err(error)))
            },
            Poll::Ready(None) => {
                self.finish(false);
                Poll::Ready(None)
            },
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::{Request as HttpRequest, StatusCode},
        routing::get,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn unexpected_non_json_body_is_detected_without_retaining_the_payload() {
        let directory = tempfile::tempdir().unwrap();
        let spec: utoipa::openapi::OpenApi = serde_json::from_value(serde_json::json!({
            "openapi":"3.1.0", "info":{"title":"conformance-control", "version":"1"},
            "paths":{"/empty":{"get":{"operationId":"empty_control", "responses":{
                "204":{"description":"No response body"}
            }}}}
        }))
        .unwrap();
        let observer = Observer::new(directory.path().to_path_buf(), &spec).unwrap();
        let app = Router::new()
            .route(
                "/empty",
                get(|| async {
                    (
                        StatusCode::NO_CONTENT,
                        [("content-type", "application/octet-stream")],
                        "private-body-canary",
                    )
                }),
            )
            .layer(middleware::from_fn(move |request: Request, next: Next| {
                let observer = Arc::clone(&observer);
                async move { observer.respond(request, next).await }
            }));
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/empty")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let body = axum::body::to_bytes(response.into_body(), BODY_CAP)
            .await
            .unwrap();
        assert_eq!(
            body.as_ref(),
            b"private-body-canary",
            "observer preserves actual bytes"
        );
        let cases: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(Result::unwrap)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("case-"))
            .collect();
        assert_eq!(cases.len(), 1);
        let encoded = fs::read(cases[0].path()).unwrap();
        assert!(!String::from_utf8_lossy(&encoded).contains("private-body-canary"));
        let observed: Observation = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(observed.findings[0].kind, "unexpected-response-body");
    }
}
