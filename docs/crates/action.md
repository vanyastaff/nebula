# nebula-action

Core action system for Nebula workflow engine. Provides trait-based abstractions for all types of workflow nodes.

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Action Types](#action-types)
4. [ActionResult System](#actionresult-system)
5. [ExecutionContext](#executioncontext)
6. [Examples](#examples)
7. [Testing](#testing)
8. [Best Practices](#best-practices)

## Overview

Actions are the fundamental building blocks of Nebula workflows. Each action represents a unit of work that can:
- Process input data
- Maintain state between executions
- Generate events
- Supply resources
- Handle user interactions
- Participate in transactions

## Architecture

### File Structure

```
nebula-action/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                    # Main exports and prelude
│   │
│   ├── core/                     # Core traits and types
│   │   ├── mod.rs
│   │   ├── traits.rs             # Action, ProcessAction base traits
│   │   ├── context.rs            # ExecutionContext trait
│   │   ├── result.rs             # ActionResult enum and variants
│   │   ├── metadata.rs           # ActionMetadata, ActionDefinition
│   │   ├── error.rs              # ActionError types
│   │   └── lifecycle.rs          # Lifecycle hooks
│   │
│   ├── process/                  # ProcessAction implementation
│   │   ├── mod.rs
│   │   ├── traits.rs             # ProcessAction trait
│   │   ├── simple.rs             # SimpleAction helper trait
│   │   └── builder.rs            # ProcessAction builder
│   │
│   ├── stateful/                 # StatefulAction implementation
│   │   ├── mod.rs
│   │   ├── traits.rs             # StatefulAction trait
│   │   ├── state.rs              # State management
│   │   ├── persistence.rs        # State persistence
│   │   └── migration.rs          # State migration
│   │
│   ├── trigger/                  # TriggerAction implementation
│   │   ├── mod.rs
│   │   ├── traits.rs             # TriggerAction trait
│   │   ├── event.rs              # Event types and stream
│   │   ├── context.rs            # TriggerContext
│   │   └── polling.rs            # PollingAction trait
│   │
│   ├── supply/                   # SupplyAction implementation
│   │   ├── mod.rs
│   │   ├── traits.rs             # SupplyAction trait
│   │   ├── instance.rs           # Instance management
│   │   ├── health.rs             # Health checking
│   │   └── pool.rs               # Resource pooling
│   │
│   ├── streaming/                # StreamingAction implementation
│   │   ├── mod.rs
│   │   ├── traits.rs             # StreamingAction trait
│   │   ├── backpressure.rs       # Backpressure handling
│   │   ├── window.rs             # Windowing operations
│   │   └── watermark.rs          # Watermark handling
│   │
│   ├── interactive/              # InteractiveAction implementation
│   │   ├── mod.rs
│   │   ├── traits.rs             # InteractiveAction trait
│   │   ├── request.rs            # Interaction requests
│   │   ├── response.rs           # Interaction responses
│   │   └── validation.rs         # Input validation
│   │
│   ├── transactional/            # TransactionalAction implementation
│   │   ├── mod.rs
│   │   ├── traits.rs             # TransactionalAction trait
│   │   ├── coordinator.rs        # 2PC coordinator
│   │   ├── participant.rs        # Transaction participant
│   │   └── recovery.rs           # Recovery mechanisms
│   │
│   ├── result/                   # ActionResult variants
│   │   ├── mod.rs
│   │   ├── basic.rs              # Basic result types
│   │   ├── control_flow.rs       # Control flow results
│   │   ├── parallel.rs           # Parallel execution results
│   │   ├── async_ops.rs          # Async operation results
│   │   └── specialized.rs        # Specialized results
│   │
│   ├── execution/                # Execution infrastructure
│   │   ├── mod.rs
│   │   ├── context.rs            # ExecutionContext implementation
│   │   ├── executor.rs           # Action executor
│   │   ├── scheduler.rs          # Action scheduling
│   │   └── monitoring.rs         # Execution monitoring
│   │
│   ├── testing/                  # Testing utilities
│   │   ├── mod.rs
│   │   ├── mock.rs               # Mock actions
│   │   ├── context.rs            # Test context
│   │   └── assertions.rs         # Test assertions
│   │
│   └── prelude.rs                # Common imports
│
├── examples/
│   ├── simple_action.rs          # Basic ProcessAction
│   ├── stateful_counter.rs       # StatefulAction example
│   ├── http_webhook.rs           # WebhookAction example
│   ├── kafka_trigger.rs          # TriggerAction example
│   ├── db_supply.rs              # SupplyAction example
│   └── complex_workflow.rs       # Complex action composition
│
└── tests/
    ├── integration/
    └── unit/
```

## Action Types

### 1. ProcessAction

Stateless data processing actions - the most common action type.

```rust
use nebula_action::prelude::*;

#[derive(Action)]
#[action(
    id = "example.process",
    name = "Example Process Action",
    description = "Processes data without state"
)]
pub struct ExampleProcessAction;

#[derive(Parameters)]
pub struct ProcessInput {
    #[parameter(description = "Input data")]
    pub data: String,
    
    #[parameter(description = "Processing options")]
    pub options: ProcessOptions,
}

#[derive(Serialize)]
pub struct ProcessOutput {
    pub result: String,
    pub metadata: HashMap<String, Value>,
}

#[async_trait]
impl ProcessAction for ExampleProcessAction {
    type Input = ProcessInput;
    type Output = ProcessOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Process the input
        let result = self.process_data(&input.data, &input.options)?;
        
        // Log the operation
        context.log_info(&format!("Processed {} bytes", input.data.len()));
        
        // Return success
        Ok(ActionResult::Success(ProcessOutput {
            result,
            metadata: self.generate_metadata(&input),
        }))
    }
}
```

### 2. StatefulAction

Actions that maintain state between executions.

```rust
#[derive(Action)]
#[action(
    id = "example.stateful",
    name = "Stateful Counter",
    description = "Counts items with state persistence"
)]
pub struct CounterAction;

#[derive(Serialize, Deserialize, Default)]
pub struct CounterState {
    pub count: u64,
    pub last_updated: Option<DateTime<Utc>>,
    pub history: Vec<CountEvent>,
}

#[derive(Parameters)]
pub struct CounterInput {
    #[parameter(description = "Increment amount")]
    pub increment: u64,
    
    #[parameter(description = "Tag for this count")]
    pub tag: Option<String>,
}

#[derive(Serialize)]
pub struct CounterOutput {
    pub previous_count: u64,
    pub current_count: u64,
    pub total_events: usize,
}

#[async_trait]
impl StatefulAction for CounterAction {
    type State = CounterState;
    type Input = CounterInput;
    type Output = CounterOutput;
    
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let previous_count = state.count;
        
        // Update state
        state.count += input.increment;
        state.last_updated = Some(Utc::now());
        state.history.push(CountEvent {
            timestamp: Utc::now(),
            increment: input.increment,
            tag: input.tag,
        });
        
        // Check if we should continue or break
        if state.count >= 1000 {
            Ok(ActionResult::Break {
                output: CounterOutput {
                    previous_count,
                    current_count: state.count,
                    total_events: state.history.len(),
                },
                reason: BreakReason::Completed,
            })
        } else {
            Ok(ActionResult::Continue {
                output: CounterOutput {
                    previous_count,
                    current_count: state.count,
                    total_events: state.history.len(),
                },
                progress: LoopProgress {
                    current_iteration: state.history.len(),
                    total_items: Some(1000),
                    processed_items: state.count as usize,
                    percentage: Some((state.count as f32 / 1000.0) * 100.0),
                    estimated_time_remaining: None,
                    status_message: Some(format!("Counted {} items", state.count)),
                },
                delay: None,
            })
        }
    }
    
    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        old_version: semver::Version,
    ) -> Result<Self::State, ActionError> {
        // Handle state migration between versions
        if old_version.major < 2 {
            // Migrate from v1 to v2 format
            let mut state: CounterState = serde_json::from_value(old_state)?;
            // Apply migrations...
            Ok(state)
        } else {
            Ok(serde_json::from_value(old_state)?)
        }
    }
}
```

### 3. TriggerAction

Event sources that initiate workflows.

```rust
#[derive(Action)]
#[action(
    id = "kafka.consumer",
    name = "Kafka Consumer Trigger",
    description = "Consumes messages from Kafka topics"
)]
pub struct KafkaConsumerTrigger;

#[derive(Parameters)]
pub struct KafkaConfig {
    #[parameter(description = "Kafka brokers")]
    pub brokers: Vec<String>,
    
    #[parameter(description = "Topic to consume")]
    pub topic: String,
    
    #[parameter(description = "Consumer group ID")]
    pub group_id: String,
    
    #[parameter(description = "Auto offset reset", default = "latest")]
    pub auto_offset_reset: String,
}

#[derive(Serialize, Clone)]
pub struct KafkaEvent {
    pub topic: String,
    pub partition: i32,
    pub offset: i64,
    pub key: Option<String>,
    pub value: String,
    pub timestamp: DateTime<Utc>,
}

#[async_trait]
impl TriggerAction for KafkaConsumerTrigger {
    type Config = KafkaConfig;
    type Event = KafkaEvent;
    
    async fn start(
        &self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        // Create Kafka consumer
        let consumer = self.create_consumer(&config).await?;
        
        // Create event stream
        let stream = stream::unfold(consumer, |consumer| async move {
            match consumer.recv().await {
                Ok(message) => {
                    let event = KafkaEvent {
                        topic: message.topic().to_string(),
                        partition: message.partition(),
                        offset: message.offset(),
                        key: message.key().map(|k| String::from_utf8_lossy(k).to_string()),
                        value: String::from_utf8_lossy(message.payload()?).to_string(),
                        timestamp: Utc::now(),
                    };
                    Some((Ok(event), consumer))
                }
                Err(e) => Some((Err(ActionError::TriggerError(e.to_string())), consumer))
            }
        });
        
        Ok(Box::pin(stream))
    }
    
    async fn stop(&self) -> Result<(), ActionError> {
        // Cleanup logic
        Ok(())
    }
}
```

### 4. SupplyAction

Resource providers that supply long-lived resources to other actions.

```rust
#[derive(Action)]
#[action(
    id = "postgres.connection",
    name = "PostgreSQL Connection Pool",
    description = "Provides PostgreSQL database connections"
)]
pub struct PostgresSupplier;

#[derive(Parameters)]
pub struct PostgresConfig {
    #[parameter(description = "Database URL", sensitive = true)]
    pub database_url: String,
    
    #[parameter(description = "Maximum connections", default = 10)]
    pub max_connections: u32,
    
    #[parameter(description = "Connection timeout", default = "30s")]
    pub connection_timeout: Duration,
}

#[async_trait]
impl SupplyAction for PostgresSupplier {
    type Config = PostgresConfig;
    type Resource = PgPool;
    
    async fn create(
        &self,
        config: Self::Config,
        context: &ExecutionContext,
    ) -> Result<Self::Resource, ActionError> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect_timeout(config.connection_timeout)
            .connect(&config.database_url)
            .await?;
        
        context.log_info(&format!(
            "Created PostgreSQL pool with {} max connections",
            config.max_connections
        ));
        
        Ok(pool)
    }
    
    async fn destroy(&self, resource: Self::Resource) -> Result<(), ActionError> {
        resource.close().await;
        Ok(())
    }
    
    async fn health_check(&self, resource: &Self::Resource) -> Result<HealthStatus, ActionError> {
        match sqlx::query("SELECT 1").fetch_one(resource).await {
            Ok(_) => Ok(HealthStatus::Healthy),
            Err(e) => Ok(HealthStatus::Unhealthy {
                reason: e.to_string(),
                recoverable: true,
            }),
        }
    }
}
```

## ActionResult System

ActionResult controls the flow of workflow execution:

```rust
pub enum ActionResult<T> {
    // Basic Results
    Success(T),
    Skip { reason: String },
    Retry { after: Duration, reason: String },
    
    // Control Flow
    Continue { output: T, progress: LoopProgress, delay: Option<Duration> },
    Break { output: T, reason: BreakReason },
    Branch { branch: BranchSelection, output: T, decision_metadata: Option<Value> },
    
    // Parallel Execution
    Parallel {
        results: Vec<(NodeId, ActionResult<T>)>,
        aggregation: AggregationStrategy,
        partial_failure_ok: bool,
    },
    
    // Conditional Branching
    ConditionalBranch {
        condition_met: bool,
        branch_taken: BranchId,
        output: T,
        alternative_outputs: HashMap<BranchId, T>,
    },
    
    // Async Operations
    AsyncOperation {
        operation_id: String,
        estimated_duration: Duration,
        poll_interval: Duration,
        initial_status: T,
    },
    
    // Accumulation
    Accumulate {
        current_value: T,
        accumulator_state: AccumulatorState,
        continue_accumulation: bool,
    },
    
    // Waiting
    Wait {
        wait_condition: WaitCondition,
        timeout: Option<Duration>,
        partial_output: Option<T>,
    },
    
    // Routing
    Route { port: PortKey, data: T },
    MultiOutput(HashMap<PortKey, T>),
    
    // Streaming
    StreamItem {
        output: T,
        stream_metadata: StreamMetadata,
        side_outputs: Option<HashMap<String, T>>,
    },
    
    // Transactions
    TransactionPrepared {
        transaction_id: String,
        rollback_data: T,
        vote: TransactionVote,
        expires_at: DateTime<Utc>,
    },
    
    // Interactive
    InteractionRequired {
        interaction_request: InteractionRequest,
        state_output: T,
        response_timeout: Duration,
    },
}
```

### Using ActionResult

```rust
// Example: Conditional branching based on input
async fn execute(
    &self,
    input: Self::Input,
    context: &ExecutionContext,
) -> Result<ActionResult<Self::Output>, ActionError> {
    if input.value > 100 {
        Ok(ActionResult::Branch {
            branch: BranchSelection::Choice { selected: "high_value".to_string() },
            output: self.process_high_value(input)?,
            decision_metadata: Some(json!({
                "threshold": 100,
                "actual_value": input.value
            })),
        })
    } else {
        Ok(ActionResult::Branch {
            branch: BranchSelection::Choice { selected: "low_value".to_string() },
            output: self.process_low_value(input)?,
            decision_metadata: None,
        })
    }
}

// Example: Async operation for long-running tasks
async fn execute(
    &self,
    input: Self::Input,
    context: &ExecutionContext,
) -> Result<ActionResult<Self::Output>, ActionError> {
    let job_id = self.start_long_operation(input).await?;
    
    Ok(ActionResult::AsyncOperation {
        operation_id: job_id,
        estimated_duration: Duration::from_secs(300), // 5 minutes
        poll_interval: Duration::from_secs(10), // Check every 10 seconds
        initial_status: Self::Output {
            status: OperationStatus::Started,
            progress: 0.0,
            result: None,
        },
    })
}
```

## ExecutionContext

The ExecutionContext provides access to runtime services:

```rust
pub trait ExecutionContext: Send + Sync {
    // Identification
    fn execution_id(&self) -> &ExecutionId;
    fn node_id(&self) -> &NodeId;
    fn workflow_id(&self) -> &WorkflowId;
    
    // Logging
    fn log_info(&self, message: &str);
    fn log_warning(&self, message: &str);
    fn log_error(&self, message: &str);
    fn log_debug(&self, message: &str);
    
    // Metrics
    fn record_metric(&self, name: &str, value: f64, tags: &[(&str, &str)]);
    fn increment_counter(&self, name: &str, value: f64, tags: &[(&str, &str)]);
    fn start_timer(&self, name: &str) -> Timer;
    
    // Variables
    async fn get_variable(&self, name: &str) -> Option<Value>;
    async fn set_variable(&self, name: &str, value: Value) -> Result<(), ContextError>;
    
    // Resources and Clients
    async fn get_client<T>(&self, auth_type: &str) -> Result<T, ContextError>
    where
        T: 'static + Send + Sync + Clone;
    
    async fn get_resource<T>(&self) -> Option<Arc<T>>
    where
        T: 'static + Send + Sync;
    
    // Credentials
    async fn get_credential(&self, credential_id: &str) -> Result<Token, ContextError>;
    
    // Node Outputs
    async fn get_node_output(&self, node_id: &str) -> Option<Value>;
    
    // Cancellation
    fn is_cancelled(&self) -> bool;
    fn cancellation_token(&self) -> &CancellationToken;
    
    // Temporary Files
    async fn create_temp_file(&self, name: &str) -> Result<TempFile, ContextError>;
    
    // Events
    async fn emit_event(&self, event: Event) -> Result<(), ContextError>;
}
```

### Using ExecutionContext

```rust
async fn execute(
    &self,
    input: Self::Input,
    context: &ExecutionContext,
) -> Result<ActionResult<Self::Output>, ActionError> {
    // Logging
    context.log_info(&format!("Processing request: {}", input.id));
    
    // Metrics
    let timer = context.start_timer("processing_duration");
    
    // Get authenticated client
    let api_client = context.get_client::<ApiClient>("api_key").await?;
    
    // Access variables
    if let Some(cache_key) = context.get_variable("cache_key").await {
        // Use cached data
    }
    
    // Check cancellation
    if context.is_cancelled() {
        return Err(ActionError::Cancelled);
    }
    
    // Process
    let result = api_client.process(&input).await?;
    
    // Record metrics
    timer.stop_and_record();
    context.increment_counter("requests_processed", 1.0, &[
        ("status", "success"),
        ("type", &input.request_type),
    ]);
    
    // Set output variable
    context.set_variable("last_result", json!(result)).await?;
    
    Ok(ActionResult::Success(result))
}
```

## Examples

### Example 1: HTTP Request Action

```rust
use nebula_action::prelude::*;
use reqwest::Client;

#[derive(Action)]
#[action(
    id = "http.request",
    name = "HTTP Request",
    description = "Makes HTTP requests to external APIs"
)]
#[auth(optional = ["api_key", "oauth2", "basic_auth"])]
pub struct HttpRequestAction {
    client: Client,
}

#[derive(Parameters)]
pub struct HttpInput {
    #[parameter(description = "Request URL")]
    pub url: String,
    
    #[parameter(description = "HTTP method", default = "GET")]
    pub method: String,
    
    #[parameter(description = "Request headers", default = {})]
    pub headers: HashMap<String, String>,
    
    #[parameter(description = "Request body")]
    pub body: Option<Value>,
    
    #[parameter(description = "Timeout in seconds", default = 30)]
    pub timeout: u64,
}

#[derive(Serialize)]
pub struct HttpOutput {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Value,
    pub elapsed_ms: u64,
}

impl HttpRequestAction {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

#[async_trait]
impl ProcessAction for HttpRequestAction {
    type Input = HttpInput;
    type Output = HttpOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let start = std::time::Instant::now();
        
        // Build request
        let mut request = match input.method.to_uppercase().as_str() {
            "GET" => self.client.get(&input.url),
            "POST" => self.client.post(&input.url),
            "PUT" => self.client.put(&input.url),
            "DELETE" => self.client.delete(&input.url),
            "PATCH" => self.client.patch(&input.url),
            _ => return Err(ActionError::InvalidInput {
                field: "method".to_string(),
                reason: format!("Unsupported HTTP method: {}", input.method),
            }),
        };
        
        // Add headers
        for (key, value) in &input.headers {
            request = request.header(key, value);
        }
        
        // Add body
        if let Some(body) = &input.body {
            request = request.json(body);
        }
        
        // Set timeout
        request = request.timeout(Duration::from_secs(input.timeout));
        
        // Add authentication if available
        if let Ok(token) = context.get_credential("api_key").await {
            request = request.header("Authorization", format!("Bearer {}", token.value.expose()));
        }
        
        // Execute request
        context.log_info(&format!("Making {} request to {}", input.method, input.url));
        
        let response = request.send().await.map_err(|e| {
            context.log_error(&format!("HTTP request failed: {}", e));
            ActionError::ExternalServiceError {
                service: "http".to_string(),
                error: e.to_string(),
            }
        })?;
        
        let status = response.status().as_u16();
        let headers: HashMap<String, String> = response.headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        
        let body = response.json::<Value>().await.unwrap_or(Value::Null);
        
        let elapsed_ms = start.elapsed().as_millis() as u64;
        
        // Record metrics
        context.record_metric("http_request_duration_ms", elapsed_ms as f64, &[
            ("method", &input.method),
            ("status", &status.to_string()),
        ]);
        
        Ok(ActionResult::Success(HttpOutput {
            status,
            headers,
            body,
            elapsed_ms,
        }))
    }
}
```

### Example 2: Paginated Data Fetcher

```rust
#[derive(Action)]
#[action(
    id = "data.paginated_fetch",
    name = "Paginated Data Fetcher",
    description = "Fetches all pages from a paginated API"
)]
pub struct PaginatedFetchAction;

#[derive(Serialize, Deserialize, Default)]
pub struct PaginationState {
    pub all_items: Vec<Value>,
    pub current_page: usize,
    pub next_token: Option<String>,
    pub total_pages: Option<usize>,
}

#[derive(Parameters)]
pub struct PaginationInput {
    #[parameter(description = "API endpoint")]
    pub endpoint: String,
    
    #[parameter(description = "Items per page", default = 100)]
    pub page_size: usize,
    
    #[parameter(description = "Maximum pages to fetch")]
    pub max_pages: Option<usize>,
}

#[derive(Serialize)]
pub struct PaginationOutput {
    pub items: Vec<Value>,
    pub total_fetched: usize,
    pub pages_processed: usize,
    pub has_more: bool,
}

#[async_trait]
impl StatefulAction for PaginatedFetchAction {
    type State = PaginationState;
    type Input = PaginationInput;
    type Output = PaginationOutput;
    
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Check limits
        if let Some(max_pages) = input.max_pages {
            if state.current_page >= max_pages {
                return Ok(ActionResult::Break {
                    output: PaginationOutput {
                        items: state.all_items.clone(),
                        total_fetched: state.all_items.len(),
                        pages_processed: state.current_page,
                        has_more: false,
                    },
                    reason: BreakReason::MaxIterationsReached { limit: max_pages },
                });
            }
        }
        
        // Fetch page
        let client = context.get_client::<HttpClient>("api").await?;
        let page_data = self.fetch_page(&client, &input.endpoint, state).await?;
        
        // Update state
        state.all_items.extend(page_data.items.clone());
        state.current_page += 1;
        state.next_token = page_data.next_token.clone();
        
        let current_output = PaginationOutput {
            items: page_data.items,
            total_fetched: state.all_items.len(),
            pages_processed: state.current_page,
            has_more: page_data.next_token.is_some(),
        };
        
        // Decide whether to continue
        if page_data.next_token.is_some() {
            Ok(ActionResult::Continue {
                output: current_output,
                progress: LoopProgress {
                    current_iteration: state.current_page,
                    total_items: state.total_pages,
                    processed_items: state.all_items.len(),
                    percentage: None,
                    estimated_time_remaining: None,
                    status_message: Some(format!("Fetched page {}", state.current_page)),
                },
                delay: Some(Duration::from_millis(100)), // Rate limiting
            })
        } else {
            Ok(ActionResult::Break {
                output: PaginationOutput {
                    items: state.all_items.clone(),
                    total_fetched: state.all_items.len(),
                    pages_processed: state.current_page,
                    has_more: false,
                },
                reason: BreakReason::Completed,
            })
        }
    }
}
```

### Example 3: Multi-Service Aggregator

```rust
#[derive(Action)]
#[action(
    id = "aggregate.multi_service",
    name = "Multi-Service Aggregator",
    description = "Aggregates data from multiple services in parallel"
)]
#[auth(telegram_bot, openai_api, postgres_db)]
pub struct MultiServiceAggregator;

#[derive(Parameters)]
pub struct AggregateInput {
    #[parameter(description = "User ID to aggregate data for")]
    pub user_id: String,
    
    #[parameter(description = "Services to query")]
    pub services: Vec<ServiceType>,
    
    #[parameter(description = "Aggregation strategy")]
    pub strategy: AggregationStrategy,
}

#[derive(Serialize)]
pub struct AggregateOutput {
    pub user_summary: UserSummary,
    pub service_results: HashMap<String, Value>,
    pub ai_insights: String,
    pub notification_sent: bool,
}

#[async_trait]
impl ProcessAction for MultiServiceAggregator {
    type Input = AggregateInput;
    type Output = AggregateOutput;
    
    async fn execute(
        &self,
        input: Self::Input,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Launch parallel queries
        let mut tasks = vec![];
        
        if input.services.contains(&ServiceType::Database) {
            let db_task = self.query_database(input.user_id.clone(), context.clone());
            tasks.push(("database", db_task));
        }
        
        if input.services.contains(&ServiceType::ExternalApi) {
            let api_task = self.query_external_api(input.user_id.clone(), context.clone());
            tasks.push(("external_api", api_task));
        }
        
        if input.services.contains(&ServiceType::Cache) {
            let cache_task = self.query_cache(input.user_id.clone(), context.clone());
            tasks.push(("cache", cache_task));
        }
        
        // Wait for all results
        let results = futures::future::join_all(
            tasks.into_iter().map(|(name, task)| async move {
                (name, task.await)
            })
        ).await;
        
        // Process results based on strategy
        let (service_results, user_summary) = match input.strategy {
            AggregationStrategy::FirstSuccess => {
                self.process_first_success(results)?
            }
            AggregationStrategy::AllRequired => {
                self.process_all_required(results)?
            }
            AggregationStrategy::BestEffort => {
                self.process_best_effort(results)?
            }
        };
        
        // Generate AI insights
        let openai = context.get_client::<OpenAIClient>("openai_api").await?;
        let ai_insights = self.generate_insights(&openai, &user_summary).await?;
        
        // Send notification
        let telegram = context.get_client::<TelegramBot>("telegram_bot").await?;
        let notification_sent = self.send_summary_notification(
            &telegram,
            &input.user_id,
            &ai_insights
        ).await.is_ok();
        
        Ok(ActionResult::Success(AggregateOutput {
            user_summary,
            service_results,
            ai_insights,
            notification_sent,
        }))
    }
}
```

## Testing

### Unit Testing Actions

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::testing::*;
    
    #[tokio::test]
    async fn test_process_action_success() {
        // Create action
        let action = ExampleProcessAction::new();
        
        // Create test context
        let context = TestContext::new()
            .with_variable("test_var", json!("test_value"))
            .with_credential("api_key", TestCredential::api_key("test-key-123"));
        
        // Create input
        let input = ProcessInput {
            data: "test data".to_string(),
            options: ProcessOptions::default(),
        };
        
        // Execute
        let result = action.execute(input, &context).await.unwrap();
        
        // Assert
        match result {
            ActionResult::Success(output) => {
                assert!(!output.result.is_empty());
                assert!(output.metadata.contains_key("processed_at"));
            }
            _ => panic!("Expected Success result"),
        }
        
        // Verify metrics were recorded
        assert_eq!(context.get_counter("processed_items"), Some(1.0));
    }
    
    #[tokio::test]
    async fn test_stateful_action_state_persistence() {
        let action = CounterAction::new();
        let mut state = CounterState::default();
        let context = TestContext::new();
        
        // First execution
        let input1 = CounterInput { increment: 10, tag: Some("test".to_string()) };
        let result1 = action.execute_with_state(input1, &mut state, &context).await.unwrap();
        
        assert!(matches!(result1, ActionResult::Continue { .. }));
        assert_eq!(state.count, 10);
        
        // Second execution
        let input2 = CounterInput { increment: 20, tag: None };
        let result2 = action.execute_with_state(input2, &mut state, &context).await.unwrap();
        
        assert!(matches!(result2, ActionResult::Continue { .. }));
        assert_eq!(state.count, 30);
        assert_eq!(state.history.len(), 2);
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_action_with_real_services() {
    // Create test environment
    let env = TestEnvironment::new()
        .with_service("postgres", PostgresContainer::new())
        .with_service("redis", RedisContainer::new())
        .start().await;
    
    // Create action with real dependencies
    let action = DatabaseAction::new();
    let context = env.create_context()
        .with_resource(env.get_service::<PgPool>("postgres"))
        .with_resource(env.get_service::<RedisClient>("redis"))
        .build();
    
    // Test with real services
    let input = DatabaseInput {
        query: "SELECT * FROM users WHERE active = true".to_string(),
        cache_result: true,
    };
    
    let result = action.execute(input, &context).await.unwrap();
    
    // Verify
    match result {
        ActionResult::Success(output) => {
            assert!(!output.rows.is_empty());
            assert!(output.cached);
        }
        _ => panic!("Expected Success"),
    }
    
    // Cleanup
    env.cleanup().await;
}
```

## Idempotency Support

### Automatic Idempotency for Actions

Actions can be made idempotent by implementing the `IdempotentAction` trait:

```rust
use nebula_action::idempotency::*;

#[derive(Action)]
#[action(id = "payment.process")]
#[idempotent] // Enable idempotency
pub struct ProcessPaymentAction;

#[derive(Parameters, Hash)]
pub struct PaymentInput {
    #[parameter(description = "Payment amount")]
    pub amount: Decimal,
    
    #[parameter(description = "Customer ID")]
    pub customer_id: String,
    
    // Excluded from idempotency key
    #[parameter(idempotency_exclude = true)]
    pub timestamp: DateTime<Utc>,
    
    // User-provided idempotency key
    #[parameter(idempotency_key = true)]
    pub idempotency_key: Option<String>,
}

#[async_trait]
impl IdempotentAction for ProcessPaymentAction {
    fn idempotency_config(&self) -> IdempotencyConfig {
        IdempotencyConfig {
            enabled: true,
            key_strategy: IdempotencyKeyStrategy::Hybrid {
                user_key_prefix: true,
                content_suffix: true,
            },
            deduplication_window: Duration::from_hours(24),
            conflict_behavior: ConflictBehavior::ReturnPrevious,
            storage_backend: IdempotencyStorageBackend::TierSpecific,
            result_caching: ResultCachingConfig {
                enabled: true,
                ttl: Duration::from_hours(48),
                compress_large_results: true,
            },
        }
    }
    
    async fn is_safe_to_retry(
        &self,
        input: &Self::Input,
        previous_result: &Self::Output,
        context: &ExecutionContext,
    ) -> Result<bool, IdempotencyError> {
        // Safe to retry only if payment failed
        Ok(!previous_result.success)
    }
}
```

### Idempotent Execution

The execution context automatically handles idempotency:

```rust
// Actions are executed with automatic idempotency
let result = context.execute_action(&action, input).await?;

// If this is a replay, metadata will indicate it
if result.is_replay() {
    context.log_info("Returning cached result from previous execution");
}
```

### Workflow-Level Idempotency

Workflows support checkpointing for idempotent execution:

```rust
let workflow_engine = WorkflowEngine::builder()
    .with_idempotency(IdempotencyConfig {
        checkpoint_strategy: CheckpointStrategy::AfterEachNode,
        checkpoint_storage: CheckpointStorage::Persistent,
        deduplication_window: Duration::from_hours(24),
    })
    .build();

// Execute workflow with idempotency key
let result = workflow_engine.execute(
    workflow,
    input,
    ExecutionOptions {
        idempotency_key: Some("order-12345".to_string()),
        resume_from_checkpoint: true,
    },
).await?;
```

## Best Practices

### 1. Action Design

- **Single Responsibility**: Each action should do one thing well
- **Idempotency**: Design actions to be idempotent when possible
- **Error Handling**: Use specific error types, not generic errors
- **Resource Management**: Clean up resources in lifecycle hooks

### 2. Idempotency Best Practices

- **Key Generation**: Use content-based keys for automatic deduplication
- **Time Windows**: Set appropriate deduplication windows
- **Safe Retries**: Implement `is_safe_to_retry` for critical operations
- **Result Caching**: Cache expensive operation results

### 2. Input/Output Design

- **Validation**: Validate inputs thoroughly
- **Defaults**: Provide sensible defaults for optional parameters
- **Serialization**: Ensure inputs/outputs are serializable
- **Documentation**: Document all parameters clearly

### 3. State Management

- **Minimal State**: Keep state as small as possible
- **Migration**: Always implement state migration for versioning
- **Persistence**: Consider state size for persistence backends
- **Consistency**: Ensure state updates are atomic

### 4. Performance

- **Async Operations**: Use async for I/O operations
- **Cancellation**: Check cancellation token regularly
- **Timeouts**: Set appropriate timeouts for external calls
- **Metrics**: Record performance metrics

### 5. Testing

- **Unit Tests**: Test core logic with mocked dependencies
- **Integration Tests**: Test with real services when critical
- **Error Cases**: Test error handling paths
- **State Transitions**: Test all state transitions

### 6. Security

- **Input Sanitization**: Sanitize all external inputs
- **Credential Handling**: Never log credentials
- **Authentication**: Use context credentials properly
- **Authorization**: Implement proper access controls

## Advanced Topics

### Custom Action Types

Create your own action types by implementing the base Action trait:

```rust
#[async_trait]
pub trait CustomAction: Action {
    type CustomConfig: DeserializeOwned + Send + Sync;
    type CustomOutput: Serialize + Send + Sync;
    
    async fn custom_execute(
        &self,
        config: Self::CustomConfig,
        context: &ExecutionContext,
    ) -> Result<Self::CustomOutput, ActionError>;
}
```

### Action Composition

Compose actions to create more complex behaviors:

```rust
pub struct CompositeAction {
    actions: Vec<Box<dyn Action>>,
    composition_strategy: CompositionStrategy,
}

impl CompositeAction {
    pub async fn execute_composed(
        &self,
        input: Value,
        context: &ExecutionContext,
    ) -> Result<Value, ActionError> {
        match self.composition_strategy {
            CompositionStrategy::Sequential => {
                let mut result = input;
                for action in &self.actions {
                    result = action.execute(result, context).await?;
                }
                Ok(result)
            }
            CompositionStrategy::Parallel => {
                // Execute all actions in parallel
                let results = futures::future::join_all(
                    self.actions.iter().map(|a| a.execute(input.clone(), context))
                ).await;
                // Aggregate results
                self.aggregate_results(results)
            }
        }
    }
}
```

### Dynamic Action Loading

Load actions dynamically at runtime:

```rust
pub struct ActionRegistry {
    actions: HashMap<String, Box<dyn ActionFactory>>,
}

impl ActionRegistry {
    pub fn register<A: Action + Default + 'static>(&mut self, id: &str) {
        self.actions.insert(
            id.to_string(),
            Box::new(DefaultActionFactory::<A>::new())
        );
    }
    
    pub fn create_action(&self, id: &str) -> Result<Box<dyn Action>, ActionError> {
        self.actions
            .get(id)
            .ok_or_else(|| ActionError::ActionNotFound(id.to_string()))?
            .create()
    }
}
```

## Migration Guide

### From v0.1 to v0.2

1. **ActionResult changes**:
   - `ActionResult::Output(T)` → `ActionResult::Success(T)`
   - New variants added for advanced control flow

2. **Context API changes**:
   - `context.get_service()` → `context.get_client()`
   - Added credential support

3. **State management**:
   - State migration now required for StatefulAction
   - Added state versioning support

## Performance Considerations

1. **Memory Usage**: Actions should minimize memory allocation
2. **Async Boundaries**: Use `.await` points judiciously
3. **Pooling**: Reuse expensive resources via SupplyAction
4. **Caching**: Cache results when appropriate
5. **Metrics**: Monitor action performance in production

## Troubleshooting

### Common Issues

1. **State Corruption**: Implement proper state validation
2. **Resource Leaks**: Use lifecycle hooks for cleanup
3. **Timeout Errors**: Set appropriate timeouts
4. **Serialization Failures**: Test with real data

### Debug Mode

Enable debug logging for actions:

```rust
env_logger::Builder::from_env(
    env_logger::Env::default().default_filter_or("nebula_action=debug")
).init();
```

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines.

## License

Licensed under MIT or Apache-2.0 at your option.