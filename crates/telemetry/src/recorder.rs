//! Execution recording: resource usage, per-call enrichment, and buffered sink.
//!
//! ## Recording tiers
//!
//! **Tier 1** — [`ResourceUsageRecord`]: produced automatically by the resource
//! layer (e.g. instrumented guard) on drop. No author effort required.
//!
//! **Tier 2** — [`CallRecord`]: optional enrichment (request/response, operation name)
//! that resource (or action) authors can record via [`Recorder::record_call`].
//!
//! ## Buffered recording
//!
//! [`BufferedRecorder`] collects records via a non-blocking MPSC channel and
//! flushes them in batches to a pluggable [`RecordSink`]. This avoids blocking
//! the hot path while ensuring records are eventually persisted or exported.
//!
//! ## Quick Start
//!
//! ```no_run
//! use nebula_telemetry::recorder::{BufferedRecorder, BufferedRecorderConfig, LogSink};
//!
//! # async fn example() {
//! let recorder = BufferedRecorder::start(
//!     BufferedRecorderConfig::default(),
//!     LogSink,
//! );
//! // Use as `Arc<dyn Recorder>` — records are buffered and flushed asynchronously.
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nebula_core::ResourceKey;
use tokio::sync::mpsc;
use tracing;

use crate::trace::TraceContext;

// ── DropReason ───────────────────────────────────────────────────────────────

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

// ── ResourceUsageRecord — Tier 1 (automatic) ────────────────────────────────

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

// ── CallRecord — Tier 2 (optional enrichment) ───────────────────────────────

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

// ── Recorder trait ───────────────────────────────────────────────────────────

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

// ── NoopRecorder ─────────────────────────────────────────────────────────────

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

// ── RecordEntry ──────────────────────────────────────────────────────────────

/// A single buffered entry — either a usage record or a call record.
#[derive(Debug)]
#[non_exhaustive]
pub enum RecordEntry {
    /// Tier 1: automatic resource usage.
    Usage(ResourceUsageRecord),
    /// Tier 2: optional per-call enrichment (boxed to reduce enum size).
    Call(Box<CallRecord>),
}

// ── RecordSink trait ─────────────────────────────────────────────────────────

/// Pluggable flush destination for buffered records.
///
/// Implementors receive batches of records and persist or export them.
/// The flush method is async to support I/O-bound sinks (database, HTTP).
#[async_trait::async_trait]
pub trait RecordSink: Send + Sync + 'static {
    /// Flush a batch of records to the sink.
    ///
    /// Called periodically by the background task when records are available.
    /// Implementations should handle errors internally (log and continue).
    async fn flush(&self, records: Vec<RecordEntry>);
}

// ── LogSink ──────────────────────────────────────────────────────────────────

/// Default sink that writes records to `tracing::info!`.
///
/// Useful for development, debugging, and as a reference implementation.
#[derive(Debug, Clone, Copy)]
pub struct LogSink;

#[async_trait::async_trait]
impl RecordSink for LogSink {
    async fn flush(&self, records: Vec<RecordEntry>) {
        for entry in &records {
            match entry {
                RecordEntry::Usage(r) => {
                    tracing::info!(
                        resource = %r.resource_key,
                        wait_ms = r.wait_duration.as_millis() as u64,
                        hold_ms = r.hold_duration.as_millis() as u64,
                        drop_reason = ?r.drop_reason,
                        "resource usage recorded"
                    );
                }
                RecordEntry::Call(r) => {
                    tracing::info!(
                        resource = %r.resource_key,
                        operation = %r.operation,
                        duration_ms = r.duration.as_millis() as u64,
                        status = ?r.status,
                        "call recorded"
                    );
                }
            }
        }
    }
}

// ── BufferedRecorderConfig ───────────────────────────────────────────────────

/// Configuration for [`BufferedRecorder`].
#[derive(Debug, Clone)]
pub struct BufferedRecorderConfig {
    /// Maximum number of records buffered before a forced flush.
    ///
    /// Default: `1024`.
    pub buffer_size: usize,
    /// Interval between periodic flushes.
    ///
    /// Default: `5 seconds`.
    pub flush_interval: Duration,
}

