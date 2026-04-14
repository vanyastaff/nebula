# nebula-expression
n8n-compatible expression language evaluating `serde_json::Value` — used in workflow node parameter interpolation.

## Invariants
- n8n-compatible syntax; template delimiter `{{ }}`.
- All values are `serde_json::Value`.
- Recursion depth capped at 256; step limit via `EvaluationPolicy::max_eval_steps`.
- Step budget is enforced across ALL nested work in a single top-level `Evaluator::eval` call — including every lambda invocation of `map`/`filter`/`reduce`/etc. One top-level `eval` = one budget, no matter how deep the lambda nesting goes.

## Key Decisions
- `EvaluationPolicy` controls function allow/deny lists, strict mode flags, step limits, and JSON parse limits.
- Step + depth accounting lives on a stack-local `EvalFrame { depth, steps, max_steps }` threaded by `&mut` through every recursive path (`eval_with_frame`, `eval_lambda`, `eval_reduce`, `eval_filter/map/find/...`, `try_higher_order_function`, `call_function`). Closes the CO-C1-01 / issue #252 lambda DoS bypass where the previous `AtomicUsize` counter on `Evaluator` got reset by `self.eval(...)` re-entry on every lambda element. `Evaluator` itself has no mutable step state and is therefore cheap to share across tasks via `Arc`.
- `Evaluator::eval` is the sole place that constructs an `EvalFrame`. Internal recursive paths MUST call `eval_with_frame`, never `self.eval` — doing so would build a fresh frame mid-traversal and reset the budget.
- `EvaluationContext` has two scopes: `execution_vars` (`$var` syntax) and `lambda_vars` (runtime-only). `Expr::Identifier` checks only `lambda_vars`; `Expr::Variable` checks `lambda_vars` first then `execution_vars`. Lambda params (`$acc` in reduce) go via `set_lambda_var` — never `execution_vars`.

## Traps
- `ast`, `lexer`, `parser`, `eval`, `token`, `interner`, `span` modules are `#[doc(hidden)]` — unstable.
- `EvaluationContext` is per-execution, not reused.
- `eval_lambda` is `pub(crate)` — external callers cannot construct an `EvalFrame`, and exposing a wrapper that built one would reopen the CO-C1-01 DoS bypass.
- Never call `self.eval(body, ...)` inside the evaluator — always `self.eval_with_frame(body, ..., frame)`. A grep over `eval.rs` must find zero `self.eval(` hits outside the public entry point.
- **BuiltinRegistry::call footgun:** builtins receive `&Evaluator` without the caller's frame. A future builtin that recurses via `evaluator.eval(...)` would reopen CO-C1-01. Plumb `&mut EvalFrame` through the builtin call path before stabilising the public builtin API. See `Evaluator::eval` rustdoc.
- `pick`/`omit` validate all key args are strings; `pad_start`/`pad_end`/`repeat` use `get_int_arg_with_policy` (float coercion in non-strict mode).
- `some`/`every`/`find`/`find_index`/`group_by`/`flat_map` are intercepted by the evaluator as lambda-based HOFs (eval.rs `try_higher_order_function`) and never reach the builtin registry — do NOT add value-based implementations, they will be dead code.

<!-- reviewed: 2026-04-14 -->
