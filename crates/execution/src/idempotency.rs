//! Idempotency key generation.
//!
//! Deduplication itself is owned by the storage port's idempotency guard
//! (`check_and_mark`). The engine constructs a key with
//! [`IdempotencyKey::for_attempt`] / [`IdempotencyKey::for_iteration`] and
//! routes the dedup decision through that port so that durability and the
//! key namespace stay in lock-step.

use std::fmt;

use nebula_core::{ExecutionId, NodeKey};
use serde::{Deserialize, Serialize};

/// A deterministic key used to ensure exactly-once execution of a node attempt.
///
/// Two composition modes:
/// - **Stateless / one-shot:** an attempt-tagged frame of `execution_id`,
///   `node_key`, `attempt` — see [`for_attempt`](Self::for_attempt).
/// - **Stateful per-iteration:** an iteration-tagged frame additionally carrying
///   the iteration counter — see [`for_iteration`](Self::for_iteration). Spec 28
///   §9.0 makes the iteration counter load-bearing so a resumed stateful action
///   reuses the same key for the same iteration boundary on retry.
/// - **Business dedup:** callers may append a `StatefulAction::idempotency_key(state)`
///   via [`with_business_key`](Self::with_business_key) — e.g. to key payments by
///   invoice ID instead of attempt number.
///
/// # Injective encoding
///
/// The components are **length-prefixed** (`{byte_len}:{bytes}` per part, plus a
/// leading kind tag distinguishing attempt- from iteration-frames), not joined by
/// a bare `:`. A naive `format!("{execution_id}:{node_key}:{attempt}")` is
/// non-injective the moment a *variable-length* component can contain the
/// separator: an author business key of `"0:extra"` appended to a stateless
/// attempt-0 key would collide with a stateful iteration-0 attempt-0 key keyed by
/// `"extra"` — a false dedup (the classic missed-payment / double-execution bug).
/// Framing each part by its length makes the key injective regardless of what any
/// component contains, so callers never have to guarantee a component is
/// separator-free. (The string is an opaque dedup token; this format is not a
/// stable wire contract.)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey(String);

/// Append one length-prefixed component (`{byte_len}:{part}`) to `buf`.
///
/// The decimal length is itself separator-free, so the `:` after it unambiguously
/// ends the length, and the length states exactly how many following bytes belong
/// to `part`. Concatenating parts this way is injective even when a part contains
/// `:` — the property a bare colon-join lacks.
fn push_part(buf: &mut String, part: &str) {
    use std::fmt::Write as _;
    // Writing to a String is infallible; the Result is discarded deliberately.
    let _ = write!(buf, "{}:{}", part.len(), part);
}

impl IdempotencyKey {
    /// Generate a stateless key tagged `"a"` (attempt) over `execution_id`,
    /// `node_key`, `attempt`.
    ///
    /// Used for `StatelessAction`, `ResourceAction`, and control-flow nodes
    /// that do not iterate. The `attempt` counter advances on retry; a single
    /// attempt dispatches exactly once.
    #[must_use]
    pub fn for_attempt(execution_id: ExecutionId, node_key: NodeKey, attempt: u32) -> Self {
        let mut key = String::new();
        push_part(&mut key, "a");
        push_part(&mut key, &execution_id.to_string());
        push_part(&mut key, node_key.as_str());
        push_part(&mut key, &attempt.to_string());
        Self(key)
    }

    /// Generate a stateful per-iteration key tagged `"i"` (iteration) over
    /// `execution_id`, `node_key`, `iteration`, `attempt`.
    ///
    /// Called by the runtime before each `StatefulAction::execute` dispatch.
    /// The iteration counter is the same one the runtime uses to enforce
    /// `MAX_ITERATIONS` and for `StatefulCheckpoint::iteration`, so resuming
    /// after a crash reuses the exact key that was in flight. The distinct kind
    /// tag keeps the attempt- and iteration-frames in separate namespaces, so a
    /// stateless key can never collide with a stateful one.
    #[must_use]
    pub fn for_iteration(
        execution_id: ExecutionId,
        node_key: NodeKey,
        iteration: u32,
        attempt: u32,
    ) -> Self {
        let mut key = String::new();
        push_part(&mut key, "i");
        push_part(&mut key, &execution_id.to_string());
        push_part(&mut key, node_key.as_str());
        push_part(&mut key, &iteration.to_string());
        push_part(&mut key, &attempt.to_string());
        Self(key)
    }

    /// Append an author-supplied business dedup suffix — the value returned by
    /// `StatefulAction::idempotency_key(&state)` — as one length-prefixed part.
    ///
    /// Callers that want external systems (payment gateways, ticketing APIs)
    /// to dedup on their own notion of identity (invoice ID, job ID, ...) pass
    /// the business key through here. The length-prefixed framing means a
    /// business key containing `:` (or chaining two business keys) is injective —
    /// it cannot forge a different key. An empty business key leaves the base
    /// key unchanged.
    #[must_use]
    pub fn with_business_key(mut self, business: &str) -> Self {
        if !business.is_empty() {
            push_part(&mut self.0, business);
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

    /// A single business key `"a:b"` must not equal chaining `"a"` then `"b"`: the
    /// separator inside one part cannot be confused with a part boundary. Under the
    /// old bare colon-join both produced `{base}:a:b` — a collision.
    #[test]
    fn business_key_with_separator_is_injective() {
        let exec = ExecutionId::new();
        let node = node_key!("pay");
        let one = IdempotencyKey::for_attempt(exec, node.clone(), 0).with_business_key("a:b");
        let two = IdempotencyKey::for_attempt(exec, node, 0)
            .with_business_key("a")
            .with_business_key("b");
        assert_ne!(one, two, "framing must keep `a:b` distinct from `a`+`b`");
    }

    /// The attempt- and iteration-frames stay in separate namespaces even when the
    /// trailing components coincide: a stateless attempt-0 keyed by the business
    /// string `"5"` must not equal a stateful iteration-0 attempt-5 key. Under the
    /// old colon-join both flattened to `{exec}:{node}:0:5` — a false dedup across
    /// two distinct executions (the missed-payment bug).
    #[test]
    fn attempt_frame_never_collides_with_iteration_frame() {
        let exec = ExecutionId::new();
        let node = node_key!("n");
        let attempt_framed =
            IdempotencyKey::for_attempt(exec, node.clone(), 0).with_business_key("5");
        let iteration_framed = IdempotencyKey::for_iteration(exec, node, 0, 5);
        assert_ne!(attempt_framed, iteration_framed);
    }
}
