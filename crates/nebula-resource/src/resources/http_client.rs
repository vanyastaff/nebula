//! HTTP client resource implementation with connection pooling and resilience patterns

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    lifecycle::LifecycleState,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
    traits::{HealthCheckable, HealthStatus, PoolConfig, Poolable},
};

use nebula_resilience::{CircuitBreaker, CircuitBreakerConfig};

/// Configuration for HTTP client resource
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct HttpClientConfig {
    /// Base URL for the HTTP client
    pub base_url: Option<String>,
    /// Request timeout
    pub timeout: Duration,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Maximum number of connections per host
    pub max_connections_per_host: usize,
    /// Keep-alive timeout
    pub keep_alive_timeout: Duration,
    /// Default headers to include in requests
    pub default_headers: HashMap<String, String>,
    /// Whether to follow redirects
    pub follow_redirects: bool,
    /// Maximum number of redirects to follow
    pub max_redirects: usize,
    /// User agent string
    pub user_agent: Option<String>,
    /// TLS configuration
    pub tls_config: TlsConfig,
    /// Circuit breaker configuration (from nebula-resilience)
    pub circuit_breaker_config: Option<CircuitBreakerConfig>,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            max_connections_per_host: 100,
            keep_alive_timeout: Duration::from_secs(90),
            default_headers: HashMap::new(),
            follow_redirects: true,
            max_redirects: 10,
            user_agent: Some("nebula-resource/1.0".to_string()),
            tls_config: TlsConfig::default(),
            circuit_breaker_config: Some(CircuitBreakerConfig::default()),
        }
    }
}

impl ResourceConfig for HttpClientConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.timeout.is_zero() {
            return Err(ResourceError::configuration("Timeout cannot be zero"));
        }
        if self.connect_timeout.is_zero() {
            return Err(ResourceError::configuration(
                "Connect timeout cannot be zero",
            ));
        }
        if self.max_connections_per_host == 0 {
            return Err(ResourceError::configuration(
                "Max connections per host cannot be zero",
            ));
        }
        if self.max_redirects > 50 {
            return Err(ResourceError::configuration(
                "Max redirects cannot exceed 50",
            ));
        }
        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if other.base_url.is_some() {
            self.base_url = other.base_url;
        }
        if !other.timeout.is_zero() {
            self.timeout = other.timeout;
        }
        if !other.connect_timeout.is_zero() {
            self.connect_timeout = other.connect_timeout;
        }
        if other.max_connections_per_host > 0 {
            self.max_connections_per_host = other.max_connections_per_host;
        }
        self.default_headers.extend(other.default_headers);
        // Retry strategy removed - use resilience patterns externally
        if other.circuit_breaker_config.is_some() {
            self.circuit_breaker_config = other.circuit_breaker_config;
        }
    }
}

/// TLS configuration for HTTP client
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TlsConfig {
    /// Whether to verify SSL certificates
    pub verify_ssl: bool,
    /// Minimum TLS version
    pub min_tls_version: TlsVersion,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            verify_ssl: true,
            min_tls_version: TlsVersion::V1_2,
        }
    }
}

/// TLS version specification
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TlsVersion {
    /// TLS 1.2
    V1_2,
    /// TLS 1.3
    V1_3,
}

/// HTTP client resource instance
#[derive(Debug)]
pub struct HttpClientInstance {
    /// Instance metadata
    instance_id: Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<LifecycleState>,

    /// The actual HTTP client
    #[cfg(feature = "http-client")]
    client: Arc<reqwest::Client>,

    /// Circuit breaker for resilience
    circuit_breaker: Option<Arc<CircuitBreaker>>,

    config: HttpClientConfig,
}

impl HttpClientInstance {
    /// Create a new HTTP client instance
    pub fn new(
        resource_id: ResourceId,
        context: ResourceContext,
        config: HttpClientConfig,
    ) -> ResourceResult<Self> {
        #[cfg(feature = "http-client")]
        let client = {
            let mut builder = reqwest::Client::builder()
                .timeout(config.timeout)
                .connect_timeout(config.connect_timeout)
                .pool_max_idle_per_host(config.max_connections_per_host)
                .pool_idle_timeout(config.keep_alive_timeout)
                .redirect(if config.follow_redirects {
                    reqwest::redirect::Policy::limited(config.max_redirects)
                } else {
                    reqwest::redirect::Policy::none()
                });

            if let Some(ref user_agent) = config.user_agent {
                builder = builder.user_agent(user_agent);
            }

            // TLS configuration
            if !config.tls_config.verify_ssl {
                builder = builder.danger_accept_invalid_certs(true);
            }

            // Build client
            builder.build().map_err(|e| {
                ResourceError::initialization(
                    "http_client:1.0",
                    format!("Failed to create HTTP client: {}", e),
                )
            })?
        };

        // Create circuit breaker if configured
        let circuit_breaker = config
            .circuit_breaker_config
            .as_ref()
            .map(|cb_config| CircuitBreaker::with_config(cb_config.clone()))
            .transpose()
            .map_err(|e| {
                ResourceError::initialization(
                    "http_client:1.0",
                    format!("Failed to create circuit breaker: {}", e),
                )
            })?
            .map(Arc::new);

        Ok(Self {
            instance_id: Uuid::new_v4(),
            resource_id,
            context,
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(LifecycleState::Ready),
            #[cfg(feature = "http-client")]
            client: Arc::new(client),
            circuit_breaker,
            config,
        })
    }