impl Default for BufferedRecorderConfig {
    fn default() -> Self {
        Self {
            buffer_size: 1024,
            flush_interval: Duration::from_secs(5),
        }
    }
}

// ── BufferedRecorder ─────────────────────────────────────────────────────────

/// Buffered execution recorder that batches records and flushes asynchronously.
///
/// Records are submitted via the [`Recorder`] trait methods (`record_usage`,
/// `record_call`) using a non-blocking channel send. A background tokio task
/// collects entries and flushes them to a [`RecordSink`] when either the
/// buffer fills or the flush interval elapses.
///
/// On channel full, records are dropped with a warning log (back-pressure).
pub struct BufferedRecorder {
    sender: mpsc::Sender<RecordEntry>,
    shutdown: Option<ShutdownHandle>,
}

/// Handle for draining remaining records on shutdown.
struct ShutdownHandle {
    handle: tokio::task::JoinHandle<()>,
}

impl BufferedRecorder {
    /// Start the buffered recorder with the given config and sink.
    ///
    /// Spawns a background tokio task for batching and flushing.
    pub fn start<S: RecordSink>(config: BufferedRecorderConfig, sink: S) -> Self {
        let (tx, rx) = mpsc::channel(config.buffer_size);
        let sink = Arc::new(sink);

        tracing::debug!(
            buffer_size = config.buffer_size,
            flush_interval_ms = config.flush_interval.as_millis() as u64,
            "buffered recorder started"
        );

        let handle = tokio::spawn(flush_loop(rx, sink, config));

        Self {
            sender: tx,
            shutdown: Some(ShutdownHandle { handle }),
        }
    }

    /// Shut down the recorder: close the channel and drain remaining records.
    ///
    /// Returns after the background task has flushed all remaining entries.
    pub async fn shutdown(mut self) {
        // Drop sender to signal the background task to stop
        drop(self.sender.clone());
        // The real sender is still held — we need to take ownership
        if let Some(handle) = self.shutdown.take() {
            // Drop the sender field to close the channel
            drop(std::mem::replace(&mut self.sender, mpsc::channel(1).0));
            match handle.handle.await {
                Ok(()) => {
                    tracing::info!("buffered recorder shutdown complete");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "buffered recorder task panicked during shutdown");
                }
            }
        }
    }
}

impl Recorder for BufferedRecorder {
    fn record_usage(&self, record: ResourceUsageRecord) {
        if self.sender.try_send(RecordEntry::Usage(record)).is_err() {
            tracing::warn!("buffered recorder channel full, dropping usage record");
        }
    }

    fn record_call(&self, record: CallRecord) {
        if self
            .sender
            .try_send(RecordEntry::Call(Box::new(record)))
            .is_err()
        {
            tracing::warn!("buffered recorder channel full, dropping call record");
        }
    }
}

// ── Background flush loop ────────────────────────────────────────────────────

