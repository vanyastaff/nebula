//! Linear evidence for credential refresh attempts.
//!
//! A refresh implementation receives exactly one [`RefreshAttempt`]. Consuming
//! that value is the only supported way to report whether provider dispatch
//! never began, completed with a known response, or has an unknown outcome.

use std::future::Future;

use crate::{
    CredentialContext, ReauthReason,
    contract::RefreshExecutionMode,
    error::{
        RefreshDiagnosticCode, RefreshErrorKind, RefreshFailureSpec, RefreshNotAppliedContext,
        RefreshNotAppliedPhase, RetryAdvice,
    },
};

const EXECUTION_MODE_MISMATCH_CODE: &str = "refresh.execution_mode_mismatch";

fn execution_mode_mismatch_report() -> RefreshReport {
    let failure = RefreshFailureSpec::new(RefreshErrorKind::ProtocolError, RetryAdvice::Never);
    // The value is a fixed framework-owned literal whose shape is covered by
    // the contract test below. Keep the parse boundary panic-free so a future
    // diagnostic-code tightening cannot turn a declaration error into a
    // process abort.
    let failure = match RefreshDiagnosticCode::parse(EXECUTION_MODE_MISMATCH_CODE) {
        Ok(code) => failure.with_diagnostic_code(code),
        Err(_) => failure,
    };
    RefreshReport(RefreshReportKind::NotApplied(Box::new(
        RefreshNotAppliedContext::from_spec(RefreshNotAppliedPhase::BeforeDispatch, failure),
    )))
}

/// Runtime-created capability for one credential refresh attempt.
///
/// The value is deliberately neither `Clone` nor `Copy`. Dispatch consumes it,
/// so a transport failure cannot subsequently be reclassified as a
/// replay-safe pre-dispatch failure.
#[must_use = "a refresh attempt must be converted into a refresh report"]
pub struct RefreshAttempt<'ctx> {
    context: &'ctx CredentialContext,
    execution_mode: RefreshExecutionMode,
}

impl<'ctx> RefreshAttempt<'ctx> {
    pub(crate) const fn new(
        context: &'ctx CredentialContext,
        execution_mode: RefreshExecutionMode,
    ) -> Self {
        Self {
            context,
            execution_mode,
        }
    }

    /// Context for request preparation and provider transport access.
    #[must_use]
    pub const fn context(&self) -> &'ctx CredentialContext {
        self.context
    }

    /// Report a failure proven to have occurred before dispatch began.
    pub fn not_dispatched(self, failure: RefreshFailureSpec) -> RefreshReport {
        RefreshReport(RefreshReportKind::NotApplied(Box::new(
            RefreshNotAppliedContext::from_spec(RefreshNotAppliedPhase::BeforeDispatch, failure),
        )))
    }

    /// Conservatively report that the provider-side outcome is unknown.
    pub const fn outcome_unknown(self) -> RefreshReport {
        RefreshReport(RefreshReportKind::OutcomeUnknown)
    }

    /// Report a completed providerless refresh.
    ///
    /// This is intended for credential kinds whose refresh transition is
    /// entirely local. Provider-backed implementations should use
    /// [`dispatch`](Self::dispatch).
    pub fn local_refresh_completed(self) -> RefreshReport {
        match self.execution_mode {
            RefreshExecutionMode::Local => RefreshReport(RefreshReportKind::LocallyRefreshed),
            RefreshExecutionMode::Provider => execution_mode_mismatch_report(),
        }
    }

    /// Report that local state lacks the material required to refresh.
    pub const fn missing_refresh_material(self) -> RefreshReport {
        RefreshReport(RefreshReportKind::ReauthRequired {
            reason: ReauthReason::MissingRefreshMaterial,
            phase: RefreshReauthPhase::BeforeDispatch,
        })
    }

    /// Cross the provider dispatch boundary.
    ///
    /// In provider mode, any error returned by `operation` is intentionally
    /// discarded and represented only by
    /// [`RefreshDispatchError::OutcomeUnknown`]. In local mode the closure is
    /// not invoked at all and [`RefreshDispatchError::ModeMismatch`] is
    /// returned, preserving exact before-dispatch evidence. A completed
    /// provider operation yields response evidence that can classify the
    /// response.
    pub async fn dispatch<T, E, Operation, OperationFuture>(
        self,
        operation: Operation,
    ) -> Result<CompletedDispatch<T>, RefreshDispatchError>
    where
        Operation: FnOnce() -> OperationFuture + Send,
        OperationFuture: Future<Output = Result<T, E>> + Send,
    {
        if matches!(self.execution_mode, RefreshExecutionMode::Local) {
            return Err(RefreshDispatchError::ModeMismatch);
        }
        match operation().await {
            Ok(response) => Ok(CompletedDispatch {
                response,
                proof: CompletedResponseProof(()),
            }),
            Err(_) => Err(RefreshDispatchError::OutcomeUnknown),
        }
    }
}

/// A provider dispatch that produced a complete response.
#[must_use = "the completed response must be classified into a refresh report"]
pub struct CompletedDispatch<T> {
    response: T,
    proof: CompletedResponseProof,
}

impl<T> CompletedDispatch<T> {
    /// Consume the dispatch result into the response and its linear proof.
    pub fn into_parts(self) -> (T, CompletedResponseProof) {
        (self.response, self.proof)
    }
}

