# Context, Capabilities, and Core Resources

## Design Principle: Slim Context, Rich Resources

ActionContext provides only **access primitives**: resource acquisition, credential
access, port data, and execution lifecycle (heartbeat, cancellation). All domain
capabilities (binary I/O, streaming, HTTP, metrics) are **core resources** obtained
through `ctx.resource_typed<R>(key)`.

One pattern to learn, one pattern to test, one pattern to mock.

```
ActionContext (slim)          Core Resources (via resource_typed)
├── resource_typed<R>(key)    ├── BinaryStorage  — read/write/stream binary data
├── credential_typed<S>(key)  ├── StreamOutput    — real-time streaming (LLM tokens)
├── port_data(port_key)       ├── HttpClient      — HTTP requests
├── call_action(key, input)   ├── ActionLogger    — structured logging
├── heartbeat()               └── MetricsCollector — counters/histograms
└── is_cancelled()
```

---

## ExecutionGuard

Bounds accessor usage to execution lifetime. Runtime revokes on completion/cancel.

```rust
pub struct ExecutionGuard {
    alive: Arc<AtomicBool>,
}

impl ExecutionGuard {
    pub(crate) fn new() -> Self {
        Self { alive: Arc::new(AtomicBool::new(true)) }
    }

    pub fn check(&self) -> Result<(), ActionError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(ActionError::Cancelled);
        }
        Ok(())
    }

    pub(crate) fn revoke(&self) {
        self.alive.store(false, Ordering::Release);
    }
}
```

---

## ActionContext (NOT Clone)

```rust
/// Stable execution context for StatelessAction, StatefulAction, ResourceAction.
///
/// **NOT Clone.** Action receives `&ActionContext`. Accessors check ExecutionGuard
/// on every call — post-completion/cancel access returns ActionError::Cancelled.
///
/// **Slim by design:** Context provides access primitives only. All domain
/// capabilities (binary I/O, streaming, HTTP) are core resources obtained
/// through resource_typed<R>(key). This keeps context testable and extensible.
///
/// **Ownership note:** `resource_typed<R>()` returns a typed **managed handle/lease**,
/// not the raw resource object. Handle lifecycle is managed by the resource layer.
pub struct ActionContext {
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub workflow_id: WorkflowId,
    pub cancellation: CancellationToken,
    guard: ExecutionGuard,
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    action_executor: Arc<dyn ActionExecutor>,
    port_values: Option<HashMap<String, serde_json::Value>>,
    support_values: Option<HashMap<String, serde_json::Value>>,
    support_multi_values: Option<HashMap<String, Vec<serde_json::Value>>>,
}

impl ActionContext {
    // ── Resource access ──

    /// Acquire typed managed handle/lease by local alias.
    /// This is the primary capability primitive. All domain resources
    /// (BinaryStorage, StreamOutput, HttpClient, Logger, Metrics)
    /// are obtained through this method.
    pub async fn resource_typed<R: Send + Sync + 'static>(
        &self, key: &str,
    ) -> Result<R, ActionError> {
        self.guard.check()?;
        let boxed = self.resources.acquire(key).await?;
        *boxed.downcast::<R>()
            .map_err(|_| ActionError::fatal(format!(
                "resource '{}' type mismatch: expected {}", key, std::any::type_name::<R>()
            )))
    }

    // ── Credential access ──

    pub async fn credential_typed<S: Send + Sync + 'static>(
        &self, key: &str,
    ) -> Result<S, ActionError> {
        self.guard.check()?;
        let snapshot = self.credentials.get(key).await?;
        snapshot.downcast::<S>()
            .map_err(|_| ActionError::fatal(format!(
                "credential '{}' scheme mismatch: expected {}", key, std::any::type_name::<S>()
            )))
    }

    // ── Multi-port input access ──

    /// Raw upstream data from a specific input port.
    /// For nodes with multiple input ports (Merge, Join).
    pub fn port_data(&self, port_key: &str) -> Option<&serde_json::Value> {
        self.port_values.as_ref()?.get(port_key)
    }

    // ── Support port data ──

    /// Get data from a support input port (single connection).
    /// Returns error if required port is not connected.
    pub fn support_data(&self, port_key: &str) -> Result<&serde_json::Value, ActionError> {
        self.support_values
            .as_ref()
            .and_then(|m| m.get(port_key))
            .ok_or_else(|| ActionError::validation(format!(
                "support port '{}' not connected", port_key
            )))
    }

    /// Get data from a multi-connection support port (returns all connected values).
    pub fn support_data_multi(&self, port_key: &str) -> Result<Vec<&serde_json::Value>, ActionError> {
        self.support_multi_values
            .as_ref()
            .and_then(|m| m.get(port_key))
            .map(|v| v.iter().collect())
            .ok_or_else(|| ActionError::validation(format!(
                "support port '{}' not connected", port_key
            )))
    }

    // ── Action invocation ──

    /// Execute another action by key (AI tool calling, sub-steps).
    ///
    /// **Normative contract:**
    /// - Synchronous request/response composition only.
    /// - NOT a general sub-workflow executor.
    /// - Does NOT imply fan-out/fan-in, child DAG, or engine scheduling.
    /// - Callable actions MUST be declared in CapabilityManifest.
    /// - Runtime SHOULD enforce max call depth to prevent unbounded recursion.
    pub async fn call_action(
        &self,
        action_key: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ActionError> {
        self.guard.check()?;
        self.action_executor.execute(action_key, input).await
    }

    // ── Execution lifecycle ──

    /// Heartbeat for long-running actions. Checks cancellation first.
    /// Engine cancels if no heartbeat within 2× heartbeat interval.
    pub fn heartbeat(&self) -> Result<(), ActionError> {
        self.guard.check()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}
```

