# Archived From "docs/archive/nebula-complete.md"

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

