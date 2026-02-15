use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ── Supporting types ────────────────────────────────────────────────────────

/// A not-yet-available output with instructions for the engine on how
/// to obtain the final result.
///
/// The action has kicked off work (AI generation, external API call,
/// document rendering), but the result isn't ready yet. The engine
/// resolves this before passing data to downstream nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeferredOutput {
    /// Unique handle for this deferred result.
    pub handle_id: String,
    /// How the engine should obtain the result.
    pub resolution: Resolution,
    /// What type of output to expect when resolved.
    pub expected: ExpectedOutput,
    /// Current progress (updated via heartbeats).
    pub progress: Option<Progress>,
    /// Who/what is producing this output.
    pub producer: Producer,
    /// Retry policy if a resolution fails.
    pub retry: Option<crate::metadata::RetryPolicy>,
    /// Maximum time to wait before treating as failed.
    pub timeout: Option<Duration>,
}

/// How the engine resolves a deferred output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Resolution {
    /// Engine polls a target at intervals.
    Poll {
        /// What to poll.
        target: PollTarget,
        /// How often to poll.
        interval: Duration,
        /// Backoff multiplier (1.0 = constant, 2.0 = exponential).
        backoff: f64,
        /// Upper bound on a poll interval.
        max_interval: Option<Duration>,
    },
    /// Engine awaits a one-shot notification.
    Await {
        /// Correlation ID for the notification system.
        channel_id: String,
    },
    /// External system calls back via webhook or signal.
    Callback {
        /// URL or signal endpoint.
        endpoint: String,
        /// Correlation token.
        token: String,
    },
    /// Engine spawns a sub-workflow to produce the result.
    SubWorkflow {
        /// Workflow to spawn.
        workflow_id: String,
        /// Optional input data for the sub-workflow.
        input: Option<serde_json::Value>,
    },
    /// Try await first, fall back to polling after timeout.
    AwaitOrPoll {
        /// Channel to await on.
        channel_id: String,
        /// How long to wait before falling back to polling.
        fallback_after: Duration,
        /// What to poll as fallback.
        poll_target: PollTarget,
        /// Fallback poll interval.
        poll_interval: Duration,
    },
}

/// Target for poll-based resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum PollTarget {
    /// HTTP endpoint returning status + optional result.
    Http {
        /// URL to poll.
        url: String,
        /// HTTP method to use.
        method: String,
    },
    /// Re-invoke an action to check status.
    Action {
        /// Key of the action to invoke.
        action_key: String,
    },
    /// Check an external service.
    Service {
        /// Service name.
        name: String,
        /// Operation to invoke.
        operation: String,
    },
}

/// What the deferred output will resolve to.
/// Used for DAG validation without waiting for actual data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExpectedOutput {
    /// Will resolve to `ActionOutput::Value`.
    Value {
        /// Optional JSON Schema describing the expected shape.
        schema: Option<serde_json::Value>,
    },
    /// Will resolve to `ActionOutput::Binary`.
    Binary {
        /// Expected MIME content type.
        content_type: String,
    },
    /// Will resolve to `ActionOutput::Reference`.
    Reference,
    /// Will resolve to `ActionOutput::Streaming`.
    Stream,
    /// Unknown at compile time.
    Dynamic,
}

/// A stream of output chunks arriving incrementally.
///
/// The engine can collect all chunks into a final value, forward the
/// stream to a streaming-aware downstream node, or tap for progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamOutput {
    /// Unique stream identifier for subscription.
    pub stream_id: String,
    /// What kind of data is being streamed and how to consume it.
    pub mode: StreamMode,
    /// What the collected stream will produce.
    pub expected: ExpectedOutput,
    /// Current stream state (for serialization/checkpointing).
    pub state: StreamState,
    /// Backpressure configuration.
    pub buffer: Option<BufferConfig>,
}

/// Streaming mode — determines how the engine processes chunks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamMode {
    /// LLM token stream. Engine concatenates tokens into final text.
    Tokens {
        /// Model producing tokens.
        model: String,
    },
    /// Raw byte chunks (file download, binary generation).
    Bytes {
        /// MIME content type.
        content_type: String,
        /// Total expected size (for progress bars, pre-allocation).
        total_size: Option<u64>,
    },
    /// JSON patches/deltas. Engine applies patches to build final Value.
    Deltas {
        /// Delta format.
        format: DeltaFormat,
    },
    /// Server-Sent Events style: heterogeneous typed events.
    Events,
    /// Custom protocol — engine passes through, downstream interprets.
    Custom {
        /// Protocol identifier.
        protocol: String,
    },
}

