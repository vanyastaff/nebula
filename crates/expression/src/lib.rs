#![warn(clippy::all)]
#![allow(clippy::excessive_nesting)]
#![allow(clippy::needless_range_loop)]

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
//! | [`EvaluationContext`] | Runtime variable bindings (`$node`, `$execution`, `$workflow`, `$input`) |
//! | [`EvaluationPolicy`] | DoS budget (step limit, max recursion depth) |
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
//! ## Known limitation: BuiltinFunction re-entry
//!
//! `BuiltinFunction` receives `&Evaluator`, allowing built-ins to call `eval` recursively.
//! `EvaluationPolicy` step budget is the only guard. Built-in functions must be first-party
//! only — untrusted builtins can re-enter the evaluator. See `crates/expression/README.md`
//! Contract section and memory note `pitfall_expression_builtin_frame.md`.

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
pub mod maybe;
pub mod policy;
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
pub use context::{EvaluationContext, EvaluationContextBuilder};
pub use engine::{CacheOverview, ExpressionEngine};
// Re-export error types
pub use error::{ExpressionError, ExpressionErrorExt, ExpressionResult};
pub use maybe::{CachedExpression, MaybeExpression};
pub use policy::EvaluationPolicy;
// Re-export serde_json types for convenience
pub use serde_json::Value;
#[doc(hidden)]
pub use span::Span;
pub use template::{MaybeTemplate, Template};
#[doc(hidden)]
pub use template::{Position, TemplatePart};
#[doc(hidden)]
pub use token::{Token, TokenKind};

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::{
        CacheOverview, EvaluationContext, EvaluationContextBuilder, EvaluationPolicy,
        ExpressionEngine, ExpressionError, ExpressionErrorExt, ExpressionResult, MaybeExpression,
        MaybeTemplate, Template, Value,
    };
}
