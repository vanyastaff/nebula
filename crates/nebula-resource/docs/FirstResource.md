---
title: Your First Resource
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---


# Creating Your First Resource

> Пошаговое руководство по созданию кастомного ресурса

## Overview

В этом руководстве мы создадим ресурс для работы с Weather API:

- Определим структуру ресурса
- Настроим конфигурацию
- Реализуем health checks
- Добавим context awareness
- Протестируем ресурс

## Prerequisites

```toml
# Cargo.toml
[dependencies]
nebula-resource = { version = "0.2", features = ["derive", "testing"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
reqwest = { version = "0.11", features = ["json"] }
tokio = { version = "1", features = ["full"] }
thiserror = "1.0"
chrono = "0.4"
```

## Step 1: Define Resource Structure

### Basic Resource Definition

```rust
use nebula_resource::prelude::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use std::collections::HashMap;

/// Weather API Resource for getting weather data
#[derive(Resource)]
#[resource(
    id = "weather_api",
    name = "Weather API Client",
    description = "Client for OpenWeatherMap API",
    lifecycle = "global",
    capabilities = ["weather.current", "weather.forecast"],
    credentials = ["openweather_api_key"]
)]
pub struct WeatherApiResource;
```

### Resource Instance

```rust
/// Actual instance that will be created and used
pub struct WeatherApiInstance {
    id: ResourceInstanceId,
    client: reqwest::Client,
    config: WeatherApiConfig,
    api_key: String,
    metrics: ResourceMetrics,
    cache: HashMap<String, CachedWeather>,
    rate_limiter: RateLimiter,
    last_health_check: Option<Instant>,
}

/// Cached weather data
struct CachedWeather {
    data: WeatherData,
    cached_at: Instant,
}

/// Rate limiter for API calls
struct RateLimiter {
    calls: Vec<Instant>,
    max_calls_per_minute: u32,
}

impl RateLimiter {
    fn new(max_calls: u32) -> Self {
        Self {
            calls: Vec::new(),
            max_calls_per_minute: max_calls,
        }
    }
    
    async fn wait_if_needed(&mut self) {
        let now = Instant::now();
        let one_minute_ago = now - Duration::from_secs(60);
        
        // Remove old calls
        self.calls.retain(|&call| call > one_minute_ago);
        
        // Check if we need to wait
        if self.calls.len() >= self.max_calls_per_minute as usize {
            let oldest = self.calls[0];
            let wait_time = Duration::from_secs(60) - (now - oldest);
            tokio::time::sleep(wait_time).await;
            
            // Clean up after waiting
            self.calls.clear();
        }
        
        self.calls.push(now);
    }
}
```

## Step 2: Define Configuration

### Configuration Structure

```rust
/// Configuration for Weather API resource
#[derive(ResourceConfig, Serialize, Deserialize, Clone, Debug)]
pub struct WeatherApiConfig {
    /// API endpoint URL
    #[validate(url)]
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    
    /// Request timeout in seconds
    #[validate(range = "1..=60")]
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    
    /// Maximum retries for failed requests
    #[validate(range = "0..=5")]
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    
    /// Rate limit (requests per minute)
    #[validate(range = "1..=100")]
    #[serde(default = "default_rate_limit")]
    pub rate_limit: u32,
    
    /// Credential ID for API key
    #[credential(id = "openweather_api_key")]
    pub api_key_credential: String,
    
    /// Enable caching
    #[serde(default = "default_cache_enabled")]
    pub cache_enabled: bool,
    
    /// Cache TTL in seconds
    #[validate(range = "60..=3600")]
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_secs: u64,
    
    /// Units for temperature
    #[validate(one_of = ["metric", "imperial", "kelvin"])]
    #[serde(default = "default_units")]
    pub units: String,
}

// Default values
fn default_endpoint() -> String {
    "https://api.openweathermap.org/data/2.5".into()
}

fn default_timeout() -> u64 { 30 }
fn default_max_retries() -> u32 { 3 }
fn default_rate_limit() -> u32 { 60 }
fn default_cache_enabled() -> bool { true }
fn default_cache_ttl() -> u64 { 600 }
fn default_units() -> String { "metric".into() }
```

