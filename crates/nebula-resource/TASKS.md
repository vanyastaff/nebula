# Implementation Tasks: nebula-resource

## How to Use This Document

Each task includes:
- **ID**: Unique identifier (e.g., P0-T1)
- **Title**: Clear description
- **File**: Files to modify
- **Dependencies**: Which tasks must complete first
- **Complexity**: trivial/easy/medium/hard/expert
- **Estimated Hours**: Time estimate
- **Acceptance Criteria**: Definition of done

**Complexity Guide**:
- `trivial`: <2 hours, straightforward change
- `easy`: 2-4 hours, clear implementation path
- `medium`: 4-8 hours, some design needed
- `hard`: 8-16 hours, complex logic or integration
- `expert`: 16+ hours, requires deep expertise

---

# Phase 0: Foundation Cleanup (Weeks 1-2)

## Critical Safety Fixes

### P0-T1: Remove unsafe code from testing module
**File**: `src/testing/mod.rs`
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 3

**Current Issue**:
```rust
// Line 70: UNSAFE - will cause undefined behavior
Ok(Arc::new(unsafe { std::mem::zeroed() }))
```

**Tasks**:
1. Remove `get_mock<T>()` method entirely (it's broken)
2. Replace with proper trait object pattern or remove type parameter
3. Update `TestResourceManager` to return `Arc<dyn TestableResource>` instead
4. Fix all test code that uses `get_mock()`

**Acceptance Criteria**:
- [ ] No unsafe code in `testing/mod.rs`
- [ ] `cargo clippy` passes without unsafe warnings
- [ ] All tests pass
- [ ] Documentation updated

**Implementation Plan**:
```rust
// Replace get_mock with:
pub async fn get_resource(&self, resource_id: &str) -> ResourceResult<Arc<dyn TestableResource>> {
    let resources = self.resources.lock().unwrap();
    let resource = resources
        .get(resource_id)
        .ok_or_else(|| ResourceError::unavailable(resource_id, "Mock not found", false))?
        .clone();

    // Record the call
    self.call_history.lock().unwrap().push(ResourceCall::Acquire {
        resource_id: resource_id.to_string(),
        timestamp: chrono::Utc::now(),
    });

    Ok(resource)
}
```

---

### P0-T2: Add compile-time unsafe prevention
**File**: `.cargo/config.toml`, `Cargo.toml`
**Dependencies**: P0-T1
**Complexity**: trivial
**Estimated Hours**: 1

**Tasks**:
1. Create `.cargo/config.toml` with lint configuration
2. Add clippy lints to prevent unsafe in tests

**Acceptance Criteria**:
- [ ] Clippy configured to deny unsafe in test modules
- [ ] CI enforces this

**Implementation**:
```toml
# .cargo/config.toml
[target.'cfg(test)']
rustflags = ["-D", "unsafe-code"]

# OR in Cargo.toml
[lints.clippy]
unsafe_code = "forbid"  # In test modules
```

---

## Standardization

### P0-T3: Unify lock types to parking_lot
**Files**: All source files
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 2

**Current State**: Mix of `std::sync::RwLock` and `parking_lot::RwLock`

**Tasks**:
1. Search for all `std::sync::RwLock` usage
2. Replace with `parking_lot::RwLock`
3. Same for Mutex
4. Update imports
5. Run tests

**Files to modify**:
- `src/core/resource.rs`
- `src/stateful/mod.rs`
- `src/observability/mod.rs`
- `src/testing/mod.rs`
- `src/resources/*.rs`

**Acceptance Criteria**:
- [ ] Only parking_lot locks used
- [ ] All tests pass
- [ ] No std::sync locks remain

---

### P0-T4: Remove unused dependencies
**File**: `Cargo.toml`
**Dependencies**: None
**Complexity**: trivial
**Estimated Hours**: 1

**Tasks**:
1. Run `cargo udeps` to identify unused dependencies
2. Remove: deadpool, bb8 (re-add in Phase 3 when implemented)
3. Remove: arc-swap (re-add when used)
4. Update feature flags to reflect removed deps
5. Test that project still builds

**Acceptance Criteria**:
- [ ] `cargo udeps` reports zero unused deps
- [ ] Project builds successfully
- [ ] All tests pass

---

## Documentation Alignment

### P0-T5: Update README with current state
**File**: `README.md`, `docs/README.md`
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 3

**Tasks**:
1. Add "Current Status" section showing implementation percentage
2. Mark unimplemented features with "🔜 Planned" badge
3. Mark implemented features with "✅ Complete" badge
4. Add "Current Limitations" section
5. Update code examples to show actual working code
6. Remove examples of unimplemented features

**Acceptance Criteria**:
- [ ] README clearly distinguishes implemented vs planned
- [ ] All code examples in README compile and run
- [ ] "Current Limitations" section lists all known gaps

---

### P0-T6: Add planning documentation
**File**: `ARCHITECTURE_ANALYSIS.md`, `VISION.md`, `ROADMAP.md`, `TASKS.md`
**Dependencies**: None
**Complexity**: N/A (being created now)
**Estimated Hours**: N/A

**Acceptance Criteria**:
- [x] ARCHITECTURE_ANALYSIS.md created
- [x] VISION.md created
- [x] ROADMAP.md created
- [x] TASKS.md created
- [ ] All documents reviewed by stakeholders

---

## Development Infrastructure

### P0-T7: Add comprehensive benchmarks
**File**: `benches/resource_benchmarks.rs` (new)
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 4

**Tasks**:
1. Create `benches/` directory
2. Add Criterion.rs benchmarks for:
   - Resource acquisition (pool hit)
   - Resource acquisition (pool miss)
   - Pool operations (acquire/release)
   - Context creation and propagation
   - Type registry lookup
3. Establish baseline measurements
4. Document benchmark results

**Acceptance Criteria**:
- [ ] `cargo bench` runs successfully
- [ ] Baseline benchmarks documented in PERFORMANCE.md
- [ ] At least 5 different benchmark scenarios

**Example benchmark**:
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_pool_acquire(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let pool = rt.block_on(async {
        // Setup pool with resources
    });

    c.bench_function("pool_acquire_hit", |b| {
        b.to_async(&rt).iter(|| async {
            let resource = pool.acquire().await.unwrap();
            black_box(resource);
        });
    });
}
```

---

### P0-T8: Setup CI/CD pipeline
**File**: `.github/workflows/ci.yml` (new)
**Dependencies**: P0-T7
**Complexity**: medium
**Estimated Hours**: 4

**Tasks**:
1. Create GitHub Actions workflow
2. Add test job (all features)
3. Add clippy job
4. Add rustfmt job
5. Add benchmark job with regression detection
6. Add coverage job (tarpaulin or cargo-llvm-cov)
7. Configure to run on PR and main branch

**Acceptance Criteria**:
- [ ] CI runs on every PR
- [ ] All checks must pass before merge
- [ ] Coverage report generated
- [ ] Benchmark regression detection active

**Example workflow**:
```yaml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - run: cargo test --all-features

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - run: cargo clippy --all-features -- -D warnings

  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - run: cargo bench --no-fail-fast
      # Add regression detection
```

---

# Phase 1: Core Foundation (Weeks 3-6)

## Complete Core Traits

### P1-T1: Fix ResourceInstance touch() implementation
**File**: `src/core/resource.rs`
**Dependencies**: P0-T3 (lock standardization)
**Complexity**: easy
**Estimated Hours**: 2

**Current Issue**: `touch()` takes `&self` but needs to mutate `last_accessed`

**Tasks**:
1. Change `last_accessed` to use `Mutex<Option<DateTime>>` or `AtomicU64`
2. Update `touch()` to use interior mutability
3. Update all implementations of ResourceInstance
4. Add tests for concurrent touch() calls

**Acceptance Criteria**:
- [ ] `touch()` compiles without `&mut self`
- [ ] Concurrent calls to `touch()` don't race
- [ ] All ResourceInstance implementations updated

**Implementation**:
```rust
pub trait ResourceInstance: Send + Sync {
    // ... other methods ...

    fn touch(&self) {
        // Use interior mutability
        if let Some(last) = self.last_accessed_mutex() {
            *last.lock().unwrap() = Some(chrono::Utc::now());
        }
    }

    // Helper method implementations provide
    fn last_accessed_mutex(&self) -> Option<&Mutex<Option<DateTime<Utc>>>>;
}
```

---

### P1-T2: Complete Resource trait default implementations
**File**: `src/core/traits/resource.rs`
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 6

**Current State**: Many `todo!()` macros in default implementations

**Tasks**:
1. Implement `prepare_config()` to actually merge defaults
2. Implement `on_created()` to emit events
3. Implement `validate_config()` with comprehensive checks
4. Remove all `todo!()` macros
5. Add doc comments explaining default behavior

**Acceptance Criteria**:
- [ ] No `todo!()` in trait implementations
- [ ] All methods have working default implementations
- [ ] Tests for each default method

---

### P1-T3: Fix TypedResourceInstance Arc handling
**File**: `src/core/resource.rs`
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 5

**Current Issue**: Type casting between `Arc<dyn Any>` and typed instances is fragile

**Tasks**:
1. Review Arc downcasting logic in `TypedResourceInstance`
2. Add proper error handling for failed casts
3. Implement Clone correctly (Arc clone, not instance clone)
4. Add tests for type safety

**Acceptance Criteria**:
- [ ] Type-safe downcasting works correctly
- [ ] Failed casts return proper errors (not panics)
- [ ] Clone creates new Arc, not new instance

---

### P1-T4: Complete HealthCheckable implementations
**File**: `src/core/traits/instance.rs`
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 3

**Tasks**:
1. Remove stub implementations
2. Add actual health check logic for built-in resources
3. Add configurable health check intervals
4. Implement health status aggregation

**Acceptance Criteria**:
- [ ] Health checks return meaningful status
- [ ] Health status includes latency information
- [ ] Failed health checks properly reported

---

## Manager-Pool Integration

### P1-T5: Implement proper type mapping
**File**: `src/manager/mod.rs`
**Dependencies**: None
**Complexity**: hard
**Estimated Hours**: 8

**Current Issue**: String-based TypeId matching is a hack

**Tasks**:
1. Create proper TypeId → ResourceId registry
2. Use `TypeId::of::<T>()` for type-safe lookups
3. Replace string matching in `get_resource<T>()`
4. Add type safety tests

**Acceptance Criteria**:
- [ ] No string-based type matching
- [ ] Type-safe resource lookup works
- [ ] Compile-time type checking enforced
- [ ] Tests verify type safety

**Implementation**:
```rust
struct ResourceRegistry {
    // TypeId to ResourceId mapping
    type_map: DashMap<TypeId, ResourceId>,
    // ResourceId to factory mapping
    factories: DashMap<ResourceId, Arc<dyn ResourceFactory>>,
}

pub async fn get_resource<T>(&self, id: &ResourceId) -> ResourceResult<Arc<T>>
where
    T: 'static + Send + Sync,
{
    // Use TypeId for type-safe lookup
    let type_id = TypeId::of::<T>();

    // Verify type matches registered type
    if let Some(registered_id) = self.registry.read().await.type_map.get(&type_id) {
        if registered_id.value() != id {
            return Err(ResourceError::type_mismatch(...));
        }
    }

    // ... rest of implementation
}
```

---

### P1-T6: Wire ResourceManager to PoolManager
**File**: `src/manager/mod.rs`
**Dependencies**: P1-T5
**Complexity**: hard
**Estimated Hours**: 10

**Current State**: Manager creates new instances instead of using pools

**Tasks**:
1. Add PoolManager as field in ResourceManager
2. Modify `get_resource<T>()` to acquire from pool
3. Modify `create_instance()` to create pool entries
4. Add pool configuration per resource
5. Handle pool exhaustion errors
6. Add pool statistics to manager

**Acceptance Criteria**:
- [ ] Resources acquired from pools, not created each time
- [ ] Pool exhaustion handled gracefully
- [ ] Pool statistics accessible
- [ ] Tests verify pooling behavior

**Implementation outline**:
```rust
impl ResourceManager {
    pub async fn get_resource<T>(&self, id: &ResourceId) -> ResourceResult<PooledResource<T>>
    where
        T: Resource + 'static,
    {
        // 1. Check if pool exists for this resource
        let pool = self.get_or_create_pool::<T>(id).await?;

        // 2. Acquire from pool
        let instance = pool.acquire().await?;

        // 3. Wrap in guard that returns to pool on drop
        Ok(PooledResource::new(instance, pool))
    }

    async fn get_or_create_pool<T>(&self, id: &ResourceId) -> ResourceResult<Arc<Pool<T>>>
    where
        T: Resource + 'static,
    {
        // Create pool if doesn't exist
        // Use resource metadata to determine pool config
    }
}
```

---

### P1-T7: Implement dependency resolution
**File**: `src/manager/mod.rs`, `src/core/dependency.rs` (new)
**Dependencies**: P1-T6
**Complexity**: expert
**Estimated Hours**: 16

**Tasks**:
1. Create dependency graph data structure
2. Implement topological sort for initialization order
3. Detect circular dependencies at registration time
4. Cascade lifecycle events (if A depends on B, initialize B first)
5. Add dependency metadata to ResourceMetadata
6. Update ResourceManager to use dependency graph

**Acceptance Criteria**:
- [ ] Circular dependencies detected and rejected
- [ ] Dependencies initialized in correct order
- [ ] Lifecycle events cascade to dependents
- [ ] Tests for complex dependency scenarios

**Data Structure**:
```rust
// src/core/dependency.rs
pub struct DependencyGraph {
    // ResourceId -> list of dependencies
    dependencies: HashMap<ResourceId, Vec<ResourceId>>,
    // ResourceId -> list of dependents
    dependents: HashMap<ResourceId, Vec<ResourceId>>,
}

impl DependencyGraph {
    pub fn add_dependency(&mut self, resource: ResourceId, depends_on: ResourceId) -> Result<()> {
        // Add edge
        // Check for cycles using DFS
    }

    pub fn topological_sort(&self) -> Result<Vec<ResourceId>> {
        // Kahn's algorithm or DFS-based
    }

    pub fn detect_cycles(&self) -> Option<Vec<ResourceId>> {
        // DFS with cycle detection
    }
}
```

---

### P1-T8: Fix async Drop in PooledResource
**File**: `src/pool/mod.rs`
**Dependencies**: None
**Complexity**: hard
**Estimated Hours**: 8

**Current Issue**: Can't await in Drop, TODO comment exists

**Tasks**:
1. Research async Drop patterns in Rust
2. Implement proper async release (options: detached task, channel, manual release)
3. Update PooledResource to use chosen pattern
4. Add tests for edge cases (drop during shutdown, etc.)

**Acceptance Criteria**:
- [ ] No async operations in Drop
- [ ] Resources properly returned to pool on drop
- [ ] No resource leaks in tests
- [ ] Shutdown handled gracefully

**Recommended Solution**: Use a channel-based approach
```rust
impl<T> Drop for PooledResource<T> {
    fn drop(&mut self) {
        if let Some(resource) = self.resource.take() {
            // Send to background task via channel
            let pool = self.pool.clone();
            let _ = self.release_tx.try_send(ReleaseCommand {
                resource,
                pool,
            });
        }
    }
}

// Background task processes releases
async fn release_processor(mut rx: mpsc::Receiver<ReleaseCommand>) {
    while let Some(cmd) = rx.recv().await {
        cmd.pool.release_async(cmd.resource).await;
    }
}
```

---

## Complete PostgreSQL Resource

### P1-T9: Implement PostgresResource with sqlx
**File**: `src/resources/database.rs`
**Dependencies**: P1-T6 (manager-pool integration)
**Complexity**: hard
**Estimated Hours**: 12

**Current State**: Stub implementation with mock query execution

**Tasks**:
1. Add sqlx dependency to Cargo.toml
2. Implement PostgresResource using sqlx::PgPool
3. Add connection configuration (URL, pool size, timeouts)
4. Implement create() to establish connection pool
5. Implement cleanup() to close connections
6. Implement health_check() with `SELECT 1` query
7. Add query execution methods
8. Add transaction support
9. Handle connection errors properly

**Acceptance Criteria**:
- [ ] Can connect to real PostgreSQL database
- [ ] Execute queries and get results
- [ ] Connection pooling works
- [ ] Health checks detect connection issues
- [ ] Proper error handling and retries
- [ ] Integration tests with testcontainers

**Implementation outline**:
```rust
use sqlx::{PgPool, postgres::PgPoolOptions};

pub struct PostgresResource {
    metadata: ResourceMetadata,
    config: PostgresConfig,
}

pub struct PostgresInstance {
    instance_id: Uuid,
    resource_id: ResourceId,
    pool: PgPool,
    created_at: DateTime<Utc>,
    last_accessed: Mutex<Option<DateTime<Utc>>>,
    state: RwLock<LifecycleState>,
}

#[async_trait]
impl Resource for PostgresResource {
    type Config = PostgresConfig;
    type Instance = PostgresInstance;

    async fn create(&self, config: &Self::Config, ctx: &ResourceContext) -> ResourceResult<Self::Instance> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .connect_timeout(Duration::from_secs(config.timeout_seconds))
            .connect(&config.url)
            .await
            .map_err(|e| ResourceError::initialization(...))?;

        Ok(PostgresInstance {
            instance_id: Uuid::new_v4(),
            resource_id: self.metadata.id.clone(),
            pool,
            created_at: Utc::now(),
            last_accessed: Mutex::new(None),
            state: RwLock::new(LifecycleState::Ready),
        })
    }

    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        instance.pool.close().await;
        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        // Try simple query
        sqlx::query("SELECT 1").execute(&instance.pool).await.is_ok()
    }
}

