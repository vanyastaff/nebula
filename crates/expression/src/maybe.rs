//! MaybeExpression type for values that can be either concrete or expressions
//!
//! This module provides the `MaybeExpression<T>` enum which allows parameters
//! to accept either a concrete value of type T or a string expression that will
//! be evaluated at runtime.

use std::sync::OnceLock;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned, ser::SerializeMap,
};
use serde_json::Value;

use crate::{ExpressionError, ast::Expr, context::EvaluationContext, engine::ExpressionEngine};

/// Tag key used for `MaybeExpression::Expression` on the wire.
///
/// Wrapping the expression source in `{"$expr": "..."}` instead of
/// piggybacking on bare strings ensures a round-trip is lossless: a
/// literal `String` containing `{{ ... }}` no longer gets silently
/// reinterpreted as an expression on deserialize.
const EXPR_TAG: &str = "$expr";

/// Internal structure for cached expression parsing
#[derive(Debug)]
#[doc(hidden)]
pub struct CachedExpression {
    /// Source expression string
    pub source: String,
    #[doc(hidden)]
    pub ast: OnceLock<Expr>,
}

impl Clone for CachedExpression {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            ast: OnceLock::new(), // Don't clone the cached AST, let it re-parse if needed
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
/// and the parsed AST is cached for subsequent evaluations using OnceLock.
///
/// # Serialization
///
/// `Value(T)` serializes as the bare inner `T`. `Expression(...)` is
/// tagged: it serializes as `{"$expr": "<source>"}` so that a literal
/// `String` carrying `{{` / `}}` cannot be confused for an expression
/// on round-trip — the previous heuristic (`s.contains("{{")`) was
/// lossy.
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
/// {"$expr": "{{ $input.name }}"}
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
            ast: OnceLock::new(),
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
    /// use nebula_expression::{EvaluationContext, ExpressionEngine, MaybeExpression};
    /// use serde_json::Value;
    ///
    /// let engine = ExpressionEngine::new();
    /// let mut context = EvaluationContext::new();
    /// context.set_input(Value::String("Alice".to_string()));
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
            },
        }
    }
}

impl MaybeExpression<Value> {
    /// Resolve this maybe-expression to a serde_json::Value
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
                match value.as_str() {
                    Some(s) => Ok(s.to_owned()),
                    None => Ok(value.to_string()),
                }
            },
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
                value.as_i64().ok_or_else(|| {
                    ExpressionError::type_error(
                        "integer",
                        crate::value_utils::value_type_name(&value),
                    )
                })
            },
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
                crate::value_utils::to_float(&value)
                    .map_err(|e| ExpressionError::type_error("float", e))
            },
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
                Ok(crate::value_utils::to_boolean(&value))
            },
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
            Self::Expression(cached) => {
                // Tagged form: {"$expr": "<source>"} — distinct from any
                // bare string a caller might supply, including one that
                // happens to contain `{{ }}`.
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(EXPR_TAG, &cached.source)?;
                map.end()
            },
        }
    }
}

/// Detect the tagged `{"$expr": "..."}` form on the wire.
///
/// Returns the source string if and only if the JSON is exactly an
/// object with one key, `$expr`, mapping to a string. Anything else —
/// including objects that contain `$expr` alongside other keys — falls
/// through to the literal-`T` deserialize path.
fn extract_expr_tag(value: &Value) -> Option<&str> {
    let obj = value.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.get(EXPR_TAG).and_then(Value::as_str)
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
        let value = Value::deserialize(deserializer)?;

        if let Some(source) = extract_expr_tag(&value) {
            return Ok(Self::Expression(CachedExpression {
                source: source.to_string(),
                ast: OnceLock::new(),
            }));
        }

        T::deserialize(value)
            .map(Self::Value)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::EvaluationContext, engine::ExpressionEngine};

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
        context.set_input(Value::String("world".to_string()));

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
        assert!(result);
    }

    #[test]
    fn test_resolve_bool_expression() {
        let engine = ExpressionEngine::new();
        let context = EvaluationContext::new();

        let maybe = MaybeExpression::expression("{{ 5 > 3 }}");
        let result = maybe.resolve_as_bool(&engine, &context).unwrap();
        assert!(result);
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
        assert_eq!(json, r#"{"$expr":"{{ $input }}"}"#);

        let deserialized: MaybeExpression<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, maybe);
    }

    #[test]
    fn literal_string_with_braces_round_trips_as_value() {
        // Regression: pre-fix, a bare string containing `{{` and `}}` was
        // mis-routed to `Expression` on deserialize, so a payload like
        // `"hello {{ stay literal }}"` lost its meaning. With the tagged
        // form, only `{"$expr": ...}` is treated as an expression — bare
        // strings always round-trip as Value.
        let literal: MaybeExpression<String> =
            MaybeExpression::value("hello {{ stay literal }}".to_string());
        let json = serde_json::to_string(&literal).unwrap();
        assert_eq!(json, r#""hello {{ stay literal }}""#);

        let back: MaybeExpression<String> = serde_json::from_str(&json).unwrap();
        assert!(back.is_value(), "bare string must deserialize as Value");
        assert_eq!(
            back.as_value().map(String::as_str),
            Some("hello {{ stay literal }}")
        );
    }

    #[test]
    fn object_with_extra_keys_is_not_expression() {
        // Tagged form is *exactly* `{"$expr": "..."}`. An object that has
        // `$expr` alongside other keys should be treated as plain JSON
        // payload (Value variant for `MaybeExpression<Value>`), not as
        // an expression.
        let json = r#"{"$expr": "{{ x }}", "extra": 1}"#;
        let parsed: MaybeExpression<Value> = serde_json::from_str(json).unwrap();
        assert!(
            parsed.is_value(),
            "objects with extra keys must NOT be Expression"
        );
    }

    #[test]
    fn deserializing_old_bare_expression_form_now_yields_value() {
        // Documented behavioural change: the old wire format used a bare
        // string with `{{ }}` for Expression. Under the new tagged
        // form, that wire shape is interpreted as Value. Callers
        // migrating data must rewrite to `{"$expr": "..."}` form.
        let old_wire = r#""{{ $input }}""#;
        let parsed: MaybeExpression<String> = serde_json::from_str(old_wire).unwrap();
        assert!(parsed.is_value());
        assert_eq!(parsed.as_value().map(String::as_str), Some("{{ $input }}"));
    }
}
