use crate::core::CredentialError;
use std::time::SystemTime;

/// Negative cache entry
#[derive(Clone)]
pub(super) struct NegativeCache {
    pub until: SystemTime,
    pub error: CredentialError,
}