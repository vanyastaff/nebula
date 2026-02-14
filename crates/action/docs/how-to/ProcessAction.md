---
title: How to: ProcessAction
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: ProcessAction

#action #processaction #stateless

## Overview

ProcessAction is the most common action type for stateless data processing. This guide shows you how to create and use ProcessActions effectively.

## Quick Start

### Using Simple Action Macro

The fastest way to create a ProcessAction:

```rust
use nebula_action::prelude::*;

simple_action!(
    MyAction,                    // Action struct name
    "my_action",                // Action ID
    MyInput,                    // Input type
    MyOutput,                   // Output type
    |action, input, context| async move {
        // Your processing logic here
        let result = process(input);
        Ok(MyOutput { data: result })
    }
);
```

### Manual Implementation

For more control, implement the trait manually:

```rust
use nebula_action::prelude::*;
use serde::{Deserialize, Serialize};

pub struct DataTransformer {
    metadata: ActionMetadata,
    config: TransformerConfig,
}

#[derive(Deserialize)]
pub struct TransformInput {
    pub data: String,
    pub format: DataFormat,
}

#[derive(Serialize)]
pub struct TransformOutput {
    pub transformed: String,
    pub metadata: HashMap<String, Value>,
}

impl DataTransformer {
    pub fn new(config: TransformerConfig) -> Result<Self, ActionError> {
        let metadata = ActionMetadata::builder()
            .key("data.transformer")
            .name("Data Transformer")
            .description("Transforms data between formats")
            .version("1.0.0")
            .build()?;
        
        Ok(Self { metadata, config })
    }
}

// Required trait implementations
impl HasMetadata for DataTransformer {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl HasType for DataTransformer {
    fn r#type(&self) -> ActionType {
        ActionType::Process
    }
}

impl Action for DataTransformer {}

// ProcessAction implementation
#[async_trait]
impl ProcessAction for DataTransformer {
    type Input = TransformInput;
    type Output = TransformOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Log start
        context.log_info(&format!("Transforming {} data", input.format));
        
        // Start timer
        let timer = context.start_timer("transform_duration");
        
        // Process data
        let transformed = match input.format {
            DataFormat::Json => self.transform_json(&input.data)?,
            DataFormat::Xml => self.transform_xml(&input.data)?,
            DataFormat::Csv => self.transform_csv(&input.data)?,
        };
        
        // Stop timer
        timer.stop_and_record();
        
        // Create metadata
        let metadata = hashmap! {
            "original_format" => json!(input.format),
            "original_size" => json!(input.data.len()),
            "transformed_size" => json!(transformed.len()),
            "timestamp" => json!(Utc::now()),
        };
        
        Ok(ActionResult::Success(TransformOutput {
            transformed,
            metadata,
        }))
    }
}
```

## Common Patterns

### HTTP API Action

```rust
simple_action!(
    HttpApiAction,
    "http.api_call",
    ApiRequest,
    ApiResponse,
    |action, request, context| async move {
        // Get HTTP client
        let client = reqwest::Client::new();
        
        // Build request
        let mut req = client
            .request(request.method, &request.url)
            .timeout(Duration::from_secs(30));
        
        // Add headers
        for (key, value) in &request.headers {
            req = req.header(key, value);
        }
        
        // Add body if present
        if let Some(body) = &request.body {
            req = req.json(body);
        }
        
        // Send request
        let response = req.send().await
            .map_err(|e| ActionError::ExternalServiceError {
                service: "http".to_string(),
                error: e.to_string(),
            })?;
        
        // Parse response
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.json::<Value>().await?;
        
        Ok(ApiResponse {
            status,
            headers: convert_headers(headers),
            body,
        })
    }
);
```

### Data Validation Action

