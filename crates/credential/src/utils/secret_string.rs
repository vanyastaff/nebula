//! Re-export of [`nebula_core::SecretString`].
//!
//! The canonical implementation now lives in `nebula-core`. This module
//! re-exports it so that existing `crate::utils::secret_string::SecretString`
//! paths continue to resolve.

pub use nebula_core::SecretString;
