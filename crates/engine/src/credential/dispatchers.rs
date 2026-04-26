//! Engine-side capability dispatchers.
//!
//! Per Tech Spec §15.4 capability sub-trait split — these helpers bind
//! `where C: Revocable` / `where C: Testable` / `where C: Dynamic` so a
//! non-capable credential cannot reach the corresponding lifecycle path.
//! The structural barrier is identical to
//! [`CredentialResolver::resolve_with_refresh`](crate::credential::CredentialResolver::resolve_with_refresh)
//! which binds `where C: Refreshable`. Probe 4
//! (`compile_fail_engine_dispatch_capability`) cements the guarantee
//! with an `E0277` at the dispatch site for any non-capable type.
//!
//! These are stage-3 anchors: they lock in the binding shape so future
//! lifecycle wiring (П2 — rotation orchestration, runtime test panel,
//! Vault/STS dynamic lease release) extends rather than introduces the
//! capability bound. Real callers (engine rotation path, API health
//! probe endpoint, dynamic-credential reaper) materialize in П2+.
//!
//! # Why thin pass-through?
//!
//! The dispatcher functions intentionally contain no policy: timeouts,
//! retry, event emission, and metric counters belong on the caller path
//! (which differs per capability — revocation is unconditional,
//! testing is read-only, lease release races a TTL). Putting policy in
//! the dispatcher would either force one-size-fits-all defaults or
//! demand parameterization that obscures the structural binding the
//! dispatcher exists to provide. Cf. `resolve_with_refresh` which
//! adds policy precisely because there is a single canonical refresh
//! call site.

use nebula_credential::{CredentialContext, Dynamic, Revocable, Testable, error::CredentialError};

use crate::credential::TestResult;

/// Engine-side revocation dispatcher.
///
/// Bound on [`Revocable`] per Tech Spec §15.4 — non-`Revocable`
/// credentials cannot be passed; compile error at the call site
/// (`E0277`). П2+ wires real callers (engine rotation orchestrator,
/// admin-revoke API endpoint) into this entry point.
pub async fn dispatch_revoke<C: Revocable>(
    state: &mut C::State,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    <C as Revocable>::revoke(state, ctx).await
}

/// Engine-side health-probe dispatcher.
///
/// Bound on [`Testable`] per Tech Spec §15.4 — non-`Testable`
/// credentials cannot be passed; compile error at the call site
/// (`E0277`). П2+ wires real callers (admin "test credential" API
/// endpoint, periodic health-probe scheduler) into this entry point.
pub async fn dispatch_test<C: Testable>(
    scheme: &C::Scheme,
    ctx: &CredentialContext,
) -> Result<TestResult, CredentialError> {
    <C as Testable>::test(scheme, ctx).await
}

/// Engine-side lease-release dispatcher.
///
/// Bound on [`Dynamic`] per Tech Spec §15.4 — non-`Dynamic`
/// credentials cannot be passed; compile error at the call site
/// (`E0277`). П2+ wires real callers (per-execution lease reaper,
/// lease-TTL expiry reaper) into this entry point.
pub async fn dispatch_release<C: Dynamic>(
    state: &C::State,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    <C as Dynamic>::release(state, ctx).await
}
