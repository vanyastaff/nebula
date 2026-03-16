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

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::resource_key;

    // ─────────────────────────────────────────────────────────────────────────────
    // CallRecord edge cases
    // ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_call_record_with_empty_payloads() {
        tracing::debug!("testing call_record with empty payloads");
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
    fn test_call_payload_redacted() {
        tracing::debug!("testing call_payload redacted");
        let payload = CallPayload {
            summary: "GET /secrets".to_string(),
            headers: None,
            body: Some(CallBody::Redacted),
            size_bytes: Some(1024),
        };
        // Verify redacted body is preserved
        assert!(matches!(payload.body, Some(CallBody::Redacted)));
    }

    #[test]
    fn test_call_payload_oversized_content() {
        tracing::debug!("testing call_payload with oversized content");
        let large_text = "x".repeat(65536); // 64 KB
        let payload = CallPayload {
            summary: "large_payload".to_string(),
            headers: None,
            body: Some(CallBody::Text(large_text.clone())),
            size_bytes: Some(65536),
        };
        // Verify no truncation panic; body is stored as-is
        if let Some(CallBody::Text(body)) = payload.body {
            assert_eq!(body.len(), 65536);
        }
    }

    #[test]
    fn test_call_status_error_with_empty_message() {
        tracing::debug!("testing call_status error with empty message");
        let status = CallStatus::Error("".to_string());
        match status {
            CallStatus::Error(msg) => assert_eq!(msg, ""),
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn test_call_record_zero_duration() {
        tracing::debug!("testing call_record with zero duration");
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
        assert_eq!(record.duration.as_secs(), 0);
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // ResourceUsageRecord edge cases
    // ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_resource_usage_record_max_values() {
        tracing::debug!("testing resource_usage_record with max values");
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
    fn test_resource_usage_record_zero_usage() {
        tracing::debug!("testing resource_usage_record zero usage");
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
    fn test_resource_usage_record_panic_drop_reason() {
        tracing::debug!("testing resource_usage_record panic drop reason");
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
    fn test_resource_usage_record_detached_drop_reason() {
        tracing::debug!("testing resource_usage_record detached drop reason");
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
    fn test_resource_usage_record_clone() {
        tracing::debug!("testing resource_usage_record clone");
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

    // ─────────────────────────────────────────────────────────────────────────────
    // CallStatus coverage
    // ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_call_status_success() {
        tracing::debug!("testing call_status success variant");
        let status = CallStatus::Success;
        match status {
            CallStatus::Success => {} // OK
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn test_call_status_error() {
        tracing::debug!("testing call_status error variant");
        let msg = "connection timeout".to_string();
        let status = CallStatus::Error(msg.clone());
        match status {
            CallStatus::Error(m) => assert_eq!(m, msg),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_call_status_clone() {
        tracing::debug!("testing call_status clone");
        let original = CallStatus::Error("test_error".to_string());
        let cloned = original.clone();
        assert_eq!(format!("{:?}", original), format!("{:?}", cloned));
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Recorder trait
    // ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_noop_recorder_record_usage_is_noop() {
        tracing::debug!("testing noop_recorder record_usage");
        let recorder = NoopRecorder;
        let record = ResourceUsageRecord {
            resource_key: resource_key!("noop_test"),
            acquired_at: Instant::now(),
            wait_duration: Duration::from_millis(1),
            hold_duration: Duration::from_millis(10),
            drop_reason: DropReason::Released,
        };
        // Should not panic
        recorder.record_usage(record);
    }

    #[test]
    fn test_noop_recorder_record_call_is_noop() {
        tracing::debug!("testing noop_recorder record_call");
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
        // Should not panic
        recorder.record_call(record);
    }

    #[test]
    fn test_noop_recorder_is_enrichment_enabled() {
        tracing::debug!("testing noop_recorder is_enrichment_enabled");
        let recorder = NoopRecorder;
        assert!(
            !recorder.is_enrichment_enabled(),
            "NoopRecorder should have enrichment disabled"
        );
    }

    #[test]
    fn test_noop_recorder_is_object_safe() {
        tracing::debug!("testing noop_recorder is object_safe");
        let recorder: std::sync::Arc<dyn Recorder + Send + Sync> =
            std::sync::Arc::new(NoopRecorder);
        assert!(
            !recorder.is_enrichment_enabled(),
            "Arc<dyn Recorder> should work with NoopRecorder"
        );
    }

    #[test]
    fn test_noop_recorder_clone() {
        tracing::debug!("testing noop_recorder clone");
        let original = NoopRecorder;
        let cloned = original.clone();
        // Should be debuggable
        let _ = format!("{:?}", cloned);
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // CallPayload edge cases
    // ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_call_payload_binary() {
        tracing::debug!("testing call_payload binary");
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
    fn test_call_payload_with_headers() {
        tracing::debug!("testing call_payload with headers");
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
    fn test_call_payload_default() {
        tracing::debug!("testing call_payload default");
        let payload = CallPayload::default();
        assert_eq!(payload.summary, "");
        assert!(payload.headers.is_none());
        assert!(payload.body.is_none());
        assert!(payload.size_bytes.is_none());
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Drop reason coverage
    // ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_drop_reason_all_variants() {
        tracing::debug!("testing drop_reason all variants");
        let reasons = vec![
            DropReason::Released,
            DropReason::Panic,
            DropReason::Detached,
        ];
        for reason in reasons {
            assert_eq!(reason, reason); // equality works
            let _ = format!("{:?}", reason); // debug works
        }
    }

    #[test]
    fn test_drop_reason_is_copy() {
        tracing::debug!("testing drop_reason is_copy");
        let reason = DropReason::Released;
        let _copy = reason; // Can copy without move
        let _ = reason; // Original still usable
    }
}
