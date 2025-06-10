use serde::{Deserialize, Serialize};
use crate::connection::ConnectionFilter;
use crate::expression::MaybeExpression;
use crate::types::Key;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Connection {
    Flow,
    Support(SupportConnection ),
    Dynamic(DynamicConnection),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportConnection  {
    pub key: Key,
    pub name: String,
    pub description: String,
    pub required: bool,
    pub filter: ConnectionFilter,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicConnection {
    pub key: Key,
    pub name: MaybeExpression<String>,
    pub description: MaybeExpression<String>,
}

