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

/// Opaque parsed AST. In Phase 1 this is a thin newtype; Phase 4 can replace
/// the inner type with a real `nebula_expression::Ast`.
#[derive(Debug, Clone)]
pub struct ExpressionAst(pub Arc<str>);

/// An unresolved expression (e.g. `{{ $input.name }}`).
#[derive(Debug, Clone)]
pub struct Expression {
    source: Arc<str>,
    parsed: Arc<OnceLock<ExpressionAst>>,
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

    /// Lazy parse — caches the first successful parse.
    #[allow(clippy::result_large_err)]
    pub fn parse(&self) -> Result<&ExpressionAst, ValidationError> {
        Ok(self.parsed.get_or_init(|| {
            // Phase 1: no real AST — just wrap the source.
            // Phase 4 replaces this with nebula_expression::parse(&self.source).
            ExpressionAst(self.source.clone())
        }))
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
        let a1 = e.parse().unwrap() as *const _;
        let a2 = e.parse().unwrap() as *const _;
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
}
