//! ADR-0092 drain shim — deleted in step 8.
//!
//! `KeyProvider` and its built-in impls were relocated to
//! `nebula_credential::store_layer::key_provider` (Core tier) in ADR-0092
//! step 3. These re-exports keep the previous
//! `nebula_storage::credential::key_provider` public surface resolving.

// `StaticKeyProvider` is gated on `nebula-credential/test-util`. Storage's
// `test-util` feature forwards `nebula-credential/test-util` (see Cargo.toml),
// so `feature = "test-util"` here implies the upstream gate is active.
// `cfg(test)` alone is NOT sufficient: nebula-credential is compiled as a
// library dependency without `cfg(test)`, so `StaticKeyProvider` would be
// absent even when storage's own test binary is being built.
#[cfg(feature = "test-util")]
pub use nebula_credential::store_layer::key_provider::StaticKeyProvider;
pub use nebula_credential::store_layer::key_provider::{
    EnvKeyProvider, FileKeyProvider, KeyProvider, ProviderError,
};