### Configuration File

```toml
# configs/weather_api.toml
[weather_api]
endpoint = "https://api.openweathermap.org/data/2.5"
timeout_secs = 30
max_retries = 3
rate_limit = 60
cache_enabled = true
cache_ttl_secs = 600
units = "metric"

[credentials]
api_key_credential = "openweather_api_key"

# Environment-specific overrides
[development]
cache_enabled = false
rate_limit = 10

[production]
cache_ttl_secs = 1800
max_retries = 5
```

## Step 3: Implement Resource Trait

### Resource Implementation

```rust
#[async_trait]
impl Resource for WeatherApiResource {
    type Config = WeatherApiConfig;
    type Instance = WeatherApiInstance;
    
    fn metadata() -> ResourceMetadata {
        ResourceMetadata {
            id: "weather_api".into(),
            name: "Weather API Client".into(),
            description: Some("OpenWeatherMap API client with caching".into()),
            version: Version::new(1, 0, 0),
            lifecycle: ResourceLifecycle::Global,
            capabilities: vec![
                "weather.current".into(),
                "weather.forecast".into(),
                "weather.historical".into(),
            ],
            required_credentials: vec!["openweather_api_key".into()],
            dependencies: vec![],
            tags: vec!["external-api".into(), "weather".into()],
        }
    }
    
    async fn create(
        config: Self::Config,
        context: &ResourceContext,
    ) -> Result<Self::Instance, ResourceError> {
        // Validate configuration
        Self::validate_config(&config)?;
        
        // Get API key from credentials
        let api_key = context
            .get_credential(&config.api_key_credential)
            .await
            .map_err(|e| ResourceError::MissingCredential(e.to_string()))?
            .expose_secret();
        
        // Create HTTP client with configuration
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent("nebula-resource/1.0")
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ResourceError::CreationFailed(e.to_string()))?;
        
        // Test connection with a simple request
        let test_url = format!(
            "{}/weather?q=London&appid={}&units={}",
            config.endpoint, api_key, config.units
        );
        
        let test_response = client
            .get(&test_url)
            .send()
            .await
            .map_err(|e| ResourceError::CreationFailed(
                format!("Failed to connect to Weather API: {}", e)
            ))?;
        
        if !test_response.status().is_success() {
            return Err(ResourceError::CreationFailed(
                format!("Weather API returned error: {}", test_response.status())
            ));
        }
        
        context.log_info("Weather API resource created successfully");
        
        Ok(WeatherApiInstance {
            id: ResourceInstanceId::new(),
            client,
            config: config.clone(),
            api_key,
            metrics: ResourceMetrics::new("weather_api"),
            cache: HashMap::new(),
            rate_limiter: RateLimiter::new(config.rate_limit),
            last_health_check: None,
        })
    }
    
    fn validate_config(config: &Self::Config) -> Result<(), ResourceError> {
        if config.endpoint.is_empty() {
            return Err(ResourceError::InvalidConfig {
                field: "endpoint".into(),
                reason: "Endpoint cannot be empty".into(),
            });
        }
        
        if config.timeout_secs == 0 {
            return Err(ResourceError::InvalidConfig {
                field: "timeout_secs".into(),
                reason: "Timeout must be at least 1 second".into(),
            });
        }
        
        if config.rate_limit == 0 {
            return Err(ResourceError::InvalidConfig {
                field: "rate_limit".into(),
                reason: "Rate limit must be at least 1".into(),
            });
        }
        
        Ok(())
    }
    
    fn required_credentials() -> Vec<&'static str> {
        vec!["openweather_api_key"]
    }
    
    fn supports_pooling(&self) -> bool {
        false // API clients typically don't need pooling
    }
}
```

## Step 4: Implement Resource Instance

### Instance Implementation

