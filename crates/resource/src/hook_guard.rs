//! Bound + isolate third-party (`impl Topology` / `Provider`) hook futures.
//!
//! An open-topology author's hooks — `create_slot` / `accept` / `prepare`,
//! `Provider::create` / `Provider::destroy`, `on_release`,
//! `on_credential_refresh` / `on_credential_revoke` — run *inside* the
//! framework's own loops. A careless or hostile author must not be able to
//! **wedge** the framework by hanging, nor **crash** it by panicking.
//!
//! [`guard_author_hook`] is the single chokepoint every author-hook dispatch
//! funnels through: it caps the hook with a timeout and isolates an unwinding
//! panic via [`catch_unwind`](futures::FutureExt::catch_unwind), collapsing both
//! failure modes into a typed [`HookFault`] the caller maps onto its local
//! outcome. Routing every site through one combinator makes "an unbounded,
//! crash-propagating author hook" unrepresentable rather than a hazard each new
//! call site must remember to guard.

use std::{future::Future, panic::AssertUnwindSafe, time::Duration};

use nebula_core::ResourceKey;

/// Worst-case ceiling on a single author-hook dispatch when the caller carries
/// no tighter deadline of its own. A blocking hook can never hang past this; a
/// caller-supplied deadline, when present, takes precedence (it is usually
/// tighter than this backstop).
pub(crate) const DEFAULT_AUTHOR_HOOK_CEILING: Duration = Duration::from_secs(30);

/// Catch-all backstop for the **teardown** path (`Provider::destroy` /
/// `on_release`) only. The effective teardown bound is the per-resource
/// `timeout_at(cx.deadline)` derived from
/// [`Provider::teardown_budget`](crate::Provider::teardown_budget) (ADR-0093);
/// this outer ceiling must always sit *above* the largest composed deadline so
/// it never undercuts a resource that declares a budget larger than
/// [`DEFAULT_AUTHOR_HOOK_CEILING`]. It only catches a truly wedged framework
/// future (one that ignored its own per-destroy deadline). Used solely for the
/// two teardown-path [`guard_author_hook`] calls — acquire / warmup / rotation
/// keep [`DEFAULT_AUTHOR_HOOK_CEILING`].
pub(crate) const MAX_TEARDOWN_CEILING: Duration = Duration::from_mins(2);

/// How a guarded author hook failed the *framework*, independent of any error
/// the hook itself returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookFault {
    /// The hook unwound (panicked). Caught — the caller was not crashed.
    Panicked,
    /// The hook did not complete within its budget.
    TimedOut,
}

impl std::fmt::Display for HookFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Panicked => f.write_str("panicked"),
            Self::TimedOut => f.write_str("timed out"),
        }
    }
}

impl HookFault {
    /// Emit a structured observability record for a caught author-hook fault.
    ///
    /// A panic is a plugin bug (`error!`); a timeout is misbehavior or runaway
    /// load (`warn!`). `site` names the framework call site (`"acquire"`,
    /// `"warmup"`, `"release"`). This makes a misbehaving third-party hook
    /// distinguishable in logs from healthy backpressure — the framework caught
    /// and bounded the fault, but an operator must still be able to see it.
    pub(crate) fn observe(self, key: &ResourceKey, site: &'static str) {
        match self {
            HookFault::Panicked => tracing::error!(
                resource.key = %key,
                hook.site = site,
                hook.fault = "panicked",
                "author resource hook panicked — caught and isolated by the framework"
            ),
            HookFault::TimedOut => tracing::warn!(
                resource.key = %key,
                hook.site = site,
                hook.fault = "timed_out",
                "author resource hook exceeded its time budget — bounded by the framework"
            ),
        }
    }
}

/// Runs an author-supplied hook future under the framework's bound + isolate
/// guard. Returns the hook's own output on success, or a [`HookFault`] when the
/// framework had to cut it short (panic caught, or `timeout` elapsed).
///
/// The future is wrapped in [`AssertUnwindSafe`]: the caller is responsible for
/// ensuring no observable broken invariant survives a caught panic. Every
/// current site holds its consistency synchronously before the guarded await —
/// taint / revoke-epoch bump happen first, and an in-flight slot is destroyed
/// by its `SlotCreateGuard` on drop — so a caught unwind leaves no partial
/// state. **Each call site carries a `// SAFETY (unwind):` comment** stating its
/// specific no-torn-state argument; a new site must add one (the `AssertUnwindSafe`
/// is sound only because that invariant holds, so spell out why it does there).
pub(crate) async fn guard_author_hook<T>(
    timeout: Duration,
    fut: impl Future<Output = T>,
) -> Result<T, HookFault> {
    let fut = AssertUnwindSafe(fut);
    match tokio::time::timeout(timeout, futures::FutureExt::catch_unwind(fut)).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_panic)) => Err(HookFault::Panicked),
        Err(_elapsed) => Err(HookFault::TimedOut),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ok_hook_returns_its_value() {
        let out = guard_author_hook(Duration::from_secs(1), async { 41 + 1 }).await;
        assert_eq!(out, Ok(42));
    }

    #[tokio::test]
    async fn panicking_hook_is_isolated_as_panicked() {
        let out: Result<(), HookFault> = guard_author_hook(Duration::from_secs(1), async {
            panic!("careless author hook unwinds");
        })
        .await;
        assert_eq!(out, Err(HookFault::Panicked));
    }

    #[tokio::test(start_paused = true)]
    async fn hanging_hook_is_bounded_as_timed_out() {
        // `start_paused` fires the deadline instantly + deterministically, so a
        // genuine "hang forever" hook resolves without any wall-clock wait.
        let out: Result<(), HookFault> =
            guard_author_hook(Duration::from_millis(50), std::future::pending()).await;
        assert_eq!(out, Err(HookFault::TimedOut));
    }
}
