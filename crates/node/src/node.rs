//! The base Node trait.

use std::fmt::Debug;

use nebula_core::NodeKey;
use nebula_credential::CredentialDescription;
use nebula_parameter::collection::ParameterCollection;

use crate::NodeMetadata;

/// Base trait for all node types in Nebula.
///
/// A node represents a user-visible, versionable plugin unit (e.g. "Slack",
/// "HTTP Request"). It provides metadata, parameter schemas, credential
/// requirements, and references to the actions it exposes.
///
/// This trait is **object-safe** so nodes can be stored as `Arc<dyn Node>`.
pub trait Node: Send + Sync + Debug {
    /// Returns the static metadata for this node.
    fn metadata(&self) -> &NodeMetadata;

    /// The normalized, unique key identifying this node type.
    fn key(&self) -> &NodeKey {
        self.metadata().key()
    }

    /// Human-readable display name.
    fn name(&self) -> &str {
        self.metadata().name()
    }

    /// Version number (1-based).
    fn version(&self) -> u32 {
        self.metadata().version()
    }

    /// User-facing parameter definitions, if any.
    fn parameters(&self) -> Option<&ParameterCollection> {
        self.metadata().parameters()
    }

    /// Credential descriptions required by this node.
    fn credentials(&self) -> &[CredentialDescription] {
        self.metadata().credentials()
    }

    /// Action keys this node exposes.
    fn action_keys(&self) -> &[String] {
        self.metadata().action_keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal node implementation for testing.
    #[derive(Debug)]
    struct TestNode {
        meta: NodeMetadata,
    }

    impl Node for TestNode {
        fn metadata(&self) -> &NodeMetadata {
            &self.meta
        }
    }

    #[test]
    fn trait_default_methods() {
        let meta = NodeMetadata::builder("slack", "Slack")
            .version(2)
            .description("Send messages")
            .action_key("slack.send")
            .build()
            .unwrap();

        let node = TestNode { meta };

        assert_eq!(node.key().as_str(), "slack");
        assert_eq!(node.name(), "Slack");
        assert_eq!(node.version(), 2);
        assert_eq!(node.action_keys(), &["slack.send"]);
        assert!(node.parameters().is_none());
        assert!(node.credentials().is_empty());
    }

    #[test]
    fn object_safety() {
        use std::sync::Arc;

        let meta = NodeMetadata::builder("test", "Test").build().unwrap();
        let node: Arc<dyn Node> = Arc::new(TestNode { meta });

        assert_eq!(node.key().as_str(), "test");
        assert_eq!(node.version(), 1);
    }
}
