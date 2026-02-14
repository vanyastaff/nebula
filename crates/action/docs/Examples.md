---
title: Examples
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---


## Overview

This document contains practical examples of Nebula actions for common use cases. Each example includes complete code, tests, and best practices.

## ProcessAction Examples

### HTTP Request Action

Complete HTTP client action with authentication, retries, and error handling.

```rust
use nebula_action::prelude::*;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};

pub struct HttpRequestAction {
    metadata: ActionMetadata,
    client: Client,
}

#[derive(Deserialize)]
pub struct HttpRequest {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Value>,
    pub timeout_seconds: Option<u64>,
    pub follow_redirects: Option<bool>,
}

#[derive(Serialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Value,
    pub elapsed_ms: u64,
}

impl HttpRequestAction {
    pub fn new() -> Result<Self, ActionError> {
        let metadata = ActionMetadata::builder()
            .key("http.request")
            .name("HTTP Request")
            .description("Makes HTTP requests with authentication support")
            .version("1.0.0")
            .build()?;
        
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ActionError::InitializationFailed(e.to_string()))?;
        
        Ok(Self { metadata, client })
    }
}

impl HasMetadata for HttpRequestAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl HasType for HttpRequestAction {
    fn r#type(&self) -> ActionType {
        ActionType::Process
    }
}

impl Action for HttpRequestAction {}

#[async_trait]
impl ProcessAction for HttpRequestAction {
    type Input = HttpRequest;
    type Output = HttpResponse;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let start = Instant::now();
        
        // Parse method
        let method = Method::from_bytes(input.method.as_bytes())
            .map_err(|_| ActionError::InvalidInput {
                field: "method".to_string(),
                reason: format!("Invalid HTTP method: {}", input.method),
            })?;
        
        // Build request
        let mut request = self.client.request(method, &input.url);
        
        // Add headers
        for (key, value) in &input.headers {
            request = request.header(key, value);
        }
        
        // Add authentication if available
        if let Ok(token) = context.get_credential("api_key").await {
            request = request.bearer_auth(token.expose());
        }
        
        // Add body
        if let Some(body) = &input.body {
            request = request.json(body);
        }
        
        // Set timeout
        if let Some(timeout) = input.timeout_seconds {
            request = request.timeout(Duration::from_secs(timeout));
        }
        
        // Log request
        context.log_info(&format!("Making {} request to {}", input.method, input.url));
        
        // Execute request with retry logic
        let response = self.execute_with_retry(request, context).await?;
        
        // Parse response
        let status = response.status().as_u16();
        let headers = response.headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        
        let body = response.json::<Value>().await
            .unwrap_or_else(|_| Value::Null);
        
        let elapsed_ms = start.elapsed().as_millis() as u64;
        
        // Record metrics
        context.record_metric("http_request_duration_ms", elapsed_ms as f64, &[
            ("method", &input.method),
            ("status", &status.to_string()),
        ]);
        
        Ok(ActionResult::Success(HttpResponse {
            status,
            headers,
            body,
            elapsed_ms,
        }))
    }
}

impl HttpRequestAction {
    async fn execute_with_retry(
        &self,
        request: reqwest::RequestBuilder,
        context: &dyn ExecutionContext,
    ) -> Result<reqwest::Response, ActionError> {
        let mut retries = 0;
        let max_retries = 3;
        let mut delay = Duration::from_secs(1);
        
        loop {
            match request.try_clone().unwrap().send().await {
                Ok(response) if response.status().is_success() => {
                    return Ok(response);
                }
                Ok(response) if response.status().as_u16() == 429 => {
                    // Rate limited
                    if retries < max_retries {
                        context.log_warning(&format!(
                            "Rate limited, retry {} of {}",
                            retries + 1,
                            max_retries
                        ));
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                        retries += 1;
                    } else {
                        return Err(ActionError::ExternalServiceError {
                            service: "http".to_string(),
                            error: "Rate limit exceeded".to_string(),
                        });
                    }
                }
                Ok(response) => {
                    return Err(ActionError::ExternalServiceError {
                        service: "http".to_string(),
                        error: format!("HTTP {}: {}", 
                            response.status(), 
                            response.text().await.unwrap_or_default()
                        ),
                    });
                }
                Err(e) if retries < max_retries && is_transient_error(&e) => {
                    context.log_warning(&format!(
                        "Transient error: {}, retry {} of {}",
                        e,
                        retries + 1,
                        max_retries
                    ));
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                    retries += 1;
                }
                Err(e) => {
                    return Err(ActionError::NetworkError {
                        operation: "http_request".to_string(),
                        error: e.to_string(),
                    });
                }
            }
        }
    }
}

fn is_transient_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_http_get() {
        let action = HttpRequestAction::new().unwrap();
        let context = TestContext::new();
        
        let input = HttpRequest {
            url: "https://jsonplaceholder.typicode.com/posts/1".to_string(),
            method: "GET".to_string(),
            headers: HashMap::new(),
            body: None,
            timeout_seconds: Some(10),
            follow_redirects: Some(true),
        };
        
        let result = action.execute(input, &context).await.unwrap();
        
        match result {
            ActionResult::Success(response) => {
                assert_eq!(response.status, 200);
                assert!(response.body.is_object());
            }
            _ => panic!("Expected success"),
        }
    }
}
```