```rust
#[async_trait]
impl ResourceInstance for WeatherApiInstance {
    fn id(&self) -> &ResourceInstanceId {
        &self.id
    }
    
    async fn health_check(&self) -> Result<HealthStatus, ResourceError> {
        // Rate limit health checks
        if let Some(last_check) = self.last_health_check {
            if last_check.elapsed() < Duration::from_secs(30) {
                return Ok(HealthStatus::Healthy);
            }
        }
        
        // Perform health check
        let health_url = format!(
            "{}/weather?q=London&appid={}&units={}",
            self.config.endpoint, self.api_key, self.config.units
        );
        
        let start = Instant::now();
        match self.client.get(&health_url).send().await {
            Ok(response) if response.status().is_success() => {
                let latency = start.elapsed();
                self.metrics.record_health_check(true, latency);
                
                // Check performance
                if latency > Duration::from_secs(5) {
                    Ok(HealthStatus::Degraded {
                        reason: "High latency detected".into(),
                        performance_impact: 0.5,
                    })
                } else {
                    Ok(HealthStatus::Healthy)
                }
            }
            Ok(response) => {
                self.metrics.record_health_check(false, start.elapsed());
                Ok(HealthStatus::Unhealthy {
                    reason: format!("API returned status: {}", response.status()),
                    recoverable: true,
                })
            }
            Err(e) => {
                self.metrics.record_health_check(false, start.elapsed());
                Ok(HealthStatus::Unhealthy {
                    reason: format!("Connection failed: {}", e),
                    recoverable: true,
                })
            }
        }
    }
    
    async fn cleanup(&mut self) -> Result<(), ResourceError> {
        // Cleanup any resources
        self.cache.clear();
        self.metrics.flush().await;
        Ok(())
    }
    
    fn metrics(&self) -> ResourceMetrics {
        self.metrics.clone()
    }
    
    fn is_reusable(&self) -> bool {
        true // Can be reused across actions
    }
    
    async fn reset(&mut self) -> Result<(), ResourceError> {
        // Reset for reuse in pool
        self.cache.clear();
        self.metrics.reset_session_metrics();
        Ok(())
    }
}
```

## Step 5: Add Business Methods

