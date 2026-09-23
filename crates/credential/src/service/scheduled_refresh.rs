//! Credential-owned execution seam for at-least-once due-refresh delivery.

use async_trait::async_trait;
use nebula_storage_port::DueCredentialRefresh;

use crate::{CredentialLifecycleState, RetryAdvice, TenantScope};

use super::{CredentialService, CredentialServiceError};
use crate::runtime::refresh::{ScheduledRefreshDisposition, ScheduledRefreshExecutor};

#[async_trait]
impl ScheduledRefreshExecutor for CredentialService {
    async fn refresh_due(&self, candidate: DueCredentialRefresh) -> ScheduledRefreshDisposition {
        if !self.registry.is_refreshable(candidate.credential_key()) {
            return ScheduledRefreshDisposition::Unsupported;
        }

        let selector = candidate.selector();
        let scope = TenantScope::from_owner(selector.owner().clone());
        let id = selector.credential_id().to_string();
        let current = match self.get(&scope, &id).await {
            Ok(current) => current,
            Err(CredentialServiceError::NotFound { .. }) => {
                return ScheduledRefreshDisposition::NoLongerDue;
            },
            Err(_) => return ScheduledRefreshDisposition::TransientFailure,
        };
        if current.credential_key != candidate.credential_key()
            || current.expires_at != Some(candidate.expires_at())
        {
            return ScheduledRefreshDisposition::NoLongerDue;
        }
        match current.lifecycle {
            CredentialLifecycleState::Ready => {},
            CredentialLifecycleState::RefreshDeferred { .. } => {
                return ScheduledRefreshDisposition::Deferred;
            },
            CredentialLifecycleState::RefreshBlocked => {
                return ScheduledRefreshDisposition::Blocked;
            },
            CredentialLifecycleState::ReauthRequired => {
                return ScheduledRefreshDisposition::ReauthRequired;
            },
        }

        match self.refresh(&scope, &id).await {
            Ok(report) if report.refreshed => ScheduledRefreshDisposition::Refreshed,
            Ok(_) | Err(CredentialServiceError::NotFound { .. }) => {
                ScheduledRefreshDisposition::NoLongerDue
            },
            Err(CredentialServiceError::CapabilityUnsupported { .. }) => {
                ScheduledRefreshDisposition::Unsupported
            },
            Err(CredentialServiceError::RefreshNotApplied(context)) => match context.retry() {
                RetryAdvice::After(_) => ScheduledRefreshDisposition::Deferred,
                RetryAdvice::Never => ScheduledRefreshDisposition::Blocked,
            },
            Err(CredentialServiceError::ReauthRequired { .. }) => {
                ScheduledRefreshDisposition::ReauthRequired
            },
            Err(
                CredentialServiceError::OutcomeUnknown
                | CredentialServiceError::RefreshReconciliationRequired
                | CredentialServiceError::RefreshRetryGateFinalization
                | CredentialServiceError::ReauthDecisionFinalization
                | CredentialServiceError::RefreshPostProviderPersistence,
            ) => ScheduledRefreshDisposition::OutcomeUnknown,
            Err(_) => ScheduledRefreshDisposition::TransientFailure,
        }
    }
}
