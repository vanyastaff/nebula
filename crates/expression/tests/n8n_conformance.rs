//! n8n-surface conformance: the authoring shapes the crate targets.
//!
//! Each case is a source string an n8n author would plausibly write, asserted
//! against the exact value the engine must produce. This file pins the surface
//! that Ф2 introduces — methods, optional chaining, nullish coalescing, and
//! namespaces — so a refactor cannot silently drop any of it.
//!
//! These are the crate's own contracts, not a byte-for-byte clone of n8n's
//! runtime; deliberate deviations are asserted as deviations below.

#[cfg(feature = "datetime")]
use nebula_expression::RuntimeValue;
use nebula_expression::{EvaluationContext, EvaluationPolicy, ExpressionEngine, MissingLookup};
use serde_json::json;

fn engine() -> ExpressionEngine {
    ExpressionEngine::new()
}

/// The n8n-style authoring engine: missing lookups yield `Undefined`.
fn authoring_engine() -> ExpressionEngine {
    ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_missing_lookup(MissingLookup::Undefined))
}

fn context() -> EvaluationContext {
    EvaluationContext::builder()
        .input(json!({
            "items": [3, 1, 2],
            "names": ["ada", "grace", "alan"],
            "person": {"name": "Ada", "email": "ada@example.test", "address": {"city": "London"}},
            "count": 0,
            "label": "",
            "created": "2024-01-01T00:00:00Z",
        }))
        .execution_var("id", json!("exec-1"))
        .build()
}

fn evaluate(source: &str) -> serde_json::Value {
    engine()
        .evaluate(source, &context())
        .unwrap_or_else(|error| panic!("{source}: {error}"))
}

fn evaluate_authoring(source: &str) -> serde_json::Value {
    authoring_engine()
        .evaluate(source, &context())
        .unwrap_or_else(|error| panic!("{source}: {error}"))
}

// ──────────────────────────────────────────────
// Methods on values
// ──────────────────────────────────────────────

#[test]
fn string_methods_match_the_function_forms() {
    for (method, function) in [
        (
            "$input.person.name.toUpperCase()",
            "uppercase($input.person.name)",
        ),
        (
            "$input.person.name.toLowerCase()",
            "lowercase($input.person.name)",
        ),
        (
            "$input.person.name.includes('d')",
            "contains($input.person.name, 'd')",
        ),
        (
            "$input.person.name.startsWith('A')",
            "starts_with($input.person.name, 'A')",
        ),
        (
            "$input.person.name.endsWith('a')",
            "ends_with($input.person.name, 'a')",
        ),
    ] {
        assert_eq!(
            evaluate(method),
            evaluate(function),
            "method {method} must equal function {function}"
        );
    }
}

#[test]
fn array_methods_chain_without_intermediate_variables() {
    // A single expression an n8n author would write: filter, double, join.
    assert_eq!(
        evaluate_authoring("$input.items.filter(x => x > 1).map(x => x * 2).join('-')"),
        json!("6-4")
    );
    assert_eq!(
        evaluate_authoring("$input.names.map(n => n.toUpperCase()).sort()"),
        json!(["ADA", "ALAN", "GRACE"])
    );
}

#[test]
fn receiver_becomes_the_first_argument() {
    // The method surface is the same library as the function surface. Any
    // divergence here would mean two libraries drifting apart.
    assert_eq!(
        evaluate("$input.items.first()"),
        evaluate("first($input.items)")
    );
    assert_eq!(
        evaluate("$input.items.last()"),
        evaluate("last($input.items)")
    );
    assert_eq!(
        evaluate("$input.items.reverse()"),
        evaluate("reverse($input.items)")
    );
    assert_eq!(
        evaluate("$input.items.length"),
        evaluate("length($input.items)")
    );
}

#[test]
fn index_of_compares_exactly_and_reports_absence() {
    assert_eq!(evaluate("$input.items.indexOf(2)"), json!(2));
    assert_eq!(evaluate("$input.items.indexOf(99)"), json!(-1));
    // Numeric equality is exact across representations, same as `==`.
    assert_eq!(evaluate("$input.names.indexOf('grace')"), json!(1));
}

#[test]
fn javascript_reduce_argument_order_is_honored() {
    // JavaScript authors write `items.reduce((acc, x) => …, initial)`.
    assert_eq!(
        evaluate_authoring("$input.items.reduce((acc, x) => acc + x, 0)"),
        json!(6)
    );
}

#[test]
fn property_members_read_without_calls() {
    assert_eq!(evaluate("$input.items.length"), json!(3));
    assert_eq!(evaluate("$input.person.name.length"), json!(3));
    assert_eq!(evaluate("$input.person.length"), json!(3));
}

// ──────────────────────────────────────────────
// Optional chaining and nullish coalescing
// ──────────────────────────────────────────────

#[test]
fn optional_chain_short_circuits_on_missing_data() {
    // `$input` has no `missing` key; `?.` must stop there without error.
    assert_eq!(
        evaluate_authoring("$input.missing?.deep?.value"),
        json!(null)
    );
    assert_eq!(
        evaluate_authoring("$input.missing?.deep ?? 'fallback'"),
        json!("fallback")
    );
}

#[test]
fn optional_chain_passes_through_present_data() {
    assert_eq!(
        evaluate_authoring("$input.person?.address?.city"),
        json!("London")
    );
}

