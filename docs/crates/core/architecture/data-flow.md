---

# Data Flow Architecture

## Overview

Данные в Nebula проходят через несколько уровней трансформации и оптимизации. Система спроектирована для эффективной обработки как маленьких JSON объектов, так и больших бинарных файлов.

## Workflow Execution Data Flow

### 1. Trigger Phase

```mermaid
sequenceDiagram
    participant T as Trigger
    participant R as Runtime
    participant K as Kafka
    participant E as Engine
    
    T->>R: Event Received
    R->>R: Validate & Transform
    R->>K: Publish TriggerEvent
    K->>E: Consume Event
    E->>E: Create Execution
```

### 2. Execution Phase

```mermaid
graph LR
    A[Input Data] --> B[Expression Resolution]
    B --> C[Type Validation]
    C --> D[Node Execution]
    D --> E[Output Serialization]
    E --> F[Storage]
    F --> G[Next Node Input]
```

## Data Types and Storage

### Value Types Flow

```rust
// User Input → Typed Value → Validation → Storage → Next Node
pub enum DataLifecycle {
    UserInput(serde_json::Value),
    TypedValue(Value),
    ValidatedValue(ValidatedValue),
    StoredValue(StorageRef),
    RetrievedValue(Value),
}
```

### Binary Data Flow

```mermaid
graph TB
    Upload[File Upload] --> Check{Size Check}
    Check -->|< 1MB| Memory[In-Memory Storage]
    Check -->|1-100MB| Temp[Temp File Storage]
    Check -->|> 100MB| S3[S3/Object Storage]
    
    Memory --> Use[Node Usage]
    Temp --> Use
    S3 --> Use
    
    Use --> Cleanup{Execution Complete}
    Cleanup --> GC[Garbage Collection]
```

## Expression Resolution Flow

### Expression Evaluation Pipeline

```rust
// Raw Expression → Parse → AST → Resolve References → Evaluate → Result
"$nodes.http.body.users[0].email" 
    → ParseExpression
    → AST { 
        Variable("nodes"),
        Property("http"),
        Property("body"),
        Property("users"),
        Index(0),
        Property("email")
    }
    → ResolveContext { execution_id, node_outputs }
    → "user@example.com"
```

### Context Building

```mermaid
graph TD
    A[Execution State] --> B[Build Context]
    C[Node Outputs] --> B
    D[Variables] --> B
    E[Environment] --> B
    B --> F[Expression Context]
    F --> G[Available to Expressions]
```

## Streaming Data Flow

### Large Dataset Processing

```rust
pub enum ProcessingMode {
    // Entire dataset in memory
    Batch { data: Vec<Value> },
    
    // Streaming with backpressure
    Stream { 
        source: Box<dyn Stream<Item = Value>>,
        buffer_size: usize,
    },
    
    // Chunked processing
    Chunked {
        chunk_size: usize,
        processor: Box<dyn ChunkProcessor>,
    },
}
```

### Backpressure Handling

```mermaid
graph LR
    A[Fast Producer] --> B[Buffer]
    B --> C{Buffer Full?}
    C -->|Yes| D[Apply Backpressure]
    C -->|No| E[Slow Consumer]
    D --> F[Pause Producer]
    E --> G[Process Data]
    G --> B
```

## Error Data Flow

### Error Propagation

```rust
pub enum ErrorFlow {
    // Node level error - can be caught
    NodeError { 
        node_id: NodeId,
        error: Error,
        recovery: RecoveryStrategy,
    },
    
    // Workflow level error - stops execution
    WorkflowError {
        execution_id: ExecutionId,
        error: Error,
    },
    
    // System level error - requires intervention
    SystemError {
        component: Component,
        error: Error,
        impact: Impact,
    },
}
```

### Error Recovery Flow

```mermaid
stateDiagram-v2
    [*] --> Executing
    Executing --> Error: Node Fails
    Error --> Retry: Retry Strategy
    Retry --> Executing: Retry Attempt
    Retry --> Fallback: Max Retries
    Fallback --> Compensation: Has Compensation
    Fallback --> Failed: No Compensation
    Compensation --> Recovered
    Recovered --> [*]
    Failed --> [*]
```

## Performance Optimizations

### Data Locality

```rust
// Keep data close to computation
pub struct DataLocality {
    // Prefer same worker for sequential nodes
    worker_affinity: Option<WorkerId>,
    
    // Cache hot data in worker memory
    local_cache: LruCache<DataKey, Value>,
    
    // Predictive prefetching
    prefetch_queue: VecDeque<DataKey>,
}
```

### Zero-Copy Strategies

```rust
// Avoid copying data when possible
pub enum DataTransfer {
    // Same process - use Arc
    SharedMemory(Arc<Value>),
    
    // Same machine - use mmap
    MemoryMapped(MmapFile),
    
    // Different machines - use streaming
    Network(TcpStream),
}
```

---

