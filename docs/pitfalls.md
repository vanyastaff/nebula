# Pitfalls

Recurring trap classes encountered while building Nebula. Each entry
captures the *shape* of the bug, the structural fix that resolved it,
and a pointer to the code/test that prevents regression. New traps go
here when they have repeated at least twice across crates — one-off
quirks belong in commit messages.

---

## `nebula-expression`: builtin re-entry into the evaluator

**Symptom.** A builtin registered through `BuiltinRegistry` calls
`Evaluator::eval` (or `Evaluator::eval_with_frame`) recursively against
user-supplied input. The recursive call constructs a fresh `EvalFrame`,
which resets the step budget defined by `EvaluationPolicy::max_eval_steps`.
A workflow author can then build hostile inputs (for example, a custom
builtin whose body sums `$node.x` a million times) that bypass the DoS
budget and burn CPU until the entire request times out.

**History.** Originally tracked as issue #252 and audit memory
`pitfall_expression_builtin_frame`. The `lib.rs` "Known limitation"
section called this out as a *discipline rule* — "built-ins must not
be authored by untrusted code; they are first-party only" — which is
exactly the kind of guard memory `feedback_type_enforce_not_discipline`
warns against: rules enforced by review eventually drift, and "all
builtins are first-party" is one PR away from being false.

**Structural fix (landed).** `BuiltinRegistry::call` now wraps the
evaluator in `BuiltinView<'_>` (defined in `crates/expression/src/eval.rs`)
and hands that view to the registered function instead of `&Evaluator`.
The view exposes only policy-query methods — `is_strict_mode`,
`strict_conversions_enabled`, `max_json_parse_length` — so the
registered function physically cannot reach `eval()`. Re-entry through
the registry is now a compile error, not a discipline ask.

The intentional re-entry path — higher-order combinators (`filter`,
`map`, `reduce`, `flat_map`, `group_by`, `find`, `find_index`, `some`,
`every`) — is implemented inside `Evaluator` itself and calls
`eval_with_frame` with the caller's existing `EvalFrame`. The step
budget therefore continues to accumulate across every iteration.
These combinators are NOT registered through `BuiltinRegistry`, so
they bypass the `BuiltinView` boundary entirely.

**Files.**
- Type-enforced boundary: `crates/expression/src/eval.rs`
  (`BuiltinView`, `BuiltinRegistry::call` dispatch).
- Public type alias: `crates/expression/src/builtins.rs`
  (`BuiltinFunction`).
- Crate-level docs: `crates/expression/src/lib.rs`
  ("BuiltinFunction signature" section), `crates/expression/README.md`.

**Anti-pattern (do NOT introduce again).** Adding a method on
`BuiltinView` that returns `&Evaluator`, or exposing `Evaluator::eval`
through a "convenience" trait re-export. Either move the work into the
evaluator module so it can use `eval_with_frame` properly, or
restructure so the builtin produces a value rather than walking the
AST itself.
