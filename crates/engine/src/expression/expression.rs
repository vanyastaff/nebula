use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaybeExpression<T> {
    Expression(String),
    Static(T),
}