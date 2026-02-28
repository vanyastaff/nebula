# Nebula Complete Roadmap

## 🎯 Master Roadmap

### Phase 1: Core Foundation (Weeks 1-3)

#### 1. nebula-core (Week 1)
- [ ] 1.1 **Project Setup**
  - [ ] 1.1.1 Initialize crate structure
  - [ ] 1.1.2 Setup CI/CD pipeline
  - [ ] 1.1.3 Configure linting and formatting
  - [ ] 1.1.4 Add base dependencies

- [ ] 1.2 **Identifier Types**
  - [ ] 1.2.1 Implement WorkflowId with validation
  - [ ] 1.2.2 Implement NodeId with string normalization
  - [ ] 1.2.3 Implement ExecutionId with UUID
  - [ ] 1.2.4 Implement TriggerId
  - [ ] 1.2.5 Add Display and Debug traits
  - [ ] 1.2.6 Add serialization support
  - [ ] 1.2.7 Write property-based tests

- [ ] 1.3 **Error Handling**
  - [ ] 1.3.1 Design Error enum hierarchy
  - [ ] 1.3.2 Implement error contexts
  - [ ] 1.3.3 Add error conversion traits
  - [ ] 1.3.4 Create Result type alias
  - [ ] 1.3.5 Add error chaining support
  - [ ] 1.3.6 Write error documentation

- [ ] 1.4 **Core Traits**
  - [ ] 1.4.1 Define Action trait
  - [ ] 1.4.2 Define TriggerAction trait
  - [ ] 1.4.3 Define PollingAction trait
  - [ ] 1.4.4 Define SupplyAction trait
  - [ ] 1.4.5 Define ProcessAction trait
  - [ ] 1.4.6 Add async trait support
  - [ ] 1.4.7 Write trait documentation

- [ ] 1.5 **Metadata Types**
  - [ ] 1.5.1 Implement ActionMetadata
  - [ ] 1.5.2 Implement NodeMetadata
  - [ ] 1.5.3 Implement WorkflowMetadata
  - [ ] 1.5.4 Implement ParameterDescriptor
  - [ ] 1.5.5 Add builder patterns
  - [ ] 1.5.6 Add validation logic

#### 2. Value layer: serde / serde_json::Value (Week 1-2)
Отдельный crate nebula-value не используется.
- [ ] 2.1 Использовать `serde_json::Value` для данных workflow, serde для сериализации
- [ ] 2.2 Валидация поверх Value (nebula-validator / core)

#### 3. nebula-memory (Week 2)
- [ ] 3.1 **Core Structure**
  - [ ] 3.1.1 Create NebulaMemory struct
  - [ ] 3.1.2 Implement ExecutionMemory
  - [ ] 3.1.3 Implement ResourceMemory
  - [ ] 3.1.4 Implement TriggerMemory
  - [ ] 3.1.5 Add memory configuration
  - [ ] 3.1.6 Add builder pattern

- [ ] 3.2 **Caching System**
  - [ ] 3.2.1 Define Cache trait
  - [ ] 3.2.2 Implement LRU cache
  - [ ] 3.2.3 Implement TTL cache
  - [ ] 3.2.4 Add cache statistics
  - [ ] 3.2.5 Add eviction callbacks
  - [ ] 3.2.6 Add cache warming

- [ ] 3.3 **Resource Pooling**
  - [ ] 3.3.1 Create ObjectPool generic
  - [ ] 3.3.2 Add pool configuration
  - [ ] 3.3.3 Implement health checking
  - [ ] 3.3.4 Add pool metrics
  - [ ] 3.3.5 Add async acquisition
  - [ ] 3.3.6 Add timeout handling

- [ ] 3.4 **Memory Optimization**
  - [ ] 3.4.1 Implement StringInterner
  - [ ] 3.4.2 Implement CowStorage
  - [ ] 3.4.3 Add memory budgets
  - [ ] 3.4.4 Add pressure monitoring
  - [ ] 3.4.5 Implement auto-eviction
  - [ ] 3.4.6 Add memory profiling

#### 4. nebula-derive (Week 3)
- [ ] 4.1 **Macro Setup**
  - [ ] 4.1.1 Create proc-macro crate
  - [ ] 4.1.2 Setup syn and quote
  - [ ] 4.1.3 Add error handling
  - [ ] 4.1.4 Setup testing framework

