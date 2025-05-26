use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, PartialOrd, Eq)]
pub enum Value {
    #[default]
    Null,
}
