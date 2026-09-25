//! Durable write-back of refreshed material.
//!
//! A refresh holds the credential's claim across the provider call, so no other
//! material write can land meanwhile. Display writes can: a rename or a tag
//! edit only bumps the row version, and the management path allows it while a
//! refresh is in flight. A write-back fenced only by the pre-provider row
//! version would then fail after the provider already consumed the old refresh
//! token, losing the rotated one. This module fences the write-back on the
//! material authority instead, and re-bases it onto the current display
//! fields when only those moved.

use nebula_storage_port::{
    CredentialCommit, CredentialPersistence, CredentialPersistenceError, CredentialReplacement,
    CredentialReplacementFence, CredentialSelector, StoredCredential, StoredLiveCredential,
};

/// How many display-only races one write-back absorbs before giving up.
///
/// Each retry re-reads the row, so the bound only matters under a sustained
/// stream of renames; exhausting it reports the last conflict unchanged.
const MAX_DISPLAY_REBASES: usize = 3;

/// Persist refreshed material built by `build` against `stored`.
///
/// Returns the commit together with the row it was based on, whose display
/// fields are the ones now on record.
///
/// `build` receives the live row the replacement is based on and must copy its
/// display fields (name, metadata) and version. The replacement is fenced on
/// the material epoch and credential key observed before provider egress, so a
/// concurrent material change still fails closed. When the row version moved
/// but that authority did not, the row is re-read and the replacement rebuilt
/// on it: the change was display-only and must not cost the refreshed
/// material.
///
/// # Errors
///
/// The replacement's own error when it is not a version conflict, when the
/// re-read row is gone or carries different material authority, or after
/// [`MAX_DISPLAY_REBASES`] consecutive display-only conflicts.
pub(crate) async fn write_refreshed<S, F>(
    store: &S,
    selector: &CredentialSelector,
    stored: &StoredLiveCredential,
    build: F,
) -> Result<(CredentialCommit, StoredLiveCredential), CredentialPersistenceError>
where
    S: CredentialPersistence + ?Sized,
    F: Fn(&StoredLiveCredential) -> CredentialReplacement,
{
    let fence = CredentialReplacementFence::new(
        stored.material_epoch(),
        stored.credential_key().to_owned(),
    );
    let mut base = stored.clone();
    let mut rebases = 0;
    loop {
        let replacement = build(&base).with_fence(fence.clone());
        let error = match store.replace(selector, replacement).await {
            Ok(commit) => return Ok((commit, base)),
            Err(error @ CredentialPersistenceError::VersionConflict { .. }) => error,
            Err(error) => return Err(error),
        };
        if rebases == MAX_DISPLAY_REBASES {
            return Err(error);
        }
        let current = match store.get(selector).await {
            Ok(StoredCredential::Live(current)) => current,
            Ok(StoredCredential::Tombstoned(_)) | Err(_) => return Err(error),
        };
        if current.material_epoch() != stored.material_epoch()
            || current.credential_key() != stored.credential_key()
        {
            return Err(error);
        }
        base = current;
        rebases += 1;
    }
}