- [ ] 4.2 **Parameters Derive**
  - [ ] 4.2.1 Parse struct attributes
  - [ ] 4.2.2 Generate parameter_collection()
  - [ ] 4.2.3 Generate from_values()
  - [ ] 4.2.4 Add validation attributes
  - [ ] 4.2.5 Add display attributes
  - [ ] 4.2.6 Generate documentation

- [ ] 4.3 **Action Derive**
  - [ ] 4.3.1 Parse action attributes
  - [ ] 4.3.2 Generate metadata
  - [ ] 4.3.3 Generate boilerplate
  - [ ] 4.3.4 Add node attributes
  - [ ] 4.3.5 Validate at compile time

### Phase 2: Execution Engine (Weeks 4-6)

#### 5. nebula-expression (Week 4)
- [ ] 5.1 **Parser Development**
  - [ ] 5.1.1 Define grammar specification
  - [ ] 5.1.2 Implement tokenizer
  - [ ] 5.1.3 Implement recursive descent parser
  - [ ] 5.1.4 Create AST structures
  - [ ] 5.1.5 Add error recovery
  - [ ] 5.1.6 Add position tracking
  - [ ] 5.1.7 Write parser tests

- [ ] 5.2 **Core Expressions**
  - [ ] 5.2.1 Variable access ($nodes, $vars, etc)
  - [ ] 5.2.2 Property access (dot notation)
  - [ ] 5.2.3 Array indexing
  - [ ] 5.2.4 Method calls
  - [ ] 5.2.5 Literals (string, number, bool)
  - [ ] 5.2.6 Null handling

- [ ] 5.3 **Operators**
  - [ ] 5.3.1 Arithmetic operators (+, -, *, /, %)
  - [ ] 5.3.2 Comparison operators (==, !=, <, >, <=, >=)
  - [ ] 5.3.3 Logical operators (&&, ||, !)
  - [ ] 5.3.4 Ternary operator (? :)
  - [ ] 5.3.5 Null coalescing (??)
  - [ ] 5.3.6 String concatenation

- [ ] 5.4 **Functions**
  - [ ] 5.4.1 String functions (concat, substring, etc)
  - [ ] 5.4.2 Array functions (filter, map, reduce)
  - [ ] 5.4.3 Date functions (format, parse, add)
  - [ ] 5.4.4 Math functions (round, floor, ceil)
  - [ ] 5.4.5 Type conversion functions
  - [ ] 5.4.6 Custom function registration

- [ ] 5.5 **Evaluator**
  - [ ] 5.5.1 Create evaluation context
  - [ ] 5.5.2 Implement AST walker
  - [ ] 5.5.3 Add type checking
  - [ ] 5.5.4 Add short-circuit evaluation
  - [ ] 5.5.5 Add error handling
  - [ ] 5.5.6 Add performance optimization

#### 6. nebula-engine (Week 4-5)
- [ ] 6.1 **Core Engine**
  - [ ] 6.1.1 Create WorkflowEngine struct
  - [ ] 6.1.2 Implement event loop
  - [ ] 6.1.3 Add state management
  - [ ] 6.1.4 Add execution context
  - [ ] 6.1.5 Implement scheduling logic
  - [ ] 6.1.6 Add graceful shutdown

- [ ] 6.2 **DAG Processing**
  - [ ] 6.2.1 Implement topological sort
  - [ ] 6.2.2 Add cycle detection
  - [ ] 6.2.3 Implement parallel execution
  - [ ] 6.2.4 Add conditional branching
  - [ ] 6.2.5 Add loop support
  - [ ] 6.2.6 Add subworkflow support

- [ ] 6.3 **Event System**
  - [ ] 6.3.1 Define WorkflowEvent types
  - [ ] 6.3.2 Implement event bus abstraction
  - [ ] 6.3.3 Add Kafka integration
  - [ ] 6.3.4 Add event persistence
  - [ ] 6.3.5 Add event replay
  - [ ] 6.3.6 Add dead letter queue

- [ ] 6.4 **Execution Control**
  - [ ] 6.4.1 Implement pause/resume
  - [ ] 6.4.2 Add cancellation
  - [ ] 6.4.3 Add timeout handling
  - [ ] 6.4.4 Add retry logic
  - [ ] 6.4.5 Add error propagation
  - [ ] 6.4.6 Add compensation logic

