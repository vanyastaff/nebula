//! Execution trace: resource usage and optional per-call enrichment.
//!
//! **Tier 1** — [`ResourceUsageRecord`]: produced automatically by the resource
//! layer (e.g. instrumented guard) on drop. No author effort required.
//!
//! **Tier 2** — [`CallRecord`]: optional enrichment (request/response, operation name)
//! that resource (or action) authors can record via [`Recorder::record_call`].
//!
//! The engine injects a [`Recorder`] via context; the same sink can be used for
//! resource usage, action spans, and other execution trace data.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nebula_core::ResourceKey;

use crate::context::TraceContext;

// ---------------------------------------------------------------------------
// DropReason
// ---------------------------------------------------------------------------

/// How a resource guard was released. Determined automatically by the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropReason {
    /// Normal drop — instance returned to pool.
    Released,
    /// Dropped during a panic (`std::thread::panicking()` was true).
    Panic,
    /// Explicitly taken out of guard via `into_inner()`.
    Detached,
}

// ---------------------------------------------------------------------------
// ResourceUsageRecord — Tier 1 (automatic)
// ---------------------------------------------------------------------------

/// Tier 1: Automatic usage record. Produced for every acquired resource when
/// the guard is dropped.
#[derive(Debug, Clone)]
pub struct ResourceUsageRecord {
    /// Resource key (e.g. `http_client`, `postgres`).
    pub resource_key: ResourceKey,
    /// When the instance was acquired from the pool.
    pub acquired_at: Instant,
    /// Time spent waiting for an available instance (pool contention).
    pub wait_duration: Duration,
    /// Time the instance was held (from acquire until drop).
    pub hold_duration: Duration,
    /// How the guard was released.
    pub drop_reason: DropReason,
}

// ---------------------------------------------------------------------------
// CallRecord — Tier 2 (optional enrichment)
// ---------------------------------------------------------------------------

/// Tier 2: Optional per-call enrichment. Created by instance methods that call
/// [`Recorder::record_call`].
#[derive(Debug, Clone)]
pub struct CallRecord {
    /// Resource key.
    pub resource_key: ResourceKey,
    /// Human-readable operation (e.g. `"GET /users"`, `"sendMessage"`, `"SELECT ..."`).
    pub operation: String,
    /// When the call started.
    pub started_at: Instant,
    /// Call duration.
    pub duration: Duration,
    /// Success or error message.
    pub status: CallStatus,
    /// What was sent (optional).
    pub request: Option<CallPayload>,
    /// What was received (optional).
    pub response: Option<CallPayload>,
    /// Extra key-value (e.g. `status_code`, `chat_id`, `row_count`).
    pub metadata: HashMap<String, String>,
    /// Optional trace context for correlating this call with a distributed trace.
    pub trace_context: Option<TraceContext>,
}

/// Success or error outcome of a call.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CallStatus {
    /// Call succeeded.
    Success,
    /// Call failed with message.
    Error(String),
}

/// Payload of a call — generic for HTTP, DB, messaging, etc.
#[derive(Debug, Clone, Default)]
pub struct CallPayload {
    /// Short summary (e.g. `"GET https://api.example.com/users"`).
    pub summary: String,
    /// Headers or options (HTTP headers, SSH options, etc.).
    pub headers: Option<Vec<(String, String)>>,
    /// Body or command text.
    pub body: Option<CallBody>,
    /// Size in bytes if known.
    pub size_bytes: Option<u64>,
}

/// Body content — text, binary, or redacted (secrets).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CallBody {
    /// Plain text (JSON, SQL, command).
    Text(String),
    /// Binary with MIME and size.
    Binary {
        /// MIME type or label.
        mime: String,
        /// Size in bytes.
        size: u64,
    },
    /// Contains secrets — intentionally not logged.
    Redacted,
}

// ---------------------------------------------------------------------------
// Recorder trait
// ---------------------------------------------------------------------------

/// Sink for execution trace: usage and call records. Engine injects a real
/// implementation; tests use [`NoopRecorder`].
pub trait Recorder: Send + Sync {
    /// Tier 1: called automatically when a resource guard is dropped.
    fn record_usage(&self, record: ResourceUsageRecord);

    /// Tier 2: called optionally by instance methods for richer execution view.
    fn record_call(&self, record: CallRecord);

    /// Whether enrichment recording is enabled. Instance code can skip
    /// building [`CallRecord`] when this is false.
    fn is_enrichment_enabled(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// NoopRecorder
// ---------------------------------------------------------------------------

/// No-op recorder for tests and non-instrumented contexts.
#[derive(Debug, Clone, Default)]
pub struct NoopRecorder;

impl Recorder for NoopRecorder {
    fn record_usage(&self, _record: ResourceUsageRecord) {}

    fn record_call(&self, _record: CallRecord) {}

    fn is_enrichment_enabled(&self) -> bool {
        false
    }
}