### Weather Data Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherData {
    pub location: Location,
    pub current: CurrentWeather,
    pub forecast: Option<Vec<ForecastItem>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub name: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentWeather {
    pub temperature: f64,
    pub feels_like: f64,
    pub humidity: u32,
    pub pressure: u32,
    pub wind_speed: f64,
    pub description: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastItem {
    pub datetime: chrono::DateTime<chrono::Utc>,
    pub temperature: f64,
    pub description: String,
    pub precipitation: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum WeatherError {
    #[error("API error: {0}")]
    ApiError(String),
    
    #[error("Parse error: {0}")]
    ParseError(String),
    
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    
    #[error("Network error: {0}")]
    NetworkError(String),
}
```

### Weather API Methods

```rust
impl WeatherApiInstance {
    /// Get current weather for a city
    pub async fn get_current_weather(&mut self, city: &str) -> Result<WeatherData, WeatherError> {
        let cache_key = format!("current:{}", city);
        
        // Check cache if enabled
        if self.config.cache_enabled {
            if let Some(cached) = self.get_from_cache(&cache_key) {
                self.metrics.increment_counter("cache_hits", 1.0);
                return Ok(cached);
            }
            self.metrics.increment_counter("cache_misses", 1.0);
        }
        
        // Apply rate limiting
        self.rate_limiter.wait_if_needed().await;
        
        // Make API request
        let url = format!(
            "{}/weather?q={}&appid={}&units={}",
            self.config.endpoint, city, self.api_key, self.config.units
        );
        
        let response = self.request_with_retry(&url).await?;
        let weather_data = self.parse_weather_response(response).await?;
        
        // Cache result if enabled
        if self.config.cache_enabled {
            self.save_to_cache(&cache_key, weather_data.clone());
        }
        
        // Record metrics
        self.metrics.increment_counter("api_calls", 1.0);
        self.metrics.record_value("temperature", weather_data.current.temperature);
        
        Ok(weather_data)
    }
    
    /// Get weather forecast
    pub async fn get_forecast(&mut self, city: &str, days: u32) -> Result<WeatherData, WeatherError> {
        let cache_key = format!("forecast:{}:{}", city, days);
        
        // Check cache
        if self.config.cache_enabled {
            if let Some(cached) = self.get_from_cache(&cache_key) {
                return Ok(cached);
            }
        }
        
        // Apply rate limiting
        self.rate_limiter.wait_if_needed().await;
        
        // Make API request
        let url = format!(
            "{}/forecast?q={}&cnt={}&appid={}&units={}",
            self.config.endpoint, city, days * 8, self.api_key, self.config.units
        );
        
        let response = self.request_with_retry(&url).await?;
        let weather_data = self.parse_forecast_response(response).await?;
        
        // Cache result
        if self.config.cache_enabled {
            self.save_to_cache(&cache_key, weather_data.clone());
        }
        
        Ok(weather_data)
    }
    
    /// Make request with retry logic
    async fn request_with_retry(&self, url: &str) -> Result<reqwest::Response, WeatherError> {
        let mut attempts = 0;
        let mut last_error = None;
        
        while attempts <= self.config.max_retries {
            match self.client.get(url).send().await {
                Ok(response) if response.status().is_success() => {
                    return Ok(response);
                }
                Ok(response) if response.status() == 429 => {
                    return Err(WeatherError::RateLimitExceeded);
                }
                Ok(response) => {
                    last_error = Some(WeatherError::ApiError(
                        format!("API returned status: {}", response.status())
                    ));
                }
                Err(e) => {
                    last_error = Some(WeatherError::NetworkError(e.to_string()));
                }
            }
            
            attempts += 1;
            if attempts <= self.config.max_retries {
                let backoff = Duration::from_millis(100 * 2_u64.pow(attempts));
                tokio::time::sleep(backoff).await;
            }
        }
        
        Err(last_error.unwrap_or(WeatherError::NetworkError("Unknown error".into())))
    }
    
    /// Parse weather response
    async fn parse_weather_response(&self, response: reqwest::Response) -> Result<WeatherData, WeatherError> {
        let json: serde_json::Value = response.json().await
            .map_err(|e| WeatherError::ParseError(e.to_string()))?;
        
        Ok(WeatherData {
            location: Location {
                name: json["name"].as_str().unwrap_or("").into(),
                country: json["sys"]["country"].as_str().unwrap_or("").into(),
                lat: json["coord"]["lat"].as_f64().unwrap_or(0.0),
                lon: json["coord"]["lon"].as_f64().unwrap_or(0.0),
            },
            current: CurrentWeather {
                temperature: json["main"]["temp"].as_f64().unwrap_or(0.0),
                feels_like: json["main"]["feels_like"].as_f64().unwrap_or(0.0),
                humidity: json["main"]["humidity"].as_u64().unwrap_or(0) as u32,
                pressure: json["main"]["pressure"].as_u64().unwrap_or(0) as u32,
                wind_speed: json["wind"]["speed"].as_f64().unwrap_or(0.0),
                description: json["weather"][0]["description"].as_str().unwrap_or("").into(),
                icon: json["weather"][0]["icon"].as_str().unwrap_or("").into(),
            },
            forecast: None,
        })
    }
    
    /// Parse forecast response
    async fn parse_forecast_response(&self, response: reqwest::Response) -> Result<WeatherData, WeatherError> {
        let json: serde_json::Value = response.json().await
            .map_err(|e| WeatherError::ParseError(e.to_string()))?;
        
        let mut forecast_items = Vec::new();
        if let Some(list) = json["list"].as_array() {
            for item in list.iter().take(10) {
                forecast_items.push(ForecastItem {
                    datetime: chrono::DateTime::from_timestamp(
                        item["dt"].as_i64().unwrap_or(0), 0
                    ).unwrap_or_default(),
                    temperature: item["main"]["temp"].as_f64().unwrap_or(0.0),
                    description: item["weather"][0]["description"].as_str().unwrap_or("").into(),
                    precipitation: item["rain"]["3h"].as_f64().unwrap_or(0.0),
                });
            }
        }
        
        Ok(WeatherData {
            location: Location {
                name: json["city"]["name"].as_str().unwrap_or("").into(),
                country: json["city"]["country"].as_str().unwrap_or("").into(),
                lat: json["city"]["coord"]["lat"].as_f64().unwrap_or(0.0),
                lon: json["city"]["coord"]["lon"].as_f64().unwrap_or(0.0),
            },
            current: CurrentWeather {
                temperature: 0.0,
                feels_like: 0.0,
                humidity: 0,
                pressure: 0,
                wind_speed: 0.0,
                description: "".into(),
                icon: "".into(),
            },
            forecast: Some(forecast_items),
        })
    }
    
    /// Get data from cache
    fn get_from_cache(&self, key: &str) -> Option<WeatherData> {
        self.cache.get(key).and_then(|cached| {
            let ttl = Duration::from_secs(self.config.cache_ttl_secs);
            if cached.cached_at.elapsed() < ttl {
                Some(cached.data.clone())
            } else {
                None
            }
        })
    }
    
    /// Save data to cache
    fn save_to_cache(&mut self, key: &str, data: WeatherData) {
        self.cache.insert(key.to_string(), CachedWeather {
            data,
            cached_at: Instant::now(),
        });
        
        // Clean old cache entries
        let ttl = Duration::from_secs(self.config.cache_ttl_secs);
        self.cache.retain(|_, v| v.cached_at.elapsed() < ttl);
    }
}
```

## Step 6: Context-Aware Features

### Add Context Support

```rust
impl ContextAware for WeatherApiInstance {
    fn inject_context(&mut self, context: ResourceContext) {
        // Add context to metrics
        self.metrics.add_tag("workflow_id", &context.workflow_id);
        self.metrics.add_tag("execution_id", &context.execution_id);
        self.metrics.add_tag("environment", &context.environment);
        
        // Add context to HTTP headers for tracing
        self.client = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("X-Trace-Id", context.trace_id.parse().unwrap());
                headers.insert("X-Workflow-Id", context.workflow_id.parse().unwrap());
                headers
            })
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .build()
            .unwrap();
    }
}
```

## Step 7: Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_resource::testing::*;
    
    #[tokio::test]
    async fn test_weather_resource_creation() {
        let config = WeatherApiConfig {
            endpoint: "https://api.test.com".into(),
            timeout_secs: 30,
            max_retries: 3,
            rate_limit: 60,
            api_key_credential: "test_key".into(),
            cache_enabled: true,
            cache_ttl_secs: 600,
            units: "metric".into(),
        };
        
        let context = TestResourceContext::new()
            .with_credential("test_key", "test_api_key_value");
        
        let result = WeatherApiResource::create(config, &context).await;
        assert!(result.is_ok());
        
        let instance = result.unwrap();
        assert_eq!(instance.config.units, "metric");
    }
    
    #[tokio::test]
    async fn test_cache_functionality() {
        let mut instance = create_test_instance().await;
        
        // First call - should hit API
        let result1 = instance.get_current_weather("London").await;
        assert!(result1.is_ok());
        assert_eq!(instance.metrics.get_counter("cache_misses"), 1.0);
        
        // Second call - should hit cache
        let result2 = instance.get_current_weather("London").await;
        assert!(result2.is_ok());
        assert_eq!(instance.metrics.get_counter("cache_hits"), 1.0);
        
        // Results should be the same
        assert_eq!(result1.unwrap().location.name, result2.unwrap().location.name);
    }
    
    #[tokio::test]
    async fn test_rate_limiting() {
        let mut instance = create_test_instance().await;
        instance.config.rate_limit = 2; // Very low limit for testing
        
        let start = Instant::now();
        
        // Make 3 requests quickly
        for _ in 0..3 {
            let _ = instance.get_current_weather("London").await;
        }
        
        // Should have taken at least 1 minute due to rate limiting
        assert!(start.elapsed() >= Duration::from_secs(60));
    }
    
    async fn create_test_instance() -> WeatherApiInstance {
        WeatherApiInstance {
            id: ResourceInstanceId::new(),
            client: reqwest::Client::new(),
            config: WeatherApiConfig::default(),
            api_key: "test_key".into(),
            metrics: ResourceMetrics::new("test"),
            cache: HashMap::new(),
            rate_limiter: RateLimiter::new(60),
            last_health_check: None,
        }
    }
}
```

