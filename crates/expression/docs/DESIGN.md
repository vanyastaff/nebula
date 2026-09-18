# nebula-expression — design

| Field | Value |
|-------|-------|
| **Status** | Stable — leaf/core evaluation primitive |
| **Layer** | Core (depends only on `nebula-log` + `nebula-error`; no domain crate) |
| **Redesign role** | **Unaffected** by the post-ADR-0092 credential/resource redesign — no credential crate depends on it and the rewrite plans do not mention it. Indirect consumer: `nebula-resource` / `nebula-action` resolve `MaybeExpression` configs through it. |
| **Related** | PRODUCT_CANON §3.5 (`ValidValues::resolve`), ROADMAP #590 (regex cache), issue #252 (step-budget bypass fix), n8n expression syntax |

---

## 1. Purpose and boundaries

`nebula-expression` is an expression engine with n8n-compatible syntax for resolving
workflow fields dynamically. It parses and evaluates `{{ expression }}` templates
against an execution context and returns a `serde_json::Value` at its boundary. It is the resolution
backend for `nebula-schema` (`ValidValues::resolve`, Canon §3.5).

**Owns:** the expression lexer/parser/AST, the AST-walk evaluator, the builtin
function registry (`array`/`string`/`math`/`object`/`datetime`/`conversion`/`util`),
the template engine with whitespace control (`{{- -}}`), two moka-backed LRU caches
(AST + template), the DoS budget (`EvaluationPolicy`), the literal-or-expression serde
wrappers (`MaybeExpression<T>`, `MaybeTemplate`), and typed errors with structured
source positions.

**Explicitly does NOT:** store execution state (the caller builds the context), know
about the credential/resource domain, do KDF or crypto, validate schemas (`nebula-schema`
does that and merely *calls* resolution), or perform I/O — evaluation is pure over the
supplied context.

## 2. Public surface

The retained-program contract is the syntax boundary shared with schema. Consumers
cache `CompiledProgram`, not source-only AST wrappers, and call
`ExpressionEngine::evaluate_compiled(&program, &context)`. Compilation does not
resolve variables, validate result types, or apply runtime policy. Auto compilation
tries raw grammar first, preserving quoted markers and typed lone envelopes;
explicit template compilation always produces text. `has_expression_marker` is
the shared allocation-free lexical classifier for optional authored JSON shorthand,
including malformed unescaped openers. See README for escape rules and hard bounds.

| Item | Where |
|------|-------|
| `CompiledProgram::{compile, compile_expression, compile_template, source}` | `program.rs` |
| `ExpressionEngine::evaluate_compiled` | `engine.rs` |
| `has_expression_marker` | `template.rs`, re-exported from the crate root |
| `ExpressionEngine` (+ `new`/`with_cache_size`/`with_policy`) | `engine.rs` |
| `evaluate` / `parse_template` / `render_template` / `cache_overview` | `engine.rs` |
| `CacheOverview` (+ `CacheStats`) | `engine.rs` |
| `EvaluationContext` (+ `EvaluationContextBuilder`) — `$node`/`$execution`/`$workflow`/`$input` | `context.rs` |
| `EvaluationPolicy` (DoS budget: work limit + recursion depth, default 256) | `policy.rs` |
| `Template` / `MaybeTemplate` (whitespace control `{{- -}}`) | `template.rs` |
| `MaybeExpression<T>` (+ `resolve_as_value/string/integer/float/bool`); `CachedExpression` | `maybe.rs` |
| `ExpressionError` (thiserror + `nebula_error::Classify`, codes `EXPR:*`); `ExpressionResult` | `error.rs` |
| `MissingLookup` — missing-lookup policy (`Error` default, `Undefined` opt-in) | `policy.rs` |
| `parse_expression(source)` — delegates to the auto compiler and discards the program | `lib.rs` |
| `BuiltinFunction` (alias); `BuiltinRegistry` | `builtins/mod.rs` |
| `BuiltinOutput`; `BuiltinOutputBuilder`; `BuiltinOutputBound`; `BuiltinOutputLimits` | `builtins/output.rs`; `policy.rs` |
| `RuntimeValue` — evaluator value model: JSON shapes plus typed date-times and `Undefined` | `value.rs` |
| `Argument<'_>` / `BuiltinView<'_>` — value-or-lambda arguments; policy, work charging, and shared-frame lambda invocation | `eval/mod.rs` |
| `ErrorFormatter` — caller-side renderer for structured parse-error positions | `error_formatter.rs` |

