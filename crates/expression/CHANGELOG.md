# Changelog

All notable changes to `nebula-expression` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Compatibility obligations for the retained-program boundary — `CompiledProgram`,
`ProgramSyntax`, and the expression/template wire form — are stated in the
[README](README.md) *Contract* section. The crate is not published; every entry
below lands before the first release, so there is no released surface to migrate
from yet.

## [Unreleased]

### Breaking Changes

- **Evaluation is typed end to end.** The evaluator works on a new public
  `RuntimeValue` (JSON shapes plus `DateTime` and `Undefined`) instead of
  `serde_json::Value`. Plain JSON appears only at the crate boundary
  (`RuntimeValue::{to_json, from_json}`); `ExpressionEngine::evaluate` still
  returns `Value`, and `evaluate_runtime` returns the typed value. Typed values
  survive property/index chains and nesting inside arrays and objects.
- **The custom-builtin contract takes `Argument` and returns bounded output.**
  `BuiltinFunction` receives `&[Argument<'_>]` where an argument is an evaluated
  value or an unevaluated lambda, plus a `BuiltinView` and a mandatory
  `BuiltinOutputBuilder`. Lambdas are invoked through
  `BuiltinView::invoke_lambda`, which reuses the caller's frame: a registered
  builtin cannot reset the step budget or recursion depth. `Argument`,
  `BuiltinView`, and `BuiltinFunction` are now re-exported from the crate root.
- **The `ExpressionError` taxonomy is typed.** `EvalError` was split into
  `PropertyNotFound`, `KeyNotFound`, `FunctionNotAllowed`, `InvalidDate`, and
  `InvalidJson`; the four variants nothing constructed (`Validation`, `NotFound`,
  `Json`, `InvalidDate` as a `From` wrapper) are gone. `EvalError` is classified
  `validation`, not `internal`, and `Internal` is no longer retryable.
  `KeyNotFound` carries no payload, so a runtime lookup key cannot reach a
  diagnostic.
- **`Evaluator` is crate-private; `ExpressionEngine` owns the caches.** The
  engine no longer duplicates the builtin registry or policy. `Evaluator::new`,
  `Evaluator::eval`, and the public `eval` module entry points are gone from the
  supported surface.
- **`BuiltinRegistry` is crate-private.** Its `call` takes a `BuiltinView` that
  only the evaluator can construct, so an external registry could not be invoked.
  Use `ExpressionEngine::register_function`.

### Added

- **`{% if %}` / `{% for %}` template control flow.** `{% elif %}` / `{% else %}`
  chains, `loop.index` / `index0` / `first` / `last` / `length`, `{% else %}` for
  empty iterables, `{# #}` comments, and `{%- -%}` whitespace control. Blocks
  nest arbitrarily and compile into a tree; block structure errors (unclosed,
  mismatched, unknown tags) fail at compile time with the offending tag's
  position.
- **Method calls, optional chaining, and namespaces.** `items.filter(x => x > 1)`
  is the same call as `filter(items, x => x > 1)`; `?.` short-circuits a nullish
  receiver; `??` coalesces on `Null`/`Undefined` only. JavaScript names
  (`toUpperCase`, `includes`, `indexOf`) and Luxon date methods (`plus`, `diff`,
  `toFormat`) alias the registered builtins. `Math`, `JSON`, `Object`, `Number`,
  and `Array` dispatch to receiverless builtins.
- **Property members without a call.** `arr.length`, `str.length`, object
  `length`, and the date getters (`year`, `month`, `day`, `hour`, `minute`,
  `second`) read directly; an object's own keys always win.
- **Multi-parameter lambdas.** `(acc, x) => acc + x` parses; `reduce` honors the
  JavaScript `(fn, initial)` argument order while keeping the single-parameter
  `$acc` form.
- **Calendar date units.** `date_add` / `date_subtract` accept `months` and
  `years` through `checked_add_months` / `checked_sub_months`, so
  `Jan 31 + 1 month` clamps to `Feb 29` in a leap year.
- **`$json`**, the n8n spelling of the current item, aliasing `$input`.
- **`MissingLookup`** policy: `Error` (default, keeps schema resolution's loud
  failure) or `Undefined` (n8n-style authoring).
- `ExpressionEngine::evaluate_runtime` / `evaluate_compiled_runtime` for callers
  that want typed values instead of the JSON boundary.
- `RuntimeValue`, `Position`, `TemplatePart`, `Argument`, `BuiltinView`, and
  `BuiltinFunction` are part of the documented public surface.

### Removed

- `interner` module and its `parking_lot` dependency (no call sites).
- `ExpressionErrorExt` (ten one-line forwarders to inherent constructors).
- The `filter`/`map`/`reduce` builtin stubs (unreachable behind the higher-order
  dispatch they were shadowed by).
- `Token::is_operator`, `TokenKind::is_literal`, `can_add_as_int`, `is_numeric`,
  `Expr::{is_literal, as_literal}`, `Template::get_template`, cache-size getters,
  `pub Value` re-export, and the `prelude` module.
- The duplicate `tests/mod.rs` target that ran `builtin_functions.rs` twice.
- Unused `insta`, `rstest`, and `pretty_assertions` dev-dependencies.

### Fixed

- A missing index key no longer echoes the runtime key in the diagnostic
  (the redaction test caught the regression during the `RuntimeValue` migration).
- `reduce`'s single-parameter `$acc` form now works from real source; the lexer
  strips the sigil, so the bound name is `acc`, and only hand-built AST tests had
  masked it.
- Runtime lookup-key redaction is structural: `KeyNotFound` has no field to hold
  a key.
- `EvalError` no longer tells resilience layers that an author's type error is a
  transient crate fault worth retrying.
- Parse failures carry a structured `Position` instead of a pre-rendered
  ASCII-art diagnostic; rendering is the caller's concern via `ErrorFormatter`.

### Observability

- DoS guards keep their `nebula_expression::dos` warnings. The default work
  budget is pinned against the parser's expression ceiling by a test, so the two
  guards cannot contradict each other.
- `missing_docs`, `unreachable_pub`, and rustdoc intra-doc links are clean; the
  crate builds without warnings under `cargo doc --all-features`.
