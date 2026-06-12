# nebula-expression — Agent orientation
> Agent quick-map for `crates/expression/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Shared expression evaluator that resolves `{{ expression }}` templates (n8n-compatible syntax) against execution-time context — the resolution backend `nebula-schema`'s `ValidValues::resolve` step calls.
**Layer:** Core — depends only downward (root AGENTS.md -> Layered Dependency Map).

## Commands
- `cargo check -p nebula-expression`
- `cargo nextest run -p nebula-expression`  ·  doctests: `cargo test -p nebula-expression --doc`
- Features: `default = cache,regex,datetime,uuid` (`full` = all). `datetime` adds IANA tz args; `regex`/`cache` pull in `moka` for true LRU.
- Bench: `cargo bench -p nebula-expression --bench baseline`. Examples: `cargo run -p nebula-examples --example expression_template_rendering`.

## Key files
- `src/lib.rs` — public re-exports + `parse_expression` (parse-only entrypoint; template-parser-authoritative dispatch, not substring match)
- `src/engine.rs` — `ExpressionEngine` + LRU AST cache (`evaluate`, `evaluate_template`, `cache_overview`)
- `src/eval.rs` — `Evaluator` / `EvalFrame` AST walker; `BuiltinView` (policy-only handle) + higher-order combinators
- `src/context.rs` — `EvaluationContext` (`$node`/`$execution`/`$workflow`/`$input`) + builder
- `src/policy.rs` — `EvaluationPolicy` DoS budget (step limit, recursion depth)
- `src/maybe.rs` — `MaybeExpression<T>` typed serde wrapper (literal vs expression)
- `src/template.rs` — `Template` / `MaybeTemplate`; `{{- -}}` whitespace control

## Conventions & never-do
- `BuiltinFunction` takes `BuiltinView<'_>` (policy queries only), NOT `Evaluator` — builtins physically cannot recurse into AST eval; do NOT add an eval handle to that signature (type-enforced fix for the issue #252 step-budget bypass).
- Higher-order combinators (`filter`/`map`/`reduce`/…) live in `eval.rs` and call `eval_with_frame` with the caller's `EvalFrame` so the step budget accumulates across iterations — never re-route them through the builtin registry.
- `EvaluationPolicy` must bound every evaluation (recursion depth default 256 + step budget); exceeding either returns `ExpressionError`, never panics or loops.
- NOT a validation engine (`nebula-validator`), schema system (`nebula-schema`), or HTML template engine — keep scope to `{{ }}` field resolution.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · `docs/PRODUCT_CANON.md` §3.5 (expression context at resolve step) · `docs/pitfalls.md` (builtin-frame step-budget pitfall)
