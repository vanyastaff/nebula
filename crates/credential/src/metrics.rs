//! Credential operation metrics.
//!
//! Provides structured metric emission for credential lifecycle operations.
//! Metrics are emitted through an injected emitter (typically from the context),
//! not through a global/static registry.
//!
//! See spec 22 §3.12 for the full metrics catalog.

use crate::CredentialKey;

/// Well-known metric counter names for credential operations.
///
/// Use these constants with a [`MetricsEmitter`](nebula_core::accessor::MetricsEmitter)
/// to ensure consistent metric naming across the codebase.
pub struct CredentialMetrics;

impl CredentialMetrics {
    // ── Counter names ──────────────────────────────────────────────────

    /// Total credential resolutions attempted.
    pub const RESOLVE_TOTAL: &'static str = "nebula.credential.resolve_total";

    /// Total credential refreshes attempted.
    pub const REFRESH_TOTAL: &'static str = "nebula.credential.refresh_total";

    /// Total credential refresh failures.
    pub const REFRESH_FAILED_TOTAL: &'static str = "nebula.credential.refresh_failed_total";

    /// Total credential connectivity tests.
    pub const TEST_TOTAL: &'static str = "nebula.credential.test_total";

    /// Total credential rotations completed.
    pub const ROTATIONS_TOTAL: &'static str = "nebula.credential.rotations_total";

    /// Total dynamic credential leases issued.
    pub const DYNAMIC_LEASE_ISSUED_TOTAL: &'static str =
        "nebula.credential.dynamic_lease_issued_total";

    /// Total dynamic credential leases expired or released.
    pub const DYNAMIC_LEASE_RELEASED_TOTAL: &'static str =
        "nebula.credential.dynamic_lease_released_total";

    /// Total tamper detection events.
    pub const TAMPER_DETECTION_TOTAL: &'static str = "nebula.credential.tamper_detection_total";

    // ── Label keys ─────────────────────────────────────────────────────

    /// Label: credential key (e.g., "github_token").
    pub const LABEL_CREDENTIAL_KEY: &'static str = "credential_key";

    /// Label: operation outcome ("success" | "failure").
    pub const LABEL_OUTCOME: &'static str = "outcome";

    /// Label: whether credential is dynamic ("true" | "false").
    pub const LABEL_DYNAMIC: &'static str = "dynamic";

    /// Label: refresh failure reason.
    pub const LABEL_FAILURE_REASON: &'static str = "reason";

    // ── Helper methods ─────────────────────────────────────────────────

    /// Standard labels for a credential operation.
    #[must_use]
    pub fn credential_labels<'a>(
        key: &'a CredentialKey,
        outcome: &'a str,
    ) -> [(&'static str, &'a str); 2] {
        [
            (Self::LABEL_CREDENTIAL_KEY, key.as_str()),
            (Self::LABEL_OUTCOME, outcome),
        ]
    }

    /// Emit a counter increment through the given emitter.
    pub fn emit_counter(
        emitter: &dyn nebula_core::accessor::MetricsEmitter,
        name: &str,
        key: &CredentialKey,
        outcome: &str,
    ) {
        let labels = Self::credential_labels(key, outcome);
        emitter.counter(name, 1, &labels);
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::credential_key;

    use super::*;

    #[test]
    fn credential_labels_returns_expected_pairs() {
        let key = credential_key!("github_token");
        let labels = CredentialMetrics::credential_labels(&key, "success");
        assert_eq!(labels[0], ("credential_key", "github_token"));
        assert_eq!(labels[1], ("outcome", "success"));
    }

    #[test]
    fn constants_have_expected_prefix() {
        assert!(CredentialMetrics::RESOLVE_TOTAL.starts_with("nebula.credential."));
        assert!(CredentialMetrics::REFRESH_TOTAL.starts_with("nebula.credential."));
        assert!(CredentialMetrics::ROTATIONS_TOTAL.starts_with("nebula.credential."));
    }
}
