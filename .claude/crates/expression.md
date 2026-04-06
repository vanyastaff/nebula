# nebula-expression
n8n-compatible expression language evaluating `serde_json::Value` — used in workflow node parameter interpolation.

## Invariants
- n8n-compatible syntax; template delimiter `{{ }}`.
- All values are `serde_json::Value`.
- Recursion depth capped at 256; step limit via `EvaluationPolicy::max_eval_steps`.

## Key Decisions
- `EvaluationPolicy` controls function allow/deny lists, strict mode flags, step limits, and JSON parse limits.
- Step counter uses `AtomicUsize` (not `Cell`) because `Evaluator` is shared across threads via `Arc<ExpressionEngine>`.
- `eval()` resets the step counter; `eval_counting()` preserves it for lambda iterations in higher-order functions (map/filter/reduce).

## Traps
- `ast`, `lexer`, `parser`, `eval`, `token`, `interner`, `span` modules are `#[doc(hidden)]` — unstable.
- `EvaluationContext` is per-execution, not reused.
- `eval_lambda` calls `eval_counting` (not `eval`) so iterations share the step budget.

<!-- reviewed: 2026-04-06 — added max_eval_steps DoS prevention -->
