//! Idempotency key generation.
//!
//! Deduplication itself is owned by `nebula_storage::ExecutionRepo` via
//! `check_idempotency` / `mark_idempotent`. The engine constructs a key with
//! [`IdempotencyKey::generate`] and routes the dedup decision through the
//! repository so that durability and the key namespace stay in lock-step.

use std::fmt;

use nebula_core::{ExecutionId, NodeKey};
use serde::{Deserialize, Serialize};

/// A deterministic key used to ensure exactly-once execution of a node attempt.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Generate a deterministic idempotency key from execution context.
    #[must_use]
    pub fn generate(execution_id: ExecutionId, node_key: NodeKey, attempt: u32) -> Self {
        Self(format!("{execution_id}:{node_key}:{attempt}"))
    }

    /// Get the underlying key string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    #[test]
    fn generate_deterministic_key() {
        let exec_id = ExecutionId::new();
        let node_key = node_key!("test_node");
        let key1 = IdempotencyKey::generate(exec_id, node_key.clone(), 0);
        let key2 = IdempotencyKey::generate(exec_id, node_key.clone(), 0);
        assert_eq!(key1, key2);
    }

    #[test]
    fn different_attempts_different_keys() {
        let exec_id = ExecutionId::new();
        let node_key = node_key!("test_node");
        let key0 = IdempotencyKey::generate(exec_id, node_key.clone(), 0);
        let key1 = IdempotencyKey::generate(exec_id, node_key.clone(), 1);
        assert_ne!(key0, key1);
    }

    #[test]
    fn key_display() {
        let exec_id = ExecutionId::new();
        let node_key = node_key!("test_node");
        let key = IdempotencyKey::generate(exec_id, node_key.clone(), 2);
        let display = key.to_string();
        assert!(display.contains(&exec_id.to_string()));
        assert!(display.contains(&node_key.to_string()));
        assert!(display.ends_with(":2"));
    }

    #[test]
    fn serde_roundtrip() {
        let key = IdempotencyKey::generate(ExecutionId::new(), node_key!("test"), 3);
        let json = serde_json::to_string(&key).expect("serialize");
        let back: IdempotencyKey = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(key, back);
    }
}
