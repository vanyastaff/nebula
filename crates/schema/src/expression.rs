//! Authored sources and the retained-program evaluation boundary.

use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
};

pub use nebula_expression::{CompiledProgram, ProgramSyntax};

use crate::{ValidationError, ValuePath};

/// Boxed future returned by [`ExpressionContext::evaluate`].
///
/// Stored as a type alias so impls can write `Box::pin(async move { … })`
/// without spelling out the full `Pin<Box<dyn Future …>>` shape.
pub type EvalFuture<'a> =
    Pin<Box<dyn Future<Output = Result<serde_json::Value, ValidationError>> + Send + 'a>>;

/// Evaluation of an already admitted, compiled program.
///
/// Implementations must tolerate cancellation at any await point. Schema
/// resolution owns no external side effects and never reparses returned data.
/// The trait is object-safe for runtime adapters and test contexts.
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
    fn evaluate<'a>(&'a self, ast: &'a CompiledProgram) -> EvalFuture<'a>;
}

/// An unresolved expression (e.g. `{{ $input.name }}`).
#[derive(Clone)]
pub struct Expression {
    source: Arc<str>,
    syntax: ProgramSyntax,
    parsed: Arc<OnceLock<Result<CompiledProgram, Arc<nebula_expression::ExpressionError>>>>,
}

impl Expression {
    /// Wrap a source using [`ProgramSyntax::Auto`]. Compilation remains lazy.
    #[must_use]
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self::with_syntax(source, ProgramSyntax::Auto)
    }

    /// Wrap text interpolation, always resolving to a string, even a lone envelope.
    ///
    /// ```
    /// use nebula_schema::{Expression, ProgramSyntax};
    /// let expression = Expression::template("{{ 7 }}");
    /// assert_eq!(expression.syntax(), ProgramSyntax::Template);
    /// ```
    #[must_use]
    pub fn template(source: impl Into<Arc<str>>) -> Self {
        Self::with_syntax(source, ProgramSyntax::Template)
    }

    /// Wrap source with immutable syntax and a fresh, shared compilation cache.
    ///
    /// ```
    /// use nebula_schema::{Expression, ProgramSyntax};
    /// let expression = Expression::with_syntax("7", ProgramSyntax::Expression);
    /// assert_eq!(expression.syntax(), ProgramSyntax::Expression);
    /// ```
    #[must_use]
    pub fn with_syntax(source: impl Into<Arc<str>>, syntax: ProgramSyntax) -> Self {
        Self {
            source: source.into(),
            syntax,
            parsed: Arc::new(OnceLock::new()),
        }
    }

    /// Return the raw expression source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Return the authored grammar without compiling the source.
    #[must_use]
    pub const fn syntax(&self) -> ProgramSyntax {
        self.syntax
    }

    /// Lazy parse — caches the first parse result (success or error).
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `expression.parse` if parsing fails.
    pub fn parse(&self) -> Result<&CompiledProgram, ValidationError> {
        self.parse_at(&ValuePath::root())
    }

    /// Lazy parse with caller-provided path context for errors.
    ///
    /// The parse result is cached (success or syntax failure), while the
    /// returned [`ValidationError`] path is attached per call site.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `expression.parse` if parsing fails.
    #[tracing::instrument(level = "debug", skip_all, fields(path = %path, syntax = ?self.syntax))]
    pub fn parse_at(&self, path: &ValuePath) -> Result<&CompiledProgram, ValidationError> {
        match self.parsed.get_or_init(|| {
            CompiledProgram::compile_with_syntax(self.source(), self.syntax).map_err(Arc::new)
        }) {
            Ok(program) => Ok(program),
            Err(error) => Err(ValidationError::builder("expression.parse")
                .at(path.clone())
                .message("expression syntax is invalid")
                .private_source(Arc::clone(error))
                .build()),
        }
    }
}

impl fmt::Debug for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Expression")
            .field("source_bytes", &self.source.len())
            .field("syntax", &self.syntax)
            .field("compiled", &self.parsed.get().map(Result::is_ok))
            .finish_non_exhaustive()
    }
}

