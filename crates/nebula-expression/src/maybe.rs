//! MaybeExpression type for values that can be either concrete or expressions
//!
//! This module provides the `MaybeExpression<T>` enum which allows parameters
//! to accept either a concrete value of type T or a string expression that will
//! be evaluated at runtime.

use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::core::ast::Expr;
use crate::engine::ExpressionEngine;
use nebula_value::Value;
use once_cell::sync::OnceCell;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Internal structure for cached expression parsing
#[derive(Debug)]
#[doc(hidden)]
pub struct CachedExpression {
    /// Source expression string
    pub source: String,
    #[doc(hidden)]
    pub ast: OnceCell<Expr>,
}

impl Clone for CachedExpression {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            ast: OnceCell::new(), // Don't clone the cached AST, let it re-parse if needed
        }
    }
}

impl PartialEq for CachedExpression {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

/// A value that can be either concrete or an expression to be evaluated
///
/// This is useful for workflow parameters that can accept both static values
/// and dynamic expressions.
///
/// # Lazy Parsing
///
/// When using the Expression variant, the expression is parsed lazily on first use
/// and the parsed AST is cached for subsequent evaluations using OnceCell.
///
/// # Serialization
///
/// When serializing, both variants are serialized as their inner value.
/// When deserializing strings, the type automatically detects expressions
/// by looking for `{{` and `}}` delimiters.
///
/// # Examples
///
/// ```rust
/// use nebula_expression::MaybeExpression;
///
/// // Concrete value
/// let value: MaybeExpression<String> = MaybeExpression::value("hello".to_string());
///
/// // Expression (using the expression() constructor)
/// let expr: MaybeExpression<String> = MaybeExpression::expression("{{ $input.name }}");
/// ```
///
/// When serialized as JSON:
/// ```json
/// // Concrete value
/// "hello"
///
/// // Expression
/// "{{ $input.name }}"
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum MaybeExpression<T> {
    /// A concrete value
    Value(T),
    /// An expression string to be evaluated (with cached AST)
    Expression(CachedExpression),
}

impl<T> MaybeExpression<T> {
    /// Create a new concrete value
    pub fn value(value: T) -> Self {
        Self::Value(value)
    }

    /// Create a new expression
    pub fn expression(expr: impl Into<String>) -> Self {
        Self::Expression(CachedExpression {
            source: expr.into(),
            ast: OnceCell::new(),
        })
    }

    /// Check if this is a concrete value
    pub fn is_value(&self) -> bool {
        matches!(self, Self::Value(_))
    }

    /// Check if this is an expression
    pub fn is_expression(&self) -> bool {
        matches!(self, Self::Expression(_))
    }

    /// Get the concrete value if this is a Value variant
    pub fn as_value(&self) -> Option<&T> {
        match self {
            Self::Value(v) => Some(v),
            Self::Expression(_) => None,
        }
    }

    /// Get the expression string if this is an Expression variant
    pub fn as_expression(&self) -> Option<&str> {
        match self {
            Self::Value(_) => None,
            Self::Expression(cached) => Some(&cached.source),
        }
    }

    /// Convert into the concrete value if this is a Value variant
    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Value(v) => Some(v),
            Self::Expression(_) => None,
        }
    }

    /// Convert into the expression string if this is an Expression variant
    pub fn into_expression(self) -> Option<String> {
        match self {
            Self::Value(_) => None,
            Self::Expression(cached) => Some(cached.source),
        }
    }
}

impl<T> MaybeExpression<T>
where
    T: TryFrom<Value>,
    <T as TryFrom<Value>>::Error: Into<ExpressionError>,
{
    /// Resolve this maybe-expression to a concrete value
    ///
    /// If this is a Value variant, returns the value directly.
    /// If this is an Expression variant, evaluates the expression and converts the result to T.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_expression::{MaybeExpression, ExpressionEngine, EvaluationContext};
    /// use nebula_value::Value;
    ///
    /// let engine = ExpressionEngine::new();
    /// let mut context = EvaluationContext::new();
    /// context.set_input(Value::text("Alice"));
    ///
    /// // Concrete value
    /// let maybe: MaybeExpression<String> = MaybeExpression::value("Bob".to_string());
    /// // This won't work directly because String doesn't implement TryFrom<Value>
    /// // Need to use resolve_as_value instead
    ///
    /// // Expression
    /// let maybe: MaybeExpression<String> = MaybeExpression::expression("{{ $input }}");
    /// ```
    pub fn resolve(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<T, ExpressionError>
    where
        T: Clone,
    {
        match self {
            Self::Value(v) => Ok(v.clone()),
            Self::Expression(cached) => {
                let value = engine.evaluate(&cached.source, context)?;
                T::try_from(value).map_err(Into::into)
            }
        }
    }
}

impl MaybeExpression<Value> {
    /// Resolve this maybe-expression to a nebula_value::Value
    ///
    /// If this is a Value variant, returns the value directly.
    /// If this is an Expression variant, evaluates the expression.
    pub fn resolve_as_value(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<Value, ExpressionError> {
        match self {
            Self::Value(v) => Ok(v.clone()),
            Self::Expression(cached) => engine.evaluate(&cached.source, context),
        }
    }
}

impl MaybeExpression<String> {
    /// Resolve this maybe-expression to a String
    ///
    /// If this is a Value variant, returns the string directly.
    /// If this is an Expression variant, evaluates the expression and converts to string.
    pub fn resolve_as_string(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<String, ExpressionError> {
        match self {
            Self::Value(s) => Ok(s.clone()),
            Self::Expression(cached) => {
                let value = engine.evaluate(&cached.source, context)?;
                Ok(value.to_string())
            }
        }
    }
}

impl MaybeExpression<i64> {
    /// Resolve this maybe-expression to an integer
    pub fn resolve_as_integer(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<i64, ExpressionError> {
        match self {
            Self::Value(i) => Ok(*i),
            Self::Expression(cached) => {
                let value = engine.evaluate(&cached.source, context)?;
                value.to_integer().map_err(Into::into)
            }
        }
    }
}

impl MaybeExpression<f64> {
    /// Resolve this maybe-expression to a float
    pub fn resolve_as_float(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<f64, ExpressionError> {
        match self {
            Self::Value(f) => Ok(*f),
            Self::Expression(cached) => {
                let value = engine.evaluate(&cached.source, context)?;
                value.to_float().map_err(Into::into)
            }
        }
    }
}

impl MaybeExpression<bool> {
    /// Resolve this maybe-expression to a boolean
    pub fn resolve_as_bool(
        &self,
        engine: &ExpressionEngine,
        context: &EvaluationContext,
    ) -> Result<bool, ExpressionError> {
        match self {
            Self::Value(b) => Ok(*b),
            Self::Expression(cached) => {
                let value = engine.evaluate(&cached.source, context)?;
                Ok(value.to_boolean())
            }
        }
    }
}

impl<T> Default for MaybeExpression<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::Value(T::default())
    }
}

impl<T> From<T> for MaybeExpression<T> {
    fn from(value: T) -> Self {
        Self::Value(value)
    }
}

// Serialization
impl<T> Serialize for MaybeExpression<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Value(v) => v.serialize(serializer),
            Self::Expression(cached) => cached.source.serialize(serializer),
        }
    }
}