/// Delta format for streaming patches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DeltaFormat {
    /// RFC 7396.
    JsonMergePatch,
    /// RFC 6902.
    JsonPatch,
}

/// Current state of a stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamState {
    /// Created but not started.
    Pending,
    /// Actively producing.
    Active {
        /// Number of chunks received so far.
        chunks_received: u64,
    },
    /// Paused (backpressure, rate limit).
    Paused,
    /// Done — all data received.
    Completed,
    /// Error during streaming.
    Failed {
        /// Error description.
        error: String,
    },
}

/// Backpressure configuration for streams.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BufferConfig {
    /// Maximum number of buffered items.
    pub capacity: usize,
    /// What to do when buffer is full.
    pub on_overflow: Overflow,
}

/// Overflow strategy when a stream buffer is full.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Overflow {
    /// Block the producer until space is available.
    Block,
    /// Drop the oldest item in the buffer.
    DropOldest,
    /// Drop the newest (incoming) item.
    DropNewest,
    /// Return an error to the producer.
    Error,
}

/// Who/what is producing the output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Producer {
    /// Kind of producer.
    pub kind: ProducerKind,
    /// Specific name (model name, service name, tool name).
    pub name: Option<String>,
    /// Version.
    pub version: Option<String>,
}

/// Kind of output producer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProducerKind {
    /// AI model (LLM, image gen, etc.).
    AiModel,
    /// External API.
    ExternalApi,
    /// Local computation.
    LocalCompute,
    /// Sub-workflow.
    SubWorkflow,
    /// Human-in-the-loop.
    Human,
    /// Hardware device.
    Device,
}

/// Progress information, updated via heartbeats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// Completion fraction (0.0 to 1.0).
    pub fraction: f64,
    /// Human-readable status.
    pub message: Option<String>,
    /// Estimated time remaining in milliseconds.
    pub eta_ms: Option<u64>,
}

/// Metadata about how an output was produced.
/// Attached to outputs at the engine level via [`OutputEnvelope`], not inside `ActionOutput`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutputMeta {
    /// How this output was produced.
    pub origin: Option<OutputOrigin>,
    /// Timing information.
    pub timing: Option<Timing>,
    /// Cost/resource usage.
    pub cost: Option<Cost>,
    /// Caching information.
    pub cache: Option<CacheInfo>,
    /// Free-form annotations.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub annotations: HashMap<String, serde_json::Value>,
    /// OpenTelemetry trace ID.
    pub trace_id: Option<String>,
}

/// How the output was produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum OutputOrigin {
    /// Computed by action code.
    Computed,
    /// Generated by AI model.
    Ai {
        /// Model identifier.
        model: String,
        /// Provider name.
        provider: String,
    },
    /// Fetched from external source.
    External {
        /// Source identifier.
        source: String,
    },
    /// From cache (previous run).
    Cached {
        /// Original run that produced the cached value.
        original_run: String,
    },
    /// Human-provided.
    Human {
        /// Optional user identifier.
        user_id: Option<String>,
    },
    /// Passthrough from input.
    Passthrough,
}

/// Timing information for output production.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timing {
    /// When production started.
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// When production completed.
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Wall-clock time in milliseconds.
    pub wall_time_ms: Option<u64>,
    /// Queue/wait time (useful for AI API calls).
    pub queue_time_ms: Option<u64>,
}

/// Cost/resource usage for output production.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// Estimated monetary cost in USD cents.
    pub usd_cents: Option<f64>,
    /// LLM token usage.
    pub tokens: Option<TokenUsage>,
}

/// LLM token usage breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input tokens consumed.
    pub input: u64,
    /// Output tokens produced.
    pub output: u64,
    /// Tokens served from cache.
    pub cached: Option<u64>,
}

