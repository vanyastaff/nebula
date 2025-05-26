use std::sync::Arc;

use crate::node::{Node, NodeError, NodeVersions};
use crate::types::Key;

pub enum NodeType {
    Single(Arc<dyn Node>),
    Versions(NodeVersions),
}

impl NodeType {
    pub fn single<N: Node + 'static>(node: N) -> Self {
        NodeType::Single(Arc::new(node))
    }

    pub fn versioned<N: Node + 'static>(node: N) -> Result<Self, NodeError> {
        let mut versions = NodeVersions::new();
        versions.add(node)?;
        Ok(NodeType::Versions(versions))
    }

    /// Returns the key of the node
    pub fn key(&self) -> &Key {
        match self {
            NodeType::Single(node) => node.key(),
            NodeType::Versions(versions) => versions.key().unwrap(),
        }
    }

    /// Returns the node with the specified version
    pub fn get_node(&mut self, version: Option<u32>) -> Result<Arc<dyn Node>, NodeError> {
        match self {
            NodeType::Single(node) => {
                if let Some(v) = version {
                    // If a specific version is requested, check if it matches
                    if node.version() == v {
                        Ok(Arc::clone(node))
                    } else {
                        Err(NodeError::VersionNotFound(v, node.key().clone()))
                    }
                } else {
                    // If no specific version is requested, return the single node
                    Ok(Arc::clone(node))
                }
            }
            NodeType::Versions(versions) => match version {
                Some(v) => versions.get(v),
                None => versions.get_latest(),
            },
        }
    }

    /// Returns the latest version of the node
    pub fn get_latest(&mut self) -> Result<Arc<dyn Node>, NodeError> {
        match self {
            NodeType::Single(node) => Ok(Arc::clone(node)),
            NodeType::Versions(versions) => versions.get_latest(),
        }
    }

    /// Adds a new version of the node
    pub fn add_version<N: Node + 'static>(&mut self, node: N) -> Result<(), NodeError> {
        match self {
            NodeType::Single(_) => {
                *self = NodeType::versioned(node)?;
                Ok(())
            }
            NodeType::Versions(versions) => {
                versions.add(node)?;
                Ok(())
            }
        }
    }

    /// Checks if the node is versioned
    pub fn is_versioned(&self) -> bool {
        matches!(self, NodeType::Versions(_))
    }

    /// Returns all available versions of the node
    pub fn versions(&self) -> Vec<u32> {
        match self {
            NodeType::Single(node) => vec![node.version()],
            NodeType::Versions(versions) => versions.versions(),
        }
    }
}