#[async_trait]
impl HealthCheckable for PostgresInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        let start = Instant::now();
        match sqlx::query("SELECT 1").execute(&self.pool).await {
            Ok(_) => Ok(HealthStatus::Healthy {
                message: Some("Connection pool healthy".into()),
                latency: start.elapsed(),
            }),
            Err(e) => Ok(HealthStatus::Unhealthy {
                error: e.to_string(),
                since: Instant::now(),
            }),
        }
    }
}

impl PostgresInstance {
    pub async fn execute(&self, query: &str) -> ResourceResult<u64> {
        self.touch();
        sqlx::query(query)
            .execute(&self.pool)
            .await
            .map(|r| r.rows_affected())
            .map_err(|e| ResourceError::internal(...))
    }

    pub async fn transaction<F, R>(&self, f: F) -> ResourceResult<R>
    where
        F: FnOnce(&mut sqlx::Transaction<sqlx::Postgres>) -> BoxFuture<'_, Result<R>>,
    {
        let mut tx = self.pool.begin().await?;
        let result = f(&mut tx).await?;
        tx.commit().await?;
        Ok(result)
    }
}
```

---

### P1-T10: Add testcontainers integration tests
**File**: `tests/postgres_integration.rs` (new)
**Dependencies**: P1-T9
**Complexity**: medium
**Estimated Hours**: 6

**Tasks**:
1. Add testcontainers dependency
2. Create integration test that:
   - Starts PostgreSQL container
   - Creates PostgresResource
   - Executes queries
   - Verifies results
   - Tests connection pooling
   - Tests health checks
   - Cleans up

**Acceptance Criteria**:
- [ ] Integration tests run with `cargo test --test postgres_integration`
- [ ] Tests start/stop containers automatically
- [ ] Tests cover all major PostgreSQL operations
- [ ] Tests verify pooling behavior

**Example test**:
```rust
use testcontainers::*;

