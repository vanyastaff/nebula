//! Idempotency key generation.
//!
//! Deduplication itself is owned by `nebula_storage::ExecutionRepo` via
//! `check_idempotency` / `mark_idempotent`. The engine constructs a key with
//! [`IdempotencyKey::for_attempt`] / [`IdempotencyKey::for_iteration`] and
//! routes the dedup decision through the repository so that durability and
//! the key namespace stay in lock-step.

use std::fmt;

use nebula_core::{ExecutionId, NodeKey};
use serde::{Deserialize, Serialize};

/// A deterministic key used to ensure exactly-once execution of a node attempt.
///
/// Two composition modes:
/// - **Stateless / one-shot:** `{execution_id}:{node_key}:{attempt}` — see
///   [`for_attempt`](Self::for_attempt).
/// - **Stateful per-iteration:** `{execution_id}:{node_key}:{iteration}:{attempt}` — see
///   [`for_iteration`](Self::for_iteration). Spec 28 §9.0 makes the iteration counter load-bearing
///   so a resumed stateful action reuses the same key for the same iteration boundary on retry.
/// - **Business dedup:** callers may append a `StatefulAction::idempotency_key(state)` via
///   [`with_business_key`](Self::with_business_key) — e.g. to key payments by invoice ID instead of
///   attempt number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Generate a stateless key — `{execution_id}:{node_key}:{attempt}`.
    ///
    /// Used for `StatelessAction`, `ResourceAction`, and control-flow nodes
    /// that do not iterate. The `attempt` counter advances on retry; a single
    /// attempt dispatches exactly once.
    #[must_use]
    pub fn for_attempt(execution_id: ExecutionId, node_key: NodeKey, attempt: u32) -> Self {
        Self(format!("{execution_id}:{node_key}:{attempt}"))
    }

    /// Generate a stateful per-iteration key — `{execution_id}:{node_key}:{iteration}:{attempt}`.
    ///
    /// Called by the runtime before each `StatefulAction::execute` dispatch.
    /// The iteration counter is the same one the runtime uses to enforce
    /// `MAX_ITERATIONS` and for `StatefulCheckpoint::iteration`, so resuming
    /// after a crash reuses the exact key that was in flight.
    #[must_use]
    pub fn for_iteration(
        execution_id: ExecutionId,
        node_key: NodeKey,
        iteration: u32,
        attempt: u32,
    ) -> Self {
        Self(format!("{execution_id}:{node_key}:{iteration}:{attempt}"))
    }

    /// Deprecated alias for [`for_attempt`](Self::for_attempt); kept while
    /// callers migrate.
    #[must_use]
    #[deprecated(note = "use IdempotencyKey::for_attempt")]
    pub fn generate(execution_id: ExecutionId, node_key: NodeKey, attempt: u32) -> Self {
        Self::for_attempt(execution_id, node_key, attempt)
    }

    /// Append an author-supplied business dedup suffix — the value returned by
    /// `StatefulAction::idempotency_key(&state)`. Format: `{base}:{business}`.
    ///
    /// Callers that want external systems (payment gateways, ticketing APIs)
    /// to dedup on their own notion of identity (invoice ID, job ID, ...) pass
    /// the business key through here. An empty business key leaves the base
    /// key unchanged.
    #[must_use]
    pub fn with_business_key(mut self, business: &str) -> Self {
        if !business.is_empty() {
            self.0.push(':');
            self.0.push_str(business);
        }
        self
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
    fn for_attempt_is_deterministic() {
        let exec_id = ExecutionId::new();
        let node_key = node_key!("test_node");
        let key1 = IdempotencyKey::for_attempt(exec_id, node_key.clone(), 0);
        let key2 = IdempotencyKey::for_attempt(exec_id, node_key, 0);
        assert_eq!(key1, key2);
    }

    #[test]
    fn different_attempts_produce_different_keys() {
        let exec_id = ExecutionId::new();
        let node_key = node_key!("test_node");
        let key0 = IdempotencyKey::for_attempt(exec_id, node_key.clone(), 0);
        let key1 = IdempotencyKey::for_attempt(exec_id, node_key, 1);
        assert_ne!(key0, key1);
    }

    #[test]
    fn for_iteration_embeds_iteration_counter() {
        let exec_id = ExecutionId::new();
        let node_key = node_key!("test_node");
        let it0 = IdempotencyKey::for_iteration(exec_id, node_key.clone(), 0, 0);
        let it1 = IdempotencyKey::for_iteration(exec_id, node_key.clone(), 1, 0);
        assert_ne!(it0, it1);
        // Same iteration + same attempt re-runs reuse the exact key.
        let it0_again = IdempotencyKey::for_iteration(exec_id, node_key, 0, 0);
        assert_eq!(it0, it0_again);
    }

    #[test]
    fn for_iteration_and_for_attempt_are_distinct_namespaces() {
        let exec_id = ExecutionId::new();
        let node_key = node_key!("n");
        let attempt = IdempotencyKey::for_attempt(exec_id, node_key.clone(), 0);
        let iter = IdempotencyKey::for_iteration(exec_id, node_key, 0, 0);
        assert_ne!(attempt, iter);
    }

    #[test]
    fn with_business_key_appends_suffix() {
        let exec_id = ExecutionId::new();
        let with_invoice = IdempotencyKey::for_iteration(exec_id, node_key!("pay"), 0, 0)
            .with_business_key("invoice_42");
        assert!(with_invoice.as_str().ends_with(":invoice_42"));
        // Same (exec, node, iter, attempt) + same business key → same key.
        let again = IdempotencyKey::for_iteration(exec_id, node_key!("pay"), 0, 0)
            .with_business_key("invoice_42");
        assert_eq!(with_invoice, again);
    }

    #[test]
    fn with_business_key_ignores_empty_suffix() {
        let exec_id = ExecutionId::new();
        let base = IdempotencyKey::for_iteration(exec_id, node_key!("pay"), 0, 0);
        assert_eq!(base.clone(), base.with_business_key(""));
    }

    #[test]
    fn key_display_includes_all_components() {
        let exec_id = ExecutionId::new();
        let node_key = node_key!("test_node");
        let key = IdempotencyKey::for_attempt(exec_id, node_key.clone(), 2);
        let display = key.to_string();
        assert!(display.contains(&exec_id.to_string()));
        assert!(display.contains(&node_key.to_string()));
        assert!(display.ends_with(":2"));
    }

    #[test]
    fn serde_roundtrip() {
        let key = IdempotencyKey::for_attempt(ExecutionId::new(), node_key!("test"), 3);
        let json = serde_json::to_string(&key).expect("serialize");
        let back: IdempotencyKey = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(key, back);
    }
}
