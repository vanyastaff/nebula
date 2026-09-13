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
/// ```
/// use std::time::Duration;
/// use nebula_credential::{
///     Credential, CredentialContext, CredentialMetadataDraft, Dynamic,
///     SecretString, scheme::SecretToken,
/// };
/// use nebula_credential::error::CredentialError;
/// use nebula_credential::resolve::StaticResolveResult;
/// use nebula_core::credential_key;
///
/// struct VaultDbCred;
///
/// # impl Credential for VaultDbCred {
/// #     type Properties = serde_json::Value;
/// #     type Scheme = SecretToken;
/// #     type State = SecretToken;
/// #     const KEY: &'static str = "vault_db_cred";
/// #     fn metadata() -> CredentialMetadataDraft {
/// #         CredentialMetadataDraft::new(
/// #             credential_key!("vault_db_cred"), nebula_credential::metadata_name!("Vault DB"), "demo",
/// #         )
/// #     }
/// #     fn project(state: &SecretToken) -> SecretToken { state.clone() }
/// #     async fn resolve(
/// #         _properties: &Self::Properties,
/// #         _ctx: &CredentialContext,
/// #     ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
/// #         Ok(StaticResolveResult::Complete(SecretToken::new(SecretString::new(""))))
/// #     }
/// # }
/// impl Dynamic for VaultDbCred {
///     const LEASE_TTL: Option<Duration> = Some(Duration::from_secs(300));
///
///     async fn release(
///         state: &SecretToken,
///         _ctx: &CredentialContext,
///     ) -> Result<(), CredentialError> {
///         // Revoke the ephemeral Vault lease identified by `state`.
///         let _ = state;
///         Ok(())
///     }
/// }
///
/// // Dynamic-lease capability is encoded by trait membership.
/// fn assert_dynamic<C: Dynamic>() {}
/// assert_dynamic::<VaultDbCred>();
/// assert_eq!(VaultDbCred::LEASE_TTL, Some(Duration::from_secs(300)));
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