async fn flush_loop(
    mut rx: mpsc::Receiver<RecordEntry>,
    sink: Arc<dyn RecordSink>,
    config: BufferedRecorderConfig,
) {
    let mut buffer: Vec<RecordEntry> = Vec::with_capacity(config.buffer_size);
    let mut interval = tokio::time::interval(config.flush_interval);
    // First tick completes immediately — skip it
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !buffer.is_empty() {
                    let batch = std::mem::replace(
                        &mut buffer,
                        Vec::with_capacity(config.buffer_size),
                    );
                    tracing::debug!(count = batch.len(), "flushing buffered records (interval)");
                    sink.flush(batch).await;
                }
            }
            entry = rx.recv() => {
                match entry {
                    Some(record) => {
                        buffer.push(record);
                        if buffer.len() >= config.buffer_size {
                            let batch = std::mem::replace(
                                &mut buffer,
                                Vec::with_capacity(config.buffer_size),
                            );
                            tracing::debug!(count = batch.len(), "flushing buffered records (buffer full)");
                            sink.flush(batch).await;
                        }
                    }
                    None => {
                        // Channel closed — drain remaining and exit
                        if !buffer.is_empty() {
                            let count = buffer.len();
                            tracing::debug!(count, "flushing remaining buffered records on shutdown");
                            sink.flush(buffer).await;
                        }
                        return;
                    }
                }
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nebula_core::resource_key;

    use super::*;

    // ─────────────────────────────────────────────────────────────────────────
    // Recording types
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn call_record_with_empty_payloads() {
        let record = CallRecord {
            resource_key: resource_key!("test_resource"),
            operation: "test_op".to_string(),
            started_at: Instant::now(),
            duration: Duration::from_millis(100),
            status: CallStatus::Success,
            request: None,
            response: None,
            metadata: HashMap::new(),
            trace_context: None,
        };
        assert_eq!(record.operation, "test_op");
        assert!(record.request.is_none());
        assert!(record.response.is_none());
    }

    #[test]
    fn call_payload_redacted() {
        let payload = CallPayload {
            summary: "GET /secrets".to_string(),
            headers: None,
            body: Some(CallBody::Redacted),
            size_bytes: Some(1024),
        };
        assert!(matches!(payload.body, Some(CallBody::Redacted)));
    }

    #[test]
    fn call_payload_oversized_content() {
        let large_text = "x".repeat(65536);
        let payload = CallPayload {
            summary: "large_payload".to_string(),
            headers: None,
            body: Some(CallBody::Text(large_text.clone())),
            size_bytes: Some(65536),
        };
        if let Some(CallBody::Text(body)) = payload.body {
            assert_eq!(body.len(), 65536);
        }
    }

    #[test]
    fn call_status_error_with_empty_message() {
        let status = CallStatus::Error("".to_string());
        match status {
            CallStatus::Error(msg) => assert_eq!(msg, ""),
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn call_record_zero_duration() {
        let record = CallRecord {
            resource_key: resource_key!("zero_duration"),
            operation: "instant_op".to_string(),
            started_at: Instant::now(),
            duration: Duration::ZERO,
            status: CallStatus::Success,
            request: None,
            response: None,
            metadata: HashMap::new(),
            trace_context: None,
        };
        assert_eq!(record.duration, Duration::ZERO);
    }

    #[test]
    fn resource_usage_record_max_values() {
        let record = ResourceUsageRecord {
            resource_key: resource_key!("max_test"),
            acquired_at: Instant::now(),
            wait_duration: Duration::new(u64::MAX, 999_999_999),
            hold_duration: Duration::new(u64::MAX, 999_999_999),
            drop_reason: DropReason::Released,
        };
        assert_eq!(record.wait_duration.as_secs(), u64::MAX);
        assert_eq!(record.hold_duration.as_secs(), u64::MAX);
    }

    #[test]
    fn resource_usage_record_zero_usage() {
        let record = ResourceUsageRecord {
            resource_key: resource_key!("zero_usage"),
            acquired_at: Instant::now(),
            wait_duration: Duration::ZERO,
            hold_duration: Duration::ZERO,
            drop_reason: DropReason::Released,
        };
        assert_eq!(record.wait_duration, Duration::ZERO);
        assert_eq!(record.hold_duration, Duration::ZERO);
    }

    #[test]
    fn resource_usage_record_panic_drop_reason() {
        let record = ResourceUsageRecord {
            resource_key: resource_key!("panic_drop"),
            acquired_at: Instant::now(),
            wait_duration: Duration::from_millis(10),
            hold_duration: Duration::from_millis(100),
            drop_reason: DropReason::Panic,
        };
        assert_eq!(record.drop_reason, DropReason::Panic);
    }

    #[test]
    fn resource_usage_record_detached_drop_reason() {
        let record = ResourceUsageRecord {
            resource_key: resource_key!("detached"),
            acquired_at: Instant::now(),
            wait_duration: Duration::from_millis(5),
            hold_duration: Duration::from_millis(50),
            drop_reason: DropReason::Detached,
        };
        assert_eq!(record.drop_reason, DropReason::Detached);
    }

    #[test]
    fn resource_usage_record_clone() {
        let record = ResourceUsageRecord {
            resource_key: resource_key!("clone_test"),
            acquired_at: Instant::now(),
            wait_duration: Duration::from_millis(20),
            hold_duration: Duration::from_millis(200),
            drop_reason: DropReason::Released,
        };
        let cloned = record.clone();
        assert_eq!(record.resource_key, cloned.resource_key);
        assert_eq!(record.wait_duration, cloned.wait_duration);
        assert_eq!(record.hold_duration, cloned.hold_duration);
        assert_eq!(record.drop_reason, cloned.drop_reason);
    }

    #[test]
    fn call_status_success() {
        let status = CallStatus::Success;
        assert!(matches!(status, CallStatus::Success));
    }

    #[test]
    fn call_status_error() {
        let msg = "connection timeout".to_string();
        let status = CallStatus::Error(msg.clone());
        match status {
            CallStatus::Error(m) => assert_eq!(m, msg),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn call_status_clone() {
        let original = CallStatus::Error("test_error".to_string());
        let cloned = original.clone();
        assert_eq!(format!("{:?}", original), format!("{:?}", cloned));
    }

    #[test]
    fn noop_recorder_record_usage_is_noop() {
        let recorder = NoopRecorder;
        let record = ResourceUsageRecord {
            resource_key: resource_key!("noop_test"),
            acquired_at: Instant::now(),
            wait_duration: Duration::from_millis(1),
            hold_duration: Duration::from_millis(10),
            drop_reason: DropReason::Released,
        };
        recorder.record_usage(record);
    }

    #[test]
    fn noop_recorder_record_call_is_noop() {
        let recorder = NoopRecorder;
        let record = CallRecord {
            resource_key: resource_key!("noop_test"),
            operation: "no_op".to_string(),
            started_at: Instant::now(),
            duration: Duration::from_millis(5),
            status: CallStatus::Success,
            request: None,
            response: None,
            metadata: HashMap::new(),
            trace_context: None,
        };
        recorder.record_call(record);
    }

    #[test]
    fn noop_recorder_is_enrichment_enabled() {
        let recorder = NoopRecorder;
        assert!(!recorder.is_enrichment_enabled());
    }

    #[test]
    fn noop_recorder_is_object_safe() {
        let recorder: std::sync::Arc<dyn Recorder + Send + Sync> =
            std::sync::Arc::new(NoopRecorder);
        assert!(!recorder.is_enrichment_enabled());
    }

    #[test]
    fn noop_recorder_clone() {
        let original = NoopRecorder;
        let cloned = original.clone();
        let _ = format!("{:?}", cloned);
    }

    #[test]
    fn call_payload_binary() {
        let payload = CallPayload {
            summary: "binary_file".to_string(),
            headers: None,
            body: Some(CallBody::Binary {
                mime: "application/octet-stream".to_string(),
                size: 2048,
            }),
            size_bytes: Some(2048),
        };
        if let Some(CallBody::Binary { mime, size }) = payload.body {
            assert_eq!(mime, "application/octet-stream");
            assert_eq!(size, 2048);
        } else {
            panic!("expected Binary variant");
        }
    }

    #[test]
    fn call_payload_with_headers() {
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Authorization".to_string(), "Bearer token123".to_string()),
        ];
        let payload = CallPayload {
            summary: "request".to_string(),
            headers: Some(headers.clone()),
            body: Some(CallBody::Text(r#"{"key":"value"}"#.to_string())),
            size_bytes: Some(16),
        };
        assert_eq!(payload.headers, Some(headers));
    }

    #[test]
    fn call_payload_default() {
        let payload = CallPayload::default();
        assert_eq!(payload.summary, "");
        assert!(payload.headers.is_none());
        assert!(payload.body.is_none());
        assert!(payload.size_bytes.is_none());
    }

    #[test]
    fn drop_reason_all_variants() {
        let reasons = vec![
            DropReason::Released,
            DropReason::Panic,
            DropReason::Detached,
        ];
        for reason in reasons {
            assert_eq!(reason, reason);
            let _ = format!("{:?}", reason);
        }
    }

    #[test]
    fn drop_reason_is_copy() {
        let reason = DropReason::Released;
        let _copy = reason;
        let _ = reason;
    }

    // ─────────────────────────────────────────────────────────────────────────
    // BufferedRecorder
    // ─────────────────────────────────────────────────────────────────────────

    /// Test sink that collects flushed records for assertions.
    #[derive(Debug, Clone, Default)]
    struct CollectingSink {
        flushed: Arc<Mutex<Vec<RecordEntry>>>,
    }

    #[async_trait::async_trait]
    impl RecordSink for CollectingSink {
        async fn flush(&self, records: Vec<RecordEntry>) {
            self.flushed.lock().expect("lock poisoned").extend(records);
        }
    }

    fn make_usage_record(key: &str) -> ResourceUsageRecord {
        ResourceUsageRecord {
            resource_key: nebula_core::ResourceKey::new(key).expect("valid test key"),
            acquired_at: Instant::now(),
            wait_duration: Duration::from_millis(1),
            hold_duration: Duration::from_millis(10),
            drop_reason: DropReason::Released,
        }
    }

    fn make_call_record(key: &str, op: &str) -> CallRecord {
        CallRecord {
            resource_key: nebula_core::ResourceKey::new(key).expect("valid test key"),
            operation: op.to_owned(),
            started_at: Instant::now(),
            duration: Duration::from_millis(5),
            status: CallStatus::Success,
            request: None,
            response: None,
            metadata: HashMap::new(),
            trace_context: None,
        }
    }

    #[tokio::test]
    async fn records_are_flushed_after_interval() {
        let sink = CollectingSink::default();
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 100,
                flush_interval: Duration::from_millis(50),
            },
            sink.clone(),
        );

        recorder.record_usage(make_usage_record("db"));
        recorder.record_call(make_call_record("db", "SELECT"));

        tokio::time::sleep(Duration::from_millis(150)).await;

        let flushed = sink.flushed.lock().unwrap();
        assert_eq!(flushed.len(), 2);
    }

    #[tokio::test]
    async fn records_are_flushed_when_buffer_fills() {
        let sink = CollectingSink::default();
        let buffer_size = 4;
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size,
                flush_interval: Duration::from_secs(60),
            },
            sink.clone(),
        );

        for i in 0..buffer_size {
            recorder.record_usage(make_usage_record(&format!("res-{i}")));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;

        let flushed = sink.flushed.lock().unwrap();
        assert_eq!(flushed.len(), buffer_size);
    }

    #[tokio::test]
    async fn back_pressure_drops_without_panic() {
        let sink = CollectingSink::default();
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 2,
                flush_interval: Duration::from_secs(60),
            },
            sink.clone(),
        );

        for i in 0..100 {
            recorder.record_usage(make_usage_record(&format!("res-{i}")));
        }
    }

    #[tokio::test]
    async fn shutdown_drains_remaining_records() {
        let sink = CollectingSink::default();
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 100,
                flush_interval: Duration::from_secs(60),
            },
            sink.clone(),
        );

        recorder.record_usage(make_usage_record("db"));
        recorder.record_call(make_call_record("api", "POST /users"));

        tokio::time::sleep(Duration::from_millis(10)).await;

        recorder.shutdown().await;

        let flushed = sink.flushed.lock().unwrap();
        assert_eq!(flushed.len(), 2);
    }

    #[tokio::test]
    async fn log_sink_does_not_panic() {
        let recorder = BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 10,
                flush_interval: Duration::from_millis(50),
            },
            LogSink,
        );

        recorder.record_usage(make_usage_record("http"));
        recorder.record_call(make_call_record("http", "GET /health"));

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    async fn produce_records(rec: Arc<BufferedRecorder>, thread_id: usize) {
        for i in 0..25 {
            rec.record_usage(make_usage_record(&format!("t{thread_id}-r{i}")));
        }
    }

    #[tokio::test]
    async fn multiple_concurrent_producers() {
        let sink = CollectingSink::default();
        let recorder = Arc::new(BufferedRecorder::start(
            BufferedRecorderConfig {
                buffer_size: 1000,
                flush_interval: Duration::from_millis(50),
            },
            sink.clone(),
        ));

        let mut handles = Vec::new();
        for t in 0..4 {
            handles.push(tokio::spawn(produce_records(Arc::clone(&recorder), t)));
        }

        for h in handles {
            h.await.unwrap();
        }

        tokio::time::sleep(Duration::from_millis(150)).await;

        let flushed = sink.flushed.lock().unwrap();
        assert_eq!(flushed.len(), 100);
    }
}
