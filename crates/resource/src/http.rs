//! HTTP client resource for Nebula.
//!
//! Provides a pooled `reqwest::Client` instance with configurable
//! timeouts and connection settings, exposed as a `Resource` that
//! can be managed by the `Manager`. Supports Tier 2 tracing via
//! [`Recorder`](nebula_telemetry::Recorder) when enrichment is enabled.

use std::convert::TryFrom;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nebula_core::ResourceKey;
use nebula_telemetry::{CallPayload, CallRecord, CallStatus, Recorder};

use crate::context::Context;
use crate::error::{Error, Result};
use crate::metadata::{ResourceCategory, ResourceMetadata};
use crate::resource::{Config, Resource};
use reqwest::Client;
use serde::Deserialize;
use url::Url;

/// Configuration for the shared HTTP client resource.
#[derive(Debug, Clone, Deserialize)]
pub struct HttpResourceConfig {
    /// Optional base URL to prefix relative requests.
    pub base_url: Option<String>,
    /// Request timeout in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Maximum idle connections per host.
    #[serde(default)]
    pub pool_max_idle_per_host: Option<usize>,
    /// Optional user agent string for all requests.
    #[serde(default)]
    pub user_agent: Option<String>,
}

impl Config for HttpResourceConfig {
    fn validate(&self) -> Result<()> {
        if let Some(base) = &self.base_url
            && let Err(err) = Url::parse(base)
        {
            return Err(Error::configuration(format!(
                "invalid HttpResource base_url '{base}': {err}"
            )));
        }
        Ok(())
    }
}

/// Pooled HTTP client instance managed by the resource layer.
///
/// Use [`get`](Self::get) / [`post`](Self::post) for simple requests with optional
/// Tier 2 call recording. Use [`client`](Self::client) for full control.
#[derive(Clone)]
pub struct HttpResourceInstance {
    client: Client,
    base_url: Option<Url>,
    resource_key: ResourceKey,
    recorder: Arc<dyn Recorder>,
}

impl HttpResourceInstance {
    /// Access the underlying `reqwest::Client` for advanced use.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Base URL configured for this client, if any.
    pub fn base_url(&self) -> Option<&Url> {
        self.base_url.as_ref()
    }

    /// Resolve a path to a full URL (joining with base_url if set).
    fn resolve_url(&self, path: &str) -> Result<Url> {
        let path = path.trim_start_matches('/');
        if path.starts_with("http://") || path.starts_with("https://") {
            Url::parse(path).map_err(|e| Error::configuration(format!("invalid URL: {e}")))
        } else if let Some(base) = &self.base_url {
            base.join(path)
                .map_err(|e| Error::configuration(format!("invalid path: {e}")))
        } else {
            Url::parse(path).map_err(|e| Error::configuration(format!("invalid URL: {e}")))
        }
    }

    /// Record a call for Tier 2 enrichment when enabled.
    #[allow(clippy::too_many_arguments)] // all args map to CallRecord fields
    fn record_call(
        &self,
        operation: String,
        started_at: Instant,
        duration: Duration,
        status: CallStatus,
        request_summary: Option<String>,
        response_summary: Option<String>,
        status_code: Option<u16>,
    ) {
        if !self.recorder.is_enrichment_enabled() {
            return;
        }
        let mut metadata = std::collections::HashMap::new();
        if let Some(code) = status_code {
            metadata.insert("status_code".to_string(), code.to_string());
        }
        self.recorder.record_call(CallRecord {
            resource_key: self.resource_key.clone(),
            operation,
            started_at,
            duration,
            status,
            request: request_summary.map(|summary| CallPayload {
                summary,
                headers: None,
                body: None,
                size_bytes: None,
            }),
            response: response_summary.map(|summary| CallPayload {
                summary,
                headers: None,
                body: None,
                size_bytes: None,
            }),
            metadata,
        });
    }