#### 7. nebula-storage (Week 5)
- [ ] 7.1 **Storage Traits**
  - [ ] 7.1.1 Define StorageBackend trait
  - [ ] 7.1.2 Define WorkflowStorage trait
  - [ ] 7.1.3 Define ExecutionStorage trait
  - [ ] 7.1.4 Define BinaryStorage trait
  - [ ] 7.1.5 Add async methods
  - [ ] 7.1.6 Add transaction support

- [ ] 7.2 **Query System**
  - [ ] 7.2.1 Create query builder
  - [ ] 7.2.2 Add filtering support
  - [ ] 7.2.3 Add pagination
  - [ ] 7.2.4 Add sorting
  - [ ] 7.2.5 Add aggregation
  - [ ] 7.2.6 Add full-text search

- [ ] 7.3 **PostgreSQL Implementation**
  - [ ] 7.3.1 Setup sqlx integration
  - [ ] 7.3.2 Create migration system
  - [ ] 7.3.3 Implement workflow storage
  - [ ] 7.3.4 Implement execution storage
  - [ ] 7.3.5 Add connection pooling
  - [ ] 7.3.6 Add query optimization

- [ ] 7.4 **Caching Layer**
  - [ ] 7.4.1 Add read-through cache
  - [ ] 7.4.2 Add write-through cache
  - [ ] 7.4.3 Add cache invalidation
  - [ ] 7.4.4 Add distributed cache support
  - [ ] 7.4.5 Add cache statistics

#### 8. nebula-binary (Week 5-6)
- [ ] 8.1 **Binary Handling**
  - [ ] 8.1.1 Define BinaryData types
  - [ ] 8.1.2 Implement BinaryStorage trait
  - [ ] 8.1.3 Add streaming support
  - [ ] 8.1.4 Add chunked uploads
  - [ ] 8.1.5 Add resumable uploads
  - [ ] 8.1.6 Add progress tracking

- [ ] 8.2 **Storage Strategies**
  - [ ] 8.2.1 Implement InMemory storage
  - [ ] 8.2.2 Implement Temp file storage
  - [ ] 8.2.3 Implement S3 storage
  - [ ] 8.2.4 Add storage migration
  - [ ] 8.2.5 Add automatic tiering
  - [ ] 8.2.6 Add garbage collection

- [ ] 8.3 **Optimization**
  - [ ] 8.3.1 Add compression support
  - [ ] 8.3.2 Add deduplication
  - [ ] 8.3.3 Add content addressing
  - [ ] 8.3.4 Add CDN integration
  - [ ] 8.3.5 Add bandwidth limiting

### Phase 3: Runtime & Workers (Weeks 7-9)

#### 9. nebula-runtime (Week 7)
- [ ] 9.1 **Runtime Core**
  - [ ] 9.1.1 Create Runtime struct
  - [ ] 9.1.2 Implement lifecycle management
  - [ ] 9.1.3 Add configuration system
  - [ ] 9.1.4 Add health monitoring
  - [ ] 9.1.5 Add metrics collection
  - [ ] 9.1.6 Add graceful shutdown

- [ ] 9.2 **Trigger Management**
  - [ ] 9.2.1 Create TriggerManager
  - [ ] 9.2.2 Implement trigger lifecycle
  - [ ] 9.2.3 Add trigger activation
  - [ ] 9.2.4 Add trigger deactivation
  - [ ] 9.2.5 Add trigger state persistence
  - [ ] 9.2.6 Add trigger health checks

- [ ] 9.3 **Event Processing**
  - [ ] 9.3.1 Implement event listener
  - [ ] 9.3.2 Add event routing
  - [ ] 9.3.3 Add event transformation
  - [ ] 9.3.4 Add event filtering
  - [ ] 9.3.5 Add backpressure handling
  - [ ] 9.3.6 Add event metrics

- [ ] 9.4 **Coordination**
  - [ ] 9.4.1 Add workflow assignment
  - [ ] 9.4.2 Implement leader election
  - [ ] 9.4.3 Add distributed locking
  - [ ] 9.4.4 Add runtime discovery
  - [ ] 9.4.5 Add load balancing
  - [ ] 9.4.6 Add failover handling

