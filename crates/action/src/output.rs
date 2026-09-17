use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};

// ── Supporting types ────────────────────────────────────────────────────────

/// Minimal retry config for deferred output resolution.
///
/// All fields are public for ergonomic construction. Call
/// [`DeferredRetryConfig::validate`] before handing the config to
/// a resolver to catch malformed values (NaN / non-finite /
/// non-positive coefficient). The resolver uses
/// `Duration::mul_f64(backoff_coefficient)`, which panics on NaN
/// or overflow — the validator is the only thing between you and
/// that panic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeferredRetryConfig {
    /// Maximum number of attempts.
    pub max_attempts: u32,
    /// Initial delay between retries.
    pub initial_interval: Duration,
    /// Backoff multiplier. MUST be finite and strictly positive —
    /// validator rejects `NaN`, `±inf`, and values ≤ 0.
    pub backoff_coefficient: f64,
    /// Upper bound on delay.
    pub max_interval: Option<Duration>,
    /// Error type names that should NOT be retried.
    pub non_retryable_errors: Vec<String>,
}

impl DeferredRetryConfig {
    /// Validate that the config is usable by a resolver.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ActionError::Validation`] if:
    /// - `max_attempts` is 0
    /// - `initial_interval` is [`Duration::ZERO`]
    /// - `backoff_coefficient` is `NaN`, `±inf`, or ≤ 0
    ///
    /// The resolver uses `Duration::mul_f64(backoff_coefficient)` to
    /// scale retry intervals, and that function panics on NaN or
    /// overflow. Validation catches those cases at config-build time
    /// instead of at retry time. A zero `initial_interval` would fire
    /// the first retry with no delay and — paired with any finite
    /// `backoff_coefficient` — stays at zero forever, busy-looping the
    /// resolver.
    pub fn validate(&self) -> Result<(), crate::ActionError> {
        use crate::error::ValidationReason;

        if self.max_attempts == 0 {
            return Err(crate::ActionError::validation(
                "deferred_retry.max_attempts",
                ValidationReason::OutOfRange,
                Some("max_attempts must be >= 1"),
            ));
        }
        if self.initial_interval.is_zero() {
            return Err(crate::ActionError::validation(
                "deferred_retry.initial_interval",
                ValidationReason::OutOfRange,
                Some("initial_interval must be > 0"),
            ));
        }
        if !self.backoff_coefficient.is_finite() || self.backoff_coefficient <= 0.0 {
            return Err(crate::ActionError::validation(
                "deferred_retry.backoff_coefficient",
                ValidationReason::OutOfRange,
                Some(format!(
                    "backoff_coefficient must be finite and > 0, got {}",
                    self.backoff_coefficient
                )),
            ));
        }
        Ok(())
    }
}

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
    /// Retry config if a resolution fails.
    pub retry: Option<DeferredRetryConfig>,
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
    /// Unknown at compile time.
    Dynamic,
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
///
/// Intentionally does not derive `Eq`/`Hash`: [`Progress::fraction`] is
/// `f64` and NaN breaks reflexivity. Use [`Progress::fraction`] directly
/// for comparisons.
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
///
/// Intentionally does not derive `Eq`/`Hash`: [`Cost::usd_cents`] is `f64`
/// and NaN breaks reflexivity. Compare token counts via [`Cost::tokens`]
/// if deterministic equality is required.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// Estimated monetary cost in USD cents.
    pub usd_cents: Option<f64>,
    /// LLM token usage.
    pub tokens: Option<TokenUsage>,
}

/// LLM token usage breakdown.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input tokens consumed.
    pub input: u64,
    /// Output tokens produced.
    pub output: u64,
    /// Tokens served from cache.
    pub cached: Option<u64>,
}

/// Caching information for an output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
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
/// nodes. Variants cover immediate data, deferred (lazy) results, binary
/// payloads, external references, collections, and empty outputs.
///
/// ## Relationship with `ActionResult`
///
/// `ActionResult` controls **workflow flow** (success, skip, branch, wait).
/// `ActionOutput` describes **data and its delivery state**.
///
/// An action can return `ActionResult::Success { output: ActionOutput::Deferred(..) }`
/// meaning: "I successfully initiated generation — here's the handle."
/// The engine resolves the Deferred before passing data to downstream nodes.
///
/// ## Stream kind
///
/// Stream actions fold their chunk stream into a single
/// `ActionOutput::Value` before returning; there is no inline streaming
/// variant on this enum. The fold happens inside the stream adapter and the
/// engine receives a plain `Value`.
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
            Self::Collection(items) => {
                ActionOutput::Collection(items.into_iter().map(|item| item.map(f)).collect())
            },
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
            Self::Collection(items) => {
                let mapped = items
                    .into_iter()
                    .map(|item| item.try_map(f))
                    .collect::<Result<Vec<_>, E>>()?;
                Ok(ActionOutput::Collection(mapped))
            },
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
            Self::Deferred(_) => true,
            Self::Collection(items) => items.iter().any(ActionOutput::needs_resolution),
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
            retry: Some(DeferredRetryConfig {
                max_attempts: 3,
                initial_interval: Duration::from_secs(2),
                backoff_coefficient: 2.0,
                max_interval: Some(Duration::from_secs(30)),
                non_retryable_errors: vec!["content_policy_violation".into()],
            }),
            timeout: Some(Duration::from_mins(2)),
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
            timeout: Some(Duration::from_mins(5)),
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
}

/// Binary data carried inline or stored externally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryData {
    /// MIME content type (e.g. `"image/png"`, `"application/pdf"`).
    pub content_type: String,
    /// Where the bytes live.
    pub data: BinaryStorage,
    /// Advertised size in bytes.
    ///
    /// For `BinaryStorage::Stored`, this is the authoritative size
    /// (the bytes are not in memory to measure).
    ///
    /// For `BinaryStorage::Inline`, this SHOULD equal `bytes.len()`
    /// — but the field is public and can be set out of sync.
    /// Consumers that need an authoritative answer must use
    /// [`BinaryData::effective_size`], which returns the actual
    /// inline byte length for `Inline` and the advertised `size`
    /// for `Stored`. Size-limit checks MUST use `effective_size()`
    /// or they can be bypassed by passing oversize inline bytes
    /// with a falsified `size`.
    pub size: u64,
    /// Optional metadata (e.g. filename, dimensions).
    pub metadata: Option<serde_json::Value>,
}

impl BinaryData {
    /// Authoritative size in bytes.
    ///
    /// Returns the actual inline byte count for
    /// [`BinaryStorage::Inline`] and the advertised `size` field for
    /// [`BinaryStorage::Stored`]. Prefer this over reading `size`
    /// directly when enforcing size limits — the raw field can be
    /// out of sync with inline contents.
    #[must_use]
    pub fn effective_size(&self) -> u64 {
        match &self.data {
            BinaryStorage::Inline(bytes) => bytes.len() as u64,
            BinaryStorage::Stored { .. } => self.size,
        }
    }
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[path = "output_tests.rs"]
mod tests;