    /// Make an HTTP GET request
    pub async fn get(&self, url: &str) -> ResourceResult<HttpResponse> {
        self.touch();
        self.execute_with_resilience(|| async {
            self.request(HttpMethod::Get, url, None, None).await
        })
        .await
    }

    /// Make an HTTP GET request with headers
    pub async fn get_with_headers(
        &self,
        url: &str,
        headers: HashMap<String, String>,
    ) -> ResourceResult<HttpResponse> {
        self.touch();
        self.execute_with_resilience(|| async {
            self.request(HttpMethod::Get, url, Some(headers.clone()), None)
                .await
        })
        .await
    }

    /// Make an HTTP POST request
    pub async fn post(&self, url: &str, body: Vec<u8>) -> ResourceResult<HttpResponse> {
        self.touch();
        self.execute_with_resilience(|| async {
            self.request(HttpMethod::Post, url, None, Some(body.clone()))
                .await
        })
        .await
    }

    /// Make an HTTP POST request with JSON body
    #[cfg(feature = "serde")]
    pub async fn post_json<T: Serialize>(
        &self,
        url: &str,
        body: &T,
    ) -> ResourceResult<HttpResponse> {
        self.touch();
        let json = serde_json::to_vec(body).map_err(|e| {
            ResourceError::internal("http_client:1.0", format!("Failed to serialize JSON: {e}"))
        })?;

        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        self.execute_with_resilience(|| async {
            self.request(
                HttpMethod::Post,
                url,
                Some(headers.clone()),
                Some(json.clone()),
            )
            .await
        })
        .await
    }

    /// Make an HTTP PUT request
    pub async fn put(&self, url: &str, body: Vec<u8>) -> ResourceResult<HttpResponse> {
        self.touch();
        self.execute_with_resilience(|| async {
            self.request(HttpMethod::Put, url, None, Some(body.clone()))
                .await
        })
        .await
    }

    /// Make an HTTP DELETE request
    pub async fn delete(&self, url: &str) -> ResourceResult<HttpResponse> {
        self.touch();
        self.execute_with_resilience(|| async {
            self.request(HttpMethod::Delete, url, None, None).await
        })
        .await
    }

    /// Make an HTTP HEAD request
    pub async fn head(&self, url: &str) -> ResourceResult<HttpResponse> {
        self.touch();
        // HEAD requests should not use retry/circuit breaker
        self.request(HttpMethod::Head, url, None, None).await
    }

    /// Execute a request with resilience patterns (retry + circuit breaker)
    async fn execute_with_resilience<F, Fut>(&self, mut f: F) -> ResourceResult<HttpResponse>
    where
        F: FnMut() -> Fut + Send,
        Fut: Future<Output = ResourceResult<HttpResponse>> + Send,
    {
        // Check circuit breaker before request
        if let Some(ref cb) = self.circuit_breaker {
            if cb.is_open().await {
                return Err(ResourceError::CircuitBreakerOpen {
                    resource_id: "http_client:1.0".to_string(),
                    retry_after_ms: None,
                });
            }
        }

        // Execute request
        let result = f().await;

        // Record success/failure in circuit breaker
        if let Some(ref cb) = self.circuit_breaker {
            match &result {
                Ok(_) => cb.record_success().await,
                Err(_) => cb.record_failure().await,
            }
        }

        result
    }

    /// Make a raw HTTP request
    #[cfg(feature = "http-client")]
    async fn request(
        &self,
        method: HttpMethod,
        url: &str,
        headers: Option<HashMap<String, String>>,
        body: Option<Vec<u8>>,
    ) -> ResourceResult<HttpResponse> {
        // Build full URL
        let full_url = if let Some(ref base) = self.config.base_url {
            if url.starts_with("http://") || url.starts_with("https://") {
                url.to_string()
            } else {
                format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    url.trim_start_matches('/')
                )
            }
        } else {
            url.to_string()
        };

