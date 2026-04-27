//! Reverse-index validation + write helpers for `Manager::register`.
//!
//! The two helpers split what was a single fallible `register_inner` so the
//! `register<R>` body can validate the credential-binding contract **before**
//! mutating the registry. Without the split, a credential-bearing resource
//! supplied without a `credential_id` would land in the registry first and
//! only then trigger `Error::missing_credential_id` — leaving an orphan
//! entry that subsequent retry attempts would see as "already registered."
//!
//! - [`validate_credential_binding`] is pure (no side effects); call it FIRST.
//! - [`write_reverse_index`] writes the `credential_resources` reverse-index entry; call it AFTER
//!   the registry write completes. Validation has already passed, so the write is infallible by
//!   construction.

use std::{any::TypeId, sync::Arc, time::Duration};

use nebula_core::CredentialId;

use crate::{
    error::Error,
    manager::{
        Manager,
        rotation::{ResourceDispatcher, TypedDispatcher},
    },
    resource::Resource,
    runtime::managed::ManagedResource,
};

/// Pure validation of the credential-binding contract — no side effects.
///
/// Call this BEFORE the `Manager::registry::register` write. If it returns
/// `Err`, the caller must abort registration; if it returns `Ok`, the caller
/// is free to write the registry and then call [`write_reverse_index`].
///
/// # Errors
///
/// Returns [`Error::missing_credential_id`] when a credential-bearing
/// resource (`R::Credential != NoCredential`) is registered without a
/// `credential_id`. The `NoCredential`-bound paths emit a `tracing::warn!`
/// for the "supplied an id we'll ignore" case but never fail.
pub(super) fn validate_credential_binding<R: Resource>(
    credential_id: Option<&CredentialId>,
) -> Result<(), Error> {
    let opted_out =
        TypeId::of::<R::Credential>() == TypeId::of::<nebula_credential::NoCredential>();

    match (opted_out, credential_id) {
        (true, Some(_)) => {
            tracing::warn!(
                resource = %R::key(),
                "register: NoCredential resource provided a credential_id; ignoring"
            );
        },
        (true, None) => {
            // Normal path for NoCredential-bound resources.
        },
        (false, None) => {
            return Err(Error::missing_credential_id(R::key()));
        },
        (false, Some(_)) => {
            // Normal credential-bearing path; reverse-index write happens
            // in `write_reverse_index` after the registry write succeeds.
        },
    }
    Ok(())
}

/// Writes the `credential_resources` reverse-index entry for a credential-
/// bearing resource. Caller MUST have called [`validate_credential_binding`]
/// FIRST and observed `Ok(())`; this function is infallible by construction.
///
/// No-op for `NoCredential`-bound resources (regardless of whether a
/// `credential_id` was supplied — the warn-and-ignore path).
pub(super) fn write_reverse_index<R: Resource>(
    manager: &Manager,
    managed: Arc<ManagedResource<R>>,
    credential_id: Option<CredentialId>,
    timeout_override: Option<Duration>,
) {
    let opted_out =
        TypeId::of::<R::Credential>() == TypeId::of::<nebula_credential::NoCredential>();

    if opted_out {
        // NoCredential — never write the reverse-index. The warn-and-ignore
        // log already fired in `validate_credential_binding` if the caller
        // supplied a `credential_id` we're discarding.
        return;
    }

    if let Some(id) = credential_id {
        let dispatcher: Arc<dyn ResourceDispatcher> =
            Arc::new(TypedDispatcher::new(managed, timeout_override));
        manager
            .credential_resources
            .entry(id)
            .or_default()
            .push(dispatcher);
    }
    // The `(false, None)` case (credential-bearing without id) was already
    // rejected by `validate_credential_binding`; the type system can't see
    // that, so the `if let Some` here is the no-op fallback for completeness.
}
