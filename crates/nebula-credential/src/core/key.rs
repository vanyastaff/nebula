use domain_key::{Key, KeyDomain};

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

/// Type-safe key for credential identifiers
pub type CredentialKey = Key<CredentialDomain>;

/// Create a compile-time validated credential key
///
/// # Example
/// ```ignore
/// let key = credential_key!("my_credential");
/// ```
#[macro_export]
macro_rules! credential_key {
    ($param_name:literal) => {
        static_key!(CredentialKey, $param_name)
    };
}
