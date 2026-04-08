//! Descriptor types for plugin-provided components.
//!
//! Descriptors are lightweight, allocation-cheap structs that describe what a plugin
//! provides (actions, credentials, resources) **without** constructing any handler
//! or establishing any connection. The engine can enumerate capabilities from all
//! registered plugins without touching any external system.

use nebula_core::{ActionKey, CredentialKey, InterfaceVersion, ResourceKey};

/// Describes an action provided by a plugin without constructing the handler.
///
/// A descriptor is returned by [`Plugin::actions()`](crate::plugin::Plugin::actions) and lets the engine
/// enumerate available actions at startup or on-demand without calling into
/// external systems.
///
/// # Examples
///
/// ```
/// use nebula_plugin::descriptor::ActionDescriptor;
/// use nebula_core::{ActionKey, InterfaceVersion};
///
/// let descriptor = ActionDescriptor {
///     key: ActionKey::new("send_message").unwrap(),
///     name: "Send Message".into(),
///     description: "Sends a message to a Slack channel.".into(),
///     version: InterfaceVersion::new(1, 0),
/// };
///
/// assert_eq!(descriptor.name, "Send Message");
/// ```
#[derive(Debug, Clone)]
pub struct ActionDescriptor {
    /// Action key (e.g., `"send_message"`).
    pub key: ActionKey,
    /// Human-readable name (e.g., `"Send Message"`).
    pub name: String,
    /// Description of what the action does.
    pub description: String,
    /// Interface version for this action, used for compatibility checks.
    pub version: InterfaceVersion,
}

/// Describes a credential type provided by a plugin.
///
/// Returned by [`Plugin::credentials()`](crate::plugin::Plugin::credentials). The engine uses these descriptors to
/// know which credential schemas are available without loading actual credentials.
///
/// # Examples
///
/// ```
/// use nebula_plugin::descriptor::CredentialDescriptor;
/// use nebula_core::CredentialKey;
///
/// let descriptor = CredentialDescriptor {
///     key: CredentialKey::new("slack_oauth2").unwrap(),
///     name: "Slack OAuth2".into(),
///     description: "OAuth2 credentials for Slack API access.".into(),
/// };
///
/// assert_eq!(descriptor.name, "Slack OAuth2");
/// ```
#[derive(Debug, Clone)]
pub struct CredentialDescriptor {
    /// Credential key (e.g., `"slack_oauth2"`).
    pub key: CredentialKey,
    /// Human-readable name (e.g., `"Slack OAuth2"`).
    pub name: String,
    /// Description of the credential type and when to use it.
    pub description: String,
}

/// Describes a resource type provided by a plugin.
///
/// Returned by [`Plugin::resources()`](crate::plugin::Plugin::resources). Resources are long-lived objects (connection
/// pools, HTTP clients) that actions share within a workflow run.
///
/// # Examples
///
/// ```
/// use nebula_plugin::descriptor::ResourceDescriptor;
/// use nebula_core::ResourceKey;
///
/// let descriptor = ResourceDescriptor {
///     key: ResourceKey::new("slack_client").unwrap(),
///     name: "Slack Client".into(),
///     description: "HTTP client for Slack API calls.".into(),
/// };
///
/// assert_eq!(descriptor.name, "Slack Client");
/// ```
#[derive(Debug, Clone)]
pub struct ResourceDescriptor {
    /// Resource key (e.g., `"slack_client"`).
    pub key: ResourceKey,
    /// Human-readable name (e.g., `"Slack Client"`).
    pub name: String,
    /// Description of what the resource provides.
    pub description: String,
}

#[cfg(test)]
mod tests {
    use nebula_core::InterfaceVersion;

    use super::*;

    #[test]
    fn action_descriptor_construction() {
        let descriptor = ActionDescriptor {
            key: ActionKey::new("send_message").unwrap(),
            name: "Send Message".into(),
            description: "Sends a message.".into(),
            version: InterfaceVersion::new(1, 0),
        };

        assert_eq!(descriptor.key.as_str(), "send_message");
        assert_eq!(descriptor.name, "Send Message");
        assert_eq!(descriptor.description, "Sends a message.");
    }

    #[test]
    fn credential_descriptor_construction() {
        let descriptor = CredentialDescriptor {
            key: CredentialKey::new("slack_oauth2").unwrap(),
            name: "Slack OAuth2".into(),
            description: "OAuth2 for Slack.".into(),
        };

        assert_eq!(descriptor.key.as_str(), "slack_oauth2");
        assert_eq!(descriptor.name, "Slack OAuth2");
    }

    #[test]
    fn resource_descriptor_construction() {
        let descriptor = ResourceDescriptor {
            key: ResourceKey::new("slack_client").unwrap(),
            name: "Slack Client".into(),
            description: "HTTP client for Slack.".into(),
        };

        assert_eq!(descriptor.key.as_str(), "slack_client");
        assert_eq!(descriptor.name, "Slack Client");
    }

    #[test]
    fn descriptors_are_clone() {
        let action = ActionDescriptor {
            key: ActionKey::new("my_action").unwrap(),
            name: "My Action".into(),
            description: "Does something.".into(),
            version: InterfaceVersion::new(2, 1),
        };
        let cloned = action.clone();
        assert_eq!(cloned.key.as_str(), action.key.as_str());
        assert_eq!(cloned.name, action.name);
    }
}
