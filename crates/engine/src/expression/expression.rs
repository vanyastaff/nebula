#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaybeExpression<T> {
    Expression(String),
    Static(T),
}