#[test]
fn optional_chain_guards_nullish_receivers_not_missing_keys() {
    // `?.` guards a *nullish receiver*, not a missing key: in
    // `$input.missing?.deep` the first step is a plain access, so the strict
    // missing policy still reports it. A nullish receiver short-circuits
    // under every policy.
    assert!(
        engine()
            .evaluate("$input.missing?.deep", &context())
            .is_err(),
        "the non-optional `missing` step must still follow the missing policy"
    );
    assert_eq!(
        engine().evaluate("null?.deep", &context()).unwrap(),
        json!(null)
    );
    assert_eq!(
        engine()
            .evaluate("$input.person?.address?.city", &context())
            .unwrap(),
        json!("London")
    );
}

#[test]
fn coalesce_keeps_falsy_values_that_are_not_nullish() {
    // `??` must not collapse `false`, `0`, or `""` the way `||` does.
    assert_eq!(evaluate("$input.count ?? 5"), json!(0));
    assert_eq!(evaluate("$input.label ?? 'default'"), json!(""));
    assert_eq!(evaluate("null ?? 'default'"), json!("default"));
    assert_eq!(
        evaluate_authoring("$input.missing ?? 'default'"),
        json!("default")
    );
}

#[test]
fn coalesce_binds_looser_than_logical_or() {
    // `a || b ?? c` parses as `a || (b ?? c)`, matching the JS precedence
    // table where `??` sits below `||`.
    assert_eq!(evaluate("false || 'x' ?? 'y'"), json!(true));
}

// ──────────────────────────────────────────────
// Namespaces
// ──────────────────────────────────────────────

#[test]
fn math_namespace_maps_to_the_math_builtins() {
    assert_eq!(evaluate("Math.max(1, 5, 3)"), evaluate("max(1, 5, 3)"));
    assert_eq!(evaluate("Math.min(1, 5, 3)"), evaluate("min(1, 5, 3)"));
    assert_eq!(evaluate("Math.abs(-4)"), evaluate("abs(-4)"));
    assert_eq!(evaluate("Math.floor(3.7)"), evaluate("floor(3.7)"));
    assert_eq!(evaluate("Math.pow(2, 8)"), evaluate("pow(2, 8)"));
}

#[test]
fn json_namespace_round_trips_a_value() {
    assert_eq!(
        evaluate_authoring("JSON.parse(JSON.stringify($input.person)).name"),
        json!("Ada")
    );
}

#[test]
fn object_and_number_namespaces_map_to_the_library() {
    assert_eq!(
        evaluate("Object.keys($input.person.address)"),
        json!(["city"])
    );
    assert_eq!(evaluate("Number.parseInt('42')"), json!(42));
    assert_eq!(evaluate("Number.parseFloat('3.5')"), json!(3.5));
}

#[test]
fn unknown_namespace_member_is_a_clear_error() {
    let error = engine().evaluate("Math.nope(1)", &context()).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("Math") && message.contains("nope"),
        "namespace typo must name both the namespace and the member: {message}"
    );
}

// ──────────────────────────────────────────────
// Methods on dates
// ──────────────────────────────────────────────

#[test]
#[cfg(feature = "datetime")]
fn date_methods_map_to_the_date_builtins() {
    assert_eq!(
        evaluate("parse_date($input.created).year"),
        evaluate("date_year(parse_date($input.created))")
    );
    assert_eq!(
        evaluate_authoring("parse_date($input.created).plus(1, 'days')"),
        evaluate("date_add(parse_date($input.created), 1, 'days')")
    );
    assert_eq!(
        evaluate_authoring("parse_date($input.created).toFormat('YYYY-MM-DD')"),
        json!("2024-01-01")
    );
}

#[test]
#[cfg(feature = "datetime")]
fn calendar_units_are_not_fixed_durations() {
    // One month after Jan 31 is Feb 29 in a leap year, which no fixed
    // second count can express.
    assert_eq!(
        evaluate_authoring("parse_date('2024-01-31T00:00:00Z').plus(1, 'months')"),
        json!("2024-02-29T00:00:00Z")
    );
    assert_eq!(
        evaluate_authoring("parse_date('2023-01-31T00:00:00Z').plus(1, 'months')"),
        json!("2023-02-28T00:00:00Z")
    );
}

#[test]
#[cfg(feature = "datetime")]
fn now_is_a_typed_date_value() {
    let value = engine()
        .evaluate_runtime("$now", &context())
        .expect("`$now` evaluates");
    assert!(
        matches!(value, RuntimeValue::DateTime(_)),
        "`$now` must stay a typed date inside evaluation, got {value:?}"
    );
}

// ──────────────────────────────────────────────
// Context namespaces
// ──────────────────────────────────────────────

#[test]
fn json_aliases_the_current_item() {
    // n8n authors write `$json.field`; this crate resolves one item at a
    // time, so `$json` is exactly `$input`.
    assert_eq!(
        evaluate("$json.person.name"),
        evaluate("$input.person.name")
    );
    assert_eq!(
        evaluate_authoring("$json.items.filter(x => x > 1)"),
        evaluate_authoring("$input.items.filter(x => x > 1)")
    );
    assert_eq!(evaluate_authoring("$json.missing ?? 'x'"), json!("x"));
}

#[test]
fn json_and_input_share_one_binding() {
    // Both names must resolve to the same value, not two copies that can
    // drift when one of them is fixed.
    let context = context();
    let from_json = engine().evaluate("$json.person", &context).unwrap();
    let from_input = engine().evaluate("$input.person", &context).unwrap();
    assert_eq!(from_json, from_input);
}
