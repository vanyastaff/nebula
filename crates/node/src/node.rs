//! The base Node trait.

use std::fmt::Debug;

use nebula_core::NodeKey;

use crate::NodeComponents;
use crate::NodeMetadata;

/// Base trait for all node types in Nebula.
///
/// A node represents a user-visible, versionable plugin unit (e.g. "Slack",
/// "HTTP Request"). It provides metadata and registers its runtime components
/// (actions, credentials) via [`NodeComponents`].
///
/// This trait is **object-safe** so nodes can be stored as `Arc<dyn Node>`.
pub trait Node: Send + Sync + Debug + 'static {
    /// Returns the static metadata for this node.
    fn metadata(&self) -> &NodeMetadata;

    /// Register actions and credential requirements into `components`.
    fn register(&self, components: &mut NodeComponents);

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

        fn register(&self, _components: &mut NodeComponents) {
            // No actions to register in the test stub.
        }
    }

    #[test]
    fn trait_default_methods() {
        let meta = NodeMetadata::builder("slack", "Slack")
            .version(2)
            .description("Send messages")
            .build()
            .unwrap();

        let node = TestNode { meta };

        assert_eq!(node.key().as_str(), "slack");
        assert_eq!(node.name(), "Slack");
        assert_eq!(node.version(), 2);
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
