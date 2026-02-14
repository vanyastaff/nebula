---
title: "How to: StatefulAction"
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---

# How to: StatefulAction

#action #statefulaction #state-management

## Overview

StatefulAction maintains state between executions, enabling iterative processing, accumulation, and progress tracking. This guide shows how to create and manage stateful actions effectively.

## Quick Start

### Basic Implementation

```rust
use nebula_action::prelude::*;
use serde::{Deserialize, Serialize};

pub struct CounterAction {
    metadata: ActionMetadata,
}

// Define state structure
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct CounterState {
    pub count: u64,
    pub last_updated: Option<DateTime<Utc>>,
    pub history: Vec<CountEvent>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CountEvent {
    pub timestamp: DateTime<Utc>,
    pub increment: u64,
    pub message: Option<String>,
}

// Input/Output types
#[derive(Deserialize)]
pub struct CounterInput {
    pub increment: u64,
    pub message: Option<String>,
}

#[derive(Serialize)]
pub struct CounterOutput {
    pub previous_count: u64,
    pub current_count: u64,
    pub total_events: usize,
}

impl CounterAction {
    pub fn new() -> Result<Self, ActionError> {
        let metadata = ActionMetadata::builder()
            .key("example.counter")
            .name("Counter Action")
            .description("Maintains count state between executions")
            .version("1.0.0")
            .build()?;
        
        Ok(Self { metadata })
    }
}

// Required trait implementations
impl HasMetadata for CounterAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl HasType for CounterAction {
    fn r#type(&self) -> ActionType {
        ActionType::Stateful
    }
}

impl Action for CounterAction {}

// StatefulAction implementation
#[async_trait]
impl StatefulAction for CounterAction {
    type State = CounterState;
    type Input = CounterInput;
    type Output = CounterOutput;
    
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        let previous_count = state.count;
        
        // Update state
        state.count += input.increment;
        state.last_updated = Some(Utc::now());
        state.history.push(CountEvent {
            timestamp: Utc::now(),
            increment: input.increment,
            message: input.message,
        });
        
        // Log progress
        context.log_info(&format!(
            "Count updated: {} -> {}", 
            previous_count, 
            state.count
        ));
        
        // Create output
        let output = CounterOutput {
            previous_count,
            current_count: state.count,
            total_events: state.history.len(),
        };
        
        // Decide on control flow
        if state.count >= 100 {
            // Complete when reaching 100
            Ok(ActionResult::Break {
                output,
                reason: BreakReason::Completed,
            })
        } else {
            // Continue processing
            Ok(ActionResult::Continue {
                output,
                progress: LoopProgress {
                    current_iteration: state.history.len(),
                    total_items: Some(100),
                    processed_items: state.count as usize,
                    percentage: Some(state.count as f32),
                    estimated_time_remaining: None,
                    status_message: Some(format!("Count: {}/100", state.count)),
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
        if old_version.major < 1 {
            // Migrate from v0 to v1
            #[derive(Deserialize)]
            struct OldState {
                count: u64,
            }
            
            let old: OldState = serde_json::from_value(old_state)?;
            
            Ok(CounterState {
                count: old.count,
                last_updated: None,
                history: Vec::new(),
            })
        } else {
            // Current version
            Ok(serde_json::from_value(old_state)?)
        }
    }
}
```

## Common Patterns

### Batch Processing with Checkpoints

```rust
pub struct BatchProcessor {
    metadata: ActionMetadata,
    batch_size: usize,
}

#[derive(Serialize, Deserialize, Default)]
pub struct BatchState {
    pub processed_items: Vec<String>,
    pub failed_items: Vec<FailedItem>,
    pub current_batch: usize,
    pub total_batches: Option<usize>,
    pub checkpoint: ProcessingCheckpoint,
}

#[derive(Serialize, Deserialize)]
pub struct ProcessingCheckpoint {
    pub last_processed_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub offset: usize,
}

#[async_trait]
impl StatefulAction for BatchProcessor {
    type State = BatchState;
    type Input = BatchInput;
    type Output = BatchOutput;
    
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Resume from checkpoint
        let start_offset = if let Some(last_id) = &state.checkpoint.last_processed_id {
            input.items.iter()
                .position(|item| item.id == *last_id)
                .unwrap_or(0) + 1
        } else {
            0
        };
        
        // Process batch
        let batch_end = (start_offset + self.batch_size).min(input.items.len());
        let batch = &input.items[start_offset..batch_end];
        
        for item in batch {
            match self.process_item(item, context).await {
                Ok(processed) => {
                    state.processed_items.push(processed);
                    state.checkpoint.last_processed_id = Some(item.id.clone());
                }
                Err(e) => {
                    state.failed_items.push(FailedItem {
                        id: item.id.clone(),
                        error: e.to_string(),
                        timestamp: Utc::now(),
                    });
                }
            }
            
            // Check for cancellation
            if context.is_cancelled() {
                return Ok(ActionResult::Break {
                    output: self.create_output(state),
                    reason: BreakReason::UserRequested,
                });
            }
        }
        
        // Update checkpoint
        state.checkpoint.timestamp = Utc::now();
        state.checkpoint.offset = batch_end;
        state.current_batch += 1;
        
        // Determine next action
        if batch_end >= input.items.len() {
            Ok(ActionResult::Break {
                output: self.create_output(state),
                reason: BreakReason::Completed,
            })
        } else {
            Ok(ActionResult::Continue {
                output: self.create_output(state),
                progress: LoopProgress {
                    current_iteration: state.current_batch,
                    total_items: state.total_batches,
                    processed_items: state.processed_items.len(),
                    percentage: Some(
                        (batch_end as f32 / input.items.len() as f32) * 100.0
                    ),
                    estimated_time_remaining: self.estimate_time_remaining(state, input.items.len()),
                    status_message: Some(format!(
                        "Processed {}/{} items",
                        batch_end,
                        input.items.len()
                    )),
                },
                delay: Some(Duration::from_millis(100)), // Rate limiting
            })
        }
    }
}
```

