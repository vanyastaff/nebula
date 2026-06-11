//! Audit trait and value types for credential operations.
//!
//! Defines the [`AuditSink`] trait and its associated value types
//! ([`AuditEvent`], [`AuditOperation`], [`AuditResult`]) at the credential
//! contract layer so credential-tier code (e.g. the refresh coordinator) can
//! emit audit events without an upward dependency on `nebula-storage`.
//!
//! The audit **decorator** ([`nebula_storage::credential::AuditLayer`]) stays
//! in `nebula-storage` and imports these types from here.
//!
//! # Contract
//!
//! `AuditLayer` wraps any [`crate::CredentialStore`] and delegates every
//! operation to the inner store, emitting an [`AuditEvent`] to the pluggable
//! [`AuditSink`] for each call. Only metadata flows through the sink —
//! credential data never does.
//!
//! # Fail-closed invariant (no discard-and-log)
//!
//! Audit is **in-line durable**: if [`AuditSink::record`] returns an error,
//! the credential operation as a whole returns
//! [`crate::StoreError::AuditFailure`]. There is no "log-and-continue" path.

use crate::StoreError;

/// Receives audit events for logging or persistence.
///
/// Implementations might write to a file, send to an event bus, or
/// collect events in memory for testing.
///
/// # Contract
///
/// - `record` must not block the calling task for extended periods.
/// - Implementations must never inspect or log credential data.
/// - Returning `Err(StoreError)` causes the wrapping `AuditLayer` to fail the
///   whole credential operation with [`crate::StoreError::AuditFailure`]
///   (fail-closed audit contract).
pub trait AuditSink: Send + Sync {
    /// Record an audit event.
    ///
    /// # Errors
    ///
    /// Return an error when the event cannot be durably persisted.
    /// The wrapping `AuditLayer` will surface this as
    /// [`crate::StoreError::AuditFailure`] — no silent discard.
    fn record(&self, event: &AuditEvent) -> Result<(), StoreError>;
}

/// A credential store operation recorded for audit purposes.
///
/// Contains only metadata — never credential data or secrets.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// When the operation occurred.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// The credential ID involved (`"*"` for list operations).
    pub credential_id: String,
    /// What operation was performed.
    pub operation: AuditOperation,
    /// Outcome of the operation.
    pub result: AuditResult,
}

/// Type of credential store operation.
///
/// Variants without payloads describe `CredentialStore` operations
/// flowing through `AuditLayer`. Variants prefixed `RefreshCoord*`
/// describe events emitted by the engine's two-tier refresh coordinator
/// (sub-spec `docs/INTEGRATION_MODEL.md` (credential refresh coordinator)
/// §6) and carry their structured payload as enum fields. The same
/// [`AuditSink`] receives both families so operators reuse one sink
/// implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuditOperation {
    /// A credential was retrieved.
    Get,
    /// A credential was stored or updated.
    Put,
    /// A credential was deleted.
    Delete,
    /// Credential IDs were listed.
    List,
    /// A credential existence check was performed.
    Exists,
    /// L2 refresh claim acquired by `holder` for `ttl_secs`.
    /// Sub-spec §6 audit event.
    RefreshCoordClaimAcquired {
        /// Replica that holds the claim.
        holder: String,
        /// TTL applied to the claim row.
        ttl_secs: u64,
    },
    /// Sentinel event recorded for a credential — a holder crashed
    /// mid-refresh and the reclaim sweep observed the residual
    /// `RefreshInFlight` state. `recent_count` is the rolling-window
    /// count after the new event was inserted. Sub-spec §6 audit event.
    RefreshCoordSentinelTriggered {
        /// Sentinel event count in the rolling window after this
        /// detection (includes the new event).
        recent_count: u32,
    },
    /// Credential transitioned to `ReauthRequired` after the sentinel
    /// threshold was crossed. `reason` is the textual form of the
    /// `ReauthReason` published on the event bus. Sub-spec §6 audit
    /// event.
    RefreshCoordReauthFlagged {
        /// Sanitized reason string. For sentinel-driven escalations:
        /// `"sentinel_repeated"` (the `ReauthReason::SentinelRepeated`
        /// arm's stable identifier).
        reason: String,
    },
}

/// Outcome of an audited operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuditResult {
    /// The operation completed successfully.
    Success,
    /// The requested credential was not found.
    NotFound,
    /// A version or existence conflict occurred.
    Conflict,
    /// The operation failed with a sanitized error message (no secrets).
    Error(String),
}
