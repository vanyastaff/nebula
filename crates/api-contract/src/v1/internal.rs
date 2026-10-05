//! Internal operator transport vocabulary, excluded from the supported SDK.

use serde::{Deserialize, Serialize};

/// Body returned by `POST /internal/v1/webhooks/reload`.
#[derive(Debug, Serialize, Deserialize)]
pub struct WebhookReloadReport {
    /// Activations validated from the port store.
    pub loaded: usize,
    /// Rows that surfaced a non-storage failure and were skipped.
    pub skipped: usize,
}