### Paginated Data Fetcher

```rust
pub struct PaginatedFetcher {
    metadata: ActionMetadata,
    page_size: usize,
}

#[derive(Serialize, Deserialize, Default)]
pub struct PaginationState {
    pub all_items: Vec<Item>,
    pub current_page: usize,
    pub next_token: Option<String>,
    pub has_more: bool,
    pub fetch_metadata: FetchMetadata,
}

#[derive(Serialize, Deserialize, Default)]
pub struct FetchMetadata {
    pub total_pages: usize,
    pub total_items: usize,
    pub first_fetch: Option<DateTime<Utc>>,
    pub last_fetch: Option<DateTime<Utc>>,
}

#[async_trait]
impl StatefulAction for PaginatedFetcher {
    type State = PaginationState;
    type Input = FetchConfig;
    type Output = FetchResult;
    
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Check if we should stop
        if !state.has_more && state.current_page > 0 {
            return Ok(ActionResult::Break {
                output: FetchResult {
                    items: state.all_items.clone(),
                    metadata: state.fetch_metadata.clone(),
                },
                reason: BreakReason::Completed,
            });
        }
        
        // Fetch next page
        let client = context.get_client::<HttpClient>("api").await?;
        let response = self.fetch_page(
            &client,
            &input.endpoint,
            state.next_token.as_deref(),
            self.page_size,
        ).await?;
        
        // Update state
        if state.first_fetch.is_none() {
            state.fetch_metadata.first_fetch = Some(Utc::now());
        }
        state.fetch_metadata.last_fetch = Some(Utc::now());
        
        state.all_items.extend(response.items.clone());
        state.current_page += 1;
        state.next_token = response.next_token.clone();
        state.has_more = response.next_token.is_some();
        
        state.fetch_metadata.total_pages = state.current_page;
        state.fetch_metadata.total_items = state.all_items.len();
        
        // Log progress
        context.log_info(&format!(
            "Fetched page {}: {} items (total: {})",
            state.current_page,
            response.items.len(),
            state.all_items.len()
        ));
        
        // Decide next action
        if state.has_more && state.current_page < input.max_pages.unwrap_or(usize::MAX) {
            Ok(ActionResult::Continue {
                output: FetchResult {
                    items: response.items,
                    metadata: state.fetch_metadata.clone(),
                },
                progress: LoopProgress {
                    current_iteration: state.current_page,
                    total_items: None, // Unknown total
                    processed_items: state.all_items.len(),
                    percentage: None,
                    estimated_time_remaining: None,
                    status_message: Some(format!(
                        "Fetched {} pages, {} total items",
                        state.current_page,
                        state.all_items.len()
                    )),
                },
                delay: Some(Duration::from_millis(input.rate_limit_ms)),
            })
        } else {
            Ok(ActionResult::Break {
                output: FetchResult {
                    items: state.all_items.clone(),
                    metadata: state.fetch_metadata.clone(),
                },
                reason: if state.has_more {
                    BreakReason::MaxIterationsReached { 
                        limit: input.max_pages.unwrap() 
                    }
                } else {
                    BreakReason::Completed
                },
            })
        }
    }
}
```

### Accumulator Pattern