#[tokio::test]
async fn test_postgres_resource_lifecycle() {
    let docker = clients::Cli::default();
    let postgres = docker.run(images::postgres::Postgres::default());

    let config = PostgresConfig {
        url: format!("postgres://postgres:postgres@localhost:{}/postgres",
                     postgres.get_host_port_ipv4(5432)),
        max_connections: 10,
        // ...
    };

    let resource = PostgresResource::new(metadata);
    let ctx = ResourceContext::new(...);
    let instance = resource.create(&config, &ctx).await.unwrap();

    // Test query execution
    let rows = instance.execute("CREATE TABLE test (id INT)").await.unwrap();
    assert_eq!(rows, 0);

    // Test health check
    let health = instance.health_check().await.unwrap();
    assert!(matches!(health, HealthStatus::Healthy { .. }));

    // Cleanup
    resource.cleanup(instance).await.unwrap();
}
```

---

## Health Check System

### P1-T11: Implement background health checker
**File**: `src/observability/health.rs` (new)
**Dependencies**: P1-T9
**Complexity**: hard
**Estimated Hours**: 10

**Tasks**:
1. Create HealthCheckScheduler struct
2. Implement periodic health check runner
3. Add configurable intervals per resource
4. Implement health status aggregation
5. Add unhealthy resource quarantine logic
6. Implement recovery workflow
7. Integrate with ResourceManager

**Acceptance Criteria**:
- [ ] Health checks run automatically in background
- [ ] Configurable intervals per resource type
- [ ] Unhealthy resources detected within configured interval
- [ ] Recovery workflow triggers automatically
- [ ] Health history tracked

**Implementation**:
```rust
pub struct HealthCheckScheduler {
    manager: Weak<ResourceManager>,
    config: HealthCheckConfig,
    tasks: DashMap<ResourceId, JoinHandle<()>>,
}

