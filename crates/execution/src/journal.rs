//! Execution journal for audit and replay.

use chrono::{DateTime, Utc};
use nebula_core::NodeKey;
use serde::{Deserialize, Serialize};

use crate::status::ExecutionStatus;

/// A journal entry recording a significant event during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum JournalEntry {
    /// The execution was started.
    ExecutionStarted {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
    },

    /// A node was scheduled for execution.
    NodeScheduled {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
        /// The node that was scheduled.
        node_key: NodeKey,
    },

    /// A node started executing.
    NodeStarted {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
        /// The node that started.
        node_key: NodeKey,
        /// Which attempt number (0-indexed).
        attempt: u32,
    },

    /// A node completed successfully.
    NodeCompleted {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
        /// The node that completed.
        node_key: NodeKey,
        /// Output size in bytes.
        output_bytes: u64,
    },

    /// A node failed.
    NodeFailed {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
        /// The node that failed.
        node_key: NodeKey,
        /// Error message.
        error: String,
    },

    /// A node was skipped.
    NodeSkipped {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
        /// The node that was skipped.
        node_key: NodeKey,
        /// Reason for skipping.
        reason: String,
    },

    /// The entire execution completed successfully.
    ExecutionCompleted {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
        /// Final execution status.
        status: ExecutionStatus,
    },

    /// The entire execution failed.
    ExecutionFailed {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
        /// Error message.
        error: String,
    },

    /// A cancellation was requested.
    CancellationRequested {
        /// When the event occurred.
        timestamp: DateTime<Utc>,
        /// Reason for cancellation.
        reason: String,
    },
}

impl JournalEntry {
    /// Get the timestamp of this entry.
    #[must_use]
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::ExecutionStarted { timestamp }
            | Self::NodeScheduled { timestamp, .. }
            | Self::NodeStarted { timestamp, .. }
            | Self::NodeCompleted { timestamp, .. }
            | Self::NodeFailed { timestamp, .. }
            | Self::NodeSkipped { timestamp, .. }
            | Self::ExecutionCompleted { timestamp, .. }
            | Self::ExecutionFailed { timestamp, .. }
            | Self::CancellationRequested { timestamp, .. } => *timestamp,
        }
    }

    /// Get the node ID associated with this entry, if any.
    #[must_use]
    pub fn node_key(&self) -> Option<NodeKey> {
        match self {
            Self::NodeScheduled { node_key, .. }
            | Self::NodeStarted { node_key, .. }
            | Self::NodeCompleted { node_key, .. }
            | Self::NodeFailed { node_key, .. }
            | Self::NodeSkipped { node_key, .. } => Some(node_key.clone()),
            Self::ExecutionStarted { .. }
            | Self::ExecutionCompleted { .. }
            | Self::ExecutionFailed { .. }
            | Self::CancellationRequested { .. } => None,
        }
    }

    /// Returns `true` if this is a node-level event.
    #[must_use]
    pub fn is_node_event(&self) -> bool {
        self.node_key().is_some()
    }

    /// Returns `true` if this is an execution-level event.
    #[must_use]
    pub fn is_execution_event(&self) -> bool {
        self.node_key().is_none()
    }

    /// Serialize this entry to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize an entry from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn execution_started_entry() {
        let ts = now();
        let entry = JournalEntry::ExecutionStarted { timestamp: ts };
        assert_eq!(entry.timestamp(), ts);
        assert!(entry.is_execution_event());
        assert!(!entry.is_node_event());
        assert!(entry.node_key().is_none());
    }

    #[test]
    fn node_scheduled_entry() {
        let ts = now();
        let nid = node_key!("nid");
        let entry = JournalEntry::NodeScheduled {
            timestamp: ts,
            node_key: nid.clone(),
        };
        assert_eq!(entry.node_key(), Some(nid));
        assert!(entry.is_node_event());
        assert!(!entry.is_execution_event());
    }

    #[test]
    fn node_started_entry() {
        let entry = JournalEntry::NodeStarted {
            timestamp: now(),
            node_key: node_key!("test"),
            attempt: 0,
        };
        assert!(entry.is_node_event());
    }

    #[test]
    fn node_completed_entry() {
        let nid = node_key!("nid");
        let entry = JournalEntry::NodeCompleted {
            timestamp: now(),
            node_key: nid.clone(),
            output_bytes: 1024,
        };
        assert_eq!(entry.node_key(), Some(nid));
    }

    #[test]
    fn node_failed_entry() {
        let entry = JournalEntry::NodeFailed {
            timestamp: now(),
            node_key: node_key!("test"),
            error: "timeout".into(),
        };
        assert!(entry.is_node_event());
    }

    #[test]
    fn node_skipped_entry() {
        let entry = JournalEntry::NodeSkipped {
            timestamp: now(),
            node_key: node_key!("test"),
            reason: "condition not met".into(),
        };
        assert!(entry.is_node_event());
    }

    #[test]
    fn execution_completed_entry() {
        let entry = JournalEntry::ExecutionCompleted {
            timestamp: now(),
            status: ExecutionStatus::Completed,
        };
        assert!(entry.is_execution_event());
    }

    #[test]
    fn cancellation_requested_entry() {
        let entry = JournalEntry::CancellationRequested {
            timestamp: now(),
            reason: "user requested".into(),
        };
        assert!(entry.is_execution_event());
        assert!(entry.node_key().is_none());
    }

    #[test]
    fn serde_roundtrip_all_variants() {
        let nid = node_key!("nid");
        let ts = now();

        let entries = vec![
            JournalEntry::ExecutionStarted { timestamp: ts },
            JournalEntry::NodeScheduled {
                timestamp: ts,
                node_key: nid.clone(),
            },
            JournalEntry::NodeStarted {
                timestamp: ts,
                node_key: nid.clone(),
                attempt: 0,
            },
            JournalEntry::NodeCompleted {
                timestamp: ts,
                node_key: nid.clone(),
                output_bytes: 512,
            },
            JournalEntry::NodeFailed {
                timestamp: ts,
                node_key: nid.clone(),
                error: "err".into(),
            },
            JournalEntry::NodeSkipped {
                timestamp: ts,
                node_key: nid,
                reason: "skip".into(),
            },
            JournalEntry::ExecutionCompleted {
                timestamp: ts,
                status: ExecutionStatus::Completed,
            },
            JournalEntry::ExecutionFailed {
                timestamp: ts,
                error: "fatal".into(),
            },
            JournalEntry::CancellationRequested {
                timestamp: ts,
                reason: "shutdown".into(),
            },
        ];

        for entry in &entries {
            let json = entry.to_json().unwrap();
            let back = JournalEntry::from_json(&json).unwrap();
            assert_eq!(entry.timestamp(), back.timestamp());
            assert_eq!(entry.node_key(), back.node_key());
        }
    }
}