#### 10. nebula-worker (Week 7-8)
- [ ] 10.1 **Worker Core**
  - [ ] 10.1.1 Create Worker struct
  - [ ] 10.1.2 Implement work loop
  - [ ] 10.1.3 Add task acquisition
  - [ ] 10.1.4 Add resource management
  - [ ] 10.1.5 Add health reporting
  - [ ] 10.1.6 Add graceful shutdown

- [ ] 10.2 **Execution Environment**
  - [ ] 10.2.1 Create execution sandbox
  - [ ] 10.2.2 Add resource limits
  - [ ] 10.2.3 Add timeout enforcement
  - [ ] 10.2.4 Add memory isolation
  - [ ] 10.2.5 Add CPU throttling
  - [ ] 10.2.6 Add I/O limits

- [ ] 10.3 **Node Execution**
  - [ ] 10.3.1 Implement node loader
  - [ ] 10.3.2 Add input preparation
  - [ ] 10.3.3 Add output handling
  - [ ] 10.3.4 Add error handling
  - [ ] 10.3.5 Add progress reporting
  - [ ] 10.3.6 Add execution metrics

- [ ] 10.4 **Worker Pool**
  - [ ] 10.4.1 Create WorkerPool manager
  - [ ] 10.4.2 Add dynamic scaling
  - [ ] 10.4.3 Add work distribution
  - [ ] 10.4.4 Add load balancing
  - [ ] 10.4.5 Add worker health checks
  - [ ] 10.4.6 Add pool metrics

#### 11. nebula-node-registry (Week 8-9)
- [ ] 11.1 **Registry Core**
  - [ ] 11.1.1 Create NodeRegistry struct
  - [ ] 11.1.2 Implement discovery system
  - [ ] 11.1.3 Add registration API
  - [ ] 11.1.4 Add version management
  - [ ] 11.1.5 Add dependency resolution
  - [ ] 11.1.6 Add registry persistence

- [ ] 11.2 **Plugin Loading**
  - [ ] 11.2.1 Implement library loader
  - [ ] 11.2.2 Add symbol resolution
  - [ ] 11.2.3 Add ABI compatibility check
  - [ ] 11.2.4 Add hot reloading
  - [ ] 11.2.5 Add isolation
  - [ ] 11.2.6 Add unloading

- [ ] 11.3 **Git Integration**
  - [ ] 11.3.1 Add git clone support
  - [ ] 11.3.2 Add build automation
  - [ ] 11.3.3 Add version tracking
  - [ ] 11.3.4 Add update checking
  - [ ] 11.3.5 Add rollback support
  - [ ] 11.3.6 Add signature verification

- [ ] 11.4 **Cache Management**
  - [ ] 11.4.1 Implement node cache
  - [ ] 11.4.2 Add cache warming
  - [ ] 11.4.3 Add cache eviction
  - [ ] 11.4.4 Add cache metrics
  - [ ] 11.4.5 Add distributed cache
  - [ ] 11.4.6 Add cache persistence

### Phase 4: Developer Experience (Weeks 10-12)

#### 12. nebula-sdk (Week 10)
- [ ] 12.1 **Core SDK**
  - [ ] 12.1.1 Create prelude module
  - [ ] 12.1.2 Export core types
  - [ ] 12.1.3 Export derive macros
  - [ ] 12.1.4 Add utility functions
  - [ ] 12.1.5 Add type aliases
  - [ ] 12.1.6 Add documentation

- [ ] 12.2 **HTTP Utilities**
  - [ ] 12.2.1 Create HTTP client wrapper
  - [ ] 12.2.2 Add retry logic
  - [ ] 12.2.3 Add timeout handling
  - [ ] 12.2.4 Add response parsing
  - [ ] 12.2.5 Add authentication
  - [ ] 12.2.6 Add request building

- [ ] 12.3 **Data Utilities**
  - [ ] 12.3.1 Add JSON helpers
  - [ ] 12.3.2 Add CSV parsing
  - [ ] 12.3.3 Add XML parsing
  - [ ] 12.3.4 Add data transformation
  - [ ] 12.3.5 Add validation helpers
  - [ ] 12.3.6 Add serialization helpers

- [ ] 12.4 **Testing Utilities**
  - [ ] 12.4.1 Create MockContext
  - [ ] 12.4.2 Add test helpers
  - [ ] 12.4.3 Add assertion macros
  - [ ] 12.4.4 Add fixture support
  - [ ] 12.4.5 Add snapshot testing
  - [ ] 12.4.6 Add performance testing