        // Build request
        let mut req = match method {
            HttpMethod::Get => self.client.get(&full_url),
            HttpMethod::Post => self.client.post(&full_url),
            HttpMethod::Put => self.client.put(&full_url),
            HttpMethod::Delete => self.client.delete(&full_url),
            HttpMethod::Head => self.client.head(&full_url),
            HttpMethod::Patch => self.client.patch(&full_url),
        };

        // Add default headers
        for (key, value) in &self.config.default_headers {
            req = req.header(key, value);
        }

        // Add custom headers
        if let Some(headers) = headers {
            for (key, value) in headers {
                req = req.header(key, value);
            }
        }

        // Add body if present
        if let Some(body) = body {
            req = req.body(body);
        }

        // Execute request
        let response = req.send().await.map_err(|e| {
            if e.is_timeout() {
                ResourceError::Timeout {
                    resource_id: "http_client:1.0".to_string(),
                    timeout_ms: self.config.timeout.as_millis() as u64,
                    operation: "http_request".to_string(),
                }
            } else if e.is_connect() {
                ResourceError::internal("http_client:1.0", format!("Connection failed: {}", e))
            } else {
                ResourceError::internal("http_client:1.0", format!("Request failed: {}", e))
            }
        })?;

        // Convert response
        let status = response.status().as_u16();
        let mut headers_map = HashMap::new();

        for (key, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                headers_map.insert(key.to_string(), value_str.to_string());
            }
        }

        let body = response.bytes().await.map_err(|e| {
            ResourceError::internal(
                "http_client:1.0",
                format!("Failed to read response body: {}", e),
            )
        })?;

        Ok(HttpResponse {
            status,
            headers: headers_map,
            body: body.to_vec(),
        })
    }

    /// Make a raw HTTP request (non-reqwest fallback)
    #[cfg(not(feature = "http-client"))]
    async fn request(
        &self,
        method: HttpMethod,
        _url: &str,
        _headers: Option<HashMap<String, String>>,
        _body: Option<Vec<u8>>,
    ) -> ResourceResult<HttpResponse> {
        let _ = method;
        Err(ResourceError::configuration(
            "HTTP client feature not enabled. Enable 'http-client' feature to use HTTP client",
        ))
    }

    /// Get circuit breaker state
    pub async fn circuit_state(&self) -> Option<nebula_resilience::CircuitState> {
        if let Some(ref cb) = self.circuit_breaker {
            Some(cb.state().await)
        } else {
            None
        }
    }

    /// Reset the circuit breaker
    pub async fn reset_circuit(&self) {
        if let Some(ref cb) = self.circuit_breaker {
            cb.reset().await;
        }
    }
}

impl ResourceInstance for HttpClientInstance {
    fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }

    fn lifecycle_state(&self) -> LifecycleState {
        *self.state.read()
    }

    fn context(&self) -> &ResourceContext {
        &self.context
    }

    fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    fn last_accessed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_accessed.lock()
    }

    fn touch(&self) {
        *self.last_accessed.lock() = Some(chrono::Utc::now());
    }
}

#[async_trait]
impl HealthCheckable for HttpClientInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        // Perform a simple health check (e.g., HEAD request to base URL)
        if let Some(ref base_url) = self.config.base_url {
            let start = std::time::Instant::now();
            match self.head(base_url).await {
                Ok(response) => {
                    let latency = start.elapsed();
                    if response.is_success() {
                        Ok(HealthStatus::healthy().with_latency(latency))
                    } else {
                        Ok(HealthStatus::unhealthy(format!(
                            "Health check returned status {}",
                            response.status
                        ))
                        .with_latency(latency))
                    }
                }
                Err(e) => {
                    let latency = start.elapsed();
                    Ok(HealthStatus::unhealthy(format!("Health check failed: {e}"))
                        .with_latency(latency))
                }
            }
        } else {
            // No base URL configured, check circuit breaker state
            if let Some(ref cb) = self.circuit_breaker {
                let state = cb.state().await;
                let is_closed = cb.is_closed().await;
                let mut status = HealthStatus::healthy();
                status = status.with_metadata("circuit_state", format!("{state:?}"));

                if is_closed {
                    Ok(status)
                } else {
                    Ok(HealthStatus::unhealthy(format!(
                        "Circuit breaker is {state:?}"
                    )))
                }
            } else {
                Ok(HealthStatus::healthy())
            }
        }
    }

    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn health_check_timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

impl Poolable for HttpClientInstance {
    fn pool_config(&self) -> PoolConfig {
        PoolConfig {
            min_size: 1,
            max_size: 10,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(300),
            max_lifetime: Duration::from_secs(3600),
            validation_interval: Duration::from_secs(60),
        }
    }