    /// GET the given path (or full URL). Records a Tier 2 call when enrichment is enabled.
    pub async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let url = self.resolve_url(path)?;
        let operation = format!("GET {}", url.as_str());
        let started = Instant::now();
        let result = self.client.get(url.clone()).send().await;
        let duration = started.elapsed();
        match &result {
            Ok(res) => {
                let status_code = res.status().as_u16();
                let response_summary = format!(
                    "{} {}",
                    status_code,
                    res.status().canonical_reason().unwrap_or("")
                );
                self.record_call(
                    operation,
                    started,
                    duration,
                    CallStatus::Success,
                    Some(url.to_string()),
                    Some(response_summary),
                    Some(status_code),
                );
            }
            Err(e) => {
                self.record_call(
                    operation,
                    started,
                    duration,
                    CallStatus::Error(e.to_string()),
                    Some(url.to_string()),
                    None,
                    None,
                );
            }
        }
        result.map_err(|e| Error::Internal {
            resource_key: self.resource_key.clone(),
            message: format!("HTTP GET failed: {e}"),
            source: Some(Box::new(e)),
        })
    }

    /// POST to the given path with an optional body. Records a Tier 2 call when enrichment is enabled.
    pub async fn post(&self, path: &str, body: Option<&[u8]>) -> Result<reqwest::Response> {
        let url = self.resolve_url(path)?;
        let operation = format!("POST {}", url.as_str());
        let started = Instant::now();
        let mut req = self.client.post(url.clone());
        if let Some(b) = body {
            req = req.body(b.to_vec());
        }
        let result = req.send().await;
        let duration = started.elapsed();
        match &result {
            Ok(res) => {
                let status_code = res.status().as_u16();
                let response_summary = format!(
                    "{} {}",
                    status_code,
                    res.status().canonical_reason().unwrap_or("")
                );
                self.record_call(
                    operation,
                    started,
                    duration,
                    CallStatus::Success,
                    Some(url.to_string()),
                    Some(response_summary),
                    Some(status_code),
                );
            }
            Err(e) => {
                self.record_call(
                    operation,
                    started,
                    duration,
                    CallStatus::Error(e.to_string()),
                    Some(url.to_string()),
                    None,
                    None,
                );
            }
        }
        result.map_err(|e| Error::Internal {
            resource_key: self.resource_key.clone(),
            message: format!("HTTP POST failed: {e}"),
            source: Some(Box::new(e)),
        })
    }
}

/// HTTP client resource backed by `reqwest::Client`.
pub struct HttpResource;

impl Resource for HttpResource {
    type Config = HttpResourceConfig;
    type Instance = HttpResourceInstance;
    fn metadata(&self) -> ResourceMetadata {
        let key =
            ResourceKey::try_from("http.client").expect("HttpResource uses a valid resource key");
        ResourceMetadata::build(
            key.clone(),
            "HTTP Client",
            "Shared HTTP client with connection pooling and timeouts",
        )
        .category(ResourceCategory::Http)
        .icon("http")
        .tag("category:network")
        .tag("protocol:http")
        .build()
    }

    fn create(
        &self,
        config: &Self::Config,
        ctx: &Context,
    ) -> impl std::future::Future<Output = Result<Self::Instance>> + Send {
        let cfg = config.clone();
        let resource_key = self.metadata().key.clone();
        let recorder = ctx.recorder();
        async move {
            let mut builder = Client::builder();

            if let Some(timeout_ms) = cfg.timeout_ms {
                builder = builder.timeout(Duration::from_millis(timeout_ms));
            }

            if let Some(max_idle) = cfg.pool_max_idle_per_host {
                builder = builder.pool_max_idle_per_host(max_idle);
            }

            if let Some(ua) = cfg.user_agent {
                builder = builder.user_agent(ua);
            }

            let client = builder.build().map_err(|err| {
                Error::configuration(format!("failed to build HTTP client: {err}"))
            })?;

            let base_url = match cfg.base_url {
                Some(url) => Some(Url::parse(&url).map_err(|err| {
                    Error::configuration(format!("invalid HttpResource base_url '{url}': {err}"))
                })?),
                None => None,
            };

            Ok(HttpResourceInstance {
                client,
                base_url,
                resource_key,
                recorder,
            })
        }
    }
}