### Data Transformation Action

Transform data between different formats with validation.

```rust
use nebula_action::prelude::*;

pub struct DataTransformer {
    metadata: ActionMetadata,
}

#[derive(Deserialize)]
pub struct TransformInput {
    pub data: Value,
    pub from_format: DataFormat,
    pub to_format: DataFormat,
    pub options: TransformOptions,
}

#[derive(Deserialize)]
pub struct TransformOptions {
    pub validate: bool,
    pub pretty_print: bool,
    pub preserve_nulls: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DataFormat {
    Json,
    Xml,
    Yaml,
    Csv,
    Toml,
}

#[derive(Serialize)]
pub struct TransformOutput {
    pub data: String,
    pub format: DataFormat,
    pub metadata: TransformMetadata,
}

#[derive(Serialize)]
pub struct TransformMetadata {
    pub original_size: usize,
    pub transformed_size: usize,
    pub processing_time_ms: u64,
    pub validation_passed: bool,
}

simple_action!(
    DataTransformer,
    "data.transformer",
    TransformInput,
    TransformOutput,
    |action, input, context| async move {
        let start = Instant::now();
        
        // Validate input if requested
        let validation_passed = if input.options.validate {
            validate_data(&input.data, &input.from_format)?
        } else {
            true
        };
        
        // Convert to intermediate format
        let intermediate = match input.from_format {
            DataFormat::Json => input.data.clone(),
            DataFormat::Xml => xml_to_json(&input.data)?,
            DataFormat::Yaml => yaml_to_json(&input.data)?,
            DataFormat::Csv => csv_to_json(&input.data)?,
            DataFormat::Toml => toml_to_json(&input.data)?,
        };
        
        // Transform to target format
        let transformed = match input.to_format {
            DataFormat::Json => {
                if input.options.pretty_print {
                    serde_json::to_string_pretty(&intermediate)?
                } else {
                    serde_json::to_string(&intermediate)?
                }
            }
            DataFormat::Xml => json_to_xml(&intermediate, &input.options)?,
            DataFormat::Yaml => json_to_yaml(&intermediate, &input.options)?,
            DataFormat::Csv => json_to_csv(&intermediate, &input.options)?,
            DataFormat::Toml => json_to_toml(&intermediate, &input.options)?,
        };
        
        let metadata = TransformMetadata {
            original_size: calculate_size(&input.data),
            transformed_size: transformed.len(),
            processing_time_ms: start.elapsed().as_millis() as u64,
            validation_passed,
        };
        
        context.log_info(&format!(
            "Transformed {} to {}: {} bytes -> {} bytes",
            format_name(&input.from_format),
            format_name(&input.to_format),
            metadata.original_size,
            metadata.transformed_size
        ));
        
        Ok(TransformOutput {
            data: transformed,
            format: input.to_format,
            metadata,
        })
    }
);
```

