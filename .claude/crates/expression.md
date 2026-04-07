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
- `EvaluationContext` has two separate variable scopes: `execution_vars` (user-set, accessed via `$var` syntax) and `lambda_vars` (runtime-only, for lambda-bound params). `Expr::Identifier` checks ONLY `lambda_vars`; `Expr::Variable` checks `lambda_vars` first then `execution_vars` via `resolve_variable`.
- Lambda parameters (including `$acc` in reduce) are stored via `set_lambda_var`, never in `execution_vars`, to prevent name collisions with real execution variables.

## Traps
- `ast`, `lexer`, `parser`, `eval`, `token`, `interner`, `span` modules are `#[doc(hidden)]` — unstable.
- `EvaluationContext` is per-execution, not reused.
- `eval_lambda` calls `eval_counting` (not `eval`) so iterations share the step budget.
- `pick`/`omit` validate that all key arguments are strings and return `expression_invalid_argument` for non-strings.
- `pad_start`, `pad_end`, `repeat` use `get_int_arg_with_policy` for the integer argument (supports float coercion in non-strict mode).

<!-- reviewed: 2026-04-07 — lambda scope isolation, pick/omit validation, integer coercion -->