```rust
pub struct ValidatorAction {
    rules: ValidationRules,
}

#[async_trait]
impl ProcessAction for ValidatorAction {
    type Input = UnvalidatedData;
    type Output = ValidatedData;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let mut errors = Vec::new();
        
        // Apply validation rules
        for rule in &self.rules.rules {
            if let Err(e) = rule.validate(&input) {
                errors.push(e);
            }
        }
        
        // Check if validation passed
        if !errors.is_empty() {
            context.log_warning(&format!("Validation failed with {} errors", errors.len()));
            
            return Err(ActionError::ValidationFailed { errors });
        }
        
        // Transform to validated type
        let validated = ValidatedData::from_unvalidated(input);
        
        Ok(ActionResult::Success(validated))
    }
}
```

### Aggregation Action

```rust
pub struct AggregatorAction;

#[async_trait]
impl ProcessAction for AggregatorAction {
    type Input = Vec<DataPoint>;
    type Output = AggregatedResult;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        if input.is_empty() {
            return Ok(ActionResult::Skip {
                reason: "No data points to aggregate".to_string(),
            });
        }
        
        let stats = AggregatedResult {
            count: input.len(),
            sum: input.iter().map(|p| p.value).sum(),
            average: input.iter().map(|p| p.value).sum::<f64>() / input.len() as f64,
            min: input.iter().map(|p| p.value).min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap(),
            max: input.iter().map(|p| p.value).max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap(),
            percentiles: calculate_percentiles(&input),
        };
        
        // Record metrics
        context.record_metric("aggregation_input_size", input.len() as f64, &[
            ("action", "aggregator"),
        ]);
        
        Ok(ActionResult::Success(stats))
    }
}
```

## Input/Output Design

### Input Validation

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInput {
    #[serde(deserialize_with = "validate_email")]
    pub email: String,
    
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    
    #[serde(rename = "type")]
    pub data_type: DataType,
    
    pub optional_field: Option<String>,
}

fn validate_email<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let email = String::deserialize(deserializer)?;
    if !email.contains('@') {
        return Err(serde::de::Error::custom("Invalid email format"));
    }
    Ok(email)
}

fn default_timeout() -> u64 {
    30
}
```

### Output Structure

```rust
#[derive(Serialize)]
pub struct ProcessOutput {
    // Main result
    pub result: ProcessedData,
    
    // Metadata
    pub metadata: OutputMetadata,
    
    // Optional debug info
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebugInfo>,
}

#[derive(Serialize)]
pub struct OutputMetadata {
    pub processed_at: DateTime<Utc>,
    pub processing_time_ms: u64,
    pub input_size: usize,
    pub output_size: usize,
}
```

## Error Handling

### Specific Error Types

```rust
impl ProcessAction for MyAction {
    async fn execute(
        &self,
        input: Self::Input,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Validate input
        if input.data.is_empty() {
            return Err(ActionError::InvalidInput {
                field: "data".to_string(),
                reason: "Data cannot be empty".to_string(),
            });
        }
        
        // Handle external service errors
        let external_data = fetch_external_data(&input.id)
            .await
            .map_err(|e| ActionError::ExternalServiceError {
                service: "data_service".to_string(),
                error: e.to_string(),
            })?;
        
        // Handle timeout
        let result = tokio::time::timeout(
            Duration::from_secs(input.timeout_seconds),
            process_data(external_data)
        ).await
        .map_err(|_| ActionError::Timeout {
            operation: "process_data".to_string(),
            duration: Duration::from_secs(input.timeout_seconds),
        })?;
        
        Ok(ActionResult::Success(result?))
    }
}
```

## Using ExecutionContext

### Logging

```rust
context.log_debug("Starting processing");
context.log_info(&format!("Processing {} items", items.len()));
context.log_warning("Slow response from external service");
context.log_error(&format!("Failed to process item: {}", error));
```

### Metrics

```rust
// Counter
context.increment_counter("items_processed", items.len() as f64, &[
    ("action", "processor"),
    ("status", "success"),
]);

// Gauge
context.record_metric("queue_size", queue.len() as f64, &[
    ("queue", "processing"),
]);