## StatefulAction Examples

### Rate Limiter Action

Stateful action that enforces rate limits.

```rust
use nebula_action::prelude::*;
use std::collections::VecDeque;

pub struct RateLimiterAction {
    metadata: ActionMetadata,
    limits: RateLimits,
}

#[derive(Deserialize)]
pub struct RateLimits {
    pub requests_per_second: f64,
    pub burst_size: usize,
    pub window_seconds: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct RateLimiterState {
    pub tokens: f64,
    pub last_refill: DateTime<Utc>,
    pub total_requests: u64,
    pub rejected_requests: u64,
    pub windows: VecDeque<WindowStats>,
}

#[derive(Serialize, Deserialize)]
pub struct WindowStats {
    pub timestamp: DateTime<Utc>,
    pub requests: u64,
    pub rejected: u64,
}

#[derive(Deserialize)]
pub struct RateLimitRequest {
    pub tokens_required: f64,
    pub priority: RequestPriority,
    pub metadata: HashMap<String, Value>,
}

#[derive(Deserialize)]
pub enum RequestPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Serialize)]
pub struct RateLimitResponse {
    pub allowed: bool,
    pub tokens_remaining: f64,
    pub retry_after_ms: Option<u64>,
    pub stats: RateLimitStats,
}

#[derive(Serialize)]
pub struct RateLimitStats {
    pub total_requests: u64,
    pub rejected_requests: u64,
    pub success_rate: f64,
    pub current_qps: f64,
}

#[async_trait]
impl StatefulAction for RateLimiterAction {
    type State = RateLimiterState;
    type Input = RateLimitRequest;
    type Output = RateLimitResponse;
    
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let now = Utc::now();
        
        // Refill tokens based on time elapsed
        let elapsed = (now - state.last_refill).num_milliseconds() as f64 / 1000.0;
        let tokens_to_add = elapsed * self.limits.requests_per_second;
        state.tokens = (state.tokens + tokens_to_add).min(self.limits.burst_size as f64);
        state.last_refill = now;
        
        // Clean old windows
        while let Some(window) = state.windows.front() {
            if (now - window.timestamp).num_seconds() > self.limits.window_seconds as i64 {
                state.windows.pop_front();
            } else {
                break;
            }
        }
        
        // Check priority bypass
        let tokens_required = match input.priority {
            RequestPriority::Critical => 0.0, // Always allow critical
            RequestPriority::High => input.tokens_required * 0.5,
            RequestPriority::Normal => input.tokens_required,
            RequestPriority::Low => input.tokens_required * 1.5,
        };
        
        // Check if request can be served
        let allowed = state.tokens >= tokens_required;
        
        if allowed {
            state.tokens -= tokens_required;
            state.total_requests += 1;
            
            context.log_debug(&format!(
                "Rate limit passed: {} tokens used, {} remaining",
                tokens_required,
                state.tokens
            ));
        } else {
            state.rejected_requests += 1;
            
            context.log_warning(&format!(
                "Rate limit exceeded: {} tokens required, {} available",
                tokens_required,
                state.tokens
            ));
        }
        
        // Update current window
        if let Some(window) = state.windows.back_mut() {
            if (now - window.timestamp).num_seconds() < 1 {
                if allowed {
                    window.requests += 1;
                } else {
                    window.rejected += 1;
                }
            } else {
                state.windows.push_back(WindowStats {
                    timestamp: now,
                    requests: if allowed { 1 } else { 0 },
                    rejected: if allowed { 0 } else { 1 },
                });
            }
        } else {
            state.windows.push_back(WindowStats {
                timestamp: now,
                requests: if allowed { 1 } else { 0 },
                rejected: if allowed { 0 } else { 1 },
            });
        }
        
        // Calculate retry after
        let retry_after_ms = if !allowed {
            let tokens_needed = tokens_required - state.tokens;
            Some((tokens_needed / self.limits.requests_per_second * 1000.0) as u64)
        } else {
            None
        };
        
        // Calculate stats
        let stats = RateLimitStats {
            total_requests: state.total_requests,
            rejected_requests: state.rejected_requests,
            success_rate: if state.total_requests > 0 {
                (state.total_requests - state.rejected_requests) as f64 / state.total_requests as f64
            } else {
                1.0
            },
            current_qps: self.calculate_qps(&state.windows),
        };
        
        let response = RateLimitResponse {
            allowed,
            tokens_remaining: state.tokens,
            retry_after_ms,
            stats,
        };
        
        if allowed {
            Ok(ActionResult::Success(response))
        } else {
            Ok(ActionResult::Retry {
                after: Duration::from_millis(retry_after_ms.unwrap()),
                reason: "Rate limit exceeded".to_string(),
            })
        }
    }
}

impl RateLimiterAction {
    fn calculate_qps(&self, windows: &VecDeque<WindowStats>) -> f64 {
        if windows.is_empty() {
            return 0.0;
        }
        
        let total_requests: u64 = windows.iter().map(|w| w.requests).sum();
        let duration = if let (Some(first), Some(last)) = (windows.front(), windows.back()) {
            (last.timestamp - first.timestamp).num_seconds() as f64
        } else {
            1.0
        };
        
        total_requests as f64 / duration.max(1.0)
    }
}
```