    fn is_valid_for_pool(&self) -> bool {
        matches!(
            self.lifecycle_state(),
            LifecycleState::Ready | LifecycleState::Idle
        )
    }

    fn prepare_for_pool(&mut self) -> ResourceResult<()> {
        *self.state.write() = LifecycleState::Idle;
        Ok(())
    }

    fn prepare_for_acquisition(&mut self) -> ResourceResult<()> {
        *self.state.write() = LifecycleState::InUse;
        self.touch();
        Ok(())
    }
}

/// HTTP client resource
#[derive(Debug)]
pub struct HttpClientResource;

impl HttpClientResource {
    /// Create a new HTTP client resource
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for HttpClientResource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Resource for HttpClientResource {
    type Config = HttpClientConfig;
    type Instance = HttpClientInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("http_client", "1.0"),
            "HTTP client for making web requests with resilience patterns".to_string(),
        )
        .with_tag("type", "http_client")
        .with_tag("category", "network")
        .with_tag("resilience", "circuit_breaker,retry,timeout")
        .poolable()
        .health_checkable()
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        HttpClientInstance::new(self.metadata().id, context.clone(), config.clone())
    }

    async fn cleanup(&self, _instance: Self::Instance) -> ResourceResult<()> {
        // HTTP clients don't need explicit cleanup - connections are managed by reqwest
        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        Ok(matches!(
            instance.lifecycle_state(),
            LifecycleState::Ready | LifecycleState::Idle | LifecycleState::InUse
        ))
    }
}

/// HTTP method enumeration
#[derive(Debug, Clone, Copy)]
enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Patch,
}

/// HTTP response
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Check if the response indicates success (2xx status code)
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Get response body as string
    pub fn text(&self) -> ResourceResult<String> {
        String::from_utf8(self.body.clone()).map_err(|e| {
            ResourceError::internal(
                "http_client",
                format!("Failed to parse response as UTF-8: {e}"),
            )
        })
    }

    /// Get response body as JSON
    #[cfg(feature = "serde")]
    pub fn json<T>(&self) -> ResourceResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let text = self.text()?;
        serde_json::from_str(&text).map_err(|e| {
            ResourceError::internal(
                "http_client",
                format!("Failed to parse response as JSON: {e}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_client_config_validation() {
        let mut config = HttpClientConfig::default();
        assert!(config.validate().is_ok());

        config.timeout = Duration::ZERO;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_http_client_config_merge() {
        let mut config1 = HttpClientConfig::default();
        let mut config2 = HttpClientConfig::default();

        config2.base_url = Some("https://api.example.com".to_string());
        config2.timeout = Duration::from_secs(60);
        config2
            .default_headers
            .insert("Authorization".to_string(), "Bearer token".to_string());

        config1.merge(config2);

        assert_eq!(
            config1.base_url,
            Some("https://api.example.com".to_string())
        );
        assert_eq!(config1.timeout, Duration::from_secs(60));
        assert_eq!(
            config1.default_headers.get("Authorization"),
            Some(&"Bearer token".to_string())
        );
    }

    #[tokio::test]
    async fn test_http_client_resource() {
        let resource = HttpClientResource::new();
        let metadata = resource.metadata();

        assert_eq!(metadata.id.name, "http_client");
        assert!(metadata.poolable);
        assert!(metadata.health_checkable);

        let config = HttpClientConfig::default();
        let context = ResourceContext::new(
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        );

        let instance = resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.lifecycle_state(), LifecycleState::Ready);

        let is_valid = resource.validate_instance(&instance).await.unwrap();
        assert!(is_valid);
    }

    #[test]
    fn test_http_response() {
        let response = HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: b"Hello, World!".to_vec(),
        };

        assert!(response.is_success());
        assert_eq!(response.text().unwrap(), "Hello, World!");

        let error_response = HttpResponse {
            status: 404,
            headers: HashMap::new(),
            body: Vec::new(),
        };

        assert!(!error_response.is_success());
    }

    // Retry strategy removed - use nebula-resilience patterns externally

    #[test]
    fn test_circuit_breaker_integration() {
        let config = HttpClientConfig::default();
        assert!(config.circuit_breaker_config.is_some());

        let context = ResourceContext::new(
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        );

        let instance =
            HttpClientInstance::new(ResourceId::new("http_client", "1.0"), context, config)
                .unwrap();

        assert!(instance.circuit_breaker.is_some());
    }

    #[test]
    fn test_resilience_disabled() {
        let mut config = HttpClientConfig::default();
        config.circuit_breaker_config = None;

        let context = ResourceContext::new(
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
            "test".to_string(),
        );

        let instance =
            HttpClientInstance::new(ResourceId::new("http_client", "1.0"), context, config)
                .unwrap();

        assert!(instance.circuit_breaker.is_none());
    }
}
