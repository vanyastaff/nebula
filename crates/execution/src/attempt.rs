//! Node execution attempt tracking.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{error_envelope::ErrorEnvelope, idempotency::IdempotencyKey, output::ExecutionOutput};

/// A single attempt to execute a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeAttempt {
    /// Which attempt this is (0-indexed).
    pub attempt_number: u32,
    /// Idempotency key for this attempt.
    pub idempotency_key: IdempotencyKey,
    /// When this attempt started.
    pub started_at: DateTime<Utc>,
    /// When this attempt completed (if finished).
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    /// Output data if the attempt succeeded.
    #[serde(default)]
    pub output: Option<ExecutionOutput>,
    /// Failure record if the attempt failed.
    ///
    /// Attempts ride inside the persisted `ExecutionState`, so this is a durable
    /// carrier: it holds a typed [`ErrorEnvelope`], never the failed action's own
    /// text. See [`ErrorEnvelope`] for why the provider's message is not stored.
    #[serde(default)]
    pub error: Option<ErrorEnvelope>,
    /// Size of the output in bytes.
    #[serde(default)]
    pub output_bytes: u64,
}

impl NodeAttempt {
    /// Create a new attempt that has just started.
    #[must_use]
    pub fn new(attempt_number: u32, idempotency_key: IdempotencyKey) -> Self {
        Self {
            attempt_number,
            idempotency_key,
            started_at: Utc::now(),
            completed_at: None,
            output: None,
            error: None,
            output_bytes: 0,
        }
    }

    /// Mark this attempt as successfully completed.
    pub fn complete_success(&mut self, output: ExecutionOutput, output_bytes: u64) {
        self.completed_at = Some(Utc::now());
        self.output = Some(output);
        self.output_bytes = output_bytes;
    }

    /// Mark this attempt as failed.
    pub fn complete_failure(&mut self, error: ErrorEnvelope) {
        self.completed_at = Some(Utc::now());
        self.error = Some(error);
    }

    /// Returns `true` if this attempt has finished (success or failure).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.completed_at.is_some()
    }

    /// Returns `true` if this attempt succeeded.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.output.is_some() && self.error.is_none()
    }

    /// Returns `true` if this attempt failed.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        self.error.is_some()
    }

    /// Calculate the duration of this attempt.
    #[must_use]
    pub fn duration(&self) -> Option<Duration> {
        self.completed_at
            .map(|end| (end - self.started_at).to_std().unwrap_or(Duration::ZERO))
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{ExecutionId, node_key};
    use nebula_error::{ErrorCategory, ErrorCode};

    use super::*;

    fn test_key() -> IdempotencyKey {
        IdempotencyKey::for_attempt(ExecutionId::new(), node_key!("test"), 0)
    }

    fn failure(message: &str) -> ErrorEnvelope {
        ErrorEnvelope::new(
            ErrorCode::new("ENGINE:NODE_FAILED"),
            ErrorCategory::Internal,
            false,
        )
        .with_redacted_message(message)
    }

    #[test]
    fn new_attempt() {
        let attempt = NodeAttempt::new(0, test_key());
        assert_eq!(attempt.attempt_number, 0);
        assert!(!attempt.is_complete());
        assert!(!attempt.is_success());
        assert!(!attempt.is_failure());
        assert!(attempt.duration().is_none());
    }

    #[test]
    fn complete_success() {
        let mut attempt = NodeAttempt::new(0, test_key());
        attempt.complete_success(ExecutionOutput::inline(serde_json::json!(42)), 8);
        assert!(attempt.is_complete());
        assert!(attempt.is_success());
        assert!(!attempt.is_failure());
        assert_eq!(attempt.output_bytes, 8);
    }

    #[test]
    fn complete_failure() {
        let mut attempt = NodeAttempt::new(1, test_key());
        attempt.complete_failure(failure("connection timeout"));
        assert!(attempt.is_complete());
        assert!(!attempt.is_success());
        assert!(attempt.is_failure());
        assert_eq!(
            attempt
                .error
                .as_ref()
                .and_then(ErrorEnvelope::redacted_message),
            Some("connection timeout")
        );
    }

    #[test]
    fn duration_after_completion() {
        let mut attempt = NodeAttempt::new(0, test_key());
        attempt.complete_success(ExecutionOutput::inline(serde_json::json!(null)), 0);
        let dur = attempt.duration();
        assert!(dur.is_some());
    }

    #[test]
    fn duration_before_completion() {
        let attempt = NodeAttempt::new(0, test_key());
        assert!(attempt.duration().is_none());
    }

    #[test]
    fn serde_roundtrip_success() {
        let mut attempt = NodeAttempt::new(0, test_key());
        attempt.complete_success(ExecutionOutput::inline(serde_json::json!({"ok": true})), 32);
        let json = serde_json::to_string(&attempt).unwrap();
        let back: NodeAttempt = serde_json::from_str(&json).unwrap();
        assert!(back.is_success());
        assert_eq!(back.output_bytes, 32);
    }

    #[test]
    fn serde_roundtrip_failure() {
        let mut attempt = NodeAttempt::new(2, test_key());
        attempt.complete_failure(failure("some error"));
        let json = serde_json::to_string(&attempt).unwrap();
        let back: NodeAttempt = serde_json::from_str(&json).unwrap();
        assert!(back.is_failure());
        assert_eq!(
            back.error
                .as_ref()
                .and_then(ErrorEnvelope::redacted_message),
            Some("some error")
        );
    }

    /// The attempt is a durable carrier inside the persisted `ExecutionState`.
    /// A row written before the envelope existed holds a bare error string, and
    /// reading it back as an attempt must fail rather than resurrect that string.
    ///
    /// **Falsifiability**: give `error` its pre-envelope `Option<String>` type →
    /// the decode below succeeds and the assert fails. The control decode (the
    /// un-downgraded wire, same `from_str` path) is what makes that
    /// falsifiability real: `ErrorCategory`'s `Deserialize` used
    /// `<&str>::deserialize`, which `serde_json::from_value` cannot satisfy for
    /// *any* `ErrorEnvelope` — valid or not — so without the control this test
    /// would flip on a type revert for the wrong reason, unable to distinguish
    /// "bare string refused" from "`from_value` cannot decode any envelope."
    #[test]
    fn legacy_attempt_row_with_a_bare_error_string_fails_to_decode() {
        let mut attempt = NodeAttempt::new(0, test_key());
        attempt.complete_failure(failure("boom"));
        let mut wire = serde_json::to_value(&attempt).unwrap();
        assert!(
            wire["error"].is_object(),
            "fixture must persist a typed record, got: {:?}",
            wire["error"]
        );

        // Control: the un-downgraded wire must decode `Ok` on the exact path
        // (`from_str`) the downgraded wire is decoded on below.
        let control = serde_json::from_str::<NodeAttempt>(&wire.to_string());
        assert!(
            control.is_ok(),
            "the un-downgraded wire must decode via from_str: {control:?}"
        );

        wire["error"] = serde_json::json!("provider said: token abc123");

        let decoded = serde_json::from_str::<NodeAttempt>(&wire.to_string());

        let err = decoded.expect_err("a bare error string is not an envelope");
        let message = err.to_string();
        assert!(
            message.contains("ErrorEnvelope"),
            "the refusal must name the type it refused to decode as, got: {message}"
        );
    }

    #[test]
    fn attempt_number_preserved() {
        let attempt = NodeAttempt::new(5, test_key());
        assert_eq!(attempt.attempt_number, 5);
    }
}