## TriggerAction Examples

### Database Change Stream Trigger

Monitor database changes and emit events.

```rust
use nebula_action::prelude::*;
use mongodb::{Client, Database};
use futures::stream::StreamExt;

pub struct MongoChangeStreamTrigger {
    metadata: ActionMetadata,
    client: Option<Client>,
}

#[derive(Deserialize)]
pub struct ChangeStreamConfig {
    pub connection_string: String,
    pub database: String,
    pub collection: String,
    pub operation_types: Vec<OperationType>,
    pub pipeline: Option<Vec<Document>>,
}

#[derive(Deserialize)]
pub enum OperationType {
    Insert,
    Update,
    Delete,
    Replace,
}

#[derive(Serialize, Clone)]
pub struct ChangeEvent {
    pub id: String,
    pub operation: String,
    pub document_id: String,
    pub full_document: Option<Value>,
    pub update_description: Option<UpdateDescription>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, Clone)]
pub struct UpdateDescription {
    pub updated_fields: HashMap<String, Value>,
    pub removed_fields: Vec<String>,
}

#[async_trait]
impl TriggerAction for MongoChangeStreamTrigger {
    type Config = ChangeStreamConfig;
    type Event = ChangeEvent;
    
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        // Connect to MongoDB
        let client = Client::with_uri_str(&config.connection_string)
            .await
            .map_err(|e| ActionError::ExternalServiceError {
                service: "mongodb".to_string(),
                error: e.to_string(),
            })?;
        
        let db = client.database(&config.database);
        let collection = db.collection::<Document>(&config.collection);
        
        // Build pipeline
        let mut pipeline = config.pipeline.unwrap_or_default();
        
        // Add operation type filter
        if !config.operation_types.is_empty() {
            let ops: Vec<String> = config.operation_types
                .iter()
                .map(|op| match op {
                    OperationType::Insert => "insert",
                    OperationType::Update => "update",
                    OperationType::Delete => "delete",
                    OperationType::Replace => "replace",
                })
                .map(String::from)
                .collect();
            
            pipeline.insert(0, doc! {
                "$match": {
                    "operationType": { "$in": ops }
                }
            });
        }
        
        // Create change stream
        let mut change_stream = collection
            .watch(pipeline, None)
            .await
            .map_err(|e| ActionError::TriggerError(e.to_string()))?;
        
        self.client = Some(client);
        
        context.log_info(&format!(
            "Change stream started for {}.{}",
            config.database,
            config.collection
        ));
        
        // Convert to event stream
        let stream = async_stream::stream! {
            while let Some(result) = change_stream.next().await {
                match result {
                    Ok(change) => {
                        let event = ChangeEvent {
                            id: change.id.to_string(),
                            operation: change.operation_type.to_string(),
                            document_id: extract_document_id(&change),
                            full_document: change.full_document.map(|d| bson_to_json(d)),
                            update_description: change.update_description.map(|desc| {
                                UpdateDescription {
                                    updated_fields: bson_document_to_hashmap(desc.updated_fields),
                                    removed_fields: desc.removed_fields,
                                }
                            }),
                            timestamp: change.cluster_time
                                .map(|t| DateTime::from_timestamp(t.timestamp, 0).unwrap())
                                .unwrap_or_else(Utc::now),
                        };
                        
                        yield event;
                    }
                    Err(e) => {
                        context.log_error(&format!("Change stream error: {}", e));
                        // Continue listening
                    }
                }
            }
        };
        
        Ok(Box::pin(stream))
    }
    
    async fn stop(&mut self) -> Result<(), ActionError> {
        self.client = None;
        Ok(())
    }
}
```