### Mock for Testing

```rust
#[derive(AutoMock)]
impl WeatherApiInstance {
    // Methods will be automatically mocked
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    
    #[tokio::test]
    async fn test_with_mock() {
        let mut mock = MockWeatherApiInstance::new();
        
        mock.expect_get_current_weather()
            .with(eq("London"))
            .times(1)
            .returning(|_| Ok(WeatherData {
                location: Location {
                    name: "London".into(),
                    country: "UK".into(),
                    lat: 51.5074,
                    lon: -0.1278,
                },
                current: CurrentWeather {
                    temperature: 20.0,
                    feels_like: 19.0,
                    humidity: 65,
                    pressure: 1013,
                    wind_speed: 5.0,
                    description: "Clear sky".into(),
                    icon: "01d".into(),
                },
                forecast: None,
            }));
        
        let result = mock.get_current_weather("London").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().current.temperature, 20.0);
    }
}
```

## Step 8: Usage in Actions

### Use in Action

```rust
use nebula_action::prelude::*;

#[derive(Action)]
#[action(id = "check_weather")]
#[resources([WeatherApiResource])]
pub struct CheckWeatherAction {
    pub city: String,
    pub include_forecast: bool,
}

#[async_trait]
impl ActionHandler for CheckWeatherAction {
    type Output = WeatherReport;
    
    async fn execute(&self, ctx: &ExecutionContext) -> Result<Self::Output> {
        // Get weather resource
        let mut weather = ctx.resource::<WeatherApiInstance>("weather_api").await?;
        
        // Get current weather
        let current = weather.get_current_weather(&self.city).await
            .map_err(|e| ActionError::ResourceError(e.to_string()))?;
        
        // Get forecast if requested
        let forecast = if self.include_forecast {
            Some(weather.get_forecast(&self.city, 5).await
                .map_err(|e| ActionError::ResourceError(e.to_string()))?)
        } else {
            None
        };
        
        Ok(WeatherReport {
            city: self.city.clone(),
            current_temp: current.current.temperature,
            description: current.current.description,
            forecast: forecast.and_then(|f| f.forecast),
        })
    }
}
```