/// Caching information for an output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CacheInfo {
    /// Output is not cacheable.
    Disabled,
    /// Output can be cached with this key.
    Cacheable {
        /// Cache key.
        key: String,
        /// Cache version.
        version: String,
    },
    /// This output was served from cache.
    Hit {
        /// Cache key.
        key: String,
        /// When the value was cached.
        cached_at: chrono::DateTime<chrono::Utc>,
    },
}

/// Engine-level wrapper that pairs output data with metadata.
///
/// Actions return `ActionOutput<T>`. The engine wraps it in
/// `OutputEnvelope<T>` before persisting and passing downstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputEnvelope<T = serde_json::Value> {
    /// The actual output data.
    pub output: ActionOutput<T>,
    /// Production metadata (origin, cost, timing, cache).
    pub meta: OutputMeta,
}

impl<T> OutputEnvelope<T> {
    /// Wrap an output with default (empty) metadata.
    pub fn new(output: ActionOutput<T>) -> Self {
        Self {
            output,
            meta: OutputMeta::default(),
        }
    }

    /// Wrap with specific metadata.
    pub fn with_meta(output: ActionOutput<T>, meta: OutputMeta) -> Self {
        Self { output, meta }
    }
}

// ── ActionOutput<T> ──────────────────────────────────────────────────────────

/// First-class output type for actions.
///
/// The engine dispatches on this enum to decide how to pass data between
/// nodes. Variants cover immediate data, deferred (lazy) results, and
/// streaming outputs.
///
/// ## Relationship with `ActionResult`
///
/// `ActionResult` controls **workflow flow** (success, skip, branch, wait).
/// `ActionOutput` describes **data and its delivery state**.
///
/// An action can return `ActionResult::Success { output: ActionOutput::Deferred(..) }`
/// meaning: "I successfully initiated generation — here's the handle."
/// The engine resolves the Deferred before passing data to downstream nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[non_exhaustive]
pub enum ActionOutput<T> {
    /// A structured value produced by the action.
    Value(T),
    /// Binary data (files, images, etc.).
    Binary(BinaryData),
    /// A reference to data stored externally.
    Reference(DataReference),
    /// Output that will be resolved asynchronously.
    ///
    /// The action has kicked off work (AI generation, external API call)
    /// but the result isn't ready yet. The engine resolves this before
    /// passing to downstream nodes.
    Deferred(Box<DeferredOutput>),
    /// Output arriving as a stream of chunks.
    ///
    /// The engine can collect into a final value before passing downstream,
    /// or forward the stream if downstream supports streaming.
    Streaming(StreamOutput),
    /// Multiple outputs in one (batch results, fan-out).
    Collection(Vec<ActionOutput<T>>),
    /// No output produced.
    Empty,
}

impl<T> ActionOutput<T> {
    /// Transform the inner value, preserving non-value variants unchanged.
    pub fn map<U>(self, f: &mut impl FnMut(T) -> U) -> ActionOutput<U> {
        match self {
            Self::Value(v) => ActionOutput::Value(f(v)),
            Self::Binary(b) => ActionOutput::Binary(b),
            Self::Reference(r) => ActionOutput::Reference(r),
            Self::Deferred(d) => ActionOutput::Deferred(d),
            Self::Streaming(s) => ActionOutput::Streaming(s),
            Self::Collection(items) => {
                ActionOutput::Collection(items.into_iter().map(|item| item.map(f)).collect())
            }
            Self::Empty => ActionOutput::Empty,
        }
    }

    /// Fallible transform of the inner value.
    pub fn try_map<U, E>(
        self,
        f: &mut impl FnMut(T) -> Result<U, E>,
    ) -> Result<ActionOutput<U>, E> {
        match self {
            Self::Value(v) => Ok(ActionOutput::Value(f(v)?)),
            Self::Binary(b) => Ok(ActionOutput::Binary(b)),
            Self::Reference(r) => Ok(ActionOutput::Reference(r)),
            Self::Deferred(d) => Ok(ActionOutput::Deferred(d)),
            Self::Streaming(s) => Ok(ActionOutput::Streaming(s)),
            Self::Collection(items) => {
                let mapped = items
                    .into_iter()
                    .map(|item| item.try_map(f))
                    .collect::<Result<Vec<_>, E>>()?;
                Ok(ActionOutput::Collection(mapped))
            }
            Self::Empty => Ok(ActionOutput::Empty),
        }
    }