```rust
pub struct DataAccumulator {
    metadata: ActionMetadata,
}

#[derive(Serialize, Deserialize, Default)]
pub struct AccumulatorState {
    pub accumulated_data: AccumulatedData,
    pub sources_processed: HashSet<String>,
    pub statistics: AccumulationStats,
}

#[derive(Serialize, Deserialize, Default)]
pub struct AccumulatedData {
    pub values: Vec<f64>,
    pub sum: f64,
    pub count: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[async_trait]
impl StatefulAction for DataAccumulator {
    type State = AccumulatorState;
    type Input = DataSource;
    type Output = AccumulationResult;
    
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // Check if already processed
        if state.sources_processed.contains(&input.source_id) {
            return Ok(ActionResult::Skip {
                reason: format!("Source {} already processed", input.source_id),
            });
        }
        
        // Accumulate data
        for value in input.values {
            state.accumulated_data.values.push(value);
            state.accumulated_data.sum += value;
            state.accumulated_data.count += 1;
            
            state.accumulated_data.min = Some(
                state.accumulated_data.min.map_or(value, |m| m.min(value))
            );
            state.accumulated_data.max = Some(
                state.accumulated_data.max.map_or(value, |m| m.max(value))
            );
        }
        
        state.sources_processed.insert(input.source_id.clone());
        
        // Update statistics
        state.statistics.last_update = Utc::now();
        state.statistics.sources_count = state.sources_processed.len();
        state.statistics.average = 
            state.accumulated_data.sum / state.accumulated_data.count as f64;
        
        // Create output
        let output = AccumulationResult {
            current_stats: state.statistics.clone(),
            data_snapshot: state.accumulated_data.clone(),
        };
        
        // Continue accumulating
        Ok(ActionResult::Accumulate {
            current_value: output,
            accumulator_state: AccumulatorState {
                current_count: state.accumulated_data.count,
                is_complete: false,
            },
            continue_accumulation: true,
        })
    }
}
```

## State Management

### State Persistence

```rust
// State is automatically persisted by the engine
// You can control persistence behavior:

impl StatefulAction for MyAction {
    fn state_config(&self) -> StateConfig {
        StateConfig {
            persistence: PersistenceStrategy::AfterEachExecution,
            compression: true,
            encryption: true,
            ttl: Some(Duration::from_days(30)),
        }
    }
}
```

### State Migration

```rust
impl StatefulAction for MyAction {
    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        old_version: semver::Version,
    ) -> Result<Self::State, ActionError> {
        match (old_version.major, old_version.minor) {
            (0, _) => {
                // Migrate from v0.x to current
                self.migrate_from_v0(old_state)
            }
            (1, minor) if minor < 5 => {
                // Migrate from v1.0-1.4 to current
                self.migrate_from_v1_early(old_state)
            }
            (1, _) => {
                // Migrate from v1.5+ to current
                self.migrate_from_v1_late(old_state)
            }
            _ => {
                // Try direct deserialization
                serde_json::from_value(old_state)
                    .map_err(|e| ActionError::StateMigrationFailed(e.to_string()))
            }
        }
    }
    
    fn migrate_from_v0(&self, old_state: Value) -> Result<Self::State, ActionError> {
        #[derive(Deserialize)]
        struct V0State {
            data: String,
        }
        
        let v0: V0State = serde_json::from_value(old_state)?;
        
        Ok(CurrentState {
            data: v0.data,
            version: 2,
            metadata: Default::default(),
        })
    }
}
```

### State Validation

```rust
impl StatefulAction for MyAction {
    async fn validate_state(
        &self,
        state: &Self::State,
    ) -> Result<(), ActionError> {
        // Check state consistency
        if state.total != state.items.len() {
            return Err(ActionError::StateCorrupted(
                "Total count doesn't match items".to_string()
            ));
        }
        
        // Check state bounds
        if state.index >= state.items.len() {
            return Err(ActionError::StateCorrupted(
                "Index out of bounds".to_string()
            ));
        }
        
        Ok(())
    }
}
```

## Control Flow

### Loop Control

