//! Expression value wrapper — lazy parse via OnceLock.

use std::sync::{Arc, OnceLock};

use crate::{error::ValidationError, path::FieldPath};

/// Minimal contract required to evaluate an expression at runtime.
///
/// Implement this to bridge nebula-schema's resolution phase with any
/// expression engine. The real evaluator lives in `nebula-expression`;
/// this trait is the integration seam so Phase 1 tests can use a stub.
///
/// # Example
///
/// ```rust
/// use nebula_schema::{ExpressionAst, ExpressionContext, ValidationError};
///
/// struct ConstCtx(serde_json::Value);
///
/// #[async_trait::async_trait]
/// impl ExpressionContext for ConstCtx {
///     async fn evaluate(
///         &self,
///         _ast: &ExpressionAst,
///     ) -> Result<serde_json::Value, ValidationError> {
///         Ok(self.0.clone())
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait ExpressionContext: Send + Sync {
    /// Evaluate a parsed expression AST and return the resulting JSON value.
    ///
    /// Errors should use code `"expression.runtime"`.
    async fn evaluate(&self, ast: &ExpressionAst) -> Result<serde_json::Value, ValidationError>;
}

/// Opaque parsed AST wrapper.
///
/// The parse/grammar source of truth is `nebula-expression`; this struct
/// intentionally exposes only the original source so schema consumers are not
/// coupled to expression crate internals.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExpressionAst {
    /// Raw expression source — the only payload exposed in Phase 1.
    pub(crate) source: Arc<str>,
}

impl ExpressionAst {
    /// Borrow the raw expression source.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// An unresolved expression (e.g. `{{ $input.name }}`).
#[derive(Debug, Clone)]
pub struct Expression {
    source: Arc<str>,
    parsed: Arc<OnceLock<Result<ExpressionAst, Arc<str>>>>,
}

impl Expression {
    /// Wrap an expression source string.
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            parsed: Arc::new(OnceLock::new()),
        }
    }

    /// Return the raw expression source.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Lazy parse — caches the first parse result (success or error).
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn parse(&self) -> Result<&ExpressionAst, ValidationError> {
        self.parse_at(&FieldPath::root())
    }

    /// Lazy parse with caller-provided path context for errors.
    ///
    /// The parse result is cached (success or syntax failure), while the
    /// returned [`ValidationError`] path is attached per call site.
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn parse_at(&self, path: &FieldPath) -> Result<&ExpressionAst, ValidationError> {
        match self.parsed.get_or_init(|| {
            parse_expression_source(self.source())
                .map(|()| ExpressionAst {
                    source: self.source.clone(),
                })
                .map_err(Arc::<str>::from)
        }) {
            Ok(ast) => Ok(ast),
            Err(message) => Err(ValidationError::builder("expression.parse")
                .at(path.clone())
                .message(message.to_string())
                .param("source", self.source.to_string())
                .build()),
        }
    }

    /// Build a parse error tagged for this expression.
    #[allow(dead_code)]
    pub(crate) fn parse_error(
        &self,
        msg: impl Into<std::borrow::Cow<'static, str>>,
    ) -> ValidationError {
        ValidationError::builder("expression.parse")
            .at(FieldPath::root())
            .message(msg)
            .param("source", self.source.to_string())
            .build()
    }
}

fn parse_expression_source(source: &str) -> Result<(), String> {
    nebula_expression::parse_expression(source).map_err(|e| e.to_string())
}

impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_parse_is_cached() {
        let e = Expression::new("{{ $x }}");
        let a1 = std::ptr::from_ref(e.parse().unwrap());
        let a2 = std::ptr::from_ref(e.parse().unwrap());
        assert_eq!(a1, a2, "parse should cache the same AST instance");
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
        // params stored as Arc<[(Cow, Value)]>
        let found = err
            .params
            .iter()
            .any(|(k, v)| k.as_ref() == "source" && v.as_str() == Some("bad"));
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
        assert_eq!(err.path.to_string(), "foo.bar");
    }
}