impl HealthCheckScheduler {
    pub fn start(&self) {
        // For each resource, spawn a health check task
    }

    async fn check_resource_health(&self, resource_id: ResourceId) {
        let mut interval = tokio::time::interval(self.config.interval);

        loop {
            interval.tick().await;

            let manager = self.manager.upgrade()?;
            let resource = manager.get_by_id(&resource_id).await?;

            match resource.health_check().await {
                Ok(HealthStatus::Healthy { .. }) => {
                    // Mark as healthy
                }
                Ok(HealthStatus::Unhealthy { .. }) => {
                    // Quarantine and attempt recovery
                    self.handle_unhealthy_resource(&resource_id).await;
                }
                Err(e) => {
                    // Log error
                }
            }
        }
    }

    async fn handle_unhealthy_resource(&self, resource_id: &ResourceId) {
        // 1. Mark as quarantined
        // 2. Emit event
        // 3. Schedule recovery attempt
        // 4. If recovery succeeds, mark as healthy
        // 5. If recovery fails after N attempts, mark as failed
    }
}
```

---

### P1-T12: Add health status aggregation
**File**: `src/observability/health.rs`
**Dependencies**: P1-T11
**Complexity**: medium
**Estimated Hours**: 4

**Tasks**:
1. Create HealthAggregator struct
2. Implement overall system health calculation
3. Add per-resource-type health rollup
4. Implement health history tracking
5. Add health metrics

**Acceptance Criteria**:
- [ ] Overall system health status available
- [ ] Per-resource-type health aggregation
- [ ] Health trends tracked over time
- [ ] Health metrics exported

---

# Phase 2: Core Resources (Weeks 7-12)

## Database Resources

### P2-T1: Implement MySQL Resource
**File**: `src/resources/mysql.rs` (new)
**Dependencies**: P1-T9 (PostgreSQL as template)
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Create mysql.rs based on database.rs pattern
2. Use sqlx with MySQL driver
3. Implement MySQL-specific configuration
4. Add health checks
5. Add integration tests with testcontainers

**Acceptance Criteria**:
- [ ] Can connect to MySQL
- [ ] Execute queries
- [ ] Health checks work
- [ ] Integration tests pass

---

### P2-T2: Implement MongoDB Resource
**File**: `src/resources/mongodb.rs` (new)
**Dependencies**: P1-T9
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Add mongodb dependency
2. Implement MongoResource with mongodb driver
3. Add connection string configuration
4. Implement CRUD operations
5. Add health checks (ping command)
6. Add integration tests

**Acceptance Criteria**:
- [ ] Can connect to MongoDB
- [ ] CRUD operations work
- [ ] Health checks work
- [ ] Integration tests pass

---

### P2-T3: Create database abstraction helpers
**File**: `src/resources/database/common.rs` (new)
**Dependencies**: P2-T1, P2-T2
**Complexity**: medium
**Estimated Hours**: 6

**Tasks**:
1. Extract common configuration patterns
2. Create shared error types
3. Add query tracing helpers
4. Create connection pool helpers

**Acceptance Criteria**:
- [ ] Code shared between database resources
- [ ] Reduced duplication
- [ ] Consistent error handling

---

## Cache Resources

### P2-T4: Implement Redis Resource
**File**: `src/resources/cache/redis.rs` (new)
**Dependencies**: P1-T9
**Complexity**: hard
**Estimated Hours**: 10

**Tasks**:
1. Add redis dependency
2. Implement RedisResource with redis-rs
3. Add cluster mode support
4. Implement get/set/delete operations
5. Add pub/sub support
6. Add health checks (PING)
7. Add integration tests

**Acceptance Criteria**:
- [ ] Can connect to Redis
- [ ] Get/set/delete operations work
- [ ] Cluster mode supported
- [ ] Pub/sub works
- [ ] Health checks work
- [ ] Integration tests pass

---

### P2-T5: Implement in-memory cache Resource
**File**: `src/resources/cache/memory.rs` (new)
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 6

**Tasks**:
1. Choose cache library (moka, mini-moka, or lru)
2. Implement MemoryCacheResource
3. Add TTL support
4. Implement eviction strategies
5. Add size limits
6. Add unit tests

**Acceptance Criteria**:
- [ ] In-memory cache works
- [ ] TTL evicts entries
- [ ] Size limits enforced
- [ ] Thread-safe
- [ ] Tests pass

---

## HTTP Client Resource

### P2-T6: Implement HTTP Client Resource
**File**: `src/resources/http_client.rs` (new)
**Dependencies**: P1-T9
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Use reqwest for HTTP client
2. Implement HttpClientResource
3. Add connection pooling (built-in to reqwest)
4. Add retry logic with exponential backoff
5. Add timeout handling
6. Add request/response logging
7. Add integration tests

**Acceptance Criteria**:
- [ ] Can make HTTP requests
- [ ] Connection pooling works
- [ ] Retry on transient errors
- [ ] Timeouts enforced
- [ ] Integration tests pass

**Implementation**:
```rust
use reqwest::Client;

