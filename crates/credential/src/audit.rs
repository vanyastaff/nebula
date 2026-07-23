//! Audit trait and value types for credential operations.
//!
//! Defines the [`AuditSink`] trait and its associated value types
//! ([`AuditEvent`], [`AuditOperation`], [`AuditResult`]) at the credential
//! contract layer so credential-tier code (e.g. the refresh coordinator) can
//! emit audit events without an upward dependency on `nebula-storage`.
//!
//! The audit **decorator** (`nebula_storage::credential::AuditLayer`) stays
//! in `nebula-storage` and imports these types from here.
//!
//! # Contract
//!
//! `AuditLayer` wraps any [`crate::CredentialPersistence`] and delegates every
//! operation to the inner store, emitting an [`AuditEvent`] to the pluggable
//! [`AuditSink`] for each call. Only metadata flows through the sink —
//! credential data never does.
//!
//! # Observation boundary
//!
//! This interim sink is a non-authoritative observation. It does not share a
//! transaction with credential persistence, so a recording failure must never
//! turn a confirmed store mutation into an error, trigger a retry, or attempt
//! compensation. Atomic mutation-plus-audit evidence requires the K3
//! backend-owned outbox/operation ledger.

use crate::CredentialPersistenceError;

/// Receives audit events for logging or persistence.
///
/// Implementations might write to a file, send to an event bus, or
/// collect events in memory for testing.
///
/// # Contract
///
/// - `record` must not block the calling task for extended periods.
/// - Implementations must never inspect or log credential data.
/// - Returning `Err(CredentialPersistenceError)` asks the wrapping `AuditLayer`
///   to emit bounded failure telemetry. It never changes the persistence
///   result.
pub trait AuditSink: Send + Sync {
    /// Record an audit event.
    ///
    /// # Errors
    ///
    /// Return an error when the event cannot be accepted by this sink. The
    /// sink's own contract determines whether acceptance means durable
    /// persistence, an outbox append, or structured-log delivery.
    /// The wrapping `AuditLayer` observes the error through bounded telemetry
    /// and preserves the authoritative persistence result.
    fn record(&self, event: &AuditEvent) -> Result<(), CredentialPersistenceError>;
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
/// Variants without payloads describe `CredentialPersistence` operations
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
    /// A new live credential was created.
    Create,
    /// Mutable state of a live credential was replaced.
    Replace,
    /// A live credential transitioned to a terminal tombstone.
    Tombstone,
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
    /// The sentinel threshold was crossed and the reclaim sweep emitted
    /// its reauthentication-required observation. This audit operation
    /// does not claim that the credential row was durably mutated.
    /// `reason` is the textual form of the `ReauthReason` observation.
    RefreshCoordReauthThresholdReached {
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