/// Runtime adapter using the engine\'s current registry and evaluation policy.
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
    fn evaluate<'a>(&'a self, program: &'a CompiledProgram) -> EvalFuture<'a> {
        Box::pin(async move {
            self.engine
                .evaluate_compiled(program, &self.ctx)
                .map_err(|error| {
                    ValidationError::builder("expression.runtime")
                        .message("expression evaluation failed")
                        .private_source(error)
                        .build()
                })
        })
    }
}

/// Equality compares authored syntax and exact source, never parsed ASTs.
/// Canonical tree identity uses the same pair; whitespace is not normalized.
impl PartialEq for Expression {
    fn eq(&self, other: &Self) -> bool {
        self.syntax == other.syntax && self.source == other.source
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
        assert_eq!(
            a1, a2,
            "parse should cache the same compiled program instance"
        );
    }

    #[test]
    fn clones_share_source() {
        let e = Expression::new("{{ $y }}");
        let c = e.clone();
        assert_eq!(e.source(), c.source());
    }

    #[test]
    fn parse_error_redacts_source_but_retains_typed_cause() {
        let error = Expression::new("{{ 'PRIVATE_SOURCE' + }}")
            .parse()
            .unwrap_err();
        assert_eq!(error.code(), "expression.parse");
        assert!(!format!("{error:?}").contains("PRIVATE_SOURCE"));
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("PRIVATE_SOURCE")
        );
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn parse_invalid_expression_returns_expression_parse() {
        let e = Expression::new("{{ 1 + }}");
        let err = e.parse().unwrap_err();
        assert_eq!(err.code(), "expression.parse");
    }

    #[test]
    fn parse_at_uses_requested_path() {
        let e = Expression::new("{{ 1 + }}");
        let err = e
            .parse_at(&ValuePath::parse("/foo/bar").expect("valid path"))
            .unwrap_err();
        assert_eq!(err.path().to_string(), "/foo/bar");
    }

    #[tokio::test]
    async fn engine_context_evaluates_input_template() {
        use serde_json::json;

        use crate::{AuthoredValue, Field, Schema, field_key};

        let schema = Schema::builder()
            .add(Field::string(field_key!("greeting")))
            .build()
            .expect("schema builds");
        let values = AuthoredValue::from_template_json(json!({"greeting": "{{ $input.name }}"}))
            .expect("values parse");
        let valid = schema.validate(values).expect("values validate");
        let ctx = EngineExpressionContext::with_input(json!({"name": "world"}));
        let resolved = valid.resolve(&ctx).await.expect("resolve succeeds");
        assert_eq!(resolved.get(&field_key!("greeting")), Some(&json!("world")));
    }

    #[tokio::test]
    async fn engine_context_renders_inline_template() {
        use serde_json::json;

        use crate::{AuthoredValue, Field, Schema, field_key};

        let schema = Schema::builder()
            .add(Field::string(field_key!("greeting")))
            .build()
            .expect("schema builds");
        let values =
            AuthoredValue::from_template_json(json!({"greeting": "hello {{ $input.name }}"}))
                .expect("values parse");
        let valid = schema.validate(values).expect("values validate");
        let ctx = EngineExpressionContext::with_input(json!({"name": "world"}));
        let resolved = valid.resolve(&ctx).await.expect("resolve succeeds");
        assert_eq!(
            resolved.get(&field_key!("greeting")),
            Some(&json!("hello world"))
        );
    }

    #[test]
    fn resolve_expression_value_preserves_typed_lone_envelope() {
        let engine = nebula_expression::ExpressionEngine::new();
        let mut ctx = nebula_expression::EvaluationContext::new();
        ctx.set_input(serde_json::json!({"count": 7}));

        let value = engine
            .evaluate_compiled(
                &CompiledProgram::compile("{{ $input.count + 1 }}").unwrap(),
                &ctx,
            )
            .expect("typed lone envelope evaluates");
        assert_eq!(value, serde_json::json!(8));
    }

    #[test]
    fn resolve_expression_value_renders_mixed_template() {
        let engine = nebula_expression::ExpressionEngine::new();
        let mut ctx = nebula_expression::EvaluationContext::new();
        ctx.set_input(serde_json::json!({"name": "world"}));

        let value = engine
            .evaluate_compiled(
                &CompiledProgram::compile("hello {{ $input.name }}").unwrap(),
                &ctx,
            )
            .expect("mixed template renders");
        assert_eq!(value, serde_json::json!("hello world"));
    }
}