```rust
// Continue with progress
Ok(ActionResult::Continue {
    output,
    progress: LoopProgress {
        current_iteration: iteration,
        total_items: Some(total),
        processed_items: processed,
        percentage: Some((processed as f32 / total as f32) * 100.0),
        estimated_time_remaining: Some(Duration::from_secs(eta_seconds)),
        status_message: Some(format!("Processing item {}/{}", processed, total)),
    },
    delay: Some(Duration::from_millis(100)), // Rate limiting
})

// Break on completion
Ok(ActionResult::Break {
    output,
    reason: BreakReason::Completed,
})

// Break on condition
Ok(ActionResult::Break {
    output,
    reason: BreakReason::ConditionMet {
        condition: "threshold_reached".to_string(),
    },
})

// Break on error
Ok(ActionResult::Break {
    output: partial_output,
    reason: BreakReason::Error {
        message: "Critical error occurred".to_string(),
    },
})
```

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::testing::*;
    
    #[tokio::test]
    async fn test_state_updates() {
        let action = CounterAction::new().unwrap();
        let mut state = CounterState::default();
        let context = TestContext::new();
        
        // First execution
        let input1 = CounterInput {
            increment: 10,
            message: Some("First".to_string()),
        };
        
        let result1 = action.execute_with_state(
            input1,
            &mut state,
            &context
        ).await.unwrap();
        
        assert_eq!(state.count, 10);
        assert_eq!(state.history.len(), 1);
        assert!(matches!(result1, ActionResult::Continue { .. }));
        
        // Second execution
        let input2 = CounterInput {
            increment: 15,
            message: None,
        };
        
        let result2 = action.execute_with_state(
            input2,
            &mut state,
            &context
        ).await.unwrap();
        
        assert_eq!(state.count, 25);
        assert_eq!(state.history.len(), 2);
    }
    
    #[tokio::test]
    async fn test_state_migration() {
        let action = MyAction::new().unwrap();
        
        // Old state format
        let old_state = json!({
            "count": 42,
            "data": "test"
        });
        
        let old_version = semver::Version::new(0, 9, 0);
        
        let migrated = action.migrate_state(old_state, old_version)
            .await
            .unwrap();
        
        assert_eq!(migrated.count, 42);
        assert!(migrated.metadata.is_some());
    }
}
```

### State Persistence Test

```rust
#[tokio::test]
async fn test_state_persistence() {
    let storage = MockStateStorage::new();
    let action = CounterAction::new().unwrap();
    let context = TestContext::with_storage(storage.clone());
    
    let state_id = "test_counter";
    
    // Save state
    let mut state = CounterState {
        count: 42,
        last_updated: Some(Utc::now()),
        history: vec![],
    };
    
    storage.save_state(state_id, &state).await.unwrap();
    
    // Load state
    let loaded: CounterState = storage.load_state(state_id)
        .await
        .unwrap()
        .unwrap();
    
    assert_eq!(loaded.count, 42);
}
```

## Best Practices

### ✅ DO's

1. **Keep state minimal** - Only store essential data
2. **Implement state migration** - Handle version changes gracefully
3. **Validate state consistency** - Check invariants
4. **Use checkpoints** - Enable resume on failure
5. **Clean up old state** - Set appropriate TTLs
6. **Log state transitions** - Aid debugging

### ❌ DON'Ts

1. **Don't store large objects** - Use references or IDs
2. **Don't mutate state outside execute** - Keep it predictable
3. **Don't ignore migration** - Old states will fail
4. **Don't forget error recovery** - State might be corrupted
5. **Don't leak sensitive data** - Encrypt if needed

## Templates

### StatefulAction Template

```rust
// <% tp.file.cursor() %>
use nebula_action::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct <%= tp.file.title %>State {
    // TODO: Define state fields
}

#[derive(Deserialize)]
pub struct <%= tp.file.title %>Input {
    // TODO: Define input fields
}

#[derive(Serialize)]
pub struct <%= tp.file.title %>Output {
    // TODO: Define output fields
}

pub struct <%= tp.file.title %>Action {
    metadata: ActionMetadata,
}

impl <%= tp.file.title %>Action {
    pub fn new() -> Result<Self, ActionError> {
        let metadata = ActionMetadata::builder()
            .key("<%= tp.file.title.toLowerCase() %>")
            .name("<%= tp.file.title %> Action")
            .description("TODO: Add description")
            .version("1.0.0")
            .build()?;
        
        Ok(Self { metadata })
    }
}

impl HasMetadata for <%= tp.file.title %>Action {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl HasType for <%= tp.file.title %>Action {
    fn r#type(&self) -> ActionType {
        ActionType::Stateful
    }
}

impl Action for <%= tp.file.title %>Action {}

#[async_trait]
impl StatefulAction for <%= tp.file.title %>Action {
    type State = <%= tp.file.title %>State;
    type Input = <%= tp.file.title %>Input;
    type Output = <%= tp.file.title %>Output;
    
    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        context: &dyn ExecutionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError> {
        // TODO: Implement state logic
        
        Ok(ActionResult::Success(<%= tp.file.title %>Output {
            // TODO: Return output
        }))
    }
    
    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        old_version: semver::Version,
    ) -> Result<Self::State, ActionError> {
        // TODO: Implement migration logic
        Ok(serde_json::from_value(old_state)?)
    }
}
```

## Related Documentation

- [[Action Types#StatefulAction]] - StatefulAction overview
- [[Action Lifecycle]] - State lifecycle management
- [[Action Result System#Control Flow Results]] - Loop control
- [[Examples#StatefulAction Examples]] - More examples