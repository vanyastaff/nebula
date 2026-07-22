//! Regression proof that one-time OAuth callback credentials never enter traces.

use axum::{body::Body, http::Request};
use nebula_api::{ApiConfig, AppState, app};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tower::ServiceExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const STATE_CANARY: &str = "STATE_CANARY_DO_NOT_TRACE";
const CODE_CANARY: &str = "CODE_CANARY_DO_NOT_TRACE";
const PATH_CANARY: &str = "PATH_CANARY_DO_NOT_TRACE";

#[tokio::test]
async fn oauth_callback_trace_records_route_but_not_query_credentials() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("oauth-callback-trace-redaction-test");
    tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()
        .expect("dedicated integration-test process must install its subscriber");

    let config = ApiConfig::for_test();
    let state =
        AppState::in_memory(config.jwt_secret.clone()).with_public_url(config.public_url.clone());
    let router = app::build_app(state, &config);
    let uri = format!("/api/v1/auth/oauth/github/callback?state={STATE_CANARY}&code={CODE_CANARY}");
    let _callback_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("callback request must build"),
        )
        .await
        .expect("callback request must reach the router");
    let _unmatched_response = router
        .oneshot(
            Request::builder()
                .uri(format!("/{PATH_CANARY}?state={STATE_CANARY}"))
                .body(Body::empty())
                .expect("unmatched request must build"),
        )
        .await
        .expect("unmatched request must reach the router");
    drop(_callback_response);
    drop(_unmatched_response);

    provider.force_flush().expect("test spans must flush");
    let spans = exporter
        .get_finished_spans()
        .expect("test exporter must return finished spans");
    let request_span = spans
        .iter()
        .find(|span| span.name.as_ref() == "http.request")
        .expect("global HTTP request span must be exported");

    let route = request_span
        .attributes
        .iter()
        .find(|attribute| attribute.key.as_str() == "http.route")
        .map(|attribute| attribute.value.to_string())
        .expect("HTTP request span must carry its matched route");
    let expected_route = concat!("/api/v1/auth/oauth/", "{provider}", "/callback");
    assert_eq!(route, expected_route);
    assert!(
        spans.iter().any(|span| {
            span.name.as_ref() == "http.request"
                && span.attributes.iter().any(|attribute| {
                    attribute.key.as_str() == "http.route"
                        && attribute.value.to_string() == "<unmatched>"
                })
        }),
        "unmatched requests must use the fixed low-cardinality route marker"
    );

    let trace_dump = format!("{spans:?}");
    for forbidden in [STATE_CANARY, CODE_CANARY, PATH_CANARY, "?state=", "&code="] {
        assert!(
            !trace_dump.contains(forbidden),
            "OAuth callback trace leaked forbidden query material: {forbidden}"
        );
    }
}
