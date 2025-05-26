use std::any::Any;
use std::fmt::Debug;

use downcast_rs::{Downcast, impl_downcast};

use crate::node::metadata::NodeMetadata;
use crate::types::Key;

/// Base trait for all node types in the system.
/// Provides common functionality and metadata for nodes.
pub trait Node: Downcast + Any + Debug {
    /// Returns the metadata associated with this node.
    fn metadata(&self) -> &NodeMetadata;

    /// Returns the name of the node.
    fn name(&self) -> &str {
        &self.metadata().name
    }

    /// Returns the unique key of the node.
    fn key(&self) -> &Key {
        &self.metadata().key
    }

    /// Returns the version number of the node.
    fn version(&self) -> u32 {
        self.metadata().version
    }

    // /// Returns the credentials associated with this node, if any.
    // fn credentials(&self) -> Option<&Credentials> {
    //     None
    // }
    //
    // /// Returns the actions supported by this node, if any.
    // fn actions(&self) -> Option<&Actions> {
    //     None
    // }
}

impl_downcast!(Node);
