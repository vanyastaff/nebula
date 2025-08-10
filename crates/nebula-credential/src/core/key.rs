use domain_key::{Key, KeyDomain, KeyParseError, static_key};

/// Domain marker for credential keys
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CredentialDomain;

impl KeyDomain for CredentialDomain {
    const DOMAIN_NAME: &'static str = "action";
    const MAX_LENGTH: usize = 24;

    fn validation_help() -> Option<&'static str> {
        Some("Credential names should use snake_case with letters, digits, and underscores only")
    }
}

pub type CredentialKey = Key<CredentialDomain>;

#[macro_export]
macro_rules! credential_key {
    ($param_name:literal) => {
        static_key!(CredentialKey, $param_name)
    };
}


