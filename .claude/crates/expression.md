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
- `EvaluationContext` has two scopes: `execution_vars` (`$var` syntax) and `lambda_vars` (runtime-only). `Expr::Identifier` checks only `lambda_vars`; `Expr::Variable` checks `lambda_vars` first then `execution_vars`. Lambda params (`$acc` in reduce) go via `set_lambda_var` — never `execution_vars`.

## Traps
- `ast`, `lexer`, `parser`, `eval`, `token`, `interner`, `span` modules are `#[doc(hidden)]` — unstable.
- `EvaluationContext` is per-execution, not reused.
- `eval_lambda` calls `eval_counting` (not `eval`) so iterations share the step budget.
- `pick`/`omit` validate all key args are strings; `pad_start`/`pad_end`/`repeat` use `get_int_arg_with_policy` (float coercion in non-strict mode).
- `some`/`every`/`find`/`find_index`/`group_by` are intercepted by the evaluator as lambda-based HOFs (eval.rs ~856) and never reach the builtin registry — do NOT add value-based implementations, they will be dead code.
- `flat_map` is a plain builtin (no lambda): extracts a named array field from each element and flattens.

<!-- reviewed: 2026-04-07 — removed dead some/every/find/find_index/group_by builtins; HOF intercept boundary clarified -->
