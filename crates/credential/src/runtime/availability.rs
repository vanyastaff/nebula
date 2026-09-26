//! Whether a new use of a credential may be admitted.
//!
//! One credential-owned decision, shared by every path that hands material to a
//! consumer (the resolver and slot projection), so they cannot drift apart on
//! what a durable operation means for new use:
//!
//! | Operation status | New use |
//! |---|---|
//! | open, no reauthorization pending | admitted |
//! | open, reauthorization required | denied |
//! | refresh crossing the provider boundary | joined for a bounded wait, then busy |
//! | revoke in flight | denied |
//! | legacy unclassified operation in flight | denied |
//! | operation awaiting reconciliation (refresh or revoke) | denied |
//!
//! Uses already admitted keep the material they were handed; this governs only
//! new ones.

use std::time::Duration;

use nebula_storage_port::store::{
    CredentialIncidentRef, CredentialOperationKind, CredentialOperationStatus,
};

/// How long a new use waits for a refresh already crossing the provider
/// boundary before it is answered as busy.
pub(crate) const REFRESH_JOIN_WAIT: Duration = Duration::from_secs(5);
/// First re-check of a joined refresh; later re-checks back off to
/// [`REFRESH_JOIN_MAX_PAUSE`].
pub(crate) const REFRESH_JOIN_FIRST_PAUSE: Duration = Duration::from_millis(25);
/// Longest pause between re-checks of a joined refresh.
pub(crate) const REFRESH_JOIN_MAX_PAUSE: Duration = Duration::from_millis(400);
/// When a caller turned away by a refresh still in flight should try again.
pub(crate) const REFRESH_BUSY_RETRY_AFTER: Duration = Duration::from_secs(1);

/// The decision for one new use of a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialUseAvailability {
    /// The material read with this status may be used.
    Admit,
    /// A refresh is crossing the provider boundary. The use waits for it
    /// (bounded) and then re-reads, because a committed refresh supersedes the
    /// material read with this status.
    RefreshCrossing,
    /// The use is refused.
    Denied(CredentialUseDenial),
}

/// Why a new use of a credential is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CredentialUseDenial {
    /// Interactive reauthorization must complete first.
    ReauthRequired,
    /// A non-refresh operation (revoke, or one recorded before operations were
    /// typed) is crossing the provider boundary.
    OperationInFlight {
        /// The operation in flight.
        operation: CredentialOperationKind,
    },
    /// An operation's provider outcome is unknown and awaits reconciliation.
    Reconciliation {
        /// The operation whose outcome is unknown.
        operation: CredentialOperationKind,
        /// The incident a reconciliation must name.
        incident: CredentialIncidentRef,
    },
}

/// Classify a new use of a credential under `status`.
pub(crate) const fn classify_use(status: CredentialOperationStatus) -> CredentialUseAvailability {
    match status {
        CredentialOperationStatus::Open {
            reauth_required: true,
            ..
        } => CredentialUseAvailability::Denied(CredentialUseDenial::ReauthRequired),
        CredentialOperationStatus::Open { .. } => CredentialUseAvailability::Admit,
        CredentialOperationStatus::InFlight {
            operation: CredentialOperationKind::Refresh,
        } => CredentialUseAvailability::RefreshCrossing,
        CredentialOperationStatus::InFlight { operation } => {
            CredentialUseAvailability::Denied(CredentialUseDenial::OperationInFlight { operation })
        },
        CredentialOperationStatus::ReconciliationRequired {
            operation,
            incident,
        } => CredentialUseAvailability::Denied(CredentialUseDenial::Reconciliation {
            operation,
            incident,
        }),
    }
}

#[cfg(test)]
#[path = "availability_tests.rs"]
mod tests;
