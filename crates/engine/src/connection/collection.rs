use std::collections::HashMap;
use crate::connection::connection::Connection;
use crate::types::Key;

#[derive(Debug, Clone, Default)]
pub struct ConnectionCollection {
    connections: HashMap<Key, Connection>,
}