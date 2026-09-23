//! Secret-free durable discovery of credentials due for automatic refresh.

use std::{fmt, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{CredentialId, CredentialSelector};

/// Maximum look-ahead accepted by the scheduling port: 365 days.
pub const MAX_CREDENTIAL_REFRESH_HORIZON_SECS: u64 = 365 * 24 * 60 * 60;

/// Backend-clock look-ahead used to discover credentials approaching expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialRefreshHorizon(Duration);

impl Default for CredentialRefreshHorizon {
    fn default() -> Self {
        Self(Duration::from_mins(5))
    }
}

impl CredentialRefreshHorizon {
    /// Construct a bounded horizon, conservatively rounding fractions up.
    pub fn new(value: Duration) -> Result<Self, CredentialRefreshHorizonError> {
        let seconds = value
            .as_secs()
            .checked_add(u64::from(value.subsec_nanos() != 0))
            .ok_or(CredentialRefreshHorizonError)?;
        if seconds > MAX_CREDENTIAL_REFRESH_HORIZON_SECS {
            return Err(CredentialRefreshHorizonError);
        }
        Ok(Self(Duration::from_secs(seconds)))
    }

    /// Return the normalized whole-second horizon.
    #[must_use]
    pub const fn get(self) -> Duration {
        self.0
    }
}

/// The requested refresh horizon exceeds the supported bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("credential refresh schedule horizon exceeds the supported bound")]
pub struct CredentialRefreshHorizonError;

/// Bounded number of candidates returned by one scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialRefreshPageSize(u16);

impl Default for CredentialRefreshPageSize {
    fn default() -> Self {
        Self(100)
    }
}

impl CredentialRefreshPageSize {
    /// Largest admitted scan page.
    pub const MAX: u16 = 1_000;

    /// Construct a non-zero bounded page size.
    pub const fn new(value: u16) -> Result<Self, CredentialRefreshPageSizeError> {
        if value == 0 || value > Self::MAX {
            return Err(CredentialRefreshPageSizeError);
        }
        Ok(Self(value))
    }

    /// Return the admitted page size.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Refresh schedule page size lies outside `1..=1000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("credential refresh schedule page size is outside the supported range")]
pub struct CredentialRefreshPageSizeError;

/// Stable keyset cursor for a due-credential scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRefreshCursor {
    expires_at: DateTime<Utc>,
    credential_id: CredentialId,
}

impl CredentialRefreshCursor {
    /// Construct a cursor from the last candidate in a page.
    #[must_use]
    pub const fn new(expires_at: DateTime<Utc>, credential_id: CredentialId) -> Self {
        Self {
            expires_at,
            credential_id,
        }
    }

    /// Return the expiry component of the stable ordering key.
    #[must_use]
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Return the credential-id component of the stable ordering key.
    #[must_use]
    pub const fn credential_id(&self) -> CredentialId {
        self.credential_id
    }
}

/// One secret-free credential candidate due for automatic refresh.
#[derive(Clone, PartialEq, Eq)]
pub struct DueCredentialRefresh {
    selector: CredentialSelector,
    credential_key: String,
    expires_at: DateTime<Utc>,
}

impl DueCredentialRefresh {
    /// Construct a due candidate from a backend row.
    #[must_use]
    pub fn new(
        selector: CredentialSelector,
        credential_key: String,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            selector,
            credential_key,
            expires_at,
        }
    }

    /// Borrow the mandatory owner-qualified selector.
    #[must_use]
    pub const fn selector(&self) -> &CredentialSelector {
        &self.selector
    }

    /// Borrow the registered credential type key.
    #[must_use]
    pub fn credential_key(&self) -> &str {
        &self.credential_key
    }

    /// Return the material expiry that made this credential due.
    #[must_use]
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Return the stable cursor naming this candidate's position.
    #[must_use]
    pub fn cursor(&self) -> CredentialRefreshCursor {
        CredentialRefreshCursor::new(self.expires_at, self.selector.credential_id())
    }
}

impl fmt::Debug for DueCredentialRefresh {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DueCredentialRefresh")
            .field("credential_key", &self.credential_key)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Closed, payload-free due-scan failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialRefreshScheduleError {
    /// Backend access or query execution failed.
    #[error("credential refresh schedule is unavailable")]
    Unavailable,
    /// A selected row violates the closed schedule projection contract.
    #[error("credential refresh schedule record is corrupt")]
    CorruptRecord,
}

/// Read-only backend-clock discovery of credentials due for automatic refresh.
///
/// Delivery is intentionally at-least-once. The credential refresh claim is
/// the sole cross-replica dispatch fence; callers must pass every candidate
/// through that coordinator before provider egress.
#[async_trait]
pub trait CredentialRefreshSchedule: Send + Sync + fmt::Debug + 'static {
    /// Return one stable keyset page ordered by `(expires_at, credential_id)`.
    ///
    /// Tombstones, reauthentication-required rows, permanent retry blocks,
    /// and retry deferrals whose backend-authored deadline has not elapsed are
    /// omitted. Both the due cutoff and retry admission use one backend clock.
    async fn scan_due(
        &self,
        after: Option<&CredentialRefreshCursor>,
        horizon: CredentialRefreshHorizon,
        limit: CredentialRefreshPageSize,
    ) -> Result<Vec<DueCredentialRefresh>, CredentialRefreshScheduleError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizon_rounds_up_and_page_size_is_bounded() {
        assert_eq!(
            CredentialRefreshHorizon::new(Duration::from_nanos(1))
                .expect("fractional horizon rounds up")
                .get(),
            Duration::from_secs(1)
        );
        assert!(
            CredentialRefreshHorizon::new(Duration::from_secs(
                MAX_CREDENTIAL_REFRESH_HORIZON_SECS + 1
            ))
            .is_err()
        );
        assert!(CredentialRefreshPageSize::new(0).is_err());
        assert!(CredentialRefreshPageSize::new(CredentialRefreshPageSize::MAX).is_ok());
        assert!(CredentialRefreshPageSize::new(CredentialRefreshPageSize::MAX + 1).is_err());
    }

    #[test]
    fn candidate_debug_omits_owner_and_credential_id() {
        let owner = crate::CredentialOwner::from_canonical("secret-owner-canary");
        let id = CredentialId::new();
        let candidate = DueCredentialRefresh::new(
            CredentialSelector::new(owner, id),
            "oauth2".to_owned(),
            DateTime::from_timestamp(1_800_000_000, 0).expect("fixture timestamp is valid"),
        );
        let debug = format!("{candidate:?}");
        assert!(!debug.contains("secret-owner-canary"));
        assert!(!debug.contains(&id.to_string()));
    }
}