## Step 9: Register and Use

### Registration

```rust
use nebula_resource::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create resource manager
    let manager = ResourceManager::builder()
        .with_config_dir("./configs")
        .build()
        .await?;
    
    // Register weather resource
    manager.register(
        "weather_api",
        WeatherApiResource,
    ).await?;
    
    // Load configuration
    manager.load_config("weather_api", "weather_api.toml").await?;
    
    // Use in workflow
    let workflow = Workflow::builder()
        .add_action(CheckWeatherAction {
            city: "London".into(),
            include_forecast: true,
        })
        .build();
    
    let result = workflow.execute(&manager).await?;
    println!("Weather report: {:?}", result);
    
    Ok(())
}
```

## Summary

Вы успешно создали свой первый ресурс! Основные компоненты:

1. **Resource Definition** - Структура с метаданными
2. **Configuration** - Валидируемая конфигурация
3. **Instance** - Реальная реализация с бизнес-логикой
4. **Health Checks** - Мониторинг состояния
5. **Caching** - Оптимизация производительности
6. **Rate Limiting** - Защита от превышения лимитов
7. **Context Awareness** - Интеграция с workflow
8. **Testing** - Unit и integration тесты

## Next Steps

- [[How-To/CreateResource|Advanced Resource Creation]] - Продвинутые техники
- [[How-To/StatefulResource|Stateful Resources]] - Ресурсы с состоянием
- [[How-To/PooledResource|Pooled Resources]] - Пулирование ресурсов
- [[Examples/|More Examples]] - Больше примеров
