//! HTTP client resource implementation

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
    traits::{HealthCheckable, HealthStatus, Poolable, PoolConfig},
};

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
    /// Retry configuration
    pub retry_config: RetryConfig,
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
            retry_config: RetryConfig::default(),
        }
    }
}

impl ResourceConfig for HttpClientConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.timeout.is_zero() {
            return Err(ResourceError::configuration("Timeout cannot be zero"));
        }
        if self.connect_timeout.is_zero() {
            return Err(ResourceError::configuration("Connect timeout cannot be zero"));
        }
        if self.max_connections_per_host == 0 {
            return Err(ResourceError::configuration("Max connections per host cannot be zero"));
        }
        if self.max_redirects > 50 {
            return Err(ResourceError::configuration("Max redirects cannot exceed 50"));
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
        // Merge other fields as needed
    }
}

/// TLS configuration for HTTP client
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TlsConfig {
    /// Whether to verify SSL certificates
    pub verify_ssl: bool,
    /// CA certificate bundle path
    pub ca_bundle_path: Option<String>,
    /// Client certificate path
    pub client_cert_path: Option<String>,
    /// Client private key path
    pub client_key_path: Option<String>,
    /// Minimum TLS version
    pub min_tls_version: TlsVersion,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            verify_ssl: true,
            ca_bundle_path: None,
            client_cert_path: None,
            client_key_path: None,
            min_tls_version: TlsVersion::V1_2,
        }
    }
}

/// TLS version specification
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum TlsVersion {
    /// TLS 1.0 (deprecated)
    V1_0,
    /// TLS 1.1 (deprecated)
    V1_1,
    /// TLS 1.2
    V1_2,
    /// TLS 1.3
    V1_3,
}

/// Retry configuration for HTTP requests
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: usize,
    /// Base delay between retries
    pub base_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Backoff strategy
    pub backoff_strategy: BackoffStrategy,
    /// HTTP status codes that should trigger a retry
    pub retryable_status_codes: Vec<u16>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_strategy: BackoffStrategy::Exponential,
            retryable_status_codes: vec![408, 429, 502, 503, 504],
        }
    }
}

/// Backoff strategy for retries
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed,
    /// Linear increase in delay
    Linear,
    /// Exponential increase in delay
    Exponential,
    /// Exponential with jitter
    ExponentialWithJitter,
}

/// HTTP client resource instance
pub struct HttpClientInstance {
    /// Instance metadata
    instance_id: Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<LifecycleState>,

    /// The actual HTTP client (would use reqwest, hyper, etc.)
    client: Arc<HttpClient>,
    config: HttpClientConfig,
}

impl HttpClientInstance {
    /// Create a new HTTP client instance
    pub fn new(
        resource_id: ResourceId,
        context: ResourceContext,
        config: HttpClientConfig,
    ) -> ResourceResult<Self> {
        let client = HttpClient::new(&config)?;

        Ok(Self {
            instance_id: Uuid::new_v4(),
            resource_id,
            context,
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(LifecycleState::Ready),
            client: Arc::new(client),
            config,
        })
    }

    /// Get the underlying HTTP client
    pub fn client(&self) -> &HttpClient {
        &self.client
    }

    /// Get the configuration
    pub fn config(&self) -> &HttpClientConfig {
        &self.config
    }

    /// Make an HTTP GET request
    pub async fn get(&self, url: &str) -> ResourceResult<HttpResponse> {
        self.touch();
        self.client.get(url).await
    }

    /// Make an HTTP POST request
    pub async fn post(&self, url: &str, body: Vec<u8>) -> ResourceResult<HttpResponse> {
        self.touch();
        self.client.post(url, body).await
    }

    /// Make an HTTP PUT request
    pub async fn put(&self, url: &str, body: Vec<u8>) -> ResourceResult<HttpResponse> {
        self.touch();
        self.client.put(url, body).await
    }

    /// Make an HTTP DELETE request
    pub async fn delete(&self, url: &str) -> ResourceResult<HttpResponse> {
        self.touch();
        self.client.delete(url).await
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
            match self.client.head(base_url).await {
                Ok(_) => Ok(HealthStatus::Healthy),
                Err(e) => Ok(HealthStatus::Unhealthy {
                    reason: format!("Health check failed: {}", e),
                    recoverable: true,
                }),
            }
        } else {
            // No base URL configured, assume healthy
            Ok(HealthStatus::Healthy)
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
        matches!(self.lifecycle_state(), LifecycleState::Ready | LifecycleState::Idle)
    }

    fn prepare_for_pool(&mut self) -> ResourceResult<()> {
        // Reset any request-specific state
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
pub struct HttpClientResource;

impl HttpClientResource {
    /// Create a new HTTP client resource
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
            "HTTP client for making web requests".to_string(),
        )
        .with_tag("type", "http_client")
        .with_tag("category", "network")
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

        HttpClientInstance::new(
            self.metadata().id,
            context.clone(),
            config.clone(),
        )
    }

    async fn cleanup(&self, _instance: Self::Instance) -> ResourceResult<()> {
        // HTTP clients typically don't need explicit cleanup
        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        // Check if the instance is in a valid state
        Ok(matches!(
            instance.lifecycle_state(),
            LifecycleState::Ready | LifecycleState::Idle | LifecycleState::InUse
        ))
    }
}

// Simplified HTTP client implementation
// In a real implementation, this would use reqwest, hyper, or similar
struct HttpClient {
    config: HttpClientConfig,
}

impl HttpClient {
    fn new(config: &HttpClientConfig) -> ResourceResult<Self> {
        // Validate and create the client
        Ok(Self {
            config: config.clone(),
        })
    }

    async fn get(&self, _url: &str) -> ResourceResult<HttpResponse> {
        // Implementation would use actual HTTP library
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        })
    }

    async fn post(&self, _url: &str, _body: Vec<u8>) -> ResourceResult<HttpResponse> {
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        })
    }

    async fn put(&self, _url: &str, _body: Vec<u8>) -> ResourceResult<HttpResponse> {
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        })
    }

    async fn delete(&self, _url: &str) -> ResourceResult<HttpResponse> {
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        })
    }

    async fn head(&self, _url: &str) -> ResourceResult<HttpResponse> {
        Ok(HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        })
    }
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
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Get response body as string
    pub fn text(&self) -> ResourceResult<String> {
        String::from_utf8(self.body.clone()).map_err(|e| {
            ResourceError::internal("http_client", format!("Failed to parse response as UTF-8: {}", e))
        })
    }

    /// Get response body as JSON
    pub fn json<T>(&self) -> ResourceResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let text = self.text()?;
        serde_json::from_str(&text).map_err(|e| {
            ResourceError::internal("http_client", format!("Failed to parse response as JSON: {}", e))
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
        config2.default_headers.insert("Authorization".to_string(), "Bearer token".to_string());

        config1.merge(config2);

        assert_eq!(config1.base_url, Some("https://api.example.com".to_string()));
        assert_eq!(config1.timeout, Duration::from_secs(60));
        assert_eq!(config1.default_headers.get("Authorization"), Some(&"Bearer token".to_string()));
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
}