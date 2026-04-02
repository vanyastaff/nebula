# Architecture

## Problem Statement

- business problem:
  - workflows require dynamic value computation from runtime context without custom code for every transformation.
- technical problem:
  - provide an expression language with predictable semantics, performance, and safety under untrusted user inputs.

## Current Architecture

- module map:
  - `lexer` + `parser`: tokenization and AST construction
  - `core`: AST/token/span/error/interner primitives
  - `eval`: AST evaluator and operator/function semantics
  - `builtins`: string/math/array/object/datetime/conversion/util functions
  - `context`: `EvaluationContext` and builder
  - `template`: `Template` + `MaybeTemplate` rendering with positions/whitespace controls
  - `engine`: `ExpressionEngine` with optional expression/template caches
  - `error` + `error_formatter`: structured errors and human-readable formatting
- data/control flow:
  1. runtime provides `EvaluationContext`.
  2. `ExpressionEngine` parses expression/template (cache optional).
  3. evaluator executes AST with context + builtins.
  4. returns `serde_json::Value` or `ExpressionError`.
- known bottlenecks:
  - heavy dynamic expressions and deeply nested pipelines
  - template-heavy workloads without cache tuning

## Target Architecture

- target module map:
  - keep current modular split, harden contracts around builtins/context.
- public contract boundaries:
  - stable: `ExpressionEngine`, `EvaluationContext`, `Template`, `MaybeExpression`, `MaybeTemplate`, `ExpressionError`.
  - internal/hidden: low-level AST/token parser types (already `#[doc(hidden)]`).
- internal invariants:
  - recursion depth and regex safety guards always enforced.
  - parse/eval errors remain deterministic and actionable.
  - cache layer must never alter expression semantics.

## Design Reasoning

- key trade-off 1:
  - feature-rich DSL improves developer velocity but increases semantic stability burden.
- key trade-off 2:
  - caching improves performance but requires clear invalidation and memory controls.
- rejected alternatives:
  - using only generic JSONPath-like query language rejected because business logic needs richer transforms and conditions.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal, Prefect, Airflow.

- Adopt:
  - n8n-like template and expression ergonomics for workflow users.
  - explicit runtime context variables and function registry model.
- Reject:
  - opaque script execution as default path for common transforms.
- Defer:
  - full static type system and ahead-of-time expression compilation backend.

## Breaking Changes (if any)

- change:
  - future normalization of function naming/semantics and strict compatibility modes.
- impact:
  - existing expressions may require migration for renamed or redefined behavior.
- mitigation:
  - compatibility mode flags and migration lints prior to major cutover.

## Open Questions

- Q1: should expression language support strict mode with no implicit type coercions?
- Q2: should evaluator expose cost budget limits per expression for stronger isolation?