/// Evidence that provider dispatch produced a complete response.
#[must_use = "completed response evidence must be converted into a refresh report"]
pub struct CompletedResponseProof(());

impl CompletedResponseProof {
    /// Report that the complete response was applied to local state.
    pub const fn refreshed(self) -> RefreshReport {
        RefreshReport(RefreshReportKind::ProviderRefreshed)
    }

    /// Report that the provider rejected the refresh grant.
    pub const fn provider_rejected(self) -> RefreshReport {
        RefreshReport(RefreshReportKind::ReauthRequired {
            reason: ReauthReason::ProviderRejected,
            phase: RefreshReauthPhase::ProviderConfirmed,
        })
    }

    /// Report that the complete response proves no provider effect occurred.
    pub fn confirmed_not_applied(self, failure: RefreshFailureSpec) -> RefreshReport {
        RefreshReport(RefreshReportKind::NotApplied(Box::new(
            RefreshNotAppliedContext::from_spec(
                RefreshNotAppliedPhase::ProviderConfirmedNotApplied,
                failure,
            ),
        )))
    }

    /// Report that the complete response cannot prove the provider outcome.
    pub const fn outcome_unknown(self) -> RefreshReport {
        RefreshReport(RefreshReportKind::OutcomeUnknown)
    }
}

/// Failure to establish completed provider-response evidence.
#[derive(Debug)]
#[must_use = "a dispatch failure must be converted into a refresh report"]
#[non_exhaustive]
pub enum RefreshDispatchError {
    /// The credential declared a providerless local refresh but attempted to
    /// cross the provider boundary. The operation closure was not invoked.
    ModeMismatch,
    /// Provider dispatch began but its outcome cannot be proven.
    OutcomeUnknown,
}

impl RefreshDispatchError {
    /// Convert the failure into its phase-correct fail-closed report.
    pub fn into_report(self) -> RefreshReport {
        match self {
            Self::ModeMismatch => execution_mode_mismatch_report(),
            Self::OutcomeUnknown => RefreshReport(RefreshReportKind::OutcomeUnknown),
        }
    }
}

/// Opaque disposition returned by [`crate::Refreshable::refresh`].
///
/// Only linear refresh evidence can construct this value. The runtime consumes
/// it through a crate-private exhaustive inspection.
#[must_use = "the runtime must consume the refresh disposition"]
pub struct RefreshReport(RefreshReportKind);

impl RefreshReport {
    pub(crate) fn into_kind(self) -> RefreshReportKind {
        self.0
    }
}

pub(crate) enum RefreshReportKind {
    ProviderRefreshed,
    LocallyRefreshed,
    ReauthRequired {
        reason: ReauthReason,
        phase: RefreshReauthPhase,
    },
    NotApplied(Box<RefreshNotAppliedContext>),
    OutcomeUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshReauthPhase {
    BeforeDispatch,
    ProviderConfirmed,
}

// These witnesses encode one-way phase transitions. Accidentally deriving
// `Clone` or `Copy` would let an implementation retain an earlier-phase proof
// after crossing the provider boundary.
static_assertions::assert_not_impl_any!(RefreshAttempt<'static>: Clone, Copy);
static_assertions::assert_not_impl_any!(CompletedDispatch<()>: Clone, Copy);
static_assertions::assert_not_impl_any!(CompletedResponseProof: Clone, Copy);
static_assertions::assert_not_impl_any!(RefreshDispatchError: Clone, Copy);
static_assertions::assert_not_impl_any!(RefreshReport: Clone, Copy);
static_assertions::assert_not_impl_any!(RefreshNotAppliedContext: Clone, Copy);

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    #[tokio::test]
    async fn local_mode_cannot_cross_or_fake_the_provider_boundary() {
        let context = CredentialContext::for_owner("owner");
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_operation = Arc::clone(&calls);

        let result = RefreshAttempt::new(&context, RefreshExecutionMode::Local)
            .dispatch(move || async move {
                calls_in_operation.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(())
            })
            .await;
        let Err(failure) = result else {
            panic!("local mode must reject provider dispatch");
        };

        assert!(matches!(failure, RefreshDispatchError::ModeMismatch));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let report = failure.into_report().into_kind();
        assert!(
            matches!(report, RefreshReportKind::NotApplied(_)),
            "a local/provider mode mismatch must remain before-dispatch, never ProviderRefreshed"
        );
    }

    #[test]
    fn provider_mode_cannot_report_a_local_refresh_as_outcome_unknown() {
        let context = CredentialContext::for_owner("owner");

        let report = RefreshAttempt::new(&context, RefreshExecutionMode::Provider)
            .local_refresh_completed()
            .into_kind();
        let RefreshReportKind::NotApplied(context) = report else {
            panic!("the unused provider boundary proves an exact before-dispatch refusal");
        };

        assert_eq!(context.phase(), RefreshNotAppliedPhase::BeforeDispatch);
        assert_eq!(context.kind(), RefreshErrorKind::ProtocolError);
        assert_eq!(context.retry(), RetryAdvice::Never);
        assert_eq!(
            context.diagnostic_code().map(RefreshDiagnosticCode::as_str),
            Some("refresh.execution_mode_mismatch")
        );
    }
}