pub struct HttpClientResource {
    metadata: ResourceMetadata,
}

pub struct HttpClientInstance {
    client: Client,
    config: HttpClientConfig,
    // ... standard instance fields
}

impl HttpClientInstance {
    pub async fn get(&self, url: &str) -> ResourceResult<reqwest::Response> {
        self.touch();
        self.client
            .get(url)
            .timeout(self.config.timeout)
            .send()
            .await
            .map_err(|e| ResourceError::internal(...))
    }

    pub async fn post(&self, url: &str, body: impl Into<reqwest::Body>) -> ResourceResult<reqwest::Response> {
        // Similar implementation
    }
}
```

---

## Message Queue Resources

### P2-T7: Implement Kafka Producer Resource
**File**: `src/resources/message_queue/kafka_producer.rs` (new)
**Dependencies**: P1-T9
**Complexity**: hard
**Estimated Hours**: 12

**Tasks**:
1. Add rdkafka dependency
2. Implement KafkaProducerResource
3. Add producer configuration
4. Implement send message operation
5. Add partition strategies
6. Add delivery guarantees (at-least-once, etc.)
7. Add health checks
8. Add integration tests with testcontainers

**Acceptance Criteria**:
- [ ] Can publish messages to Kafka
- [ ] Partition strategies work
- [ ] Delivery guarantees enforced
- [ ] Health checks work
- [ ] Integration tests pass

---

### P2-T8: Implement Kafka Consumer Resource
**File**: `src/resources/message_queue/kafka_consumer.rs` (new)
**Dependencies**: P2-T7
**Complexity**: hard
**Estimated Hours**: 12

**Tasks**:
1. Implement KafkaConsumerResource
2. Add consumer group support
3. Implement offset management
4. Add auto-commit strategies
5. Add message polling
6. Add integration tests

**Acceptance Criteria**:
- [ ] Can consume messages from Kafka
- [ ] Consumer groups work
- [ ] Offset management correct
- [ ] Integration tests pass

---

# Phase 3: Advanced Features (Weeks 13-16)

## Resilience Patterns

### P3-T1: Implement Circuit Breaker
**File**: `src/core/resilience/circuit_breaker.rs` (new)
**Dependencies**: None
**Complexity**: hard
**Estimated Hours**: 10

**Tasks**:
1. Implement circuit breaker state machine (Closed, Open, Half-Open)
2. Add configurable failure thresholds
3. Add configurable timeout and recovery
4. Integrate with ResourceError
5. Add per-resource circuit breaker instances
6. Add metrics for circuit breaker state
7. Add tests for all states and transitions

**Acceptance Criteria**:
- [ ] Circuit opens after threshold failures
- [ ] Circuit half-opens after timeout
- [ ] Circuit closes after successful operations in half-open
- [ ] Metrics track circuit breaker states
- [ ] Tests cover all transitions

**Implementation**:
```rust
pub struct CircuitBreaker {
    state: Arc<RwLock<CircuitBreakerState>>,
    config: CircuitBreakerConfig,
    failure_count: AtomicUsize,
    last_failure: AtomicU64, // timestamp
}

#[derive(Debug, Clone)]
enum CircuitBreakerState {
    Closed,
    Open { opened_at: Instant },
    HalfOpen { test_count: usize },
}

impl CircuitBreaker {
    pub async fn call<F, T>(&self, f: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        // Check state
        match &*self.state.read().await {
            CircuitBreakerState::Open { opened_at } => {
                if opened_at.elapsed() > self.config.timeout {
                    // Try half-open
                    self.transition_to_half_open().await;
                } else {
                    return Err(Error::CircuitBreakerOpen);
                }
            }
            CircuitBreakerState::Closed | CircuitBreakerState::HalfOpen { .. } => {}
        }

        // Execute operation
        match f.await {
            Ok(result) => {
                self.on_success().await;
                Ok(result)
            }
            Err(e) => {
                self.on_failure().await;
                Err(e)
            }
        }
    }

