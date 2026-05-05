//! Shared execution context for composed resilience policies.

use std::{future::Future, time::Duration};

use crate::{CallError, CancellationContext, Deadline, sink::PolicyScope};

/// Execution context shared by a resilience policy stack.
///
/// A workflow runtime often has one cancellation token, one action deadline, and
/// one low-cardinality scope for a protected call. Passing those as separate
/// parameters makes composition easy to misuse. `PolicyContext` groups them into
/// one value that can be threaded through pipeline execution and future
/// standalone policy APIs.
#[derive(Debug, Clone)]
pub struct PolicyContext {
    cancellation: Option<CancellationContext>,
    deadline: Option<Deadline>,
    scope: PolicyScope,
}

impl PolicyContext {
    /// Create an empty policy context.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            cancellation: None,
            deadline: None,
            scope: PolicyScope::empty(),
        }
    }

    /// Create a context with cancellation.
    #[must_use]
    pub fn from_cancellation(cancellation: CancellationContext) -> Self {
        Self::empty().with_cancellation(cancellation)
    }

    /// Create a context with a deadline starting now.
    #[must_use]
    pub fn with_timeout(timeout: Duration) -> Self {
        Self::empty().with_deadline(Deadline::after(timeout))
    }

    /// Attach cancellation.
    #[must_use]
    pub fn with_cancellation(mut self, cancellation: CancellationContext) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    /// Attach a deadline.
    #[must_use]
    pub const fn with_deadline(mut self, deadline: Deadline) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Attach observability scope.
    #[must_use]
    pub fn with_scope(mut self, scope: PolicyScope) -> Self {
        self.scope = scope;
        self
    }

    /// Create a child context with a child cancellation token and the same
    /// deadline/scope.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            cancellation: self.cancellation.as_ref().map(CancellationContext::child),
            deadline: self.deadline,
            scope: self.scope.clone(),
        }
    }

    /// Get the cancellation context, if present.
    #[must_use]
    pub const fn cancellation(&self) -> Option<&CancellationContext> {
        self.cancellation.as_ref()
    }

    /// Clone the cancellation context for async branches that need ownership.
    #[must_use]
    pub(crate) fn cancellation_cloned(&self) -> Option<CancellationContext> {
        self.cancellation.clone()
    }

    /// Get the deadline, if present.
    #[must_use]
    pub const fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }

    /// Get the observability scope.
    #[must_use]
    pub const fn scope(&self) -> &PolicyScope {
        &self.scope
    }

    /// Whether cancellation has already been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(CancellationContext::is_cancelled)
    }

    pub(crate) fn cancelled_error<E>(&self) -> CallError<E> {
        self.cancellation
            .as_ref()
            .map_or_else(CallError::cancelled, CancellationContext::cancelled_error)
    }

    pub(crate) fn is_deadline_expired(&self) -> bool {
        self.deadline.is_some_and(|deadline| {
            deadline
                .remaining()
                .is_none_or(|remaining| remaining.is_zero())
        })
    }

    pub(crate) async fn run_result<T, E, Fut>(&self, future: Fut) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, CallError<E>>> + Send,
    {
        if self.is_cancelled() {
            return Err(self.cancelled_error());
        }

        match (self.cancellation.as_ref(), self.deadline) {
            (None, None) => future.await,
            (Some(cancellation), None) => {
                tokio::select! {
                    result = future => result,
                    () = cancellation.token().cancelled() => Err(cancellation.cancelled_error()),
                }
            },
            (None, Some(deadline)) => deadline.timeout(future).await?,
            (Some(cancellation), Some(deadline)) => {
                let remaining = deadline.remaining_or_timeout()?;
                tokio::select! {
                    result = future => result,
                    () = cancellation.token().cancelled() => Err(cancellation.cancelled_error()),
                    () = tokio::time::sleep(remaining) => Err(CallError::Timeout(deadline.budget())),
                }
            },
        }
    }
}

impl Default for PolicyContext {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn child_preserves_deadline_and_scope_and_links_cancellation() {
        let cancellation = CancellationContext::with_reason("shutdown");
        let deadline = Deadline::after(Duration::from_secs(5));
        let context = PolicyContext::from_cancellation(cancellation.clone())
            .with_deadline(deadline)
            .with_scope(PolicyScope::empty().tenant_id("tenant-a"));

        let child = context.child();
        cancellation.cancel();

        assert!(child.is_cancelled());
        assert_eq!(child.deadline(), Some(deadline));
        assert_eq!(child.scope().tenant_id.as_deref(), Some("tenant-a"));
    }
}