    /// Extract the inner value, returning `None` for non-value variants.
    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Value(v) => Some(v),
            _ => None,
        }
    }

    /// Borrow the inner value, returning `None` for non-value variants.
    pub fn as_value(&self) -> Option<&T> {
        match self {
            Self::Value(v) => Some(v),
            _ => None,
        }
    }

    /// Returns `true` if this is a `Value` variant.
    pub fn is_value(&self) -> bool {
        matches!(self, Self::Value(_))
    }

    /// Returns `true` if this is a `Binary` variant.
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary(_))
    }

    /// Returns `true` if this is a `Reference` variant.
    pub fn is_reference(&self) -> bool {
        matches!(self, Self::Reference(_))
    }

    /// Returns `true` if this is a `Deferred` variant.
    pub fn is_deferred(&self) -> bool {
        matches!(self, Self::Deferred(_))
    }

    /// Returns `true` if this is a `Streaming` variant.
    pub fn is_streaming(&self) -> bool {
        matches!(self, Self::Streaming(_))
    }

    /// Returns `true` if this is a `Collection` variant.
    pub fn is_collection(&self) -> bool {
        matches!(self, Self::Collection(_))
    }

    /// Returns `true` if this is an `Empty` variant.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Returns `true` if the engine must resolve this output
    /// before passing to downstream nodes.
    pub fn needs_resolution(&self) -> bool {
        match self {
            Self::Deferred(_) | Self::Streaming(_) => true,
            Self::Collection(items) => items.iter().any(|o| o.needs_resolution()),
            _ => false,
        }
    }

    // ── Ergonomic constructors ──────────────────────────────────────

    /// Create a deferred output for AI generation (image, audio, video).
    pub fn deferred_ai(
        handle_id: impl Into<String>,
        model: impl Into<String>,
        _provider: impl Into<String>,
        resolution: Resolution,
        expected: ExpectedOutput,
    ) -> Self {
        Self::Deferred(Box::new(DeferredOutput {
            handle_id: handle_id.into(),
            resolution,
            expected,
            progress: None,
            producer: Producer {
                kind: ProducerKind::AiModel,
                name: Some(model.into()),
                version: None,
            },
            retry: Some(crate::metadata::RetryPolicy {
                max_attempts: 3,
                initial_interval: Duration::from_secs(2),
                backoff_coefficient: 2.0,
                max_interval: Some(Duration::from_secs(30)),
                non_retryable_errors: vec!["content_policy_violation".into()],
            }),
            timeout: Some(Duration::from_secs(120)),
        }))
    }

    /// Create a deferred output for document generation (PDF, DOCX, etc.).
    pub fn deferred_document(
        handle_id: impl Into<String>,
        content_type: impl Into<String>,
        resolution: Resolution,
    ) -> Self {
        Self::Deferred(Box::new(DeferredOutput {
            handle_id: handle_id.into(),
            resolution,
            expected: ExpectedOutput::Binary {
                content_type: content_type.into(),
            },
            progress: Some(Progress {
                fraction: 0.0,
                message: Some("Generating document...".into()),
                eta_ms: None,
            }),
            producer: Producer {
                kind: ProducerKind::LocalCompute,
                name: None,
                version: None,
            },
            retry: None,
            timeout: Some(Duration::from_secs(300)),
        }))
    }

    /// Create a deferred output waiting for an external callback.
    pub fn deferred_callback(
        handle_id: impl Into<String>,
        endpoint: impl Into<String>,
        token: impl Into<String>,
        expected: ExpectedOutput,
        timeout: Option<Duration>,
    ) -> Self {
        Self::Deferred(Box::new(DeferredOutput {
            handle_id: handle_id.into(),
            resolution: Resolution::Callback {
                endpoint: endpoint.into(),
                token: token.into(),
            },
            expected,
            progress: None,
            producer: Producer {
                kind: ProducerKind::ExternalApi,
                name: None,
                version: None,
            },
            retry: None,
            timeout,
        }))
    }

    /// Create an LLM token stream output.
    pub fn llm_stream(stream_id: impl Into<String>, model: impl Into<String>) -> Self {
        Self::Streaming(StreamOutput {
            stream_id: stream_id.into(),
            mode: StreamMode::Tokens {
                model: model.into(),
            },
            expected: ExpectedOutput::Value { schema: None },
            state: StreamState::Pending,
            buffer: None,
        })
    }

    /// Create a binary stream (file download, progressive render).
    pub fn byte_stream(
        stream_id: impl Into<String>,
        content_type: impl Into<String>,
        total_size: Option<u64>,
    ) -> Self {
        Self::Streaming(StreamOutput {
            stream_id: stream_id.into(),
            mode: StreamMode::Bytes {
                content_type: content_type.into(),
                total_size,
            },
            expected: ExpectedOutput::Binary {
                content_type: "application/octet-stream".into(),
            },
            state: StreamState::Pending,
            buffer: None,
        })
    }
}

