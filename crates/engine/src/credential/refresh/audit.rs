//! Refresh-coordinator audit-event emission (sub-spec §6).
//!
//! Sits on top of the storage `AuditSink` so operators reuse one sink
//! implementation for both `CredentialStore` operations and refresh
//! coordination events.
//!
//! Three events surface here, mirroring the spec's audit list:
//!
//! - `RefreshCoordClaimAcquired { credential_id, holder, ttl_secs }` — fires once per L2 claim
//!   acquired by `RefreshCoordinator::refresh_coalesced`.
//! - `RefreshCoordSentinelTriggered { credential_id, recent_count }` — fires once per
//!   sentinel-event detection by the reclaim sweep.
//! - `RefreshCoordReauthFlagged { credential_id, reason }` — fires once when the rolling-window
//!   threshold is crossed and the sweep emits `CredentialEvent::ReauthRequired`.
//!
//! Sink failures are logged at `warn` level but do NOT propagate to the
//! caller. Audit on the refresh path is observational; failing the
//! refresh on a sink hiccup would re-create the n8n #13088 retry storm
//! the coordinator was built to prevent. The `CredentialStore` audit
//! wrapper retains its fail-closed semantics (ADR-0028 inv 4) — they
//! apply to mutating store operations, not to refresh events.

use nebula_core::CredentialId;
use nebula_storage::credential::{AuditEvent, AuditOperation, AuditResult, AuditSink};

/// Emit an audit event for an L2 claim acquisition.
pub(super) fn emit_claim_acquired(
    sink: Option<&dyn AuditSink>,
    credential_id: &CredentialId,
    holder: &str,
    ttl_secs: u64,
) {
    let Some(sink) = sink else { return };
    let event = AuditEvent {
        timestamp: chrono::Utc::now(),
        credential_id: credential_id.to_string(),
        operation: AuditOperation::RefreshCoordClaimAcquired {
            holder: holder.to_owned(),
            ttl_secs,
        },
        result: AuditResult::Success,
    };
    if let Err(e) = sink.record(&event) {
        tracing::warn!(?e, cred = %credential_id, "refresh-coord audit sink failed for ClaimAcquired");
    }
}

/// Emit an audit event for a sentinel detection.
pub(super) fn emit_sentinel_triggered(
    sink: Option<&dyn AuditSink>,
    credential_id: &CredentialId,
    recent_count: u32,
) {
    let Some(sink) = sink else { return };
    let event = AuditEvent {
        timestamp: chrono::Utc::now(),
        credential_id: credential_id.to_string(),
        operation: AuditOperation::RefreshCoordSentinelTriggered { recent_count },
        result: AuditResult::Success,
    };
    if let Err(e) = sink.record(&event) {
        tracing::warn!(
            ?e,
            cred = %credential_id,
            "refresh-coord audit sink failed for SentinelTriggered"
        );
    }
}

/// Emit an audit event for a `ReauthRequired` escalation.
pub(super) fn emit_reauth_flagged(
    sink: Option<&dyn AuditSink>,
    credential_id: &CredentialId,
    reason: &str,
) {
    let Some(sink) = sink else { return };
    let event = AuditEvent {
        timestamp: chrono::Utc::now(),
        credential_id: credential_id.to_string(),
        operation: AuditOperation::RefreshCoordReauthFlagged {
            reason: reason.to_owned(),
        },
        result: AuditResult::Success,
    };
    if let Err(e) = sink.record(&event) {
        tracing::warn!(
            ?e,
            cred = %credential_id,
            "refresh-coord audit sink failed for ReauthFlagged"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nebula_credential::StoreError;

    use super::*;

    struct CollectingSink {
        events: Mutex<Vec<AuditEvent>>,
    }

    impl CollectingSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }
        fn events(&self) -> Vec<AuditEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl AuditSink for CollectingSink {
        fn record(&self, event: &AuditEvent) -> Result<(), StoreError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn emit_claim_acquired_records_event_with_holder_and_ttl() {
        let sink = Arc::new(CollectingSink::new());
        let cid = CredentialId::new();
        emit_claim_acquired(Some(&*sink), &cid, "replica-A", 30);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].credential_id, cid.to_string());
        assert!(matches!(
            events[0].operation,
            AuditOperation::RefreshCoordClaimAcquired { ref holder, ttl_secs }
                if holder == "replica-A" && ttl_secs == 30
        ));
        assert_eq!(events[0].result, AuditResult::Success);
    }

    #[test]
    fn emit_sentinel_triggered_records_count() {
        let sink = Arc::new(CollectingSink::new());
        let cid = CredentialId::new();
        emit_sentinel_triggered(Some(&*sink), &cid, 2);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].operation,
            AuditOperation::RefreshCoordSentinelTriggered { recent_count: 2 }
        ));
    }

    #[test]
    fn emit_reauth_flagged_records_reason() {
        let sink = Arc::new(CollectingSink::new());
        let cid = CredentialId::new();
        emit_reauth_flagged(Some(&*sink), &cid, "sentinel_repeated");

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].operation,
            AuditOperation::RefreshCoordReauthFlagged { ref reason } if reason == "sentinel_repeated"
        ));
    }

    /// Sink failure must NOT panic; the warn log replaces propagation.
    #[test]
    fn emit_with_no_sink_is_noop() {
        let cid = CredentialId::new();
        emit_claim_acquired(None, &cid, "replica-A", 30);
        emit_sentinel_triggered(None, &cid, 1);
        emit_reauth_flagged(None, &cid, "sentinel_repeated");
        // No assertion needed — must not panic.
    }
}
