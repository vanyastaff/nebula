//! Expression value wrapper — single-parse via `nebula_expression::CachedExpression`.

use std::{borrow::Cow, sync::Arc};

use nebula_expression::CachedExpression;

use crate::{error::ValidationError, path::FieldPath};

/// Minimal contract required to evaluate an expression at runtime.
///
/// Implement this to bridge nebula-schema's resolution phase with an
/// expression engine. The real evaluator lives in `nebula-expression`; this
/// trait is the integration seam so tests can use a stub.
///
/// The seam is **synchronous**: the expression evaluator is synchronous and
/// resolution performs no I/O, so wrapping it in a boxed future bought nothing
/// but per-node allocation. Implementors evaluate the pre-parsed
/// [`ExpressionAst`] — typically via
/// [`ExpressionEngine::evaluate_cached`](nebula_expression::ExpressionEngine::evaluate_cached) —
/// which reuses the AST parsed at schema-validation time (no re-parse).
///
/// # Example
///
/// ```rust
/// use nebula_schema::{ExpressionAst, ExpressionContext, ValidationError};
///
/// struct ConstCtx(serde_json::Value);
///
/// impl ExpressionContext for ConstCtx {
///     fn evaluate(&self, _ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError> {
///         Ok(self.0.clone())
///     }
/// }
/// ```
pub trait ExpressionContext: Send + Sync {
    /// Evaluate a parsed expression and return the resulting JSON value.
    ///
    /// Errors should use code `"expression.runtime"`.
    ///
    /// # Errors
    ///
    /// Returns a `ValidationError` when evaluation fails.
    fn evaluate(&self, ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError>;
}

/// A parsed, reusable expression handle.
///
/// Wraps `nebula_expression::CachedExpression` so the source is parsed exactly
/// once — at schema-validation time — and the same AST is reused at resolve
/// time. The `Arc` makes cloning cheap and keeps the cached AST shared.
#[derive(Debug, Clone)]
pub struct ExpressionAst {
    cached: Arc<CachedExpression>,
}

impl ExpressionAst {
    /// Borrow the underlying cached expression (for `evaluate_cached`).
    #[must_use]
    pub fn cached(&self) -> &CachedExpression {
        &self.cached
    }

    /// Borrow the raw expression source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.cached.source
    }
}

/// An unresolved expression (e.g. `{{ $input.name }}`).
#[derive(Debug, Clone)]
pub struct Expression {
    cached: Arc<CachedExpression>,
}

impl Expression {
    /// Wrap an expression source string.
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            cached: Arc::new(CachedExpression::new(source)),
        }
    }

    /// Return the raw expression source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.cached.source
    }

    /// Parse the expression, returning a reusable [`ExpressionAst`].
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `expression.parse` if parsing fails.
    pub fn parse(&self) -> Result<ExpressionAst, ValidationError> {
        self.parse_at(&FieldPath::root())
    }

    /// Parse with caller-provided path context for errors.
    ///
    /// Forces the parse now (a syntax check) and caches the AST inside the
    /// shared `CachedExpression`, so resolve-time evaluation reuses it without
    /// re-parsing. The parse result (success or failure) is cached; the
    /// returned error path is attached per call site.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `expression.parse` if parsing fails.
    pub fn parse_at(&self, path: &FieldPath) -> Result<ExpressionAst, ValidationError> {
        match self.cached.ast() {
            Ok(_) => Ok(ExpressionAst {
                cached: Arc::clone(&self.cached),
            }),
            Err(e) => Err(ValidationError::builder("expression.parse")
                .at_field(path.to_string())
                .message(e.to_string())
                .param("source", self.cached.source.clone())
                .build()),
        }
    }

    /// Build a parse error tagged for this expression.
    #[allow(dead_code)]
    pub(crate) fn parse_error(&self, msg: impl Into<Cow<'static, str>>) -> ValidationError {
        ValidationError::builder("expression.parse")
            .message(msg)
            .param("source", self.cached.source.clone())
            .build()
    }
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.cached.source == other.cached.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_caches_ast_across_calls() {
        let e = Expression::new("{{ $x }}");
        let a1 = e.parse().unwrap();
        let a2 = e.parse().unwrap();
        // Both handles share the same cached expression instance.
        assert!(Arc::ptr_eq(&a1.cached, &a2.cached));
    }

    #[test]
    fn clones_share_source() {
        let e = Expression::new("{{ $y }}");
        let c = e.clone();
        assert_eq!(e.source(), c.source());
    }

    #[test]
    fn parse_error_carries_source_param() {
        let e = Expression::new("bad");
        let err = e.parse_error("boom");
        assert_eq!(err.code, "expression.parse");
        let found = err
            .params()
            .iter()
            .any(|(k, v)| k.as_ref() == "source" && v.as_ref() == "bad");
        assert!(found, "source param not found");
    }

    #[test]
    fn parse_invalid_expression_returns_expression_parse() {
        let e = Expression::new("{{ 1 + }}");
        let err = e.parse().unwrap_err();
        assert_eq!(err.code, "expression.parse");
    }

    #[test]
    fn parse_at_uses_requested_path() {
        let e = Expression::new("{{ 1 + }}");
        let err = e
            .parse_at(&FieldPath::parse("foo.bar").expect("valid path"))
            .unwrap_err();
        assert_eq!(err.field.as_deref(), Some("/foo/bar"));
    }
}
