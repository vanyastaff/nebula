//! Spec §6 #7 (structural half): a raw credential backend is unusable
//! without the builder's secure layered composition
//! `Audit(Cache(Encryption(raw)))`.
//!
//! `CredentialService::__from_parts` is the only assembly point and it is
//! `pub(crate)`; the struct fields are private. An external caller that
//! tries to bypass `CredentialServiceBuilder` — handing the service a
//! raw, unencrypted store directly — cannot even *name* the constructor.
//! Expected: a privacy error (E0624 — associated function is private),
//! NOT an arity/type error (we never get far enough to type-check args).

use nebula_credential::InMemoryPendingStore;
use nebula_credential_runtime::CredentialService;
use nebula_storage::credential::InMemoryStore;

fn main() {
    // `InMemoryStore` is a raw `CredentialStore`. Reaching the private
    // `__from_parts` to wrap it directly (skipping the EncryptionLayer)
    // is the abuse this probe forbids. Referencing the path is enough —
    // the privacy check fires before argument type-checking.
    let _bypass = CredentialService::<InMemoryStore, InMemoryPendingStore>::__from_parts;
}