#### 13. nebula-api (Week 10-11)
- [ ] 13.1 **REST API**
  - [ ] 13.1.1 Setup Axum framework
  - [ ] 13.1.2 Create workflow endpoints
  - [ ] 13.1.3 Create execution endpoints
  - [ ] 13.1.4 Create node endpoints
  - [ ] 13.1.5 Add authentication
  - [ ] 13.1.6 Add rate limiting

- [ ] 13.2 **GraphQL** — отложен; API только REST + WebSocket

- [ ] 13.3 **WebSocket Support**
  - [ ] 13.3.1 Implement WebSocket handler
  - [ ] 13.3.2 Add real-time updates
  - [ ] 13.3.3 Add execution streaming
  - [ ] 13.3.4 Add log streaming
  - [ ] 13.3.5 Add metrics streaming
  - [ ] 13.3.6 Add connection management

- [ ] 13.4 **API Documentation**
  - [ ] 13.4.1 Add OpenAPI spec
  - [ ] 13.4.2 Add OpenAPI/WebSocket docs
  - [ ] 13.4.3 Add example requests
  - [ ] 13.4.4 Add error codes
  - [ ] 13.4.5 Add rate limit docs
  - [ ] 13.4.6 Add webhook docs

#### 14. Standard Nodes (Week 11-12)
- [ ] 14.1 **HTTP Nodes**
  - [ ] 14.1.1 Create HTTP Request node
  - [ ] 14.1.2 Create HTTP Response node
  - [ ] 14.1.3 Create Webhook trigger
  - [ ] 14.1.4 Add authentication support
  - [ ] 14.1.5 Add proxy support
  - [ ] 14.1.6 Add retry configuration

- [ ] 14.2 **Data Transform Nodes**
  - [ ] 14.2.1 Create JSON Transform node
  - [ ] 14.2.2 Create CSV Parser node
  - [ ] 14.2.3 Create Data Mapper node
  - [ ] 14.2.4 Create Filter node
  - [ ] 14.2.5 Create Aggregation node
  - [ ] 14.2.6 Create Sort node

- [ ] 14.3 **Database Nodes**
  - [ ] 14.3.1 Create PostgreSQL node
  - [ ] 14.3.2 Create MySQL node
  - [ ] 14.3.3 Create MongoDB node
  - [ ] 14.3.4 Add query builder
  - [ ] 14.3.5 Add transaction support
  - [ ] 14.3.6 Add connection pooling

- [ ] 14.4 **Utility Nodes**
  - [ ] 14.4.1 Create Logger node
  - [ ] 14.4.2 Create Delay node
  - [ ] 14.4.3 Create Conditional node
  - [ ] 14.4.4 Create Loop node
  - [ ] 14.4.5 Create Error Handler node
  - [ ] 14.4.6 Create Notification node

### Phase 5: Production Features (Weeks 13-16)

#### 15. Performance Optimization (Week 13)
- [ ] 15.1 **Profiling**
  - [ ] 15.1.1 Add performance benchmarks
  - [ ] 15.1.2 Identify bottlenecks
  - [ ] 15.1.3 Add flame graphs
  - [ ] 15.1.4 Memory profiling
  - [ ] 15.1.5 CPU profiling
  - [ ] 15.1.6 I/O profiling

- [ ] 15.2 **Optimization**
  - [ ] 15.2.1 Optimize serialization
  - [ ] 15.2.2 Add zero-copy where possible
  - [ ] 15.2.3 Optimize allocations
  - [ ] 15.2.4 Add SIMD optimizations
  - [ ] 15.2.5 Optimize database queries
  - [ ] 15.2.6 Add caching strategies

#### 16. Monitoring & Observability (Week 14)
- [ ] 16.1 **Metrics**
  - [ ] 16.1.1 Integrate Prometheus
  - [ ] 16.1.2 Add custom metrics
  - [ ] 16.1.3 Add dashboards
  - [ ] 16.1.4 Add alerts
  - [ ] 16.1.5 Add SLI/SLO tracking
  - [ ] 16.1.6 Add capacity planning

