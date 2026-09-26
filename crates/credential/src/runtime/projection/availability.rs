//! Secret-free availability observation of a bound credential.
//!
//! A resource row bound to a credential needs to know, cheaply and often,
//! whether the credential may be used right now at the material it already
//! holds: a credential can deny use without changing material (it needs
//! reauthentication, or a revoke or an unreconciled operation blocks it). The
//! observer answers from **one** operational-head read — the same
//! owner-qualified, secret-free checks slot projection runs before it decrypts
//! anything — and never loads or decrypts material. Consumers compare the
//! observed `(material_epoch, revision)` with what they installed and project
//! (decrypt) only when the material actually changed.

use std::{future::Future, pin::Pin};

use nebula_storage_port::store::CredentialOperationKind;
use tokio_util::sync::CancellationToken;

use crate::runtime::availability::{CredentialUseAvailability, CredentialUseDenial, classify_use};
use crate::runtime::state_source::StateSource;
use crate::{CredentialId, CredentialKey, CredentialPersistenceError, TenantScope};

/// Whether a new use of the credential may be admitted now.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialAvailability {
    /// New uses are admitted at the observed material.
    Available,
    /// A refresh is crossing the provider boundary. Not a block: the refresh
    /// either commits new material or leaves the current one usable.
    RefreshInFlight,
    /// New uses are refused until the block clears.
    Blocked(CredentialBlock),
}

/// Why a credential refuses new uses at its current material.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialBlock {
    /// Interactive reauthentication must complete first.
    ReauthRequired,
    /// A non-refresh operation (a revoke) is crossing the provider boundary.
    OperationInFlight {
        /// The operation in flight.
        operation: CredentialOperationKind,
    },
    /// An operation's provider outcome is unknown and awaits reconciliation.
    ReconciliationRequired {
        /// The operation whose outcome is unknown.
        operation: CredentialOperationKind,
    },
}

/// One secret-free observation of a credential's availability and the
/// material it was observed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialAvailabilityObservation {
    material_epoch: u64,
    revision: u64,
    availability: CredentialAvailability,
}

impl CredentialAvailabilityObservation {
    /// Construct an observation for a trusted observer adapter.
    #[must_use]
    pub const fn new(
        material_epoch: u64,
        revision: u64,
        availability: CredentialAvailability,
    ) -> Self {
        Self {
            material_epoch,
            revision,
            availability,
        }
    }

    /// Backend-authored material epoch observed.
    #[must_use]
    pub const fn material_epoch(&self) -> u64 {
        self.material_epoch
    }

    /// Persisted aggregate revision observed with the material epoch.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether new uses are admitted.
    #[must_use]
    pub const fn availability(&self) -> CredentialAvailability {
        self.availability
    }
}

/// Closed, secret-free failures of [`CredentialAvailabilityObserver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialObserveError {
    /// No live operational head: the id is absent, belongs to another
    /// tenant, or the credential is tombstoned. A consumer that must tell a
    /// tombstone apart resolves the slot.
    #[error("credential has no live operational head")]
    Absent,
    /// The stored credential contract differs from the expected one.
    #[error("credential key does not match slot contract")]
    WrongCredentialKey,
    /// Credential persistence is temporarily unavailable.
    #[error("credential persistence is temporarily unavailable")]
    Unavailable,
    /// The configured external credential source is not available.
    #[error("credential source is unavailable")]
    SourceUnavailable,
    /// Stored state is inconsistent.
    #[error("credential state is invalid")]
    InvalidState,
    /// The observation was cancelled.
    #[error("credential observation cancelled")]
    Cancelled,
}

/// Object-safe, secret-free availability read of an owner-qualified
/// credential. Exactly one operational-head read; no material load, no
/// decryption, no projection.
pub trait CredentialAvailabilityObserver: Send + Sync {
    /// Observe whether `credential_id` may be used now, and at which material.
    fn observe_availability<'a>(
        &'a self,
        scope: &'a TenantScope,
        credential_id: CredentialId,
        expected_key: CredentialKey,
        cancel: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<CredentialAvailabilityObservation, CredentialObserveError>>
                + Send
                + 'a,
        >,
    >;
}

/// Shared implementation behind every observer: the owner-qualified head
/// checks of slot projection, stopping before any material read.
pub(crate) async fn observe_availability_with(
    store: &dyn crate::CredentialPersistence,
    source: &StateSource,
    scope: &TenantScope,
    credential_id: CredentialId,
    expected_key: CredentialKey,
    cancel: CancellationToken,
) -> Result<CredentialAvailabilityObservation, CredentialObserveError> {
    if !matches!(source, StateSource::LocalEncrypted) {
        return Err(CredentialObserveError::SourceUnavailable);
    }
    let selector = scope.selector(credential_id);
    let operational_head = cancel
        .run_until_cancelled(store.get_operational_head(&selector))
        .await
        .ok_or(CredentialObserveError::Cancelled)?
        .map_err(|error| match error {
            CredentialPersistenceError::NotFound => CredentialObserveError::Absent,
            CredentialPersistenceError::Unavailable
            | CredentialPersistenceError::OutcomeUnknown => CredentialObserveError::Unavailable,
            _ => CredentialObserveError::InvalidState,
        })?;
    let head = operational_head.head();
    let actual_key = CredentialKey::new(head.credential_key())
        .map_err(|_| CredentialObserveError::InvalidState)?;
    if actual_key != expected_key {
        return Err(CredentialObserveError::WrongCredentialKey);
    }
    let availability = match classify_use(operational_head.status()) {
        CredentialUseAvailability::Admit if head.reauth_required() => {
            CredentialAvailability::Blocked(CredentialBlock::ReauthRequired)
        },
        CredentialUseAvailability::Admit => CredentialAvailability::Available,
        CredentialUseAvailability::RefreshCrossing => CredentialAvailability::RefreshInFlight,
        CredentialUseAvailability::Denied(denial) => {
            CredentialAvailability::Blocked(match denial {
                CredentialUseDenial::ReauthRequired => CredentialBlock::ReauthRequired,
                CredentialUseDenial::OperationInFlight { operation } => {
                    CredentialBlock::OperationInFlight { operation }
                },
                CredentialUseDenial::Reconciliation { operation, .. } => {
                    CredentialBlock::ReconciliationRequired { operation }
                },
            })
        },
    };
    Ok(CredentialAvailabilityObservation {
        material_epoch: head.material_epoch().get() as u64,
        revision: head.version().get() as u64,
        availability,
    })
}

#[cfg(test)]
#[path = "availability_tests.rs"]
mod tests;
