---
name: Nebula pitfalls catalog
description: Recurring traps observed across crates — class of bug, why it happens, how to avoid.
status: accepted
last-reviewed: 2026-04-18
related: [STYLE.md, PRODUCT_CANON.md]
---

# Nebula pitfalls catalog

Specific classes of bug we have hit before. Read on review of any new public
type or dispatch surface — `STYLE.md` covers idioms, this file covers landmines.

## 1. `expression`: `BuiltinFunction` can re-enter `Evaluator::eval` and bypass the step budget

**Where:** `crates/expression/src/builtins.rs:25` defines

```rust
pub type BuiltinFunction =
    fn(&[Value], &Evaluator, &EvaluationContext) -> ExpressionResult<Value>;
```

A builtin receives `&Evaluator` but **no `EvalFrame`**. The CO-C1-01 step-budget
protection lives on `EvalFrame`, plumbed through `Evaluator::call_function`
(`crates/expression/src/eval.rs`). If any builtin ever calls
`evaluator.eval(...)`, it constructs a fresh `EvalFrame` mid-traversal and
reopens the step-budget bypass that #252 closed.

**Why this is here:** `BuiltinFunction` is a public type alias, so plumbing
`&mut EvalFrame` through it is a semver break. The #252 fix took the
minimum-surface path inside the evaluator only.

**How to avoid:**

- Reject any `evaluator.eval(...)` call inside a `BuiltinFunction` impl during
  review.
- Pre-1.0 (`expression` is `stable` per `MATURITY.md`), promote the signature
  to take `&mut EvalFrame` so re-entry becomes impossible at the type level —
  not "nobody does it yet".
- Until then, the `_frame: &mut EvalFrame` parameter on
  `Evaluator::call_function` is the forward-compat hook — do not remove it.

## 2. `Result<()>` conflates "pass" and "skip" — inverting combinators silently misbehave

**Where:** `crates/validator/src/rule/logic.rs:69-86, 97-109`. `Rule::Not`
cannot tell whether the child rule passed or was skipped (`Predicate` without
ctx, `Deferred` in `StaticOnly`); without the `inner_would_skip` shim it would
invert a skip into a false `not_failed` error.

**Why this happens:** when dispatch supports partial execution (schema-time vs
runtime, static vs deferred), a boolean-shaped result only carries two
outcomes — **pass** and **fail** — so the skip case has to overload `Ok(())`.
Combinators that invert results (`Not`, `Unless`) cannot distinguish
skip-as-pass from real-pass and silently flip a skip into a failure.

**Workaround in place:** `Logic::Not` calls `inner_would_skip` to look one
level into the child. The explicit comment at `logic.rs:74-75` flags the
remaining gap — **deep propagation through nested `Logic` is not handled**.

**How to avoid:**

- When designing dispatch with two execution axes (e.g. schema-time vs
  runtime, static vs deferred), decide early whether **skip** is a third
  outcome.
- If yes, lift it into the return type — a `Decision { Pass, Skip, Fail(err) }`
  enum is worth the boilerplate over a two-valued `Result`.
- If no, document explicitly that combinators which invert results will
  misbehave for skip-shaped inputs, and gate them with a per-rule shim like
  `inner_would_skip`.

## 3. `nebula-log`: OTLP / tonic exporter needs a Tokio reactor at construction time

**Where:** `crates/log/src/telemetry/otel.rs` — `build_exporter` (line 193)
calls `opentelemetry_otlp::SpanExporter::builder().with_tonic().build()`. The
companion test `build_layer_then_shutdown_is_safe` (line 293) is annotated
`#[tokio::test] async fn` for exactly this reason.

A plain `#[test]` on any function that transitively reaches `build_exporter`
panics with **"there is no reactor running"** the first time `build()` is
called — tonic's transport layer initializes a gRPC client eagerly inside
`build()`, not lazily on first export. The simple-vs-batch exporter choice
only changes export-path scheduling; the client itself still needs a reactor
to exist.

**How to avoid:**

- Any unit test in `crates/log/src/telemetry/otel.rs` that calls `build_layer`
  (or anything transitively reaching `build_exporter`) **must** be
  `#[tokio::test] async fn`, not plain `#[test]`. The dev-deps already pull
  `tokio` with `macros` + `rt-multi-thread` under `--features telemetry`.
- Pure helpers (e.g. `resolve_endpoint_from`) stay `#[test]` — only tests that
  reach `build_exporter` need the runtime.

## 5. Plugin: action / credential / resource key outside the plugin's namespace

**Symptom.** `PluginRegistry::register` (or `ResolvedPlugin::from`
directly) fails with
`PluginError::NamespaceMismatch { plugin, offending_key, kind }`.

**Cause.** A plugin's `Plugin::actions()` / `credentials()` / `resources()`
method returned an `Arc<dyn Action>` (or `AnyCredential`, `AnyResource`) whose
key does not start with `{plugin.key()}.`. Typical example: plugin keyed `slack`
returns an action keyed `api.foo`.

**Fix.** Either rename the component's key to `slack.api.foo`, or move
the component to a plugin that legitimately owns the `api.*` namespace.
The rule comes from canon §7.1 and is enforced at `ResolvedPlugin::from`
(not at dispatch time) — the bad registration cannot leak into the
runtime.

See ADR-0027.

## 4. Manual `serde::de::Visitor::visit_map`: every `next_key` must be paired with `next_value`

**Where:** `crates/validator/src/rule/deserialize.rs:93-119`. Manual
`Deserialize` impls for sum types where some variants are unit-shaped (no
payload) — the unit arms must consume the value via
`let _: serde::de::IgnoredAny = m.next_value()?;` (lines 115, 119).

**Why this is a footgun:** skipping `next_value()` happens to work on
`serde_json`'s `MapAccess` (it just moves on), but it is a contract violation
that **breaks on any non-JSON format** — RON, MessagePack, YAML, bincode. The
bug is silent under JSON-only test coverage and only surfaces when a caller
plugs in a different format.

**How to avoid:**

- For unit / no-payload variants in a map-form visitor, always consume the
  value with `let _: serde::de::IgnoredAny = m.next_value()?;`.
- Do not rely on `serde_json`'s leniency — assume someone will reuse the type
  with a stricter format.
- If a manual visitor grows to more than two-three unit variants, prefer
  `#[serde(tag = "...")]` or an explicit enum with a dedicated payload-bearing
  variant rather than hand-rolling the visitor.