## Complex Workflow Example

Example of multiple actions working together in a workflow.

```rust
use nebula_action::prelude::*;

// Data ingestion workflow with multiple stages
pub struct DataIngestionWorkflow {
    fetcher: HttpRequestAction,
    validator: DataValidatorAction,
    transformer: DataTransformer,
    storage: DatabaseStorageAction,
    notifier: NotificationAction,
}

impl DataIngestionWorkflow {
    pub async fn run(
        &self,
        source_url: String,
        context: &dyn ExecutionContext,
    ) -> Result<IngestionResult, ActionError> {
        // Stage 1: Fetch data
        context.log_info("Starting data ingestion workflow");
        
        let fetch_result = self.fetcher.execute(
            HttpRequest {
                url: source_url,
                method: "GET".to_string(),
                headers: HashMap::new(),
                body: None,
                timeout_seconds: Some(30),
                follow_redirects: Some(true),
            },
            context,
        ).await?;
        
        let raw_data = match fetch_result {
            ActionResult::Success(response) => response.body,
            _ => return Err(ActionError::ExecutionFailed("Failed to fetch data".to_string())),
        };
        
        // Stage 2: Validate data
        let validation_result = self.validator.execute(
            ValidationInput {
                data: raw_data.clone(),
                schema: load_schema()?,
                strict_mode: true,
            },
            context,
        ).await?;
        
        let validated_data = match validation_result {
            ActionResult::Success(output) => output.validated_data,
            ActionResult::Skip { reason } => {
                context.log_warning(&format!("Validation skipped: {}", reason));
                return Ok(IngestionResult::Skipped { reason });
            }
            _ => return Err(ActionError::ValidationFailed {
                errors: vec![ValidationError {
                    field: "data".to_string(),
                    code: "INVALID".to_string(),
                    message: "Data validation failed".to_string(),
                    context: None,
                }],
            }),
        };
        
        // Stage 3: Transform data
        let transform_result = self.transformer.execute(
            TransformInput {
                data: validated_data,
                from_format: DataFormat::Json,
                to_format: DataFormat::Json,
                options: TransformOptions {
                    validate: false,
                    pretty_print: false,
                    preserve_nulls: false,
                },
            },
            context,
        ).await?;
        
        let transformed_data = match transform_result {
            ActionResult::Success(output) => output.data,
            _ => return Err(ActionError::ExecutionFailed("Transform failed".to_string())),
        };
        
        // Stage 4: Store data
        let storage_result = self.storage.execute(
            StorageInput {
                data: transformed_data,
                table: "ingested_data".to_string(),
                operation: StorageOperation::Upsert,
                conflict_resolution: ConflictResolution::Replace,
            },
            context,
        ).await?;
        
        let stored_ids = match storage_result {
            ActionResult::Success(output) => output.affected_ids,
            _ => return Err(ActionError::ExecutionFailed("Storage failed".to_string())),
        };
        
        // Stage 5: Send notification
        let notification_result = self.notifier.execute(
            NotificationInput {
                channel: NotificationChannel::Email,
                recipients: vec!["admin@example.com".to_string()],
                subject: "Data Ingestion Complete".to_string(),
                body: format!("Successfully ingested {} records", stored_ids.len()),
                priority: NotificationPriority::Normal,
            },
            context,
        ).await?;
        
        match notification_result {
            ActionResult::Success(_) => {
                context.log_info("Notification sent successfully");
            }
            _ => {
                context.log_warning("Failed to send notification");
            }
        }
        
        Ok(IngestionResult::Success {
            records_processed: stored_ids.len(),
            storage_ids: stored_ids,
        })
    }
}
```