---

## Core Resources

Core resources are provided by Nebula runtime and always available.
They are registered in the global ResourceManager at startup.
Action authors declare them via `#[resource]` like any other resource.
No special API — one pattern for everything.

### BinaryStorage — read/write/stream binary data

```rust
/// Core resource for binary data I/O.
/// Works with BinaryData type (inline/stored strategy from codebase).
/// Small data stays inline, large data stored externally — transparent to author.
///
/// **Cleanup:** Runtime cleans up Stored temp files when
/// ExecutionGuard is revoked (on completion/cancel/error).
#[async_trait]
pub trait BinaryStorage: Send + Sync {
    /// Read binary to bytes (resolves Stored, returns Inline as-is).
    async fn read(&self, data: &BinaryData) -> Result<Vec<u8>, ActionError>;

    /// Stream binary data (for large files — avoids OOM).
    async fn stream(&self, data: &BinaryData)
        -> Result<Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ActionError>> + Send>>, ActionError>;

    /// Write new binary. Framework auto-selects Inline (<1MB) vs Stored (>1MB).
    async fn write(
        &self,
        data: Vec<u8>,
        filename: &str,
        content_type: &str,
    ) -> Result<BinaryData, ActionError>;

    /// Write from stream (for large data).
    async fn write_stream(
        &self,
        stream: Pin<Box<dyn Stream<Item = Result<bytes::Bytes, ActionError>> + Send>>,
        filename: &str,
        content_type: &str,
    ) -> Result<BinaryData, ActionError>;
}
```

**Usage:**
```rust
#[derive(Action)]
#[action(key = "file.convert_to_pdf", name = "Convert to PDF")]
#[resource(BinaryStorage, key = "storage")]
struct ConvertToPdf;

#[derive(ActionDeps)]
struct ConvertDeps {
    #[dep(resource = "storage")]
    storage: BinaryStorage,
}

impl SimpleAction for ConvertToPdf {
    type Input = ConvertInput;  // contains: file: BinaryData
    type Output = serde_json::Value;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<Self::Output, ActionError>
    {
        let deps = ConvertDeps::resolve(ctx).await?;
        // Read from upstream (inline or stored — transparent)
        let bytes = deps.storage.read(&input.file).await?;
        let pdf = html_to_pdf(&bytes).await.fatal()?;
        // Write — framework picks Inline or Stored based on size
        let result = deps.storage.write(pdf, "output.pdf", "application/pdf").await?;
        Ok(json!({ "file": result }))
    }
}
```

### StreamOutput — real-time streaming channel (LLM tokens, progress)

```rust
/// Core resource for real-time streaming output.
/// Engine routes chunks to downstream nodes / UI in real-time.
/// Final result still returned from execute() as usual.
///
/// **Backpressure:** Uses bounded mpsc channel (recommended buffer:
/// 1024 chunks). If producer is faster than consumer, sender awaits —
/// does not OOM.
#[async_trait]
pub trait StreamOutput: Send + Sync {
    /// Open a streaming channel.
    async fn open(&self) -> Result<StreamSender, ActionError>;
}

pub struct StreamSender {
    tx: mpsc::Sender<StreamChunk>,
}

impl StreamSender {
    pub async fn send(&self, chunk: StreamChunk) -> Result<(), ActionError> {
        self.tx.send(chunk).await
            .map_err(|_| ActionError::fatal("stream channel closed"))
    }

    pub async fn close(self) -> Result<(), ActionError> {
        drop(self.tx);
        Ok(())
    }
}

pub enum StreamChunk {
    Text(String),
    Binary(bytes::Bytes),
    Json(serde_json::Value),
}
```

