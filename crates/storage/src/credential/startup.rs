//! Why a durable credential store could not be opened.

use std::fmt;

use crate::migration::catalog::{CatalogRejection, CatalogSetupError};

/// Failure to construct a credential store that is safe to serve.
#[derive(Clone, PartialEq, Eq)]
pub enum CredentialStoreStartupError {
    /// The database is reachable but its migration ledger is not a canonical
    /// prefix of this build's catalog.
    UnsupportedSchema(CatalogRejection),
    /// The database, setup lock, or migration transaction was unavailable.
    Unavailable,
}

impl fmt::Display for CredentialStoreStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(rejection) => {
                write!(formatter, "unsupported credential schema: {rejection}")
            },
            Self::Unavailable => formatter.write_str("credential store unavailable"),
        }
    }
}

impl fmt::Debug for CredentialStoreStartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(rejection) => formatter
                .debug_tuple("UnsupportedSchema")
                .field(rejection)
                .finish(),
            Self::Unavailable => formatter.write_str("Unavailable"),
        }
    }
}

impl std::error::Error for CredentialStoreStartupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnsupportedSchema(rejection) => Some(rejection),
            Self::Unavailable => None,
        }
    }
}

impl From<CatalogSetupError> for CredentialStoreStartupError {
    fn from(error: CatalogSetupError) -> Self {
        match error {
            CatalogSetupError::Rejected(rejection) => Self::UnsupportedSchema(rejection),
            CatalogSetupError::Unavailable => Self::Unavailable,
        }
    }
}

impl crate::migration::SchemaSetupFailure for CredentialStoreStartupError {
    fn failure_kind(&self) -> crate::migration::SetupFailureKind {
        match self {
            Self::UnsupportedSchema(_) => crate::migration::SetupFailureKind::Rejected,
            Self::Unavailable => crate::migration::SetupFailureKind::Unavailable,
        }
    }
}

#[cfg(test)]
impl From<CredentialStoreStartupError> for nebula_storage_port::CredentialPersistenceError {
    fn from(_: CredentialStoreStartupError) -> Self {
        Self::Unavailable
    }
}
