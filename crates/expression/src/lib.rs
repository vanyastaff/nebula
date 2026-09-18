#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![warn(unreachable_pub)]
#![warn(missing_docs)]
#![allow(clippy::excessive_nesting)]
#![allow(clippy::needless_range_loop)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! # nebula-expression
//!
//! Expression evaluator for dynamic workflow field resolution. Evaluates
//! `{{ expression }}` interpolation, `{% if %}` / `{% for %}` blocks, and
//! `{# #}` comments against execution-time context. It is the resolution
//! backend used by `nebula-schema`'s `ValidValues::resolve` step.
//!
//! ## Quick Start
//!
//! ```
//! use nebula_expression::{EvaluationContext, ExpressionEngine};
//! use serde_json::Value;
//!
//! let engine = ExpressionEngine::new();
//! let mut context = EvaluationContext::new();
//! context.set_execution_var("id", Value::String("exec-123".to_string()));
//! let result = engine.evaluate("$execution.id", &context)?;
//! assert_eq!(result.as_str(), Some("exec-123"));
//! # Ok::<(), nebula_expression::ExpressionError>(())
//! ```
//!
//! Typed values such as date-times stay typed inside evaluation; use
//! [`ExpressionEngine::evaluate_runtime`] when the result should keep them
//! instead of rendering to JSON.
//!
//! ## Core Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`ExpressionEngine`] | Parse and evaluate expressions; optional LRU cache |
//! | [`CompiledProgram`] | Immutable syntax retained across evaluations; raw, template, or auto compilation |
//! | [`ProgramSyntax`] | Authored grammar: auto, raw expression, or text template |
//! | [`EvaluationContext`] | Runtime variable bindings (`$node`, `$execution`, `$workflow`, `$input`, `$json`) |
//! | [`EvaluationPolicy`] | Function restrictions, coercion rules, and bounded work/JSON input |
//! | [`MissingLookup`] | Whether a missing lookup errors or yields `Undefined` |
//! | [`Template`] | Pre-parsed `{{ }}` / `{% %}` template; call `.render(engine, ctx)` |
//! | [`MaybeExpression`] | Typed wrapper: literal `T` or expression string |
//! | [`MaybeTemplate`] | Text template wrapper with auto-detection |
//! | [`RuntimeValue`] | Evaluator value model: JSON shapes plus typed date-times and `Undefined` |
//! | [`ExpressionError`] | Typed evaluation error |
//!
//! ## Extending the evaluator
//!
//! [`ExpressionEngine::register_function`] adds a custom builtin. The callback
//! is a [`BuiltinFunction`]: it receives [`Argument`]s (already-evaluated values
//! or unevaluated lambdas) and a [`BuiltinView`], and returns an opaque
//! [`BuiltinOutput`] built through the mandatory [`BuiltinOutputBuilder`].
//!
//! ```
//! use nebula_expression::{
//!     BuiltinOutput, BuiltinOutputBuilder, BuiltinView, EvaluationContext,
//!     ExpressionEngine, ExpressionResult, Argument,
//! };
//!
//! fn triple(
//!     args: &[Argument<'_>],
//!     _view: BuiltinView<'_>,
//!     _context: &EvaluationContext,
//!     output: BuiltinOutputBuilder,
//! ) -> ExpressionResult<BuiltinOutput> {
//!     let value = args[0].as_value().and_then(|value| value.as_i64()).unwrap_or_default();
//!     output.signed_integer(value * 3)
//! }
//!
//! let mut engine = ExpressionEngine::new();
//! engine.register_function("triple", triple);
//! let result = engine.evaluate("triple(7)", &EvaluationContext::new())?;
//! assert_eq!(result.as_i64(), Some(21));
//! # Ok::<(), nebula_expression::ExpressionError>(())
//! ```
//!
//! Lambdas are invoked through [`BuiltinView::invoke_lambda`], which evaluates
//! the body against the caller's frame — a registered builtin cannot reset the
//! step budget or recursion depth. Higher-order combinators (`filter`, `map`,
//! `reduce`, …) are ordinary builtins built on this surface.

// Public modules - exposed for external use
pub mod ast;
pub mod builtins;
pub mod context;
pub mod engine;
pub mod error;
pub mod error_formatter;
#[doc(hidden)]
pub mod eval;
mod limits;
pub mod maybe;
pub mod policy;
mod program;
#[doc(hidden)]
pub mod span;
pub mod template;
#[doc(hidden)]
pub mod token;
pub mod value;
pub(crate) mod value_utils;

// Internal frontend modules. Exposed because the `nebula-expression-fuzz` crate
// drives the lexer and parser in isolation; not part of the supported surface.
#[doc(hidden)]
pub mod lexer;
#[doc(hidden)]
pub mod parser;

// Re-exports
pub use ast::{BinaryOp, Expr};
pub use builtins::{BuiltinFunction, BuiltinOutput, BuiltinOutputBuilder, BuiltinOutputLimit};
pub use context::{EvaluationContext, EvaluationContextBuilder};
pub use engine::{CacheOverview, ExpressionEngine};
pub use error::{ExpressionError, ExpressionResult};
pub use eval::{Argument, BuiltinView};
pub use maybe::{CachedExpression, MaybeExpression};
pub use policy::{
    BuiltinOutputBound, BuiltinOutputLimits, EvaluationPolicy, EvaluationStepLimit, MissingLookup,
};
pub use program::{CompiledProgram, ProgramSyntax};
pub use template::{MaybeTemplate, Position, Template, TemplatePart, has_expression_marker};
pub use value::RuntimeValue;

/// Parse and syntax-check a single expression source string.
///
/// This validates expression grammar without evaluating against a runtime
/// context. It is the stable parsing entrypoint for downstream crates that
/// need parse-only checks.
///
/// Uses [`CompiledProgram::compile`]: raw expression grammar takes precedence,
/// then the template parser handles envelopes and mixed text. Quoted template
/// markers are string contents. Callers that evaluate later should retain the
/// compiled program instead of discarding it through this syntax-only helper.
pub fn parse_expression(source: &str) -> ExpressionResult<()> {
    CompiledProgram::compile(source).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::parse_expression;

    #[test]
    fn parse_expression_accepts_valid_syntax() {
        let result = parse_expression("$input.count + 1");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_expression_rejects_invalid_syntax() {
        let result = parse_expression("1 +");
        assert!(result.is_err());
    }

    #[test]
    fn parse_expression_accepts_wrapped_template_expression() {
        let result = parse_expression("{{ $input.count + 1 }}");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_expression_accepts_multiple_template_expressions() {
        let result = parse_expression("{{ $a }} + {{ $b }}");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_expression_disambiguates_raw_with_brace_literal_substring() {
        // Pre-fix: `contains("{{")` mistook a raw expression containing the
        // substring `{{` (e.g. inside a string literal) for a template and
        // routed it through the template parser, which then failed because
        // the wrapping `{{ ... }}` was missing. Now the dispatch is decided
        // by the actual template parser's `expression_count()`.
        let result = parse_expression(r#"contains($input, "{{")"#);
        assert!(
            result.is_ok(),
            "raw expression with literal {{{{ substring should parse: {result:?}"
        );
    }
}
