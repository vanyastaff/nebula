use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::connection::connection::Connection;
use crate::types::Key;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ConnectionCollection {
    connections: HashMap<Key, Connection>,
}