## Testing Utilities

### Mock Action for Testing

```rust
use nebula_action::testing::*;

pub struct MockAction {
    responses: Vec<ActionResult<Value>>,
    call_count: Arc<Mutex<usize>>,
}

impl MockAction {
    pub fn new(responses: Vec<ActionResult<Value>>) -> Self {
        Self {
            responses,
            call_count: Arc::new(Mutex::new(0)),
        }
    }
    
    pub fn times_called(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

#[async_trait]
impl ProcessAction for MockAction {
    type Input = Value;
    type Output = Value;
    
    async fn execute(
        &self,
        _input: Self::Input,
        _context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let mut count = self.call_count.lock().unwrap();
        let index = *count;
        *count += 1;
        
        if index < self.responses.len() {
            Ok(self.responses[index].clone())
        } else {
            Err(ActionError::ExecutionFailed("No more mock responses".to_string()))
        }
    }
}

// Usage in tests
#[tokio::test]
async fn test_workflow_with_mocks() {
    let mock_action = MockAction::new(vec![
        ActionResult::Success(json!({ "data": "test" })),
        ActionResult::Retry {
            after: Duration::from_secs(1),
            reason: "Temporary failure".to_string(),
        },
        ActionResult::Success(json!({ "data": "retry_success" })),
    ]);
    
    let context = TestContext::new();
    
    // First call succeeds
    let result1 = mock_action.execute(json!({}), &context).await.unwrap();
    assert!(matches!(result1, ActionResult::Success(_)));
    
    // Second call triggers retry
    let result2 = mock_action.execute(json!({}), &context).await.unwrap();
    assert!(matches!(result2, ActionResult::Retry { .. }));
    
    // Third call succeeds after retry
    let result3 = mock_action.execute(json!({}), &context).await.unwrap();
    assert!(matches!(result3, ActionResult::Success(_)));
    
    assert_eq!(mock_action.times_called(), 3);
}
```

## StreamingAction Examples

### Server-Sent Events (SSE) Stream

```rust
use nebula_action::prelude::*;
use futures::stream::{Stream, StreamExt};

pub struct SseStreamAction {
    metadata: ActionMetadata,
}

#[derive(Deserialize)]
pub struct SseInput {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub retry_delay_ms: u64,
}

#[derive(Serialize, Clone)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
    pub id: Option<String>,
    pub retry: Option<u64>,
}

#[async_trait]
impl StreamingAction for SseStreamAction {
    type Input = SseInput;
    type Item = SseEvent;

    async fn stream(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<BoxStream<'static, Result<Self::Item, ActionError>>, ActionError> {
        let client = reqwest::Client::new();

        let mut request = client.get(&input.url);
        for (key, value) in &input.headers {
            request = request.header(key, value);
        }

        let response = request
            .send()
            .await
            .map_err(|e| ActionError::network_error(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ActionError::external_service_error(
                "sse",
                format!("HTTP {}", response.status())
            ));
        }

        let stream = async_stream::stream! {
            let mut bytes_stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = bytes_stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        buffer.push_str(&text);

                        // Process complete events
                        while let Some(pos) = buffer.find("\n\n") {
                            let event_text = buffer[..pos].to_string();
                            buffer = buffer[pos + 2..].to_string();

                            if let Some(event) = parse_sse_event(&event_text) {
                                yield Ok(event);
                            }
                        }
                    }
                    Err(e) => {
                        yield Err(ActionError::network_error(e.to_string()));
                        break;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }
}

fn parse_sse_event(text: &str) -> Option<SseEvent> {
    let mut event_type = "message".to_string();
    let mut data = String::new();
    let mut id = None;
    let mut retry = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim());
        } else if let Some(rest) = line.strip_prefix("id:") {
            id = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("retry:") {
            retry = rest.trim().parse().ok();
        }
    }

    if data.is_empty() {
        return None;
    }

    Some(SseEvent {
        event_type,
        data,
        id,
        retry,
    })
}
```

