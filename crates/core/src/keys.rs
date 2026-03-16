//! Normalized keys and identifiers for plugins, parameters, and credentials.
//!
//! **PluginKey** = key of the plugin *type* (e.g. `telegram_bot`, `http_request`).
//! **ActionKey** = key of a specific *action* within a plugin (e.g. `send_message`, `get_updates`).
//!
//! # Examples
//!
//! ```
//! use nebula_core::{ActionKey, CredentialKey, ParameterKey, PluginKey};
//!
//! // Keys must already be in normalized form (lowercase a-z, digits, underscores).
//! let plugin: PluginKey = "telegram_bot".parse().unwrap();
//! assert_eq!(plugin.as_str(), "telegram_bot");
//!
//! let action: ActionKey = ActionKey::new("send_message").unwrap();
//! assert_eq!(action.as_str(), "send_message");
//!
//! let param: ParameterKey = ParameterKey::new("input_value").unwrap();
//! assert_eq!(param.as_str(), "input_value");
//!
//! let cred: CredentialKey = CredentialKey::new("my_api_key").unwrap();
//! assert_eq!(cred.as_str(), "my_api_key");
//! ```

use domain_key::{define_domain, key_type};

define_domain!(pub ParameterDomain, "parameter");
key_type!(pub ParameterKey, ParameterDomain);

define_domain!(pub CredentialDomain, "credential");
key_type!(pub CredentialKey, CredentialDomain);

define_domain!(pub ActionDomain, "action");
key_type!(pub ActionKey, ActionDomain);

define_domain!(pub ResourceDomain, "resource");
key_type!(pub ResourceKey, ResourceDomain);

define_domain!(pub PluginDomain, "plugin");
key_type!(pub PluginKey, PluginDomain);

/// Constructs a [`ResourceKey`] from a string literal, validated at **compile time**.
///
/// Invalid literals are rejected by the compiler, not at runtime.
///
/// # Example
/// ```
/// use nebula_core::resource_key;
/// let k = resource_key!("postgres");
/// ```
#[macro_export]
macro_rules! resource_key {
    ($s:literal) => {{
        const _: () = assert!(
            $crate::ResourceKey::is_valid_key_const($s),
            "invalid resource key literal"
        );
        $crate::ResourceKey::new($s).unwrap()
    }};
}

/// Constructs an [`ActionKey`] from a string literal, validated at compile time.
#[macro_export]
macro_rules! action_key {
    ($s:literal) => {{
        const _: () = assert!($crate::ActionKey::is_valid_key_const($s), "invalid action key literal");
        $crate::ActionKey::new($s).unwrap()
    }};
}

/// Constructs a [`CredentialKey`] from a string literal, validated at compile time.
#[macro_export]
macro_rules! credential_key {
    ($s:literal) => {{
        const _: () = assert!($crate::CredentialKey::is_valid_key_const($s), "invalid credential key literal");
        $crate::CredentialKey::new($s).unwrap()
    }};
}

/// Constructs a [`PluginKey`] from a string literal, validated at compile time.
#[macro_export]
macro_rules! plugin_key {
    ($s:literal) => {{
        const _: () = assert!($crate::PluginKey::is_valid_key_const($s), "invalid plugin key literal");
        $crate::PluginKey::new($s).unwrap()
    }};
}

/// Constructs a [`ParameterKey`] from a string literal, validated at compile time.
#[macro_export]
macro_rules! parameter_key {
    ($s:literal) => {{
        const _: () = assert!($crate::ParameterKey::is_valid_key_const($s), "invalid parameter key literal");
        $crate::ParameterKey::new($s).unwrap()
    }};
}

#[cfg(test)]
mod tests {
    use crate::{CredentialKey, ResourceKey};

    #[test]
    fn macro_produces_correct_key() {
        let k = resource_key!("postgres");
        assert_eq!(k, ResourceKey::new("postgres").unwrap());
    }

    #[test]
    fn is_valid_key_const_matches_runtime_new() {
        let valid = ["postgres", "my-db", "http.request", "v2_api", "a"];
        for s in valid {
            assert!(ResourceKey::is_valid_key_const(s), "{s:?} should be valid");
            assert!(ResourceKey::new(s).is_ok(), "{s:?} const=true but new() failed");
        }
        let invalid = ["", "bad_", "a--b", "bad!", "my key"];
        for s in invalid {
            assert!(!ResourceKey::is_valid_key_const(s), "{s:?} should be invalid");
            assert!(CredentialKey::new(s).is_err(), "{s:?} const=false but new() succeeded");
        }
    }
}