- [ ] 16.2 **Tracing**
  - [ ] 16.2.1 Integrate OpenTelemetry
  - [ ] 16.2.2 Add distributed tracing
  - [ ] 16.2.3 Add trace sampling
  - [ ] 16.2.4 Add context propagation
  - [ ] 16.2.5 Add trace visualization
  - [ ] 16.2.6 Add performance analysis

#### 17. Security (Week 15)
- [ ] 17.1 **Authentication & Authorization**
  - [ ] 17.1.1 Add JWT support
  - [ ] 17.1.2 Add OAuth2 integration
  - [ ] 17.1.3 Add RBAC system
  - [ ] 17.1.4 Add API key management
  - [ ] 17.1.5 Add session management
  - [ ] 17.1.6 Add MFA support

- [ ] 17.2 **Security Hardening**
  - [ ] 17.2.1 Add input validation
  - [ ] 17.2.2 Add SQL injection prevention
  - [ ] 17.2.3 Add XSS prevention
  - [ ] 17.2.4 Add rate limiting
  - [ ] 17.2.5 Add encryption at rest
  - [ ] 17.2.6 Add audit logging

#### 18. Documentation & Testing (Week 16)
- [ ] 18.1 **Documentation**
  - [ ] 18.1.1 Complete API documentation
  - [ ] 18.1.2 Write architecture guide
  - [ ] 18.1.3 Create user manual
  - [ ] 18.1.4 Add deployment guide
  - [ ] 18.1.5 Create troubleshooting guide
  - [ ] 18.1.6 Add migration guide

- [ ] 18.2 **Testing**
  - [ ] 18.2.1 Achieve 80% test coverage
  - [ ] 18.2.2 Add integration tests
  - [ ] 18.2.3 Add load tests
  - [ ] 18.2.4 Add chaos testing
  - [ ] 18.2.5 Add security tests
  - [ ] 18.2.6 Add regression tests

---

# 📁 Crate Documentation

## nebula-derive

### Purpose
Procedural macros для уменьшения boilerplate кода при создании nodes и parameters.

### Responsibilities
- Генерация кода для Parameters
- Генерация кода для Actions
- Compile-time валидация
- Автоматическая документация

### Architecture
```rust
// Макросы
#[proc_macro_derive(Parameters, attributes(param, validate, display))]
#[proc_macro_derive(Action, attributes(action, node))]
#[proc_macro_attribute]
pub fn node(args: TokenStream, input: TokenStream) -> TokenStream
```

### Roadmap Details

#### 4.1 Macro Setup
- [ ] 4.1.1 **Create proc-macro crate**
  - Setup Cargo.toml with proc-macro = true
  - Add syn, quote, proc-macro2 dependencies
  - Create lib.rs structure

- [ ] 4.1.2 **Setup syn and quote**
  - Configure feature flags
  - Setup parsing infrastructure
  - Create helper modules

- [ ] 4.1.3 **Add error handling**
  - Create error types
  - Add span information
  - Implement error recovery

- [ ] 4.1.4 **Setup testing framework**
  - Add trybuild for compile tests
  - Create test fixtures
  - Setup expansion tests

#### 4.2 Parameters Derive
- [ ] 4.2.1 **Parse struct attributes**
  - Parse field types
  - Extract param attributes
  - Handle nested attributes

- [ ] 4.2.2 **Generate parameter_collection()**
  - Create ParameterCollection
  - Add each field as parameter
  - Handle Option types

- [ ] 4.2.3 **Generate from_values()**
  - Extract values by key
  - Type conversion
  - Error handling

- [ ] 4.2.4 **Add validation attributes**
  - #[validate(required)]
  - #[validate(min = 1, max = 100)]
  - #[validate(regex = "pattern")]

- [ ] 4.2.5 **Add display attributes**
  - #[display(show_when(...))]
  - #[display(hide_when(...))]
  - Conditional visibility

- [ ] 4.2.6 **Generate documentation**
  - Extract doc comments
  - Generate parameter descriptions
  - Add to metadata

---

## nebula-expression

### Purpose
Полноценный expression язык для динамического вычисления значений в workflows.

### Responsibilities
- Парсинг expressions
- Вычисление expressions
- Функции и операторы
- Type checking

### Architecture
```rust
pub struct ExpressionEngine {
    parser: Parser,
    evaluator: Evaluator,
    functions: FunctionRegistry,
    operators: OperatorRegistry,
}
```

