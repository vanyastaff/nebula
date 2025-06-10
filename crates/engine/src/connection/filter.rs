use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionFilter {
    None,
    NodeTypes(Vec<String>),
    Categories(Vec<String>),
}