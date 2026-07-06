//! Expression value wrapper — lazy parse via OnceLock.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
};

use crate::{error::ValidationError, path::FieldPath};

/// Boxed future returned by [`ExpressionContext::evaluate`].
///
/// Stored as a type alias so impls can write `Box::pin(async move { … })`
/// without spelling out the full `Pin<Box<dyn Future …>>` shape.
pub type EvalFuture<'a> =
    Pin<Box<dyn Future<Output = Result<serde_json::Value, ValidationError>> + Send + 'a>>;

/// Minimal contract required to evaluate an expression at runtime.
///
/// Implement this to bridge nebula-schema's resolution phase with any
/// expression engine. The real evaluator lives in `nebula-expression`;
/// this trait is the integration seam so tests can use a stub.
///
/// > **Status: latent.** No production crate implements this trait yet — only
/// > test and example stubs do. It is the dormant half of the
/// > [`ValidValues::resolve`](crate::ValidValues::resolve) seam (see that
/// > method's status note) and becomes load-bearing once the engine wires a
/// > real evaluator for action-input expressions.
///
/// The trait is dyn-safe: callers receive `&dyn ExpressionContext` from
/// [`ValidValues::resolve`](crate::ValidValues::resolve). The `evaluate`
/// method intentionally returns a [`EvalFuture`] (boxed future) instead of
/// using `async fn` so the trait stays object-safe under Rust 1.95 / edition
/// 2024 without an `async-trait` macro indirection.
///
/// # Example
///
/// ```rust
/// use nebula_schema::{EvalFuture, ExpressionAst, ExpressionContext, ValidationError};
///
/// struct ConstCtx(serde_json::Value);
///
/// impl ExpressionContext for ConstCtx {
///     fn evaluate<'a>(&'a self, _ast: &'a ExpressionAst) -> EvalFuture<'a> {
///         Box::pin(async move { Ok(self.0.clone()) })
///     }
/// }
/// ```
pub trait ExpressionContext: Send + Sync {
    /// Evaluate a parsed expression AST and return the resulting JSON value.
    ///
    /// Errors should use code `"expression.runtime"`.
    ///
    /// cancel-safe: implementors SHOULD be cancel-safe.
    /// [`ValidValues::resolve`](crate::ValidValues::resolve) drives this future
    /// under the caller's executor and may drop it at the `.await` if the
    /// surrounding task is cancelled, so an `evaluate` impl that
    /// performs external side effects must tolerate being dropped mid-flight
    /// (e.g. be idempotent or detach durable work via its own `spawn`).
    fn evaluate<'a>(&'a self, ast: &'a ExpressionAst) -> EvalFuture<'a>;
}

/// Opaque parsed AST wrapper.
///
/// The parse/grammar source of truth is `nebula-expression`; this struct
/// intentionally exposes only the original source so schema consumers are not
/// coupled to expression crate internals.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExpressionAst {
    /// Raw expression source — the only payload this wrapper exposes.
    pub(crate) source: Arc<str>,
}

impl ExpressionAst {
    /// Borrow the raw expression source.
    #[must_use]
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
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Lazy parse — caches the first parse result (success or error).
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `expression.parse` if parsing fails.
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
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `expression.parse` if parsing fails.
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

/// Production-ready [`ExpressionContext`] backed by [`nebula_expression::ExpressionEngine`].
///
/// Centralizes the schema↔expression bridge so callers (integration tests,
/// future engine wiring) do not reimplement `evaluate` →
/// `engine.evaluate(source, ctx)` themselves. [`ExpressionAst`] exposes only
/// the source string by design, so evaluation always routes through the engine's
/// parse+eval path.
///
/// # Example
///
/// ```rust,no_run
/// use nebula_schema::{EngineExpressionContext, Field, FieldValues, Schema, field_key};
/// use serde_json::json;
///
/// # async fn demo() -> Result<(), nebula_schema::ValidationReport> {
/// let schema = Schema::builder()
///     .add(Field::string(field_key!("greeting")))
///     .build()
///     .unwrap();
/// let values = FieldValues::from_json(json!({"greeting": "{{ $input.name }}"})).unwrap();
/// let valid = schema.validate(&values).unwrap();
/// let ctx = EngineExpressionContext::with_input(json!({"name": "world"}));
/// let resolved = valid.resolve(&ctx).await?;
/// # let resolved = valid.resolve(&ctx).await.unwrap();
/// assert_eq!(resolved.get(&field_key!("greeting")), Some(&json!("world")));
/// # Ok(())
/// # }
/// ```
pub struct EngineExpressionContext {
    engine: nebula_expression::ExpressionEngine,
    ctx: nebula_expression::EvaluationContext,
}

impl EngineExpressionContext {
    /// Wrap an engine and evaluation context.
    #[must_use]
    pub fn new(
        engine: nebula_expression::ExpressionEngine,
        ctx: nebula_expression::EvaluationContext,
    ) -> Self {
        Self { engine, ctx }
    }

    /// Convenience: default engine with `$input` bound to `input`.
    #[must_use]
    pub fn with_input(input: serde_json::Value) -> Self {
        let mut ctx = nebula_expression::EvaluationContext::new();
        ctx.set_input(input);
        Self::new(nebula_expression::ExpressionEngine::new(), ctx)
    }

    /// Borrow the underlying evaluation context (for adding `$execution` / `$node` vars).
    #[must_use]
    pub fn evaluation_context(&self) -> &nebula_expression::EvaluationContext {
        &self.ctx
    }

    /// Mutably borrow the evaluation context.
    pub fn evaluation_context_mut(&mut self) -> &mut nebula_expression::EvaluationContext {
        &mut self.ctx
    }
}

impl ExpressionContext for EngineExpressionContext {
    fn evaluate<'a>(&'a self, ast: &'a ExpressionAst) -> EvalFuture<'a> {
        let source = ast.source().to_owned();
        Box::pin(async move {
            self.engine.evaluate(&source, &self.ctx).map_err(|e| {
                ValidationError::builder("expression.runtime")
                    .message(format!("expression `{source}` failed: {e}"))
                    .build()
            })
        })
    }
}

/// Equality is **by source string, not by parsed AST** — two expressions that
/// parse to the same tree but differ in whitespace or formatting compare
/// unequal. The canonical-bytes content id keys off the same `source()` bytes
/// (`value.rs` writes `expr.source().as_bytes()`), so it shares this exact
/// limitation — neither offers AST-level / semantic dedup. A caller that needs
/// to collapse whitespace- or formatting-only differences must normalize the
/// source (or parse and compare the AST) itself. Parsing inside `eq` is
/// deliberately avoided — it would be surprising and would allocate.
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

    #[tokio::test]
    async fn engine_context_evaluates_input_template() {
        use serde_json::json;

        use crate::{Field, FieldValues, Schema, field_key};

        let schema = Schema::builder()
            .add(Field::string(field_key!("greeting")))
            .build()
            .expect("schema builds");
        let values =
            FieldValues::from_json(json!({"greeting": "{{ $input.name }}"})).expect("values parse");
        let valid = schema.validate(&values).expect("values validate");
        let ctx = EngineExpressionContext::with_input(json!({"name": "world"}));
        let resolved = valid.resolve(&ctx).await.expect("resolve succeeds");
        assert_eq!(resolved.get(&field_key!("greeting")), Some(&json!("world")));
    }
}
