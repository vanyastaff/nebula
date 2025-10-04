use crate::core::CredentialError;
use std::time::SystemTime;

/// Negative cache entry
#[derive(Clone)]
pub(crate) struct NegativeCache {
    pub(crate) until: SystemTime,
    pub(crate) error: CredentialError,
}
