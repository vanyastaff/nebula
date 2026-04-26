//! `Dynamic` sub-trait — ephemeral per-execution credentials.
//!
//! Per Tech Spec §15.4 capability sub-trait split — closes
//! security-lead findings N1 + N3 + N5. The pre-§15.4 shape declared
//! dynamic capability via `const DYNAMIC: bool = false` plus a
//! defaulted [`release`] body that returned `Ok(())` (no-op success). A
//! plugin author setting `const DYNAMIC = true` while forgetting to
//! override `release` produced a credential that *declared* dynamic
//! lease semantics but silently leaked the lease at the provider — the
//! engine treated the release call as successful, the dynamic
//! credential lingered until the provider's own TTL cleaned it up
//! (which may be never for some Vault dynamic backends). The sub-trait
//! variant in this module makes that mistake structurally impossible:
//! only credentials that explicitly `impl Dynamic` can route through
//! the engine's release path, and `release` has no defaulted body
//! (`E0046` if omitted).
//!
//! Per CP6 the receiver was corrected: production trait had a vestigial
//! `&self` (the `Self` is a ZST type-level marker, the receiver gave
//! no access). The sub-trait signature aligns with sister sub-trait
//! signatures (state + ctx, no `&self`).
//!
//! [`release`]: Dynamic::release

use std::{future::Future, time::Duration};

use crate::{Credential, CredentialContext, error::CredentialError};

/// Credentials that produce ephemeral, per-execution secrets with a
/// bounded lease (Vault database dynamic credentials, AWS STS
/// AssumeRole sessions, short-lived workload-identity tokens).
///
/// Dynamic credentials are never cached — a fresh secret is generated
/// on every resolve. The framework calls [`release`](Dynamic::release)
/// when the execution completes (success or failure) or when the lease
/// TTL expires.
///
/// # Examples
///
/// ```ignore
/// use std::time::Duration;
/// use nebula_credential::{Credential, Dynamic};
///
/// struct VaultDbCred;
///
/// // (impl Credential for VaultDbCred elided)
///
/// impl Dynamic for VaultDbCred {
///     const LEASE_TTL: Option<Duration> = Some(Duration::from_secs(300));
///
///     async fn release(
///         state: &VaultDbState,
///         ctx: &CredentialContext<'_>,
///     ) -> Result<(), CredentialError> {
///         // ... revoke Vault lease via lease_id ...
///     }
/// }
/// ```
pub trait Dynamic: Credential {
    /// Lease duration. `None` means release happens only at execution
    /// end — the framework never expires the lease autonomously.
    const LEASE_TTL: Option<Duration> = None;

    /// Release a dynamic credential lease.
    ///
    /// Called by the framework when:
    /// - The execution completes (success or failure).
    /// - The lease TTL expires.
    ///
    /// Implementations should revoke the ephemeral credential from the
    /// backing system (e.g., revoke a Vault lease, terminate an STS
    /// session). Failures surface explicitly to the caller — the
    /// framework does not silently swallow lease leaks.
    fn release(
        state: &Self::State,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send
    where
        Self: Sized;
}