### Expression Examples
```
// Простой доступ
$nodes.http_request.body.user.email

// Операторы
$nodes.calc.value * 100 + $vars.base_amount

// Функции
concat($nodes.first_name.output, " ", $nodes.last_name.output)
formatDate(now(), "YYYY-MM-DD")

// Pipe operations
$nodes.users.list 
  | filter(u => u.active) 
  | map(u => u.email)
  | join(", ")

// Условные выражения
$vars.env == "prod" ? $nodes.prod_config : $nodes.dev_config
```

---

## nebula-engine

### Purpose
Движок выполнения workflows, управляющий жизненным циклом executions.

### Responsibilities
- Orchestration workflows
- Event processing
- State management
- Scheduling

### Architecture
```rust
pub struct WorkflowEngine {
    event_bus: Arc<dyn EventBus>,
    state_store: Arc<dyn StateStore>,
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
}
```

### Event Flow
```
Trigger → Event → Engine → Scheduler → Worker
                    ↓
                State Store
```

---

## nebula-storage

### Purpose
Абстракция над различными storage backends для персистентности данных.

### Responsibilities
- Workflow definitions storage
- Execution state storage
- Query capabilities
- Transaction support

### Architecture
```rust
#[async_trait]
pub trait StorageBackend {
    async fn save_workflow(&self, workflow: &Workflow) -> Result<()>;
    async fn load_workflow(&self, id: &WorkflowId) -> Result<Workflow>;
    async fn save_execution(&self, execution: &ExecutionState) -> Result<()>;
    async fn query_executions(&self, query: Query) -> Result<Vec<ExecutionSummary>>;
}
```

---

## nebula-binary

### Purpose
Управление бинарными данными с автоматическим выбором стратегии хранения.

### Responsibilities
- Binary data storage
- Automatic tiering
- Streaming support
- Garbage collection

### Architecture
```rust
pub enum BinaryDataLocation {
    InMemory(Vec<u8>),           // < 1MB
    Temp { path: PathBuf },       // < 100MB
    Remote { key: String },       // > 100MB
    Generated { params: Value },  // On-demand
}
```

---

## nebula-runtime

### Purpose
Управление активными triggers и координация workflow executions.

### Responsibilities
- Trigger lifecycle
- Event listening
- Workflow activation
- Health monitoring

### Architecture
```rust
pub struct Runtime {
    trigger_manager: Arc<TriggerManager>,
    event_listener: Arc<EventListener>,
    workflow_coordinator: Arc<WorkflowCoordinator>,
    health_monitor: Arc<HealthMonitor>,
}
```

---

## nebula-worker

### Purpose
Процессы выполнения nodes с изоляцией и resource management.

### Responsibilities
- Node execution
- Resource isolation
- Progress reporting
- Health checks

### Architecture
```rust
pub struct Worker {
    id: WorkerId,
    executor: NodeExecutor,
    resource_manager: ResourceManager,
    sandbox: ExecutionSandbox,
}
```

---

## nebula-node-registry

### Purpose
Управление динамической загрузкой и версионированием nodes.

### Responsibilities
- Node discovery
- Plugin loading
- Version management
- Git integration

### Architecture
```rust
pub struct NodeRegistry {
    loaded_nodes: HashMap<String, LoadedNode>,
    plugin_manager: PluginManager,
    git_integrator: GitIntegrator,
    cache: NodeCache,
}
```

---

## nebula-api

### Purpose
API layer: **REST + WebSocket** (GraphQL не планируется в текущей фазе).

### Responsibilities
- REST endpoints
- WebSocket streaming (real-time, execution logs)
- Authentication

### Architecture
```rust
pub struct ApiServer {
    rest: RestApi,
    websocket: WebSocketHandler,
    auth: AuthManager,
}
```

---

## nebula-sdk

### Purpose
All-in-one SDK для разработчиков nodes с богатым набором утилит.

### Responsibilities
- Unified exports
- Helper functions
- Testing utilities
- Documentation

### Architecture
```rust
pub mod prelude {
    pub use nebula_core::*;
    pub use nebula_derive::*;
    pub use crate::http::*;
    pub use crate::data::*;
    pub use crate::testing::*;
}
```

### SDK Features
- HTTP client с retry и timeout
- JSON/CSV/XML parsing
- Crypto utilities
- Testing helpers
- Performance utilities