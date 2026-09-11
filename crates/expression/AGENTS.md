# nebula-expression — Agent orientation
> Local guide for `crates/expression/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Shared expression evaluator that resolves `{{ expression }}` templates (n8n-compatible syntax) against execution-time context — the resolution backend `nebula-schema`'s `ValidValues::resolve` step calls.
**Layer:** Core — depends only downward (root AGENTS.md -> Layered Dependency Map).

## Commands

- Features: `default = cache,regex,datetime,uuid` (`full` = all). `datetime` adds IANA tz args; `regex`/`cache` pull in `moka` for true LRU.
- Bench: `cargo bench -p nebula-expression --bench baseline`. Examples: `cargo run -p nebula-examples --example expression_template_rendering`.

## Key files

- `src/lib.rs` — public re-exports + `parse_expression` (auto compile and discard syntax)
- `src/program.rs` — opaque immutable `CompiledProgram`; raw-first auto compilation, explicit raw/template modes
- `src/engine.rs` — `ExpressionEngine` + LRU program cache (`evaluate`, `evaluate_compiled`, `render_template`, `cache_overview`)
- `src/eval.rs` — `Evaluator` / `EvalFrame` AST walker; `BuiltinView` (policy/work handle) + higher-order combinators
- `src/limits.rs` — source/token/node/depth/output bounds and default work ceiling
- `src/context.rs` — `EvaluationContext` (`$node`/`$execution`/`$workflow`/`$input`) + builder
- `src/policy.rs` — `EvaluationPolicy` DoS budget (work, recursion, input, builtin output)
- `src/builtins/output.rs` — opaque public builtin output and mandatory bounded builder
- `src/maybe.rs` — `MaybeExpression<T>` typed serde wrapper (literal vs expression)
- `src/template.rs` — `Template` / `MaybeTemplate`; `{{- -}}` whitespace control; shared lexical `has_expression_marker` classifier (recognizes malformed unescaped openers)

## Conventions & never-do

- `BuiltinFunction` takes `BuiltinView<'_>` plus `BuiltinOutputBuilder` and returns opaque `BuiltinOutput`, never raw `Value`. Do not expose a constructor or bypass for custom callbacks. The view provides policy queries and shared work charging, not evaluator re-entry; custom callbacks remain trusted, cooperative in-process code.
- Higher-order combinators (`filter`/`map`/`reduce`/…) live in `eval.rs` and call `eval_with_frame` with the caller's `EvalFrame` so the step budget accumulates across iterations — never re-route them through the builtin registry.
- `EvaluationPolicy` bounds every whole program (depth 256; default 100,000 work units) and every builtin output (bytes, strings, collections, nodes, depth); context limits only tighten engine ceilings. Template parts and higher-order evaluation share one frame.
- Stored context variables resolve as shared `Arc<Value>` snapshots. Keep evaluator property/index chains and builtin arguments borrowed; do not reintroduce deep clones per reference.
- Downstream callers retain `CompiledProgram`, never a source-only surrogate AST or duplicated raw/template dispatcher. Keep parsing distinct from runtime type and lookup validation; missing is not null.
- `ProgramSyntax` records authored intent, not effective AST/body kind. AUTO raw-first
  compilation remains AUTO; TEMPLATE always returns string. Source exports alone
  cannot reconstruct this distinction. Keep syntax immutable across clones.
- Exact mixed numeric comparison delegates to `num-cmp` without the nightly i128 feature. Do not replace it with `as_f64` or a second handwritten comparator.
- NOT a validation engine (`nebula-validator`), schema system (`nebula-schema`), or HTML template engine — keep scope to `{{ }}` field resolution.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Evaluation or builtins | Unit tests in [src/eval.rs](src/eval.rs), [builtin_functions](tests/builtin_functions.rs); retain shared step-budget and recursion-limit coverage. |
| Retained compilation / limits | [program](tests/program.rs): modes, runtime context/policy, shared budget, cache independence, parser and allocation limits. |
| Numeric behavior | [numeric](tests/numeric.rs): exact mixed boundary oracle, literals, checked arithmetic/conversions. Run in debug and release. |
| Optional builtins or caching | Repeat affected checks with default features and `--no-default-features`; then enable the individual feature being changed. `full` alone misses disabled paths. |

## See also

- `README.md` — full design · [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §3.5 (expression context at resolve step) · [docs/pitfalls.md](../../docs/pitfalls.md) (builtin-frame step-budget pitfall)