// Timer
let timer = context.start_timer("processing_duration");
let result = process_items(items).await?;
timer.stop_and_record();
```

### Variables

```rust
// Get variable
if let Some(cache_key) = context.get_variable("cache_key").await {
    // Use cached data
}

// Set variable
context.set_variable("last_processed_id", json!(item.id)).await?;
```

## Testing

### Unit Test

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::testing::*;
    
    #[tokio::test]
    async fn test_successful_processing() {
        // Arrange
        let action = DataTransformer::new(Default::default()).unwrap();
        let context = TestContext::new();
        
        let input = TransformInput {
            data: r#"{"name": "test"}"#.to_string(),
            format: DataFormat::Json,
        };
        
        // Act
        let result = action.execute(input, &context).await.unwrap();
        
        // Assert
        match result {
            ActionResult::Success(output) => {
                assert!(!output.transformed.is_empty());
                assert!(output.metadata.contains_key("original_format"));
            }
            _ => panic!("Expected Success result"),
        }
        
        // Verify metrics were recorded
        assert!(context.timer_recorded("transform_duration"));
    }
    
    #[tokio::test]
    async fn test_invalid_input() {
        let action = DataTransformer::new(Default::default()).unwrap();
        let context = TestContext::new();
        
        let input = TransformInput {
            data: "".to_string(),
            format: DataFormat::Json,
        };
        
        let result = action.execute(input, &context).await;
        
        assert!(matches!(
            result,
            Err(ActionError::InvalidInput { field, .. }) if field == "data"
        ));
    }
}
```

### Integration Test

```rust
#[tokio::test]
async fn test_with_real_service() {
    let action = HttpApiAction::new();
    let context = IntegrationContext::new()
        .with_service("http://localhost:8080")
        .build();
    
    let input = ApiRequest {
        method: Method::GET,
        url: "http://localhost:8080/api/data".to_string(),
        headers: HashMap::new(),
        body: None,
    };
    
    let result = action.execute(input, &context).await.unwrap();
    
    match result {
        ActionResult::Success(response) => {
            assert_eq!(response.status, 200);
            assert!(response.body.is_object());
        }
        _ => panic!("Expected Success"),
    }
}
```

## Best Practices

### ✅ DO's

1. **Keep actions stateless** - Don't store state in the action struct
2. **Use specific error types** - Help debugging with clear errors
3. **Log important events** - But don't over-log
4. **Record metrics** - Monitor performance and usage
5. **Validate inputs early** - Fail fast with clear messages
6. **Use timeouts** - Prevent hanging operations

### ❌ DON'Ts

1. **Don't mutate shared state** - Actions should be thread-safe
2. **Don't block the executor** - Use async operations
3. **Don't swallow errors** - Always propagate or handle appropriately
4. **Don't mix concerns** - Keep actions focused on one task
5. **Don't forget cancellation** - Check context.is_cancelled()

## Templates

### Basic ProcessAction Template

```rust
// <% tp.file.cursor() %>
use nebula_action::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct <%= tp.file.title %>Input {
    // TODO: Define input fields
}

#[derive(Serialize)]
pub struct <%= tp.file.title %>Output {
    // TODO: Define output fields
}

simple_action!(
    <%= tp.file.title %>Action,
    "<%= tp.file.title.toLowerCase() %>",
    <%= tp.file.title %>Input,
    <%= tp.file.title %>Output,
    |action, input, context| async move {
        // TODO: Implement processing logic
        
        Ok(<%= tp.file.title %>Output {
            // TODO: Return output
        })
    }
);
```

## Examples Repository

Find complete examples in:

- [[Examples#ProcessAction Examples]]
- [[Examples/HTTP Request Action]]
- [[Examples/Data Transformation Action]]
- [[Examples/Validation Action]]

## Related Documentation

- [[Action Types#ProcessAction]] - ProcessAction overview
- [[Action Lifecycle]] - Execution lifecycle
- [[Error Model]] - Error handling
- [[Action Result System]] - Result types