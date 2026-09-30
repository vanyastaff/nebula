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

**History.** Originally tracked as issue #252. The `lib.rs` "Known
limitation" section called this out as a *discipline rule* — "built-ins
must not be authored by untrusted code; they are first-party only" —
which is exactly the kind of guard that erodes: rules enforced by review
eventually drift, and "all builtins are first-party" is one PR away from
being false. Prefer a structural, type-enforced boundary over a rule
reviewers must remember.

**Structural fix (landed).** `BuiltinRegistry::call` now wraps the
evaluator in `BuiltinView<'_>` (defined in `crates/expression/src/eval/mod.rs`)
and hands that view to the registered function instead of `&Evaluator`.
The view exposes policy-query methods (`is_strict_mode`,
`strict_conversions_enabled`, `max_json_parse_length`), shared work
charging (`charge_work`, `check_output_bytes`), and bounded lambda
invocation (`invoke_lambda`, `eval_body`) — but no way to construct a
fresh frame. Every lambda body runs against the caller's frame, so the
step budget and recursion depth accumulate across invocations. A
registered function physically cannot reset either; that is a compile
error, not a discipline ask.

Higher-order combinators (`filter`, `map`, `reduce`, `flat_map`,
`group_by`, `find`, `find_index`, `some`, `every`) are ordinary
registered builtins (`crates/expression/src/builtins/higher_order.rs`)
built on that same view, so they share the caller's frame like any
other call.

**Files.**
- Type-enforced boundary: `crates/expression/src/eval/mod.rs`
  (`BuiltinView`, `Argument`, `BuiltinRegistry::call` dispatch).
- Public type alias: `crates/expression/src/builtins/mod.rs`
  (`BuiltinFunction`).
- Crate-level docs: `crates/expression/src/lib.rs`
  ("BuiltinFunction signature" section), `crates/expression/README.md`.

**Anti-pattern (do NOT introduce again).** Adding a method on
`BuiltinView` that returns `&Evaluator`, constructs an `EvalFrame`, or
otherwise evaluates an arbitrary expression outside the shared-frame
paths. Move the work into a registered builtin that takes
`&[Argument<'_>]`, or restructure so the builtin produces a value rather
than walking the AST itself.

---

## rustdoc: intra-doc links that break only under the CI doc build

**Symptom.** The crate compiles and its tests pass, but the `Documentation`
job (`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
--document-private-items`) fails on an unresolved intra-doc link. It
recurs in two shapes:

- A bracketed link whose target is not in scope at the doc comment's
  position — typically a `//!` module block or a `#[doc = "..."]`
  attribute that names a sibling or foreign type.
- A bracketed link to an item behind `#[cfg(feature = "...")]` from docs
  that are not gated on that feature. The item does not exist in the
  default-feature doc build, so the link cannot resolve.

**History.** Seen in `nebula-credential` (unresolved builder links in an
`oauth2_config` `//!` block, `664577e3`) and in `nebula-resource` (an
out-of-scope `ManagedHandle::acquire` link in `manager/acquire.rs`,
`3978132a`). Both survived a green build and test run because nothing
except rustdoc reads intra-doc links.

**Fix.** Use a plain code span (`` `Foo` ``) or a fully qualified `crate::`
path. Links to feature-gated items from ungated docs are always plain code
spans. Verify the way CI builds, not the way the symbol happens to
exist: default features (no `--features` / `--all-features`) and
`--document-private-items`, per changed crate:

```text
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items -p nebula-<name>
```

**Files.**
- Gate: the `Documentation` job in `.github/workflows/ci.yml`; the
  pre-push check (`scripts/pre-push-crate-diff.sh`) runs the same
  `cargo doc` (default features, `--document-private-items`) for changed
  crates.

**Anti-pattern (do NOT introduce again).** Treating a passing
`cargo check` / `cargo test`, or a clean `cargo doc --features x`, as
evidence that the docs build. Neither exercises the default-feature doc
configuration that CI enforces.
