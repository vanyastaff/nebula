#![forbid(unsafe_code)]
#![warn(clippy::all)]
#![warn(unreachable_pub)]
#![allow(clippy::excessive_nesting)]
#![allow(clippy::needless_range_loop)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! # nebula-expression
//!
//! Expression evaluator for dynamic workflow field resolution. Evaluates
//! `{{ expression }}` templates against execution-time context, providing the
//! resolution backend used by `nebula-schema`'s `ValidValues::resolve` step.
//!
//! **Role:** Expression Evaluator. See `crates/expression/README.md`.
//!
//! **Canon:** §3.5 (expression context used at the resolve step of the proof-token pipeline).
//!
//! **Maturity:** `stable` — `ExpressionEngine`, `EvaluationContext`, `Template`,
//! `MaybeExpression`, and `MaybeTemplate` are in active use.
//!
//! ## Core Types
//!
//! | Type | Purpose |
//! |------|---------|
//! | [`ExpressionEngine`] | Parse and evaluate expressions; optional LRU cache |
//! | [`CompiledProgram`] | Immutable syntax retained across evaluations; raw, template, or auto compilation |
//! | [`EvaluationContext`] | Runtime variable bindings (`$node`, `$execution`, `$workflow`, `$input`) |
//! | [`EvaluationPolicy`] | Function restrictions, coercion rules, and bounded work/JSON input |
//! | [`Template`] | Pre-parsed `{{ }}` template; call `.render(engine, ctx)` |
//! | [`MaybeExpression`] | Typed wrapper: literal `T` or expression string |
//! | [`MaybeTemplate`] | Text template wrapper with auto-detection |
//! | [`ExpressionError`] | Typed evaluation error |
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
//! let result = engine.evaluate("$execution.id", &context).unwrap();
//! assert_eq!(result.as_str(), Some("exec-123"));
//! ```
//!
//! ## Non-goals
//!
//! Not a validation rules engine (`nebula-validator`), not a schema system (`nebula-schema`).
//!
//! ## BuiltinFunction signature
//!
//! `BuiltinFunction` receives [`eval::BuiltinView`] for policy/work accounting
//! and a mandatory [`BuiltinOutputBuilder`]. It returns opaque [`BuiltinOutput`]
//! rather than an unchecked JSON value.
//! It does NOT expose `Evaluator::eval`, so a
//! registered builtin literally cannot recurse into AST evaluation. The
//! step-budget bypass that was historically a "discipline-only" rule
//! (issue #252, audit memory `pitfall_expression_builtin_frame.md`) is now
//! type-enforced.
//!
//! Higher-order combinators (`filter`, `map`, `reduce`, `flat_map`,
//! `group_by`, `find`, `find_index`, `some`, `every`) are NOT registered
//! through this surface — they live inside the evaluator module and call
//! `eval_with_frame` directly with the caller's `EvalFrame`, so the step
//! budget remains enforced across every iteration.

// Public modules - exposed for external use
#[doc(hidden)]
pub mod ast;
pub mod builtins;
pub mod context;
pub mod engine;
pub mod error;
pub mod error_formatter;
#[doc(hidden)]
pub mod interner;
mod limits;
pub mod maybe;
pub mod policy;
mod program;
#[doc(hidden)]
pub mod span;
pub mod template;
#[doc(hidden)]
pub mod token;
pub mod value_utils;

// Internal modules - not part of stable public API
// These are exposed for advanced use cases but may change between versions
#[doc(hidden)]
pub mod eval;
#[doc(hidden)]
pub mod lexer;
#[doc(hidden)]
pub mod parser;

// Re-exports
// Internal types - only exported for advanced use cases
// Most users should not need these types directly
#[doc(hidden)]
pub use ast::{BinaryOp, Expr};
pub use builtins::{BuiltinOutput, BuiltinOutputBuilder, BuiltinOutputLimit};
pub use context::{EvaluationContext, EvaluationContextBuilder};
pub use engine::{CacheOverview, ExpressionEngine};
// Re-export error types
pub use error::{ExpressionError, ExpressionErrorExt, ExpressionResult};
pub use maybe::{CachedExpression, MaybeExpression};
pub use policy::{BuiltinOutputBound, BuiltinOutputLimits, EvaluationPolicy, EvaluationStepLimit};
pub use program::{CompiledProgram, ProgramSyntax};
// Re-export serde_json types for convenience
pub use serde_json::Value;
#[doc(hidden)]
pub use span::Span;
pub use template::{MaybeTemplate, Template, has_expression_marker};
#[doc(hidden)]
pub use template::{Position, TemplatePart};
#[doc(hidden)]
pub use token::{Token, TokenKind};

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

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::{
        BuiltinOutput, BuiltinOutputBound, BuiltinOutputBuilder, BuiltinOutputLimit,
        BuiltinOutputLimits, CacheOverview, CompiledProgram, EvaluationContext,
        EvaluationContextBuilder, EvaluationPolicy, EvaluationStepLimit, ExpressionEngine,
        ExpressionError, ExpressionErrorExt, ExpressionResult, MaybeExpression, MaybeTemplate,
        ProgramSyntax, Template, Value, has_expression_marker,
    };
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
