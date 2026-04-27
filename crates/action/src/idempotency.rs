//! [`IdempotencyKey`] — transport-level dedup identifier.
//!
//! Returned by [`TriggerAction::idempotency_key`](crate::TriggerAction::idempotency_key)
//! per Tech Spec §15.12 F2. Engine uses the key to suppress duplicate workflow
//! executions when a trigger transport delivers the same event more than once
//! (webhook retry, queue redelivery, schedule replay).
//!
//! Not a secret — engine logs and metrics MAY include the key. For dedup
//! windows and storage, see PRODUCT_CANON §11.3 idempotency.

use std::fmt;

/// Stable per-event dedup identifier returned by a trigger.
///
/// `None` from [`TriggerAction::idempotency_key`] means the trigger does not
/// supply a dedup id — engine falls back to the transport's own dedup or
/// re-delivers events.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Build a key from any string-convertible source.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// View the key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for IdempotencyKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