    async fn on_failure(&self) {
        // Increment failure count
        let failures = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;

        // Check if should open
        if failures >= self.config.failure_threshold {
            self.transition_to_open().await;
        }
    }

    async fn on_success(&self) {
        // Reset failure count
        self.failure_count.store(0, Ordering::SeqCst);

        // If half-open, transition to closed
        if matches!(*self.state.read().await, CircuitBreakerState::HalfOpen { .. }) {
            self.transition_to_closed().await;
        }
    }
}
```

---

### P3-T2: Implement Retry Logic
**File**: `src/core/resilience/retry.rs` (new)
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 6

**Tasks**:
1. Implement exponential backoff strategy
2. Add jitter to prevent thundering herd
3. Add deadline propagation
4. Add configurable max attempts
5. Integrate with ResourceError to determine retryability
6. Add tests

**Acceptance Criteria**:
- [ ] Exponential backoff works correctly
- [ ] Jitter prevents synchronized retries
- [ ] Respects deadlines
- [ ] Only retries retryable errors
- [ ] Tests verify retry behavior

---

### P3-T3: Implement Timeout Management
**File**: `src/core/resilience/timeout.rs` (new)
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 4

**Tasks**:
1. Create timeout wrapper utilities
2. Add deadline context propagation
3. Add timeout cancellation
4. Integrate with tokio::time::timeout
5. Add tests

**Acceptance Criteria**:
- [ ] Operations timeout correctly
- [ ] Deadlines propagate through call chain
- [ ] Cancellation works properly
- [ ] Tests verify timeout behavior

---

## Metrics & Observability

### P3-T4: Implement Prometheus Exporter
**File**: `src/observability/prometheus.rs` (new)
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Add metrics-exporter-prometheus dependency
2. Implement PrometheusExporter
3. Register all resource metrics
4. Add histogram, counter, gauge support
5. Add label management (resource_id, resource_type, tenant, etc.)
6. Create HTTP endpoint for /metrics
7. Add tests

**Acceptance Criteria**:
- [ ] Metrics exported in Prometheus format
- [ ] HTTP /metrics endpoint works
- [ ] All resource metrics visible
- [ ] Labels correctly applied
- [ ] Integration test with actual Prometheus

**Example metrics**:
```
# HELP resource_acquisitions_total Total number of resource acquisitions
# TYPE resource_acquisitions_total counter
resource_acquisitions_total{resource="postgres",tenant="acme"} 1234

# HELP resource_active_count Number of active resources
# TYPE resource_active_count gauge
resource_active_count{resource="postgres"} 42

# HELP resource_acquisition_duration_seconds Resource acquisition duration
# TYPE resource_acquisition_duration_seconds histogram
resource_acquisition_duration_seconds_bucket{resource="postgres",le="0.001"} 100
resource_acquisition_duration_seconds_bucket{resource="postgres",le="0.01"} 450
```

---

### P3-T5: Implement Tracing Integration
**File**: `src/observability/tracing.rs` (new)
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 6

**Tasks**:
1. Add tracing, tracing-opentelemetry dependencies
2. Create span wrappers for resource operations
3. Implement context propagation
4. Add trace IDs to all operations
5. Configure OpenTelemetry export
6. Add tests

**Acceptance Criteria**:
- [ ] All resource operations create spans
- [ ] Context propagates across async boundaries
- [ ] Trace IDs present in logs
- [ ] Can export to Jaeger/Zipkin
- [ ] Integration test verifies traces

---

### P3-T6: Enhance Structured Logging
**File**: `src/observability/logging.rs` (new)
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 4

**Tasks**:
1. Integrate with nebula-log
2. Add contextual logging helpers
3. Add log correlation with traces
4. Add structured fields (resource_id, workflow_id, tenant_id, etc.)
5. Add tests

**Acceptance Criteria**:
- [ ] All operations logged with context
- [ ] Logs correlated with traces
- [ ] Structured fields present
- [ ] Log levels configurable

---

## Advanced Pooling

### P3-T7: Implement Weighted Round Robin Strategy
**File**: `src/pool/strategies/weighted.rs` (new)
**Dependencies**: P1-T11 (health checker)
**Complexity**: medium
**Estimated Hours**: 6

**Tasks**:
1. Create WeightedRoundRobinStrategy
2. Calculate weights based on health scores
3. Implement weighted selection algorithm
4. Add tests comparing to basic round robin

**Acceptance Criteria**:
- [ ] Healthier resources receive more traffic
- [ ] Weights update based on health changes
- [ ] Load distributed according to weights
- [ ] Tests verify weighting

---

### P3-T8: Implement Adaptive Strategy
**File**: `src/pool/strategies/adaptive.rs` (new)
**Dependencies**: P3-T7
**Complexity**: hard
**Estimated Hours**: 12

**Tasks**:
1. Create AdaptiveStrategy
2. Collect performance statistics per resource
3. Implement heuristic-based selection
4. Add performance optimization
5. Add benchmarks comparing to other strategies

**Acceptance Criteria**:
- [ ] Adapts based on performance metrics
- [ ] Better performance than static strategies
- [ ] Benchmarks show improvements
- [ ] Tests verify adaptive behavior

---

### P3-T9: Implement Pool Monitoring
**File**: `src/pool/monitoring.rs` (new)
**Dependencies**: P3-T4 (Prometheus)
**Complexity**: easy
**Estimated Hours**: 4

**Tasks**:
1. Add pool utilization metrics
2. Add wait time tracking
3. Add pool size recommendations
4. Export to Prometheus

**Acceptance Criteria**:
- [ ] Pool metrics visible in Prometheus
- [ ] Utilization tracked
- [ ] Wait times recorded
- [ ] Alerts possible based on metrics

---

## Credential Integration

### P3-T10: Implement nebula-credential integration
**File**: `src/credentials/mod.rs` (new)
**Dependencies**: None (nebula-credential assumed complete)
**Complexity**: hard
**Estimated Hours**: 10

**Tasks**:
1. Enable credentials feature
2. Integrate with nebula-credential crate
3. Add automatic credential retrieval in resource creation
4. Add credential caching
5. Add credential attributes in ResourceConfig
6. Add tests with mock credential provider

**Acceptance Criteria**:
- [ ] Resources can declare credential dependencies
- [ ] Credentials automatically retrieved
- [ ] Credentials cached appropriately
- [ ] Tests verify credential integration

**Example usage**:
```rust
#[derive(ResourceConfig)]
pub struct S3Config {
    pub bucket: String,