**Usage:**
```rust
#[derive(Action)]
#[action(key = "openai.chat", name = "OpenAI Chat")]
#[resource(HttpClient, key = "http")]
#[resource(StreamOutput, key = "stream")]
#[credential(BearerToken, key = "openai_key")]
struct OpenAIChat;

impl SimpleAction for OpenAIChat {
    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<Self::Output, ActionError>
    {
        let http = ctx.resource_typed::<HttpClient>("http").await?;
        let streamer = ctx.resource_typed::<StreamOutput>("stream").await?;
        let key = ctx.credential_typed::<BearerToken>("openai_key").await?;

        let sender = streamer.open().await?;
        let mut full = String::new();

        let mut llm_stream = openai_stream(&http, &key, &input.prompt).await.retryable()?;
        while let Some(chunk) = llm_stream.next().await {
            let token = chunk.retryable()?;
            sender.send(StreamChunk::Text(token.clone())).await?;
            full.push_str(&token);
        }
        sender.close().await?;

        Ok(json!({ "text": full }))
    }
}
```

### Core resource table

| Resource | Key | Purpose | Always available |
|----------|-----|---------|-----------------|
| `BinaryStorage` | `"storage"` | Binary data read/write/stream | ✅ |
| `StreamOutput` | `"stream"` | Real-time output streaming | ✅ |
| `HttpClient` | `"http"` | HTTP requests | ✅ |
| `ActionLogger` | `"logger"` | Structured logging | ✅ |
| `MetricsCollector` | `"metrics"` | Counters, histograms | ✅ |

Core resources are auto-registered by runtime at startup. Action authors declare
them with `#[resource]` + `resource_typed` — same pattern as any other resource.

---

## BinaryData (aligned with existing codebase)

Existing codebase already has a richer pattern than HLD's original `BinaryHandle`:
inline/stored strategy avoids unnecessary disk writes for small payloads.

**Naming:** `BinaryPayload` (enum for storage strategy) vs `BinaryStorage` (trait for I/O).
No collision — enum describes data layout, trait describes resource operations.

```rust
/// Binary data with automatic storage strategy.
/// Small data (< 1MB) stays inline in memory.
/// Large data stored on disk/S3 with path reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryData {
    pub content_type: String,
    pub size: usize,
    pub payload: BinaryPayload,
    pub metadata: Option<serde_json::Value>,
}

/// Storage strategy — framework chooses automatically based on size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BinaryPayload {
    /// Small data (< 1MB) — kept in memory, zero I/O overhead.
    Inline(Vec<u8>),
    /// Large data — stored externally, referenced by path.
    Stored {
        storage_type: StorageType,
        path: String,
        checksum: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageType {
    LocalFile,
    S3,
    TempFile,
}
```

**Usage in ActionInput:**
```rust
#[derive(ActionInput, Deserialize)]
struct ImageProcessInput {
    image: BinaryData,  // from upstream — may be Inline or Stored
    width: u32,
    height: u32,
}
```

**BinaryStorage core resource (trait) works with BinaryData:**

The `BinaryStorage` trait (defined in Core Resources section above) handles
read/write/stream operations on BinaryData. No naming collision — `BinaryPayload`
is the enum, `BinaryStorage` is the trait.

**Cleanup:** Runtime cleans up `Stored` temp files when ExecutionGuard is revoked.

---

## WaitCondition (aligned with existing codebase)

```rust
/// Condition that must be met before a waiting action resumes.
/// Naming aligned with existing codebase (Webhook/Duration, not ExternalSignal/After).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum WaitCondition {
    /// Wait for a fixed duration.
    Duration {
        #[serde(with = "duration_ms")]
        duration: Duration,
    },
    /// Wait until a specific point in time.
    Until {
        datetime: chrono::DateTime<chrono::Utc>,
    },
    /// Wait for an inbound HTTP callback (webhook resume pattern).
    /// Runtime generates unique URL keyed by (execution_id, node_id, callback_id).
    Webhook {
        callback_id: String,
    },
    /// Wait for human approval.
    Approval {
        approver: String,
        message: String,
    },
    /// Wait for another execution to complete.
    Execution {
        execution_id: ExecutionId,
    },
}
```