// Deserialization
impl<'de, T> Deserialize<'de> for MaybeExpression<T>
where
    T: DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // First try to deserialize as a string to check if it's an expression
        let value = serde_json::Value::deserialize(deserializer)?;

        if let Some(s) = value.as_str() {
            // If it's a string, check if it looks like an expression
            if is_expression(s) {
                return Ok(Self::Expression(CachedExpression {
                    source: s.to_string(),
                    ast: OnceCell::new(),
                }));
            }
        }

        // Otherwise, try to deserialize as T
        T::deserialize(value)
            .map(Self::Value)
            .map_err(serde::de::Error::custom)
    }
}

/// Check if a string looks like an expression (contains {{ }})
fn is_expression(s: &str) -> bool {
    s.contains("{{") && s.contains("}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::EvaluationContext;
    use crate::engine::ExpressionEngine;

    #[test]
    fn test_maybe_expression_value() {
        let maybe: MaybeExpression<String> = MaybeExpression::value("hello".to_string());
        assert!(maybe.is_value());
        assert!(!maybe.is_expression());
        assert_eq!(maybe.as_value(), Some(&"hello".to_string()));
        assert_eq!(maybe.as_expression(), None);
    }

    #[test]
    fn test_maybe_expression_expression() {
        let maybe: MaybeExpression<String> = MaybeExpression::expression("{{ $input }}");
        assert!(!maybe.is_value());
        assert!(maybe.is_expression());
        assert_eq!(maybe.as_value(), None);
        assert_eq!(maybe.as_expression(), Some("{{ $input }}"));
    }

    #[test]
    fn test_maybe_expression_from() {
        let maybe: MaybeExpression<i64> = 42.into();
        assert!(maybe.is_value());
        assert_eq!(maybe.as_value(), Some(&42));
    }

    #[test]
    fn test_resolve_string_value() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let maybe = MaybeExpression::value("hello".to_string());
        let result = maybe.resolve_as_string(&engine, &context).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_resolve_string_expression() {
        let engine = ExpressionEngine::new();
        let mut context = EvaluationContext::new();
        context.set_input(Value::text("world"));

        let maybe = MaybeExpression::expression("{{ $input }}");
        let result = maybe.resolve_as_string(&engine, &context).unwrap();
        assert_eq!(result, "world");
    }

    #[test]
    fn test_resolve_integer_value() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let maybe = MaybeExpression::value(42);
        let result = maybe.resolve_as_integer(&engine, &context).unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_resolve_integer_expression() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let maybe = MaybeExpression::expression("{{ 2 + 2 }}");
        let result = maybe.resolve_as_integer(&engine, &context).unwrap();
        assert_eq!(result, 4);
    }

    #[test]
    fn test_resolve_bool_value() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let maybe = MaybeExpression::value(true);
        let result = maybe.resolve_as_bool(&engine, &context).unwrap();
        assert_eq!(result, true);
    }

    #[test]
    fn test_resolve_bool_expression() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let maybe = MaybeExpression::expression("{{ 5 > 3 }}");
        let result = maybe.resolve_as_bool(&engine, &context).unwrap();
        assert_eq!(result, true);
    }

    #[test]
    fn test_serde_value() {
        let maybe: MaybeExpression<String> = MaybeExpression::value("hello".to_string());
        let json = serde_json::to_string(&maybe).unwrap();
        assert_eq!(json, r#""hello""#);

        let deserialized: MaybeExpression<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, maybe);
    }

    #[test]
    fn test_serde_expression() {
        let maybe: MaybeExpression<String> = MaybeExpression::expression("{{ $input }}");
        let json = serde_json::to_string(&maybe).unwrap();
        assert_eq!(json, r#""{{ $input }}""#);

        let deserialized: MaybeExpression<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, maybe);
    }
}