/// Binary data carried inline or stored externally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryData {
    /// MIME content type (e.g. `"image/png"`, `"application/pdf"`).
    pub content_type: String,
    /// Where the bytes live.
    pub data: BinaryStorage,
    /// Total size in bytes.
    pub size: u64,
    /// Optional metadata (e.g. filename, dimensions).
    pub metadata: Option<serde_json::Value>,
}

/// Storage location for binary data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BinaryStorage {
    /// Bytes carried inline (small payloads).
    Inline(Vec<u8>),
    /// Bytes stored externally.
    Stored {
        /// Backend identifier (e.g. `"s3"`, `"local"`).
        storage_type: String,
        /// Path or key within the storage backend.
        path: String,
        /// Optional integrity checksum (e.g. SHA-256 hex).
        checksum: Option<String>,
    },
}

/// A reference to data stored externally (not fetched yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataReference {
    /// Backend identifier (e.g. `"s3"`, `"local"`, `"database"`).
    pub storage_type: String,
    /// Path or key within the storage backend.
    pub path: String,
    /// Size in bytes (if known).
    pub size: Option<u64>,
    /// MIME content type (if known).
    pub content_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_output_value() {
        let out = ActionOutput::Value(42);
        assert!(out.is_value());
        assert!(!out.is_binary());
        assert!(!out.is_reference());
        assert!(!out.is_deferred());
        assert!(!out.is_streaming());
        assert!(!out.is_collection());
        assert!(!out.is_empty());
        assert_eq!(out.as_value(), Some(&42));
    }

    #[test]
    fn action_output_binary() {
        let out: ActionOutput<i32> = ActionOutput::Binary(BinaryData {
            content_type: "image/png".into(),
            data: BinaryStorage::Inline(vec![0x89, 0x50, 0x4E, 0x47]),
            size: 4,
            metadata: None,
        });
        assert!(out.is_binary());
        assert!(!out.is_value());
        assert_eq!(out.as_value(), None);
    }

    #[test]
    fn action_output_reference() {
        let out: ActionOutput<i32> = ActionOutput::Reference(DataReference {
            storage_type: "s3".into(),
            path: "bucket/key".into(),
            size: Some(1024),
            content_type: Some("application/json".into()),
        });
        assert!(out.is_reference());
    }

    #[test]
    fn action_output_deferred() {
        let out: ActionOutput<i32> = ActionOutput::Deferred(Box::new(DeferredOutput {
            handle_id: "handle-1".into(),
            resolution: Resolution::Await {
                channel_id: "ch-1".into(),
            },
            expected: ExpectedOutput::Value { schema: None },
            progress: None,
            producer: Producer {
                kind: ProducerKind::AiModel,
                name: Some("gpt-4".into()),
                version: None,
            },
            retry: None,
            timeout: Some(Duration::from_secs(60)),
        }));
        assert!(out.is_deferred());
        assert!(!out.is_value());
        assert!(!out.is_streaming());
        assert!(out.needs_resolution());
    }

    #[test]
    fn action_output_streaming() {
        let out: ActionOutput<i32> = ActionOutput::Streaming(StreamOutput {
            stream_id: "stream-1".into(),
            mode: StreamMode::Tokens {
                model: "claude".into(),
            },
            expected: ExpectedOutput::Value { schema: None },
            state: StreamState::Pending,
            buffer: None,
        });
        assert!(out.is_streaming());
        assert!(!out.is_deferred());
        assert!(out.needs_resolution());
    }

    #[test]
    fn action_output_collection() {
        let out: ActionOutput<i32> = ActionOutput::Collection(vec![
            ActionOutput::Value(1),
            ActionOutput::Value(2),
            ActionOutput::Empty,
        ]);
        assert!(out.is_collection());
        assert!(!out.is_value());
        assert!(!out.needs_resolution());
    }

    #[test]
    fn action_output_collection_with_deferred() {
        let out: ActionOutput<i32> = ActionOutput::Collection(vec![
            ActionOutput::Value(1),
            ActionOutput::Deferred(Box::new(DeferredOutput {
                handle_id: "h".into(),
                resolution: Resolution::Await {
                    channel_id: "ch".into(),
                },
                expected: ExpectedOutput::Dynamic,
                progress: None,
                producer: Producer {
                    kind: ProducerKind::ExternalApi,
                    name: None,
                    version: None,
                },
                retry: None,
                timeout: None,
            })),
        ]);
        assert!(out.needs_resolution());
    }

    #[test]
    fn action_output_empty() {
        let out: ActionOutput<i32> = ActionOutput::Empty;
        assert!(out.is_empty());
        assert_eq!(out.into_value(), None);
    }

    #[test]
    fn action_output_map() {
        let out = ActionOutput::Value(5);
        let mapped = out.map(&mut |n| n * 2);
        assert_eq!(mapped.into_value(), Some(10));
    }

    #[test]
    fn action_output_map_preserves_binary() {
        let out: ActionOutput<i32> = ActionOutput::Binary(BinaryData {
            content_type: "text/plain".into(),
            data: BinaryStorage::Inline(vec![]),
            size: 0,
            metadata: None,
        });
        let mapped: ActionOutput<String> = out.map(&mut |n| n.to_string());
        assert!(mapped.is_binary());
    }

    #[test]
    fn action_output_map_collection() {
        let out: ActionOutput<i32> = ActionOutput::Collection(vec![
            ActionOutput::Value(1),
            ActionOutput::Value(2),
            ActionOutput::Empty,
        ]);
        let mapped = out.map(&mut |n| n * 10);
        match mapped {
            ActionOutput::Collection(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].as_value(), Some(&10));
                assert_eq!(items[1].as_value(), Some(&20));
                assert!(items[2].is_empty());
            }
            _ => panic!("expected Collection"),
        }
    }

    #[test]
    fn action_output_try_map_ok() {
        let out = ActionOutput::Value(5);
        let mapped = out.try_map(&mut |n| Ok::<_, String>(n * 2));
        assert_eq!(mapped.unwrap().into_value(), Some(10));
    }

    #[test]
    fn action_output_try_map_err() {
        let out = ActionOutput::Value(5);
        let mapped = out.try_map(&mut |_| Err::<i32, _>("fail"));
        assert_eq!(mapped.unwrap_err(), "fail");
    }

    #[test]
    fn action_output_try_map_non_value() {
        let out: ActionOutput<i32> = ActionOutput::Empty;
        let mapped = out.try_map(&mut |_| Err::<i32, _>("should not be called"));
        assert!(mapped.unwrap().is_empty());
    }

    #[test]
    fn action_output_try_map_collection() {
        let out: ActionOutput<i32> =
            ActionOutput::Collection(vec![ActionOutput::Value(1), ActionOutput::Value(2)]);
        let mapped = out.try_map(&mut |n| Ok::<_, String>(n * 3));
        match mapped.unwrap() {
            ActionOutput::Collection(items) => {
                assert_eq!(items[0].as_value(), Some(&3));
                assert_eq!(items[1].as_value(), Some(&6));
            }
            _ => panic!("expected Collection"),
        }
    }

    #[test]
    fn action_output_try_map_collection_err() {
        let out: ActionOutput<i32> =
            ActionOutput::Collection(vec![ActionOutput::Value(1), ActionOutput::Value(2)]);
        let mapped = out.try_map(&mut |n| {
            if n == 2 { Err("bad") } else { Ok(n) }
        });
        assert_eq!(mapped.unwrap_err(), "bad");
    }

    #[test]
    fn action_output_into_value() {
        assert_eq!(ActionOutput::Value(42).into_value(), Some(42));
        assert_eq!(ActionOutput::<i32>::Empty.into_value(), None);
    }

    #[test]
    fn needs_resolution_value() {
        assert!(!ActionOutput::Value(42).needs_resolution());
    }

    #[test]
    fn needs_resolution_binary() {
        let out: ActionOutput<i32> = ActionOutput::Binary(BinaryData {
            content_type: "x".into(),
            data: BinaryStorage::Inline(vec![]),
            size: 0,
            metadata: None,
        });
        assert!(!out.needs_resolution());
    }

    #[test]
    fn needs_resolution_empty() {
        assert!(!ActionOutput::<i32>::Empty.needs_resolution());
    }

    // ── Ergonomic constructor tests ─────────────────────────────────

    #[test]
    fn deferred_ai_constructor() {
        let out = ActionOutput::<serde_json::Value>::deferred_ai(
            "gen-img-123",
            "dall-e-3",
            "openai",
            Resolution::Poll {
                target: PollTarget::Http {
                    url: "https://api.example.com/status".into(),
                    method: "GET".into(),
                },
                interval: Duration::from_secs(2),
                backoff: 1.5,
                max_interval: Some(Duration::from_secs(15)),
            },
            ExpectedOutput::Binary {
                content_type: "image/png".into(),
            },
        );
        assert!(out.is_deferred());
        assert!(out.needs_resolution());
        match &out {
            ActionOutput::Deferred(d) => {
                assert_eq!(d.handle_id, "gen-img-123");
                assert_eq!(d.producer.kind, ProducerKind::AiModel);
                assert_eq!(d.producer.name.as_deref(), Some("dall-e-3"));
                assert!(d.retry.is_some());
                assert!(d.timeout.is_some());
            }
            _ => panic!("expected Deferred"),
        }
    }

    #[test]
    fn deferred_document_constructor() {
        let out = ActionOutput::<serde_json::Value>::deferred_document(
            "doc-456",
            "application/pdf",
            Resolution::Await {
                channel_id: "ch-doc".into(),
            },
        );
        match &out {
            ActionOutput::Deferred(d) => {
                assert_eq!(d.handle_id, "doc-456");
                assert_eq!(d.producer.kind, ProducerKind::LocalCompute);
                assert!(d.progress.is_some());
                assert_eq!(d.timeout, Some(Duration::from_secs(300)));
            }
            _ => panic!("expected Deferred"),
        }
    }

    #[test]
    fn deferred_callback_constructor() {
        let out = ActionOutput::<serde_json::Value>::deferred_callback(
            "cb-789",
            "https://hooks.example.com/callback",
            "tok-abc",
            ExpectedOutput::Value { schema: None },
            Some(Duration::from_secs(3600)),
        );
        match &out {
            ActionOutput::Deferred(d) => {
                assert_eq!(d.handle_id, "cb-789");
                assert!(matches!(d.resolution, Resolution::Callback { .. }));
                assert_eq!(d.producer.kind, ProducerKind::ExternalApi);
                assert_eq!(d.timeout, Some(Duration::from_secs(3600)));
            }
            _ => panic!("expected Deferred"),
        }
    }

    #[test]
    fn llm_stream_constructor() {
        let out =
            ActionOutput::<serde_json::Value>::llm_stream("stream-1", "claude-sonnet-4-20250514");
        assert!(out.is_streaming());
        assert!(out.needs_resolution());
        match &out {
            ActionOutput::Streaming(s) => {
                assert_eq!(s.stream_id, "stream-1");
                assert!(matches!(s.mode, StreamMode::Tokens { .. }));
                assert_eq!(s.state, StreamState::Pending);
            }
            _ => panic!("expected Streaming"),
        }
    }

    #[test]
    fn byte_stream_constructor() {
        let out =
            ActionOutput::<serde_json::Value>::byte_stream("bs-1", "video/mp4", Some(1_000_000));
        match &out {
            ActionOutput::Streaming(s) => {
                assert_eq!(s.stream_id, "bs-1");
                match &s.mode {
                    StreamMode::Bytes {
                        content_type,
                        total_size,
                    } => {
                        assert_eq!(content_type, "video/mp4");
                        assert_eq!(*total_size, Some(1_000_000));
                    }
                    _ => panic!("expected Bytes mode"),
                }
            }
            _ => panic!("expected Streaming"),
        }
    }

    // ── OutputEnvelope tests ────────────────────────────────────────

    #[test]
    fn output_envelope_new() {
        let envelope = OutputEnvelope::new(ActionOutput::Value(42));
        assert_eq!(envelope.output.as_value(), Some(&42));
        assert!(envelope.meta.origin.is_none());
        assert!(envelope.meta.timing.is_none());
    }

    #[test]
    fn output_envelope_with_meta() {
        let meta = OutputMeta {
            origin: Some(OutputOrigin::Computed),
            trace_id: Some("trace-1".into()),
            ..Default::default()
        };
        let envelope = OutputEnvelope::with_meta(ActionOutput::Value("data"), meta);
        assert!(matches!(envelope.meta.origin, Some(OutputOrigin::Computed)));
        assert_eq!(envelope.meta.trace_id.as_deref(), Some("trace-1"));
    }

    // ── Serde round-trip tests ──────────────────────────────────────

    #[test]
    fn serde_deferred_output_roundtrip() {
        let deferred = DeferredOutput {
            handle_id: "h-1".into(),
            resolution: Resolution::Poll {
                target: PollTarget::Http {
                    url: "https://api.test/status".into(),
                    method: "GET".into(),
                },
                interval: Duration::from_secs(5),
                backoff: 2.0,
                max_interval: Some(Duration::from_secs(60)),
            },
            expected: ExpectedOutput::Binary {
                content_type: "image/png".into(),
            },
            progress: Some(Progress {
                fraction: 0.5,
                message: Some("Half done".into()),
                eta_ms: Some(30_000),
            }),
            producer: Producer {
                kind: ProducerKind::AiModel,
                name: Some("dall-e-3".into()),
                version: Some("v1".into()),
            },
            retry: None,
            timeout: Some(Duration::from_secs(120)),
        };

        let json = serde_json::to_string(&deferred).unwrap();
        let back: DeferredOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(deferred, back);
    }

    #[test]
    fn serde_stream_output_roundtrip() {
        let stream = StreamOutput {
            stream_id: "s-1".into(),
            mode: StreamMode::Tokens {
                model: "claude".into(),
            },
            expected: ExpectedOutput::Value { schema: None },
            state: StreamState::Active {
                chunks_received: 42,
            },
            buffer: Some(BufferConfig {
                capacity: 100,
                on_overflow: Overflow::DropOldest,
            }),
        };

        let json = serde_json::to_string(&stream).unwrap();
        let back: StreamOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(stream, back);
    }

    #[test]
    fn serde_resolution_variants() {
        let variants: Vec<Resolution> = vec![
            Resolution::Poll {
                target: PollTarget::Service {
                    name: "svc".into(),
                    operation: "check".into(),
                },
                interval: Duration::from_secs(1),
                backoff: 1.0,
                max_interval: None,
            },
            Resolution::Await {
                channel_id: "ch".into(),
            },
            Resolution::Callback {
                endpoint: "https://example.com".into(),
                token: "tok".into(),
            },
            Resolution::SubWorkflow {
                workflow_id: "wf-1".into(),
                input: Some(serde_json::json!({"key": "value"})),
            },
            Resolution::AwaitOrPoll {
                channel_id: "ch-2".into(),
                fallback_after: Duration::from_secs(10),
                poll_target: PollTarget::Action {
                    action_key: "check_status".into(),
                },
                poll_interval: Duration::from_secs(5),
            },
        ];

        for variant in &variants {
            let json = serde_json::to_string(variant).unwrap();
            let back: Resolution = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, &back);
        }
    }

    #[test]
    fn serde_action_output_deferred_roundtrip() {
        let out: ActionOutput<serde_json::Value> =
            ActionOutput::Deferred(Box::new(DeferredOutput {
                handle_id: "test".into(),
                resolution: Resolution::Await {
                    channel_id: "ch".into(),
                },
                expected: ExpectedOutput::Dynamic,
                progress: None,
                producer: Producer {
                    kind: ProducerKind::Human,
                    name: None,
                    version: None,
                },
                retry: None,
                timeout: None,
            }));

        let json = serde_json::to_string(&out).unwrap();
        let back: ActionOutput<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(out, back);
    }
}