---

## Capability Interfaces (context-level)

These are injected by runtime into context. Not resources — they are
context infrastructure that doesn't fit the resource acquire/release pattern.

### ResourceProvider (shared by ActionContext and TriggerContext)

```rust
/// Shared capability for resource/credential access.
/// Implemented by both ActionContext and TriggerContext.
/// Enables #[derive(ActionDeps)] to work with both context types.
pub trait ResourceProvider: Send + Sync {
    async fn resource_typed<R: Send + Sync + 'static>(&self, key: &str) -> Result<R, ActionError>;
    async fn credential_typed<S: Send + Sync + 'static>(&self, key: &str) -> Result<S, ActionError>;
}

// Both contexts implement ResourceProvider:
impl ResourceProvider for ActionContext { /* delegates to self.resources/credentials */ }
impl ResourceProvider for TriggerContext { /* delegates to self.resources/credentials */ }
```

```rust
/// Object-safe resource accessor injected into ActionContext.
#[async_trait]
pub trait ResourceAccessor: Send + Sync {
    async fn acquire(&self, key: &str) -> Result<Box<dyn Any + Send + Sync>, ActionError>;
    async fn exists(&self, key: &str) -> bool;
}

/// Object-safe credential accessor.
#[async_trait]
pub trait CredentialAccessor: Send + Sync {
    async fn get(&self, id: &str) -> Result<CredentialSnapshot, ActionError>;
    async fn has(&self, id: &str) -> bool;
}

/// Action executor for call_action() capability.
#[async_trait]
pub trait ActionExecutor: Send + Sync {
    async fn execute(&self, action_key: &str, input: serde_json::Value) -> Result<serde_json::Value, ActionError>;
}

/// Trigger-specific scheduling capability.
#[async_trait]
pub trait TriggerScheduler: Send + Sync {
    async fn schedule_after(&self, delay: Duration) -> Result<(), ActionError>;
    async fn schedule_at(&self, at: chrono::DateTime<chrono::Utc>) -> Result<(), ActionError>;
    async fn unschedule(&self) -> Result<(), ActionError>;
}

/// Trigger-specific execution emission capability.
#[async_trait]
pub trait ExecutionEmitter: Send + Sync {
    async fn emit(&self, input: serde_json::Value) -> Result<ExecutionId, ActionError>;
    async fn emit_and_checkpoint(&self, input: serde_json::Value, state: serde_json::Value) -> Result<ExecutionId, ActionError>;
    async fn emit_batch(&self, inputs: Vec<serde_json::Value>) -> Result<Vec<ExecutionId>, ActionError>;
    async fn execution_status(&self, id: ExecutionId) -> Result<ExecutionStatus, ActionError>;
}

/// Trigger state checkpoint sink.
#[async_trait]
pub trait TriggerCheckpointSink: Send + Sync {
    async fn save(&self, state: serde_json::Value) -> Result<(), ActionError>;
}

/// Parameter provider for trigger parameter access.
pub trait ParameterProvider: Send + Sync {
    fn get(&self, key: &str) -> Option<serde_json::Value>;
}
```

---

## CapabilityManifest (derived from ActionComponents)

Single source of truth — manifest computed from components, never authored manually.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub resources: Vec<ResourceCapability>,
    pub credentials: Vec<CredentialCapability>,
    pub host: HostCapabilities,
}

impl CapabilityManifest {
    pub fn from_components(components: &ActionComponents) -> Self { ... }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceCapability {
    pub key: String,
    pub resource_type: String,
    pub access: ResourceAccessMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ResourceAccessMode { Acquire, UseScoped }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialCapability {
    pub key: String,
    pub scheme_type: String,
    pub access: CredentialAccessMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CredentialAccessMode { Snapshot }

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct HostCapabilities: u32 {
        const LOGGING          = 0b0001;
        const NETWORK_EGRESS   = 0b0010;
        const FILESYSTEM_READ  = 0b0100;
        const FILESYSTEM_WRITE = 0b1000;
    }
}
```

### Local Alias → Global ID Mapping

```
Action:     ctx.credential("github_api")
Components: CredentialRef { key: "github_api", scheme: BearerToken }
UI:         user binds "github_api" → CredentialId("550e8400-...")
Runtime:    EnforcedCredentialAccessor { allowed: { "github_api" → "550e8400-..." } }

ctx.credential("github_api") → guard.check() → allowed? → vault.snapshot()
ctx.credential("secret_admin") → guard.check() → NOT allowed → SandboxViolation
```
