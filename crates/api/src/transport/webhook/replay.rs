//! Replay-window rejection reason mapping (M3.3 / ADR-0049).
//!
//! Maps a signature-failure reason code to the corresponding
//! replay-window rejection reason code used by the
//! [`nebula_metrics::NEBULA_WEBHOOK_REPLAY_REJECTIONS_TOTAL`] counter.
//! Only timestamp-related failure modes map to a replay reason; all
//! other failure modes (missing signature, invalid HMAC, missing secret)
//! return `None`.
//!
//! The `NEBULA_WEBHOOK_REPLAY_REJECTIONS_TOTAL` counter is
//! double-bumped by [`super::signature::record_signature_failure`] for
//! timestamp-related failures so dashboards can isolate replay-window
//! enforcement from generic signature mismatches without scraping the
//! `reason` label on the signature counter.

use nebula_metrics::{webhook_replay_rejection_reason, webhook_signature_failure_reason};

/// Map a signature-failure reason to a replay-window rejection reason
/// when the failure is timestamp-related. Returns `None` for failure
/// modes unrelated to replay enforcement (missing signature, signature
/// invalid, missing secret).
pub(super) fn replay_reason_for(reason: &str) -> Option<&'static str> {
    match reason {
        r if r == webhook_signature_failure_reason::TIMESTAMP_OUT_OF_WINDOW => {
            Some(webhook_replay_rejection_reason::TIMESTAMP_OUT_OF_WINDOW)
        },
        r if r == webhook_signature_failure_reason::TIMESTAMP_MISSING => {
            Some(webhook_replay_rejection_reason::TIMESTAMP_MISSING)
        },
        r if r == webhook_signature_failure_reason::TIMESTAMP_MALFORMED => {
            Some(webhook_replay_rejection_reason::TIMESTAMP_MALFORMED)
        },
        _ => None,
    }
}
