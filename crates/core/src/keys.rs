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

define_domain!(PrameterDomain, "parameter");
key_type!(ParameterKey, PrameterDomain);

define_domain!(CredentialDomain, "credential");
key_type!(CredentialKey, CredentialDomain);

define_domain!(ActionDomain, "action");
key_type!(ActionKey, ActionDomain);

define_domain!(ResourceDomain, "resource");
key_type!(ResourceKey, ResourceDomain);

define_domain!(PluginDomain, "plugin");
key_type!(PluginKey, PluginDomain);

#[cfg(test)]
mod tests {}
