//! Normalized keys and identifiers for plugins, parameters, and credentials.
//!
//! **PluginKey** = key of the plugin *type* (e.g. `telegram_bot`, `http_request`).
//! **ActionKey** = key of a specific *action* within a plugin (e.g. `send_message`, `get_updates`).
//! **NodeKey** = author-defined key for a workflow graph node (e.g. `fetch_users`).
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

use domain_key::{KeyParseError, define_domain, key_type};

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

define_domain!(pub NodeDomain, "node");
key_type!(pub NodeKey, NodeDomain);

/// Extract the plugin namespace from a namespaced [`ActionKey`].
///
/// Action keys follow the convention `{plugin_key}.{action_name}` enforced at
/// plugin registration time by `ResolvedPlugin`: every action key registered
/// under a plugin is required to start with `format!("{}.", plugin_key)`.
///
/// This function takes the segment **before the first `.`** as the plugin-key
/// candidate:
///
/// - `"telegram_bot.send_message"` → `Ok(PluginKey("telegram_bot"))`
/// - `"plugin.group.action"` → `Ok(PluginKey("plugin"))` (first dot only)
/// - `"echo"` (no dot) → the whole string is used → `Ok(PluginKey("echo"))`
/// - `".foo"` (empty prefix before first dot) → `Err` (empty plugin key)
///
/// The candidate is validated through the same constructor `PluginKey::new` uses,
/// so all plugin-key invariants (character set, length, structure) are enforced.
///
/// # Errors
///
/// Returns [`KeyParseError`] when the extracted prefix is not a valid plugin key
/// (e.g. empty, contains illegal characters, or exceeds the maximum length).
pub fn plugin_key_from_action_key(action: &ActionKey) -> Result<PluginKey, KeyParseError> {
    let prefix = match action.as_str().split_once('.') {
        Some((before, _)) => before,
        None => action.as_str(),
    };
    PluginKey::new(prefix)
}

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
        const _: () = assert!(
            $crate::ActionKey::is_valid_key_const($s),
            "invalid action key literal"
        );
        $crate::ActionKey::new($s).unwrap()
    }};
}

/// Constructs a [`CredentialKey`] from a string literal, validated at compile time.
#[macro_export]
macro_rules! credential_key {
    ($s:literal) => {{
        const _: () = assert!(
            $crate::CredentialKey::is_valid_key_const($s),
            "invalid credential key literal"
        );
        $crate::CredentialKey::new($s).unwrap()
    }};
}

/// Constructs a [`PluginKey`] from a string literal, validated at compile time.
#[macro_export]
macro_rules! plugin_key {
    ($s:literal) => {{
        const _: () = assert!(
            $crate::PluginKey::is_valid_key_const($s),
            "invalid plugin key literal"
        );
        $crate::PluginKey::new($s).unwrap()
    }};
}

/// Constructs a [`ParameterKey`] from a string literal, validated at compile time.
#[macro_export]
macro_rules! parameter_key {
    ($s:literal) => {{
        const _: () = assert!(
            $crate::ParameterKey::is_valid_key_const($s),
            "invalid parameter key literal"
        );
        $crate::ParameterKey::new($s).unwrap()
    }};
}

/// Constructs a [`NodeKey`] from a string literal, validated at compile time.
#[macro_export]
macro_rules! node_key {
    ($s:literal) => {{
        const _: () = assert!(
            $crate::NodeKey::is_valid_key_const($s),
            "invalid node key literal"
        );
        $crate::NodeKey::new($s).unwrap()
    }};
}

#[cfg(test)]
mod tests {
    use crate::{ActionKey, CredentialKey, NodeKey, ResourceKey};

    #[test]
    fn macro_produces_correct_key() {
        let k = resource_key!("postgres");
        assert_eq!(k, ResourceKey::new("postgres").unwrap());
    }

    #[test]
    fn node_key_works() {
        let k = node_key!("fetch_users");
        assert_eq!(k, NodeKey::new("fetch_users").unwrap());
    }

    #[test]
    fn is_valid_key_const_matches_runtime_new() {
        let valid = ["postgres", "my-db", "http.request", "v2_api", "a"];
        for s in valid {
            assert!(ResourceKey::is_valid_key_const(s), "{s:?} should be valid");
            assert!(
                ResourceKey::new(s).is_ok(),
                "{s:?} const=true but new() failed"
            );
        }
        let invalid = ["", "bad_", "a--b", "bad!", "my key"];
        for s in invalid {
            assert!(
                !ResourceKey::is_valid_key_const(s),
                "{s:?} should be invalid"
            );
            assert!(
                CredentialKey::new(s).is_err(),
                "{s:?} const=false but new() succeeded"
            );
        }
    }

    // ---- plugin_key_from_action_key ----------------------------------------

    use crate::plugin_key_from_action_key;

    #[test]
    fn plugin_key_from_action_key_extracts_plugin_namespace() {
        // Standard namespaced action: plugin prefix is the segment before the first dot.
        let action = action_key!("telegram_bot.send_message");
        let plugin = plugin_key_from_action_key(&action).unwrap();
        assert_eq!(plugin.as_str(), "telegram_bot");
    }

    #[test]
    fn plugin_key_from_action_key_splits_on_first_dot_only() {
        // Multi-segment key: only the FIRST dot is used as the split point.
        let action = action_key!("plugin.group.action");
        let plugin = plugin_key_from_action_key(&action).unwrap();
        assert_eq!(plugin.as_str(), "plugin");
    }

    #[test]
    fn plugin_key_from_action_key_whole_string_when_no_dot() {
        // No dot present: the entire action key string is the plugin-key candidate.
        let action = action_key!("echo");
        let plugin = plugin_key_from_action_key(&action).unwrap();
        assert_eq!(plugin.as_str(), "echo");
    }

    #[test]
    fn plugin_key_from_action_key_empty_prefix_is_err() {
        // A leading dot produces an empty prefix, which is not a valid plugin key.
        let action = ActionKey::new(".foo").unwrap();
        assert!(
            plugin_key_from_action_key(&action).is_err(),
            "empty prefix before first dot must be rejected"
        );
    }
}
