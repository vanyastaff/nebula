//! Node execution attempt tracking.

use chrono::{DateTime, Utc};
use nebula_action::NodeOutputData;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::idempotency::IdempotencyKey;

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
    pub output: Option<NodeOutputData>,
    /// Error message if the attempt failed.
    #[serde(default)]
    pub error: Option<String>,
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
    pub fn complete_success(&mut self, output: NodeOutputData, output_bytes: u64) {
        self.completed_at = Some(Utc::now());
        self.output = Some(output);
        self.output_bytes = output_bytes;
    }

    /// Mark this attempt as failed.
    pub fn complete_failure(&mut self, error: impl Into<String>) {
        self.completed_at = Some(Utc::now());
        self.error = Some(error.into());
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
    use super::*;
    use nebula_core::{ExecutionId, NodeId};

    fn test_key() -> IdempotencyKey {
        IdempotencyKey::generate(ExecutionId::v4(), NodeId::v4(), 0)
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
        attempt.complete_success(NodeOutputData::inline(serde_json::json!(42)), 8);
        assert!(attempt.is_complete());
        assert!(attempt.is_success());
        assert!(!attempt.is_failure());
        assert_eq!(attempt.output_bytes, 8);
    }

    #[test]
    fn complete_failure() {
        let mut attempt = NodeAttempt::new(1, test_key());
        attempt.complete_failure("connection timeout");
        assert!(attempt.is_complete());
        assert!(!attempt.is_success());
        assert!(attempt.is_failure());
        assert_eq!(attempt.error.as_deref(), Some("connection timeout"));
    }

    #[test]
    fn duration_after_completion() {
        let mut attempt = NodeAttempt::new(0, test_key());
        attempt.complete_success(NodeOutputData::inline(serde_json::json!(null)), 0);
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
        attempt.complete_success(NodeOutputData::inline(serde_json::json!({"ok": true})), 32);
        let json = serde_json::to_string(&attempt).unwrap();
        let back: NodeAttempt = serde_json::from_str(&json).unwrap();
        assert!(back.is_success());
        assert_eq!(back.output_bytes, 32);
    }

    #[test]
    fn serde_roundtrip_failure() {
        let mut attempt = NodeAttempt::new(2, test_key());
        attempt.complete_failure("some error");
        let json = serde_json::to_string(&attempt).unwrap();
        let back: NodeAttempt = serde_json::from_str(&json).unwrap();
        assert!(back.is_failure());
        assert_eq!(back.error.as_deref(), Some("some error"));
    }

    #[test]
    fn attempt_number_preserved() {
        let attempt = NodeAttempt::new(5, test_key());
        assert_eq!(attempt.attempt_number, 5);
    }
}
