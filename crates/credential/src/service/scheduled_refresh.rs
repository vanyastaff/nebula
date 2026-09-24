//! Credential-owned execution seam for at-least-once due-refresh delivery.

use async_trait::async_trait;
use nebula_storage_port::DueCredentialRefresh;
use sha2::{Digest, Sha256};

use crate::{CredentialLifecycleState, RetryAdvice, TenantScope};

use super::{CredentialService, CredentialServiceError};
use crate::runtime::refresh::{ScheduledRefreshDisposition, ScheduledRefreshExecutor};

#[async_trait]
impl ScheduledRefreshExecutor for CredentialService {
    async fn refresh_due(&self, candidate: DueCredentialRefresh) -> ScheduledRefreshDisposition {
        if !self.registry.is_refreshable(candidate.credential_key()) {
            return ScheduledRefreshDisposition::Unsupported;
        }
        let Some(policy) = self.registry.refresh_policy(candidate.credential_key()) else {
            return ScheduledRefreshDisposition::Unsupported;
        };

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
        if !is_due_with_policy(&candidate, policy) {
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
            Err(error) => exact_failure_disposition(&error)
                .unwrap_or(ScheduledRefreshDisposition::TransientFailure),
        }
    }
}

fn exact_failure_disposition(
    error: &CredentialServiceError,
) -> Option<ScheduledRefreshDisposition> {
    match error {
        CredentialServiceError::OutcomeUnknown => Some(ScheduledRefreshDisposition::OutcomeUnknown),
        CredentialServiceError::RefreshReconciliationRequired => {
            Some(ScheduledRefreshDisposition::ReconciliationRequired)
        },
        CredentialServiceError::RefreshRetryGateFinalization => {
            Some(ScheduledRefreshDisposition::RetryGateFinalization)
        },
        CredentialServiceError::ReauthDecisionFinalization => {
            Some(ScheduledRefreshDisposition::ReauthDecisionFinalization)
        },
        CredentialServiceError::RefreshPostProviderPersistence => {
            Some(ScheduledRefreshDisposition::PostProviderPersistence)
        },
        _ => None,
    }
}

fn is_due_with_policy(candidate: &DueCredentialRefresh, policy: crate::RefreshPolicy) -> bool {
    let jitter_nanos = policy.jitter.as_nanos();
    let jitter = if jitter_nanos == 0 {
        std::time::Duration::ZERO
    } else {
        let digest = Sha256::digest(candidate.selector().credential_id().to_string().as_bytes());
        let sample = u128::from_be_bytes(digest[..16].try_into().unwrap_or([0; 16]));
        let nanos = sample % jitter_nanos;
        std::time::Duration::new(
            u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX),
            u32::try_from(nanos % 1_000_000_000).unwrap_or_default(),
        )
    };
    let window = policy
        .early_refresh
        .checked_add(jitter)
        .unwrap_or(std::time::Duration::MAX);
    let Ok(window) = chrono::Duration::from_std(window) else {
        return true;
    };
    candidate.expires_at() <= candidate.observed_at() + window
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeDelta, Utc};
    use nebula_storage_port::{CredentialId, CredentialOwner, CredentialSelector};

    use super::*;

    fn candidate(
        id: CredentialId,
        expires_at: chrono::DateTime<Utc>,
        observed_at: chrono::DateTime<Utc>,
    ) -> DueCredentialRefresh {
        DueCredentialRefresh::new(
            CredentialSelector::new(CredentialOwner::from_canonical("policy-test-owner"), id),
            "policy-test".to_owned(),
            expires_at,
            observed_at,
        )
    }

    #[test]
    fn policy_gate_honours_each_credential_early_refresh_window() {
        let now =
            chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("fixture timestamp is valid");
        let due = candidate(CredentialId::new(), now + TimeDelta::minutes(9), now);

        assert!(is_due_with_policy(
            &due,
            crate::RefreshPolicy {
                early_refresh: Duration::from_mins(10),
                min_retry_backoff: Duration::ZERO,
                jitter: Duration::ZERO,
            },
        ));
        assert!(!is_due_with_policy(
            &due,
            crate::RefreshPolicy {
                early_refresh: Duration::from_mins(5),
                min_retry_backoff: Duration::ZERO,
                jitter: Duration::ZERO,
            },
        ));
    }

    #[test]
    fn jitter_is_stable_for_the_same_credential() {
        let now =
            chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("fixture timestamp is valid");
        let due = candidate(CredentialId::new(), now + TimeDelta::minutes(6), now);
        let policy = crate::RefreshPolicy {
            early_refresh: Duration::from_mins(5),
            min_retry_backoff: Duration::ZERO,
            jitter: Duration::from_mins(2),
        };

        let first = is_due_with_policy(&due, policy);
        assert_eq!(is_due_with_policy(&due, policy), first);
    }

    #[test]
    fn policy_gate_uses_the_backend_clock_sample() {
        let backend_now =
            chrono::DateTime::from_timestamp(1_800_000_000, 0).expect("fixture timestamp is valid");
        let due = candidate(
            CredentialId::new(),
            backend_now + TimeDelta::minutes(4),
            backend_now,
        );

        assert!(is_due_with_policy(
            &due,
            crate::RefreshPolicy {
                early_refresh: Duration::from_mins(5),
                min_retry_backoff: Duration::ZERO,
                jitter: Duration::ZERO,
            },
        ));
    }

    #[test]
    fn exact_finalization_failures_keep_distinct_scheduler_dispositions() {
        let cases = [
            (
                CredentialServiceError::OutcomeUnknown,
                ScheduledRefreshDisposition::OutcomeUnknown,
            ),
            (
                CredentialServiceError::RefreshReconciliationRequired,
                ScheduledRefreshDisposition::ReconciliationRequired,
            ),
            (
                CredentialServiceError::RefreshRetryGateFinalization,
                ScheduledRefreshDisposition::RetryGateFinalization,
            ),
            (
                CredentialServiceError::ReauthDecisionFinalization,
                ScheduledRefreshDisposition::ReauthDecisionFinalization,
            ),
            (
                CredentialServiceError::RefreshPostProviderPersistence,
                ScheduledRefreshDisposition::PostProviderPersistence,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(exact_failure_disposition(&error), Some(expected));
        }
    }
}