    #[credential(id = "aws_access_key")]
    pub access_key: SecretString,

    #[credential(id = "aws_secret_key")]
    pub secret_key: SecretString,
}
```

---

### P3-T11: Implement Credential Rotation
**File**: `src/credentials/rotation.rs` (new)
**Dependencies**: P3-T10
**Complexity**: hard
**Estimated Hours**: 10

**Tasks**:
1. Add credential expiry tracking
2. Implement automatic rotation on expiry
3. Add graceful connection refresh
4. Ensure zero-downtime rotation
5. Add rotation metrics
6. Add tests

**Acceptance Criteria**:
- [ ] Credentials rotate automatically on expiry
- [ ] No connection errors during rotation
- [ ] Rotation events logged and traced
- [ ] Tests verify rotation behavior

---

# Phase 4: Storage & Specialized Resources (Weeks 17-19)

## Storage Resources

### P4-T1: Implement S3 Resource
**File**: `src/resources/storage/s3.rs` (new)
**Dependencies**: P3-T10 (credentials)
**Complexity**: hard
**Estimated Hours**: 10

**Tasks**:
1. Add aws-sdk-s3 dependency
2. Implement S3Resource
3. Add multi-part upload support
4. Add presigned URL generation
5. Add health checks
6. Add integration tests with LocalStack

**Acceptance Criteria**:
- [ ] Can upload/download objects
- [ ] Multi-part upload works
- [ ] Presigned URLs generated
- [ ] Health checks work
- [ ] Integration tests pass

---

### P4-T2: Implement GCS Resource
**File**: `src/resources/storage/gcs.rs` (new)
**Dependencies**: P4-T1 (similar pattern)
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Add google-cloud-storage dependency
2. Implement GcsResource similar to S3
3. Add integration tests

**Acceptance Criteria**:
- [ ] Can upload/download objects
- [ ] Integration tests pass

---

### P4-T3: Implement Azure Blob Resource
**File**: `src/resources/storage/azure.rs` (new)
**Dependencies**: P4-T1
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Add azure-storage-blobs dependency
2. Implement AzureBlobResource similar to S3
3. Add integration tests

**Acceptance Criteria**:
- [ ] Can upload/download objects
- [ ] Integration tests pass

---

### P4-T4: Implement Local Storage Resource
**File**: `src/resources/storage/local.rs` (new)
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 4

**Tasks**:
1. Implement LocalStorageResource using std::fs
2. Add async file operations (tokio::fs)
3. Add directory management
4. Add tests

**Acceptance Criteria**:
- [ ] File operations work
- [ ] Async operations don't block
- [ ] Tests pass

---

## Specialized Resources

### P4-T5: Implement gRPC Client Resource
**File**: `src/resources/grpc.rs` (new)
**Dependencies**: P1-T9
**Complexity**: hard
**Estimated Hours**: 10

**Tasks**:
1. Add tonic dependency
2. Implement GrpcClientResource
3. Add connection pooling
4. Add streaming support
5. Add health checks (gRPC health protocol)
6. Add integration tests

**Acceptance Criteria**:
- [ ] gRPC calls work
- [ ] Streaming supported
- [ ] Health checks work
- [ ] Integration tests pass

---

### P4-T6: Implement WebSocket Resource
**File**: `src/resources/websocket.rs` (new)
**Dependencies**: P1-T9
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Add tokio-tungstenite dependency
2. Implement WebSocketResource
3. Add connection management
4. Add message handling
5. Add reconnection logic
6. Add tests

**Acceptance Criteria**:
- [ ] WebSocket connections maintained
- [ ] Messages sent/received
- [ ] Reconnection works
- [ ] Tests pass

---

### P4-T7: Implement GraphQL Client Resource
**File**: `src/resources/graphql.rs` (new)
**Dependencies**: P2-T6 (HTTP client)
**Complexity**: medium
**Estimated Hours**: 6

**Tasks**:
1. Add graphql-client dependency
2. Implement GraphQLClientResource
3. Add query/mutation support
4. Add subscription support
5. Add tests

**Acceptance Criteria**:
- [ ] GraphQL queries work
- [ ] Mutations work
- [ ] Subscriptions work
- [ ] Tests pass

---

## Resource Framework

### P4-T8: Create Resource Template Generator
**File**: `tools/resource-gen/` (new CLI tool)
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Create CLI tool with clap
2. Add templates for each resource type
3. Add code generation
4. Add best practice checks
5. Add documentation generation

**Acceptance Criteria**:
- [ ] Can generate new resource from template
- [ ] Generated code compiles
- [ ] Generated code follows best practices
- [ ] Documentation generated

**Example usage**:
```bash
cargo run --bin resource-gen -- new \
    --name my_resource \
    --type database \
    --poolable \
    --health-checkable
