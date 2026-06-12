//! Refresh-coordinator audit-event emission (sub-spec ).
//!
//! Sits on top of the credential `AuditSink` so operators reuse one sink
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
//! wrapper retains its fail-closed semantics — they
//! apply to mutating store operations, not to refresh events.

use nebula_core::CredentialId;

use crate::audit::{AuditEvent, AuditOperation, AuditResult, AuditSink};

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
