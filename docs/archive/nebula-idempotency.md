# nebula-idempotency

Comprehensive idempotency system for Nebula providing automatic deduplication, retry safety, and failure recovery at all levels.

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Idempotency Levels](#idempotency-levels)
4. [Implementation Guide](#implementation-guide)
5. [Storage Backends](#storage-backends)
6. [Workflow Checkpointing](#workflow-checkpointing)
7. [HTTP API Integration](#http-api-integration)
8. [Testing](#testing)
9. [Best Practices](#best-practices)

## Overview

nebula-idempotency provides:
- **Multi-level idempotency** - Action, Workflow, Request, and Transaction levels
- **Automatic key generation** - Content-based deduplication
- **Checkpoint/Resume** - Workflow recovery after failures
- **Tier-specific optimization** - From in-memory to distributed storage
- **Zero developer overhead** - Idempotency through simple annotations

## Architecture

### File Structure

```
nebula-idempotency/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                    # Main exports and prelude
│   │
│   ├── core/                     # Core traits and types
│   │   ├── mod.rs
│   │   ├── key.rs                # IdempotencyKey types
│   │   ├── config.rs             # Configuration types
│   │   ├── error.rs              # Error types
│   │   └── level.rs              # Idempotency levels
│   │
│   ├── action/                   # Action-level idempotency
│   │   ├── mod.rs
│   │   ├── trait.rs              # IdempotentAction trait
│   │   ├── executor.rs           # Idempotent executor
│   │   ├── cache.rs              # Result caching
│   │   └── key_generator.rs      # Key generation strategies
│   │
│   ├── workflow/                 # Workflow-level idempotency
│   │   ├── mod.rs
│   │   ├── checkpoint.rs         # Checkpointing system
│   │   ├── recovery.rs           # Failure recovery
│   │   ├── state.rs              # Workflow state management
│   │   └── strategies.rs         # Checkpoint strategies
│   │
│   ├── request/                  # Request-level idempotency
│   │   ├── mod.rs
│   │   ├── manager.rs            # Request deduplication
│   │   ├── cache.rs              # Response caching
│   │   ├── concurrent.rs         # Concurrent request handling
│   │   └── middleware.rs         # HTTP middleware
│   │
│   ├── storage/                  # Storage backends
│   │   ├── mod.rs
│   │   ├── traits.rs             # Storage traits
│   │   ├── memory.rs             # In-memory storage
│   │   ├── postgres.rs           # PostgreSQL storage
│   │   ├── redis.rs              # Redis storage
│   │   └── distributed.rs        # Distributed storage
│   │
│   ├── integration/              # Integration with core modules
│   │   ├── mod.rs
│   │   ├── action.rs             # Action system integration
│   │   ├── credential.rs         # Credential integration
│   │   ├── resource.rs           # Resource integration
│   │   └── context.rs            # ExecutionContext integration
│   │
│   └── prelude.rs                # Common imports
│
├── examples/
│   ├── action_idempotency.rs    # Idempotent actions
│   ├── workflow_checkpoint.rs    # Workflow checkpointing
│   ├── api_deduplication.rs     # HTTP API deduplication
│   └── distributed_locks.rs      # Distributed idempotency
│
└── tests/
    ├── integration/
    └── unit/
```

## Idempotency Levels

### 1. Action Level

Individual actions can be made idempotent:

```rust
use nebula_idempotency::prelude::*;

#[derive(Action)]
#[action(id = "send.email")]
#[idempotent]
pub struct SendEmailAction;

#[derive(Parameters, Hash)]
pub struct EmailInput {
    #[parameter(description = "Recipient email")]
    pub to: String,
    
    #[parameter(description = "Email subject")]
    pub subject: String,
    
    #[parameter(description = "Email body")]
    pub body: String,
    
    // Excluded from idempotency key
    #[parameter(idempotency_exclude = true)]
    pub sent_at: DateTime<Utc>,
    
    // User-provided key
    #[parameter(idempotency_key = true)]
    pub message_id: Option<String>,
}

#[async_trait]
impl IdempotentAction for SendEmailAction {
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
            result_caching: ResultCachingConfig::default(),
        }
    }
    
    async fn is_safe_to_retry(
        &self,
        input: &Self::Input,
        previous_result: &Self::Output,
        context: &ExecutionContext,
    ) -> Result<bool, IdempotencyError> {
        // Safe to retry only if email wasn't sent
        Ok(!previous_result.sent)
    }
}
```

### 2. Workflow Level

Workflows support checkpointing and resume:

```rust
#[derive(Workflow)]
#[workflow(
    id = "order.processing",
    idempotent = true,
    checkpoint_strategy = "after_each_node"
)]
pub struct OrderProcessingWorkflow {
    pub nodes: Vec<WorkflowNode>,
}

impl OrderProcessingWorkflow {
    pub async fn execute_with_idempotency(
        &self,
        input: OrderInput,
        idempotency_key: Option<String>,
    ) -> Result<OrderOutput, WorkflowError> {
        let engine = WorkflowEngine::new()
            .with_checkpoint_manager(CheckpointManager::new(
                CheckpointConfig {
                    strategy: CheckpointStrategy::AfterEachNode,
                    storage: CheckpointStorage::Persistent,
                    retention: Duration::from_days(7),
                }
            ));
        
        // Execute with automatic checkpointing
        engine.execute_idempotent(
            self,
            input,
            ExecutionOptions {
                idempotency_key,
                resume_from_checkpoint: true,
                checkpoint_on_error: true,
            },
        ).await
    }
}
```

### 3. Request Level

HTTP requests are automatically deduplicated:

```rust
use axum::{Router, middleware};
use nebula_idempotency::http::IdempotencyLayer;

let app = Router::new()
    .route("/api/orders", post(create_order))
    .route("/api/payments", post(process_payment))
    .layer(
        IdempotencyLayer::new()
            .with_key_header("Idempotency-Key")
            .with_storage(storage_backend)
            .with_ttl(Duration::from_hours(24))
            .with_conflict_strategy(ConflictStrategy::WaitForFirst {
                timeout: Duration::from_secs(30),
            })
    );

async fn create_order(
    IdempotencyKey(key): IdempotencyKey,
    Json(request): Json<CreateOrderRequest>,
) -> Result<Json<CreateOrderResponse>, ApiError> {
    // Idempotency is handled automatically by the layer
    let order = service.create_order(request).await?;
    Ok(Json(order))
}
```

### 4. Transaction Level

Distributed transactions with idempotency:

```rust
#[derive(TransactionalAction)]
#[transaction(
    idempotent = true,
    isolation = "serializable"
)]
pub struct TransferMoneyAction;

impl TransferMoneyAction {
    pub async fn execute_idempotent(
        &self,
        from_account: AccountId,
        to_account: AccountId,
        amount: Decimal,
        transaction_id: Uuid,
    ) -> Result<TransferResult, TransactionError> {
        let tx_manager = TransactionManager::new()
            .with_idempotency(IdempotencyConfig {
                deduplication_key: transaction_id.to_string(),
                deduplication_window: Duration::from_days(30),
                storage: IdempotencyStorage::Persistent,
            });
        
        // Execute with 2PC and idempotency
        tx_manager.execute_2pc(|tx| async move {
            // Debit from account
            tx.execute(
                DebitAccount { account: from_account, amount },
                IdempotencyKey::from(format!("{}-debit", transaction_id)),
            ).await?;
            
            // Credit to account
            tx.execute(
                CreditAccount { account: to_account, amount },
                IdempotencyKey::from(format!("{}-credit", transaction_id)),
            ).await?;
            
            Ok(TransferResult { transaction_id, amount })
        }).await
    }
}
```

## Implementation Guide

### Step 1: Enable Idempotency for Actions

```rust
// 1. Add idempotent attribute
#[derive(Action)]
#[idempotent]
pub struct MyAction;

// 2. Implement IdempotentAction trait
#[async_trait]
impl IdempotentAction for MyAction {
    fn idempotency_config(&self) -> IdempotencyConfig {
        IdempotencyConfig::default()
    }
}

// 3. Action is now automatically idempotent
let result = executor.execute(&action, input).await?;
```

### Step 2: Configure Key Generation

```rust
// Content-based keys (automatic)
#[derive(Parameters, Hash)]
pub struct AutoKeyInput {
    pub field1: String,
    pub field2: i32,
    
    #[idempotency_exclude]
    pub timestamp: DateTime<Utc>,
}

// User-provided keys
#[derive(Parameters)]
pub struct UserKeyInput {
    #[idempotency_key]
    pub request_id: String,
    pub data: String,
}

// Hybrid approach
impl IdempotentAction for MyAction {
    fn idempotency_config(&self) -> IdempotencyConfig {
        IdempotencyConfig {
            key_strategy: IdempotencyKeyStrategy::Hybrid {
                user_key_prefix: true,
                content_suffix: true,
            },
            ..Default::default()
        }
    }
}
```

### Step 3: Handle Conflicts

```rust
impl IdempotentAction for MyAction {
    fn idempotency_config(&self) -> IdempotencyConfig {
        IdempotencyConfig {
            conflict_behavior: ConflictBehavior::WaitForCompletion {
                timeout: Duration::from_secs(30),
            },
            ..Default::default()
        }
    }
    
    async fn merge_results(
        &self,
        previous: Self::Output,
        current: Self::Output,
        context: &ExecutionContext,
    ) -> Result<Self::Output, IdempotencyError> {
        // Custom merge logic
        if previous.version > current.version {
            Ok(previous)
        } else {
            Ok(current)
        }
    }
}
```

## Storage Backends

### In-Memory Storage (Personal Tier)

```rust
let storage = InMemoryIdempotencyStorage::new(
    InMemoryConfig {
        max_entries: 10_000,
        ttl: Duration::from_hours(1),
        cleanup_interval: Duration::from_mins(5),
    }
);
```

### PostgreSQL Storage (Enterprise Tier)

```rust
let storage = PostgresIdempotencyStorage::new(
    PostgresConfig {
        connection_pool: pg_pool,
        table_name: "idempotency_keys",
        partition_strategy: PartitionStrategy::Monthly,
    }
).await?;

// Automatic table creation
storage.ensure_schema().await?;
```

### Distributed Storage (Cloud Tier)

```rust
let storage = DistributedIdempotencyStorage::new(
    DistributedConfig {
        redis_cluster: redis_urls,
        replication_factor: 3,
        consistency_level: ConsistencyLevel::Quorum,
        sharding_strategy: ShardingStrategy::ConsistentHashing,
    }
).await?;
```

## Workflow Checkpointing

### Checkpoint Strategies

```rust
pub enum CheckpointStrategy {
    /// Checkpoint after every node
    AfterEachNode,
    
    /// Only at critical points
    CriticalPointsOnly {
        critical_nodes: HashSet<NodeId>,
    },
    
    /// Time-based checkpoints
    TimeBased {
        interval: Duration,
    },
    
    /// Adaptive ML-based
    Adaptive {
        base_strategy: Box<CheckpointStrategy>,
        ml_optimization: bool,
    },
}
```

### Recovery from Checkpoint

```rust
let checkpoint_manager = CheckpointManager::new(storage);

// Find latest checkpoint
if let Some(checkpoint) = checkpoint_manager
    .find_latest_checkpoint(&workflow_id)
    .await? 
{
    // Resume from checkpoint
    let result = workflow_engine
        .resume_from_checkpoint(checkpoint)
        .await?;
} else {
    // Start fresh
    let result = workflow_engine
        .execute(workflow, input)
        .await?;
}
```

### Checkpoint Data

```rust
#[derive(Serialize, Deserialize)]
pub struct WorkflowCheckpoint {
    pub workflow_id: WorkflowId,
    pub checkpoint_id: CheckpointId,
    pub created_at: DateTime<Utc>,
    pub completed_nodes: HashSet<NodeId>,
    pub workflow_state: WorkflowState,
    pub node_outputs: HashMap<NodeId, Value>,
    pub variables: HashMap<String, Value>,
}
```

## HTTP API Integration

### Axum Middleware

```rust
use nebula_idempotency::http::{IdempotencyLayer, IdempotencyConfig};

let idempotency_config = IdempotencyConfig {
    key_header: "Idempotency-Key".to_string(),
    key_prefix: "api".to_string(),
    response_cache_ttl: Duration::from_hours(24),
    supported_methods: vec![Method::POST, Method::PUT, Method::PATCH],
    conflict_strategy: ConflictStrategy::WaitForFirst {
        timeout: Duration::from_secs(30),
    },
};

let app = Router::new()
    .route("/api/v1/*path", service)
    .layer(IdempotencyLayer::new(idempotency_config, storage));
```

### Response Caching

```rust
// Responses are automatically cached
POST /api/orders
Idempotency-Key: unique-key-123
{
    "product_id": "prod-456",
    "quantity": 2
}

// Response
HTTP/1.1 201 Created
X-Idempotency-Key: unique-key-123
X-Idempotency-Replay: false
{
    "order_id": "order-789",
    "status": "created"
}

// Subsequent request with same key
POST /api/orders
Idempotency-Key: unique-key-123
{
    "product_id": "prod-456",
    "quantity": 2
}

// Cached response
HTTP/1.1 201 Created
X-Idempotency-Key: unique-key-123
X-Idempotency-Replay: true
X-Original-Timestamp: 2024-01-01T00:00:00Z
{
    "order_id": "order-789",
    "status": "created"
}
```

## Testing

### Unit Testing Idempotency

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_idempotency::testing::*;
    
    #[tokio::test]
    async fn test_action_idempotency() {
        let storage = MockIdempotencyStorage::new();
        let executor = IdempotentExecutor::new(storage);
        
        let action = MyIdempotentAction;
        let input = MyInput { data: "test".to_string() };
        
        // First execution
        let result1 = executor.execute(&action, input.clone()).await?;
        assert!(!result1.is_replay());
        
        // Second execution with same input
        let result2 = executor.execute(&action, input.clone()).await?;
        assert!(result2.is_replay());
        assert_eq!(result1.output, result2.output);
    }
    
    #[tokio::test]
    async fn test_concurrent_requests() {
        let storage = MockIdempotencyStorage::new();
        let manager = RequestIdempotencyManager::new(storage);
        
        let key = IdempotencyKey::from("test-key");
        
        // Simulate concurrent requests
        let (tx, rx) = oneshot::channel();
        
        // First request
        let handle1 = tokio::spawn(async move {
            let result = manager.execute_request(key.clone(), async {
                // Simulate work
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok("result".to_string())
            }).await;
            tx.send(()).unwrap();
            result
        });
        
        // Second request (concurrent)
        let handle2 = tokio::spawn(async move {
            // Wait a bit to ensure first request starts
            tokio::time::sleep(Duration::from_millis(10)).await;
            
            manager.execute_request(key, async {
                Ok("different-result".to_string())
            }).await
        });
        
        let result1 = handle1.await??;
        let result2 = handle2.await??;
        
        // Both should return same result
        assert_eq!(result1, result2);
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_workflow_checkpoint_recovery() {
    let storage = PostgresIdempotencyStorage::new(test_db()).await?;
    let checkpoint_manager = CheckpointManager::new(storage);
    
    let workflow = create_test_workflow();
    let input = WorkflowInput { data: "test" };
    
    // Execute with simulated failure
    let engine = WorkflowEngine::new()
        .with_checkpoint_manager(checkpoint_manager.clone())
        .with_node_error_injection("node3", Some(TestError::Transient));
    
    let result1 = engine.execute(&workflow, input.clone()).await;
    assert!(result1.is_err());
    
    // Verify checkpoint was created
    let checkpoint = checkpoint_manager
        .find_latest_checkpoint(&workflow.id)
        .await?
        .expect("Checkpoint should exist");
    
    assert_eq!(checkpoint.completed_nodes.len(), 2); // First 2 nodes completed
    
    // Resume from checkpoint
    let engine2 = WorkflowEngine::new()
        .with_checkpoint_manager(checkpoint_manager);
    
    let result2 = engine2.resume_from_checkpoint(checkpoint).await?;
    assert_eq!(result2.completed_nodes.len(), 4); // All nodes completed
}
```

## Best Practices

### 1. Key Generation

- **Include Relevant Fields**: Include all fields that affect the operation outcome
- **Exclude Timestamps**: Exclude non-deterministic fields like timestamps
- **Use Stable IDs**: Prefer stable identifiers over generated values

### 2. Deduplication Windows

- **Action Level**: 5 minutes to 1 hour for immediate retries
- **Workflow Level**: 24 hours to 7 days for long-running processes
- **Transaction Level**: 30 days for financial operations

### 3. Storage Selection

- **Personal Tier**: In-memory with size limits
- **Enterprise Tier**: PostgreSQL with partitioning
- **Cloud Tier**: Distributed storage with replication

### 4. Error Handling

- **Safe Retries**: Implement `is_safe_to_retry` for critical operations
- **Merge Conflicts**: Handle concurrent execution results
- **Cleanup**: Implement automatic cleanup for expired records

### 5. Performance

- **Async Operations**: Use async storage operations
- **Batch Operations**: Batch multiple idempotency checks
- **Index Optimization**: Create appropriate database indexes
- **Monitoring**: Track hit rates and storage usage

## Monitoring

### Metrics

```rust
let metrics = IdempotencyMetrics::new();

// Track deduplication
metrics.record_deduplication("action", true); // Cache hit
metrics.record_deduplication("action", false); // Cache miss

// Storage metrics
metrics.record_storage_operation("get", Duration::from_micros(150));
metrics.record_storage_size(1_234_567);

// Export to Prometheus
GET /metrics
# TYPE idempotency_deduplication_total counter
idempotency_deduplication_total{level="action",hit="true"} 1234
idempotency_deduplication_total{level="action",hit="false"} 567

# TYPE idempotency_storage_size_bytes gauge
idempotency_storage_size_bytes 1234567
```

### Dashboards

```rust
// Built-in dashboard data
let dashboard = IdempotencyDashboard::new(storage, metrics);

let stats = dashboard.get_stats().await?;
println!("Hit rate: {:.2}%", stats.hit_rate * 100.0);
println!("Active keys: {}", stats.active_keys);
println!("Storage usage: {} MB", stats.storage_bytes / 1_048_576);
```

## Troubleshooting

### Common Issues

1. **Duplicate Operations**
   - Check idempotency key generation
   - Verify deduplication window
   - Review storage backend health

2. **Checkpoint Failures**
   - Check storage permissions
   - Verify checkpoint size limits
   - Review serialization errors

3. **Performance Issues**
   - Monitor storage latency
   - Check index usage
   - Review cache hit rates

4. **Storage Growth**
   - Configure cleanup intervals
   - Set appropriate TTLs
   - Monitor storage metrics

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines.

## License

Licensed under MIT or Apache-2.0 at your option.