doc-hidden but `pub`: `ast` (`Expr`/`BinaryOp`), `lexer`, `parser`, `token`, `span`,
`Evaluator` (`eval/mod.rs`) — marked "advanced use, may change".

## 3. Dependencies and dependents

- **Deps:** `nebula-log` (path), `nebula-error` (workspace, feature `derive`), `tracing`,
  `thiserror`, `serde`, `serde_json`, `chrono`, `unicode-width`. Optional:
  `moka` (`cache`), `regex` (`regex`, deliberately pulls in moka for true-LRU regex
  caching, ROADMAP #590), `chrono-tz` (`datetime`), `uuid` (`uuid`).
  default = `cache,regex,datetime,uuid`.
- **Dependents:** `nebula-engine`, `nebula-schema`, `nebula-action`
  (default-features=false, `cache` only), `nebula-resource`
  (default-features=false, `cache` only), `examples`, `nebula-expression-fuzz`
  (features=full).

## 4. Internal architecture

Frontend: `lexer.rs`/`token.rs` → `parser.rs` → `ast.rs` (+ `span.rs` for positions).
`eval/mod.rs` is the AST-walker `Evaluator`/`EvalFrame`; higher-order combinators
(`filter`/`map`/`reduce`/`group_by`/…) go through `eval_with_frame` with the caller's
frame, and builtins receive only `BuiltinView` (no recursive-eval access).
`engine.rs` orchestrates two moka LRU caches (expr-AST + template) and their statistics.
`context.rs` holds the four variable namespaces. `template.rs` stitches literal and
expression parts together with whitespace control. `maybe.rs` is the serde
literal-or-expression layer for configs. `error.rs` holds the typed errors;
`error_formatter.rs` renders a position on the caller's side (`ParseError` carries a
structured `Position`, never a pre-rendered string).

Flow: source → compiler → immutable `CompiledProgram` → optional cache →
`evaluate_compiled` under the current `EvaluationPolicy` → `RuntimeValue` →
`serde_json::Value` at the crate boundary. Typed values (date-times) survive
property/index chains and builtin dispatch; `Undefined` renders as `null` at the
boundary. Every template
part and higher-order body shares one call-local frame. Context limits cannot
raise engine ceilings (default work 100,000 units; default JSON input 1 MiB).
Builtin argument/result materialization and allocation-heavy work use that frame.
Public custom callbacks must return opaque `BuiltinOutput` through the supplied bounded
builder; standard callbacks remain crate-private and their final values are checked.
Custom callback execution remains cooperative trusted code, not a preemptible sandbox.
Mixed numeric ordering delegates to `num-cmp`, without its nightly i128 feature.
Stored context variables are immutable `Arc<RuntimeValue>` snapshots. Evaluation
uses borrowed-or-owned runtime values internally, preserving borrows through access
chains and builtin dispatch rather than cloning the referenced graph.

## 5. Invariants and contracts

- **Canon §3.5 resolution backend.** `ValidValues::resolve` in `nebula-schema` calls the
  engine; the output is always a `serde_json::Value`.
- **DoS budget by construction.** `EvaluationPolicy` bounds work units and recursion
  depth (default 256); the budget is shared across the whole evaluation.