## TransactionalAction Examples

### Distributed Transaction with Saga

```rust
use nebula_action::prelude::*;

#[derive(Serialize, Deserialize)]
pub struct OrderCompensationData {
    pub order_id: String,
    pub payment_id: Option<String>,
    pub reservation_id: Option<String>,
}

pub struct ProcessOrderAction {
    payment_service: Arc<PaymentService>,
    inventory_service: Arc<InventoryService>,
}

#[async_trait]
impl TransactionalAction for ProcessOrderAction {
    type Input = OrderInput;
    type Output = OrderOutput;
    type CompensationData = OrderCompensationData;

    async fn execute_tx(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<(Self::Output, Self::CompensationData), ActionError> {
        let order_id = Uuid::new_v4().to_string();

        // Step 1: Reserve inventory
        let reservation_id = self.inventory_service
            .reserve(&input.items)
            .await
            .map_err(|e| ActionError::permanent(format!("Inventory reserve failed: {}", e)))?;

        context.log_info(&format!("Reserved inventory: {}", reservation_id));

        // Step 2: Process payment
        let payment_id = self.payment_service
            .charge(&input.payment_method, input.amount)
            .await
            .map_err(|e| ActionError::permanent(format!("Payment failed: {}", e)))?;

        context.log_info(&format!("Payment processed: {}", payment_id));

        let output = OrderOutput {
            order_id: order_id.clone(),
            status: "completed".to_string(),
            payment_id: payment_id.clone(),
        };

        let compensation = OrderCompensationData {
            order_id,
            payment_id: Some(payment_id),
            reservation_id: Some(reservation_id),
        };

        Ok((output, compensation))
    }

    async fn compensate(
        &self,
        data: Self::CompensationData,
        context: &Context,
    ) -> Result<(), ActionError> {
        context.log_warn(&format!("Compensating order: {}", data.order_id));

        // Refund payment if charged
        if let Some(payment_id) = data.payment_id {
            self.payment_service
                .refund(&payment_id)
                .await
                .map_err(|e| ActionError::permanent(format!("Refund failed: {}", e)))?;

            context.log_info(&format!("Payment refunded: {}", payment_id));
        }

        // Release inventory if reserved
        if let Some(reservation_id) = data.reservation_id {
            self.inventory_service
                .release(&reservation_id)
                .await
                .map_err(|e| ActionError::permanent(format!("Release failed: {}", e)))?;

            context.log_info(&format!("Inventory released: {}", reservation_id));
        }

        Ok(())
    }
}
```

## InteractiveAction Examples

### Manual Approval Workflow

