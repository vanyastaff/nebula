// HTTP resource example for Nebula.
//
// This example intentionally contains a full `Resource` implementation that was
// previously kept in `src/http.rs`, but now lives as an example reference.

use std::convert::TryFrom;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nebula_core::ResourceKey;
use nebula_resource::context::Context;
use nebula_resource::error::{Error, Result};
use nebula_resource::metadata::ResourceMetadata;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{CallPayload, CallRecord, CallStatus, ExecutionId, Recorder, WorkflowId};
use nebula_telemetry::NoopRecorder;
use reqwest::Client;
use serde::Deserialize;
use url::Url;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HttpResourceConfig {
    pub base_url: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub pool_max_idle_per_host: Option<usize>,
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

#[derive(Clone)]
pub struct HttpResourceInstance {
    client: Client,
    base_url: Option<Url>,
    resource_key: ResourceKey,
    recorder: Arc<dyn Recorder>,
}

impl HttpResourceInstance {
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn base_url(&self) -> Option<&Url> {
        self.base_url.as_ref()
    }

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

    #[allow(clippy::too_many_arguments)]
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
            trace_context: None,
        });
    }

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
}

pub struct HttpResource;

impl Resource for HttpResource {
    type Config = HttpResourceConfig;
    type Instance = HttpResourceInstance;

    fn declare_key() -> ResourceKey {
        ResourceKey::try_from("http.client").expect("HttpResource uses a valid resource key")
    }

    fn metadata(&self) -> ResourceMetadata {
        let key =
            ResourceKey::try_from("http.client").expect("HttpResource uses a valid resource key");
        ResourceMetadata::build(
            key.clone(),
            "HTTP Client",
            "Shared HTTP client with connection pooling and timeouts",
        )
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

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let pool = Pool::new(
        HttpResource,
        HttpResourceConfig {
            base_url: Some("https://example.com".to_string()),
            timeout_ms: Some(5_000),
            pool_max_idle_per_host: Some(8),
            user_agent: Some("nebula-resource-example/1.0".to_string()),
        },
        PoolConfig::default(),
    )?;

    let mut ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new());
    let recorder: Arc<dyn Recorder> = Arc::new(NoopRecorder);
    ctx = ctx.with_recorder(recorder);

    let (client, _wait) = pool.acquire(&ctx).await?;
    println!("HTTP resource acquired. base_url={:?}", client.base_url());

    drop(client);
    tokio::time::sleep(Duration::from_millis(20)).await;
    pool.shutdown().await?;
    Ok(())
}