- **No step-budget bypass from a builtin (issue #252).** Builtins receive
  `BuiltinView`, not `Evaluator` — the type forbids recursive calls that would skip the
  step counter. Higher-order combinators recurse only through `eval_with_frame` under
  the same budget.
- **Builtin output is bounded by construction.** Public callbacks cannot return raw
  `Value`; `BuiltinOutputBuilder` enforces finite total-byte, string, collection, node,
  and depth ceilings. Expanding standard builtins preflight their exact output before
  allocating it.
- **One compilation dispatcher.** Raw grammar takes precedence; only then does the
  template parser interpret unescaped delimiters. `parse_expression` and engine
  source evaluation both use `CompiledProgram::compile`.
- **Typed errors.** `ExpressionError` carries `nebula_error::Classify` with `EXPR:*` codes.
  Every variant is constructed somewhere; author-fixable failures classify as
  `validation` (never `internal`), policy denials as `authorization`, and lookups as
  `not_found`. Runtime lookup keys never appear in diagnostics:
  `KeyNotFound` is a unit variant by construction, while `PropertyNotFound` and
  `VariableNotFound` may echo the authored name.
  Parse failures carry a structured `Position`; rendering is the caller's concern.

## 6. Known tensions / debt

1. **Trusted callbacks.** `BuiltinView` exposes cooperative work charging but cannot
   preempt a custom callback that blocks or ignores its budget.
2. **Legacy datetime heuristic.** `datetime.rs` — the "legacy 2-arg shape": the second
   argument is probed as a timezone, falling back to a format string. Under the n8n
   target (§6.5) this disappears: a date becomes a typed value instead of a string that
   has to be guessed at.
3. **Wide doc-hidden pub surface.** `lexer`/`parser`/`eval`/`ast`/`token`/`span` are a
   semi-public "may change" API. Under sole-public-sdk (only `nebula-sdk` is published)
   these can honestly move to `pub(crate)`.
4. **Unmerged resolve-seam refactor.** The `refactor/error-unify-validation` branch
   touches the expression resolve seam (sync/single-parse + `From<ExpressionError>` on
   the consumer side); it is not reflected in this worktree.
5. **Path to an n8n-class engine.** The stated target is an authoring language and
   template engine at n8n's level. Landed: method calls on values (`items.filter(…)`),
   optional chaining (`?.`), nullish coalescing (`??`), namespace libraries
   (`Math`/`JSON`/`Object`/`Number`/`Array`), `$json` as the n8n spelling of the
   current item, and typed date-times with `plus`/`diff`/`toFormat` methods and
   calendar units. Still unrealized: the item model (`$item`, `$items()`, `$position`,
   `$itemIndex` with multiple outputs per node — that is an engine/workflow design, not
   a naming gap the context can invent). The template control flow (`{% if %}` /
   `{% elif %}` / `{% else %}` / `{% for %}` with `loop` variables, `{# #}` comments, and
   `{%- -%}` whitespace control) landed in `program.rs`; it is unrealized scope only for
   inheritance and macros.
6. **Methods are a syntax, not a second library.** `builtins/methods.rs` only maps
   JavaScript/Luxon names onto the registered builtins; the receiver becomes the first
   argument. Adding a method implementation there instead of a builtin would fork the
   library and is forbidden.
7. No TODO/FIXME/deprecated markers.

## 7. Role in the post-0092 credential/resource model

Unaffected — a stable foundation. Neither `nebula-credential`, `nebula-resource`, nor
`nebula-storage` depends on it along the credential path, and the rewrite plans
(ADR-0088/0092) do not mention it. The only indirect link: consumer binding in
`nebula-resource`/`nebula-action` resolves `MaybeExpression` configs through the
reduced variant (`default-features=false`, `cache` only). If crates collapse behind
`nebula-sdk`, this crate's publicity becomes an internal detail — the sdk re-export
decides.

## 8. Forward design / open questions

The crate is stable as the Canon §3.5 resolution backend, but the target frame is
wider: a template engine with control flow and an n8n-compatible expression language
(§6.5), both now landed through the expression language and `{% %}` blocks. Open items: (a) settle the doc drift from §6.1; (b) under sole-public-sdk,
narrow the doc-hidden pub modules to `pub(crate)` (§6.3); (c) pick up the resolve-seam
changes from `refactor/error-unify-validation` at merge time (§6.4); (d) decide the
value-model/ABI/block-syntax forks before 0.1.0, while the public contract is still
free.
The regex cache on moka is already a chosen direction (ROADMAP #590), not an open
question.