```

---

### P4-T9: Create Resource Testing Framework
**File**: `src/testing/framework.rs` (new)
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Create integration test helpers
2. Add mock resource utilities
3. Add property-based test generators
4. Add common test scenarios
5. Add documentation

**Acceptance Criteria**:
- [ ] Easy to write resource tests
- [ ] Common scenarios covered by helpers
- [ ] Property tests catch edge cases
- [ ] Documentation with examples

---

# Phase 5: Polish & Production Readiness (Weeks 20-24)

## Performance Optimization

### P5-T1: Profile and optimize hot paths
**File**: Various
**Dependencies**: All previous phases
**Complexity**: expert
**Estimated Hours**: 20

**Tasks**:
1. Profile with cargo flamegraph
2. Identify hot paths
3. Optimize lock contention
4. Reduce allocations
5. Add caching where appropriate
6. Re-benchmark and compare

**Acceptance Criteria**:
- [ ] 20% performance improvement on benchmarks
- [ ] No performance regressions
- [ ] Profile shows improved hot paths

---

### P5-T2: Memory optimization
**File**: Various
**Dependencies**: P5-T1
**Complexity**: hard
**Estimated Hours**: 12

**Tasks**:
1. Profile memory usage
2. Reduce per-resource overhead
3. Optimize pool memory usage
4. Fix memory leaks (if any)
5. Add memory benchmarks

**Acceptance Criteria**:
- [ ] <1MB overhead per 1000 resources
- [ ] No memory leaks detected
- [ ] Memory usage documented

---

### P5-T3: Latency optimization
**File**: Various
**Dependencies**: P5-T1
**Complexity**: hard
**Estimated Hours**: 12

**Tasks**:
1. Reduce acquisition latency
2. Minimize context switches
3. Optimize critical paths
4. Add latency benchmarks

**Acceptance Criteria**:
- [ ] p99 latency <10ms for pool hits
- [ ] p99 latency <100ms for pool misses
- [ ] Latency documented

---

## Security Hardening

### P5-T4: Conduct security audit
**File**: Various
**Dependencies**: All previous phases
**Complexity**: expert
**Estimated Hours**: 16

**Tasks**:
1. Code review for vulnerabilities
2. Dependency audit (cargo audit)
3. SAST tool integration (cargo-deny, etc.)
4. Fix all high-severity issues
5. Document security measures

**Acceptance Criteria**:
- [ ] Zero high-severity issues
- [ ] All dependencies up to date
- [ ] SAST tools pass
- [ ] Security documentation complete

---

### P5-T5: Harden credential security
**File**: `src/credentials/`
**Dependencies**: P3-T10, P3-T11
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Add encryption at rest
2. Add encryption in transit
3. Add secure credential rotation
4. Add audit logging
5. Add tests

**Acceptance Criteria**:
- [ ] Credentials never leaked
- [ ] Encryption verified
- [ ] Audit logs complete
- [ ] Security tests pass

---

### P5-T6: Enforce resource isolation
**File**: `src/manager/`, `src/pool/`
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 8

**Tasks**:
1. Add tenant boundary checks
2. Add quota enforcement
3. Add resource exhaustion protection
4. Add tests

**Acceptance Criteria**:
- [ ] Tenants cannot access each other's resources
- [ ] Quotas enforced
- [ ] Resource exhaustion prevented
- [ ] Tests verify isolation

---

## Documentation

### P5-T7: Complete API documentation
**File**: All source files
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 12

**Tasks**:
1. Add rustdoc for all public items
2. Add examples in doc comments
3. Document usage patterns
4. Document error conditions
5. Generate docs and review

**Acceptance Criteria**:
- [ ] 100% public API documented
- [ ] Examples in all doc comments
- [ ] `cargo doc` generates complete docs
- [ ] No broken links

---

### P5-T8: Write comprehensive guides
**File**: `docs/guides/` (new)
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 16

**Tasks**:
1. Getting Started guide
2. Resource implementation guide
3. Best practices guide
4. Troubleshooting guide
5. Performance tuning guide

**Acceptance Criteria**:
- [ ] 5+ guides complete
- [ ] New users can get started in <30 minutes
- [ ] Guides reviewed by users
- [ ] Guides kept up to date

---

### P5-T9: Create runnable examples
**File**: `examples/` (new)
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 12

**Tasks**:
1. Basic resource usage
2. Custom resource implementation
3. Multi-tenant setup
4. Distributed tracing setup
5. Production deployment
6. All resource types
7. Pool configuration
8. Resilience patterns
9. Monitoring setup
10. Credential management

**Acceptance Criteria**:
- [ ] 10+ runnable examples
- [ ] All examples compile and run
- [ ] Examples documented
- [ ] Examples cover common use cases

---

## Ecosystem Building

### P5-T10: Write migration guide
**File**: `docs/MIGRATION.md` (new)
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 4

**Tasks**:
1. From manual resource management
2. From v0.1 to v0.2
3. Step-by-step instructions
4. Code examples
5. Troubleshooting

**Acceptance Criteria**:
- [ ] Migration guide complete
- [ ] Validated with actual migration
- [ ] Common issues documented

---

### P5-T11: Create CLI tools
**File**: `tools/resource-cli/` (new)
**Dependencies**: None
**Complexity**: medium
**Estimated Hours**: 12

**Tasks**:
1. Resource inspector (list, inspect)
2. Health check runner
3. Metric exporter
4. Configuration validator

**Acceptance Criteria**:
- [ ] CLI tools functional
- [ ] Useful for debugging
- [ ] Documentation complete

---

### P5-T12: Create integration packs
**File**: `deploy/` (new)
**Dependencies**: None
**Complexity**: easy
**Estimated Hours**: 8

**Tasks**:
1. Docker Compose examples
2. Kubernetes manifests
3. Terraform modules
4. Helm charts

**Acceptance Criteria**:
- [ ] Easy to deploy in various environments
- [ ] Documentation for each
- [ ] Examples tested

---

## Summary Statistics

### Total Tasks by Phase
- Phase 0: 8 tasks, ~26 hours
- Phase 1: 12 tasks, ~94 hours
- Phase 2: 8 tasks, ~66 hours
- Phase 3: 11 tasks, ~84 hours
- Phase 4: 9 tasks, ~70 hours
- Phase 5: 12 tasks, ~128 hours

**Total: 60 tasks, ~468 hours (12 weeks with 2 developers)**

### Total Tasks by Complexity
- Trivial: 4 tasks (~5 hours)
- Easy: 13 tasks (~50 hours)
- Medium: 22 tasks (~160 hours)
- Hard: 16 tasks (~160 hours)
- Expert: 5 tasks (~93 hours)

### Task Dependencies
Tasks are organized to minimize blocking. Many can be parallelized, especially in Phases 2-4.

---

## Next Steps

1. Review this task list with stakeholders
2. Assign tasks to team members
3. Create GitHub issues for each task
4. Set up project board
5. Begin Phase 0 immediately