```rust
use nebula_action::prelude::*;

pub struct ApprovalAction {
    notification_service: Arc<NotificationService>,
}

#[derive(Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    pub title: String,
    pub description: String,
    pub approvers: Vec<String>,
    pub timeout_hours: u32,
}

#[derive(Serialize)]
pub struct ApprovalResponse {
    pub approved: bool,
    pub approver: String,
    pub approved_at: DateTime<Utc>,
    pub comments: Option<String>,
}

#[async_trait]
impl InteractiveAction for ApprovalAction {
    type Input = ApprovalRequest;
    type Output = ApprovalResponse;
    type InteractionData = ApprovalInteraction;

    async fn request_interaction(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<InteractionRequest<Self::InteractionData>, ActionError> {
        // Send notification to approvers
        for approver in &input.approvers {
            self.notification_service
                .send_approval_request(
                    approver,
                    &input.title,
                    &input.description,
                    &input.request_id,
                )
                .await?;
        }

        context.log_info(&format!(
            "Approval request sent to {} approvers",
            input.approvers.len()
        ));

        Ok(InteractionRequest {
            interaction_id: input.request_id.clone(),
            data: ApprovalInteraction {
                title: input.title,
                description: input.description,
                approvers: input.approvers,
            },
            timeout: Duration::from_secs(input.timeout_hours as u64 * 3600),
        })
    }

    async fn process_interaction(
        &self,
        response: InteractionResponse,
        context: &Context,
    ) -> Result<Self::Output, ActionError> {
        let data: ApprovalData = serde_json::from_value(response.data)
            .map_err(|e| ActionError::validation(format!("Invalid approval data: {}", e)))?;

        context.log_info(&format!(
            "Approval {} by {}",
            if data.approved { "granted" } else { "denied" },
            data.approver
        ));

        Ok(ApprovalResponse {
            approved: data.approved,
            approver: data.approver,
            approved_at: data.responded_at,
            comments: data.comments,
        })
    }

    async fn on_timeout(
        &self,
        interaction_data: Self::InteractionData,
        context: &Context,
    ) -> Result<Self::Output, ActionError> {
        context.log_warn(&format!(
            "Approval request '{}' timed out",
            interaction_data.title
        ));

        // Auto-reject on timeout
        Ok(ApprovalResponse {
            approved: false,
            approver: "system".to_string(),
            approved_at: Utc::now(),
            comments: Some("Automatically rejected due to timeout".to_string()),
        })
    }
}
```

## Performance Optimization Examples

### Batch Processing with Controlled Concurrency

```rust
use nebula_action::prelude::*;
use futures::stream::{self, StreamExt};

pub struct BatchProcessAction {
    batch_size: usize,
    max_concurrency: usize,
}

#[derive(Deserialize)]
pub struct BatchInput {
    pub items: Vec<serde_json::Value>,
    pub parallel: bool,
}

#[derive(Serialize)]
pub struct BatchOutput {
    pub processed: usize,
    pub failed: usize,
    pub results: Vec<ProcessedItem>,
}

#[async_trait]
impl ProcessAction for BatchProcessAction {
    type Input = BatchInput;
    type Output = BatchOutput;

    async fn process(&self, input: Self::Input, context: &Context) -> Result<Self::Output> {
        context.log_info(&format!(
            "Processing {} items in batches of {}",
            input.items.len(),
            self.batch_size
        ));

        let mut results = Vec::new();
        let mut failed = 0;

        if input.parallel {
            // Parallel processing with backpressure control
            for chunk in input.items.chunks(self.batch_size) {
                let chunk_results: Vec<_> = stream::iter(chunk)
                    .map(|item| async move {
                        self.process_single_item(item.clone(), context).await
                    })
                    .buffer_unordered(self.max_concurrency)
                    .collect()
                    .await;

                for result in chunk_results {
                    match result {
                        Ok(item) => results.push(item),
                        Err(e) => {
                            context.log_error(&format!("Item processing failed: {}", e));
                            failed += 1;
                        }
                    }
                }

                // Yield between batches
                tokio::task::yield_now().await;
            }
        } else {
            // Sequential processing
            for item in input.items {
                match self.process_single_item(item, context).await {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        context.log_error(&format!("Item processing failed: {}", e));
                        failed += 1;
                    }
                }
            }
        }

        context.record_metric("batch.processed", results.len() as f64);
        context.record_metric("batch.failed", failed as f64);

        Ok(BatchOutput {
            processed: results.len(),
            failed,
            results,
        })
    }
}

impl BatchProcessAction {
    async fn process_single_item(
        &self,
        item: serde_json::Value,
        context: &Context,
    ) -> Result<ProcessedItem, ActionError> {
        // Process individual item
        // ...implementation...
        todo!()
    }
}
```

## Related Documentation

- [[Action Types]] - Overview of all action types
- [[Action Result System]] - Understanding result types
- [[Error Model]] - Error handling patterns
- [[README#Testing]] - Testing strategies and utilities