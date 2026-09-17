use std::future::Future;

use nebula_action::testing::TestContextBuilder;
use nebula_action::{ActionError, ActionResult, StatelessAction};
use serde_json::{Value, json};

use super::{Aggregate, AggregateInput, Aggregation, OnError};

fn run(input: AggregateInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
    let action = Aggregate;
    let ctx = TestContextBuilder::new().build();
    async move { action.execute(input, &ctx).await }
}

fn extract_output(result: ActionResult<Value>) -> Value {
    result
        .into_primary_output()
        .and_then(nebula_action::ActionOutput::into_value)
        .expect("ActionResult must carry a primary output value")
}

// ── 1: non-array data is Fatal ────────────────────────────────────────────
//
// RED witness: without the type-guard arm, the object would not be
// rejected and `unwrap_err()` would panic.
#[tokio::test]
async fn non_array_data_is_fatal() {
    let input = AggregateInput {
        data: Some(json!({"x": 1})),
        group_by: vec![],
        aggregations: vec![Aggregation::Count { out: "n".into() }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for object data; got: {err:?}"
    );
}

// ── 2: null data is Fatal ─────────────────────────────────────────────────
#[tokio::test]
async fn null_data_is_fatal() {
    let input = AggregateInput {
        data: Some(json!(null)),
        group_by: vec![],
        aggregations: vec![Aggregation::Count { out: "n".into() }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for null data; got: {err:?}"
    );
}

// ── 3: empty aggregations is Fatal ────────────────────────────────────────
#[tokio::test]
async fn empty_aggregations_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"a": 1}])),
        group_by: vec![],
        aggregations: vec![],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for empty aggregations; got: {err:?}"
    );
}

// ── 4: non-object element is Fatal ────────────────────────────────────────
//
// `Value::get` on a non-object returns `None` silently. Without the
// explicit `is_object()` guard, group-key and field reads would misfire
// instead of producing an error.
//
// RED witness: remove the `is_object()` guard and the number `5` would
// produce a wrong/unexpected result rather than a Fatal error.
#[tokio::test]
async fn non_object_element_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"a": 1}, 5])),
        group_by: vec![],
        aggregations: vec![Aggregation::Count { out: "n".into() }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for non-object element; got: {err:?}"
    );
}

// ── 5: sum and count ungrouped ────────────────────────────────────────────
//
// Asserts the output is EXACTLY `[{"total":60,"n":3}]` and that `total`
// is an INTEGER `60` (not a float 60.0).
#[tokio::test]
async fn sum_and_count_ungrouped() {
    let input = AggregateInput {
        data: Some(json!([{"amount": 10}, {"amount": 20}, {"amount": 30}])),
        group_by: vec![],
        aggregations: vec![
            Aggregation::Sum {
                field: "amount".into(),
                out: "total".into(),
            },
            Aggregation::Count { out: "n".into() },
        ],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output,
        json!([{"total": 60, "n": 3}]),
        "ungrouped sum+count must return exactly one summary row"
    );
    // Assert `total` is an integer Value, not a float.
    let total_value = &output[0]["total"];
    assert_eq!(
        total_value.as_i64(),
        Some(60),
        "sum of integers must be integer 60; got: {total_value:?}"
    );
}

// ── 6: sum preserves integer type; mixed int+float returns float ──────────
//
// RED witness for integer path: if the impl always used f64, `as_i64()`
// would fail (f64 60.0 has no i64 representation in serde_json).
// RED witness for upgrade: if the impl stayed integer when seeing a float,
// the float-delta assertion would fail.
#[tokio::test]
async fn sum_preserves_integer_type() {
    // All integers → result must be integer 60.
    let integer_input = AggregateInput {
        data: Some(json!([{"x": 10}, {"x": 20}, {"x": 30}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Sum {
            field: "x".into(),
            out: "s".into(),
        }],
        on_error: OnError::Fail,
    };
    let integer_output = extract_output(run(integer_input).await.unwrap());
    assert_eq!(
        integer_output[0]["s"].as_i64(),
        Some(60),
        "integer-only sum must be i64 60; got: {:?}",
        integer_output[0]["s"]
    );

    // Mixed int + float → result must be a float.
    let mixed_input = AggregateInput {
        data: Some(json!([{"x": 10}, {"x": 20.5}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Sum {
            field: "x".into(),
            out: "s".into(),
        }],
        on_error: OnError::Fail,
    };
    let mixed_output = extract_output(run(mixed_input).await.unwrap());
    let mixed_sum = mixed_output[0]["s"]
        .as_f64()
        .expect("mixed int+float sum must be a float Number");
    assert!(
        (mixed_sum - 30.5_f64).abs() < 1e-9,
        "mixed int+float sum must be 30.5; got: {mixed_sum}"
    );
}

// ── 7: avg is always float ────────────────────────────────────────────────
#[tokio::test]
async fn avg_is_float() {
    let input = AggregateInput {
        data: Some(json!([{"x": 1}, {"x": 2}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Avg {
            field: "x".into(),
            out: "avg".into(),
        }],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    let avg_value = output[0]["avg"]
        .as_f64()
        .expect("avg must be a float Number");
    assert!(
        (avg_value - 1.5_f64).abs() < 1e-9,
        "avg of [1, 2] must be 1.5; got: {avg_value}"
    );
}

// ── 8: min/max preserve the original element value ────────────────────────
#[tokio::test]
async fn min_max_preserve_element_value() {
    let input = AggregateInput {
        data: Some(json!([{"v": 3}, {"v": 1}, {"v": 2}])),
        group_by: vec![],
        aggregations: vec![
            Aggregation::Min {
                field: "v".into(),
                out: "lo".into(),
            },
            Aggregation::Max {
                field: "v".into(),
                out: "hi".into(),
            },
        ],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(output[0]["lo"].as_i64(), Some(1), "min must be integer 1");
    assert_eq!(output[0]["hi"].as_i64(), Some(3), "max must be integer 3");
}

// ── 8b: min/max over large 64-bit IDs use EXACT comparison ────────────────
//
// 2^53, 2^53+1, 2^53+2 are distinct i64 but collapse to (near-)equal f64.
// RED witness: the old f64 comparison treated them as Equal and kept the
// first-seen value, so min == max == 2^53+1 (the first element) — both
// asserts below fail.
#[tokio::test]
async fn min_max_large_integers_exact() {
    let input = AggregateInput {
        data: Some(json!([
            {"v": 9_007_199_254_740_993_i64},
            {"v": 9_007_199_254_740_994_i64},
            {"v": 9_007_199_254_740_992_i64},
        ])),
        group_by: vec![],
        aggregations: vec![
            Aggregation::Min {
                field: "v".into(),
                out: "lo".into(),
            },
            Aggregation::Max {
                field: "v".into(),
                out: "hi".into(),
            },
        ],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output[0]["lo"].as_i64(),
        Some(9_007_199_254_740_992),
        "min must be the exact smallest large integer"
    );
    assert_eq!(
        output[0]["hi"].as_i64(),
        Some(9_007_199_254_740_994),
        "max must be the exact largest large integer"
    );
}

// ── 9: group_by produces one row per key in first-seen order ─────────────
//
// Input order: b, a, b — so "b" is first-seen before "a".
// Expected output rows in that order: [{r:"b", s:4}, {r:"a", s:2}].
//
// RED witness for order: a HashMap-iteration-ordered impl could produce
// [{r:"a",...},{r:"b",...}], failing the exact-array equality assertion.
// RED witness for grouping: if the two "b" rows are not merged, "b" would
// appear with s=1 and s=3 separately instead of s=4.
#[tokio::test]
async fn group_by_produces_one_row_per_key_in_first_seen_order() {
    let input = AggregateInput {
        data: Some(json!([
            {"r": "b", "v": 1},
            {"r": "a", "v": 2},
            {"r": "b", "v": 3}
        ])),
        group_by: vec!["r".into()],
        aggregations: vec![Aggregation::Sum {
            field: "v".into(),
            out: "s".into(),
        }],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output,
        json!([{"r": "b", "s": 4}, {"r": "a", "s": 2}]),
        "group rows must be in first-seen order (b before a); \
         and b's two values must be summed (1+3=4)"
    );
}

// ── 10: on_error=Fail on missing field is Fatal ───────────────────────────
//
// THE KEY HONESTY TEST: the default Fail policy must ACTUALLY fail when a
// numeric aggregation encounters a missing field. A naive SQL-NULL-skip
// implementation would return `Ok` with the partial (smaller-denominator)
// sum instead.
//
// RED witness: a naive skip impl returns Ok([{"total": 20}]) — which is
// not an Err, causing `unwrap_err()` to panic. The explicit dirty-value
// check with `Fail` returns a Fatal instead.
#[tokio::test]
async fn on_error_fail_on_missing_field_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([
            {"amount": 10},
            {"amount": 20},
            {"note": "no amount field here"}
        ])),
        group_by: vec![],
        aggregations: vec![Aggregation::Sum {
            field: "amount".into(),
            out: "total".into(),
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "Fail policy must produce Fatal when a field is missing; got: {err:?}"
    );
}

// ── 11: on_error=Skip ignores dirty value ─────────────────────────────────
#[tokio::test]
async fn on_error_skip_ignores_dirty_value() {
    let input = AggregateInput {
        data: Some(json!([
            {"amount": 10},
            {"amount": 20},
            {"note": "no amount field here"}
        ])),
        group_by: vec![],
        aggregations: vec![Aggregation::Sum {
            field: "amount".into(),
            out: "total".into(),
        }],
        on_error: OnError::Skip,
    };
    let output = extract_output(run(input).await.unwrap());
    // Third element is skipped; sum is 10+20=30.
    assert_eq!(
        output[0]["total"],
        json!(30),
        "Skip policy must ignore the missing-field element; partial sum must be 30"
    );
}

// ── 12: on_error=Fail on non-numeric value is Fatal ──────────────────────
#[tokio::test]
async fn on_error_fail_on_non_numeric_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"x": "hello"}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Sum {
            field: "x".into(),
            out: "s".into(),
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "Fail policy must produce Fatal for a non-numeric value; got: {err:?}"
    );
}

// ── 13: empty input, ungrouped → one zeroed summary row ──────────────────
//
// Locked table from the spec:
// count→0, sum→0, avg→null, collect→[], join→""
#[tokio::test]
async fn empty_input_ungrouped_returns_zeroed_row() {
    let input = AggregateInput {
        data: Some(json!([])),
        group_by: vec![],
        aggregations: vec![
            Aggregation::Sum {
                field: "x".into(),
                out: "sum".into(),
            },
            Aggregation::Count {
                out: "count".into(),
            },
            Aggregation::Avg {
                field: "x".into(),
                out: "avg".into(),
            },
            Aggregation::Collect {
                field: "x".into(),
                out: "collected".into(),
            },
            Aggregation::Join {
                field: "x".into(),
                out: "joined".into(),
                sep: ",".into(),
            },
        ],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output,
        json!([{"sum": 0, "count": 0, "avg": null, "collected": [], "joined": ""}]),
        "empty ungrouped input must return exactly one zeroed summary row"
    );
}

// ── 14: empty input, grouped → empty array ────────────────────────────────
#[tokio::test]
async fn empty_input_grouped_returns_empty_array() {
    let input = AggregateInput {
        data: Some(json!([])),
        group_by: vec!["r".into()],
        aggregations: vec![Aggregation::Count { out: "n".into() }],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output,
        json!([]),
        "empty grouped input must return empty array"
    );
}

// ── 15: collect and join ──────────────────────────────────────────────────
#[tokio::test]
async fn collect_and_join() {
    let input = AggregateInput {
        data: Some(json!([
            {"id": 1, "tag": "a"},
            {"id": 2, "tag": "b"},
            {"id": 3, "tag": "a"}
        ])),
        group_by: vec![],
        aggregations: vec![
            Aggregation::Collect {
                field: "id".into(),
                out: "ids".into(),
            },
            Aggregation::Join {
                field: "tag".into(),
                out: "tags".into(),
                sep: "|".into(),
            },
        ],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output[0]["ids"],
        json!([1, 2, 3]),
        "collect must gather all id values in order"
    );
    assert_eq!(
        output[0]["tags"],
        json!("a|b|a"),
        "join must concatenate tag values with '|'"
    );
}

// ── 15b: join with a non-string value under Fail is Fatal ─────────────────
//
// RED witness: the old join arm silently dropped non-string values, so this
// returned Ok with "a,b" instead of failing — `unwrap_err()` would panic.
#[tokio::test]
async fn join_non_string_under_fail_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"x": "a"}, {"x": 1}, {"x": "b"}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Join {
            field: "x".into(),
            out: "j".into(),
            sep: ",".into(),
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "join over a non-string value under Fail must be Fatal; got: {err:?}"
    );
}

// ── 15c: join with a non-string value under Skip drops that value ─────────
#[tokio::test]
async fn join_non_string_under_skip_drops_value() {
    let input = AggregateInput {
        data: Some(json!([{"x": "a"}, {"x": 1}, {"x": "b"}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Join {
            field: "x".into(),
            out: "j".into(),
            sep: ",".into(),
        }],
        on_error: OnError::Skip,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output[0]["j"],
        json!("a,b"),
        "under Skip, join must drop the non-string value and join the rest"
    );
}

// ── 15d: join still silently skips null/missing even under Fail ───────────
#[tokio::test]
async fn join_null_and_missing_skipped_under_fail() {
    let input = AggregateInput {
        data: Some(json!([{"x": "a"}, {"x": null}, {"other": 1}, {"x": "b"}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Join {
            field: "x".into(),
            out: "j".into(),
            sep: ",".into(),
        }],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output[0]["j"],
        json!("a,b"),
        "null/missing are silently skipped (documented), even under Fail"
    );
}

// ── 16: duplicate out key is Fatal ────────────────────────────────────────
#[tokio::test]
async fn duplicate_out_key_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"x": 1}])),
        group_by: vec![],
        aggregations: vec![
            Aggregation::Count { out: "n".into() },
            Aggregation::Count { out: "n".into() }, // duplicate
        ],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "duplicate out key must be Fatal; got: {err:?}"
    );
}

// ── 17: group_by missing key is Fatal ─────────────────────────────────────
//
// Cannot determine the group when a group-by field is absent on an element.
#[tokio::test]
async fn group_by_missing_key_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"r": "a", "v": 1}, {"v": 2}])), // second element missing "r"
        group_by: vec!["r".into()],
        aggregations: vec![Aggregation::Count { out: "n".into() }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "missing group_by field must be Fatal; got: {err:?}"
    );
}

// ── 18: action key is "core.aggregate" ───────────────────────────────────
#[test]
fn action_key_is_core_dot_aggregate() {
    let factory = nebula_action::GenericStatelessFactory::<Aggregate>::new()
        .expect("aggregate metadata must admit");
    assert_eq!(
        nebula_action::ActionFactory::metadata(&factory)
            .base()
            .key()
            .as_str(),
        "core.aggregate"
    );
}

// ── FIX 1: float sum overflow → Fatal, NOT silent 0 ──────────────────────
//
// Summing two 1e308 values overflows f64 to +Infinity at runtime.
// The previous implementation used `unwrap_or_else(|| Number::from(0i64))`
// which silently returned `Ok([{"total": 0.0}])` — data corruption.
//
// RED witness: the old `unwrap_or_else(0)` code returns `Ok(...)` so
// `unwrap_err()` panics. The new `ok_or_else(…)?` path returns Fatal.
#[tokio::test]
async fn float_sum_overflow_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"x": 1e308_f64}, {"x": 1e308_f64}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Sum {
            field: "x".into(),
            out: "total".into(),
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "float sum overflow must be Fatal, not silent 0; got: {err:?}"
    );
}

// ── FIX 1b: i64 sum overflow → Fatal ─────────────────────────────────────
//
// Pin the integer path's `checked_add` guard: two i64 values whose sum
// exceeds i64::MAX must fail, exactly like the float path above — never
// wrap silently.
//
// RED witness: an unchecked `+` would wrap (debug: panic; release: wrap to
// a wrong negative total) and `unwrap_err()` would panic on the Ok result.
#[tokio::test]
async fn i64_sum_overflow_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([
            {"x": i64::MAX},
            {"x": 1}
        ])),
        group_by: vec![],
        aggregations: vec![Aggregation::Sum {
            field: "x".into(),
            out: "total".into(),
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "i64 sum overflow must be Fatal, not a wrapped total; got: {err:?}"
    );
    let message = std::error::Error::source(&err)
        .map(ToString::to_string)
        .expect("fatal i64-sum-overflow error must retain its source");
    assert!(
        message.contains("aggregate: sum overflow"),
        "i64 sum overflow must name the overflow; got: {message}"
    );
}

// ── FIX 2a: null field under avg + Fail is Fatal ──────────────────────────
//
// Proves the null guard covers avg (not just sum).
// RED witness: a path that skips null without checking policy returns Ok.
#[tokio::test]
async fn avg_null_under_fail_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"x": 1}, {"x": null}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Avg {
            field: "x".into(),
            out: "avg".into(),
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "avg with null field under Fail must be Fatal; got: {err:?}"
    );
}

// ── FIX 2b: null field under min + Fail is Fatal ──────────────────────────
#[tokio::test]
async fn min_null_under_fail_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"x": 5}, {"x": null}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Min {
            field: "x".into(),
            out: "lo".into(),
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "min with null field under Fail must be Fatal; got: {err:?}"
    );
}

// ── FIX 2c: null field under max + Fail is Fatal ──────────────────────────
#[tokio::test]
async fn max_null_under_fail_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"x": 5}, {"x": null}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Max {
            field: "x".into(),
            out: "hi".into(),
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "max with null field under Fail must be Fatal; got: {err:?}"
    );
}

// ── FIX 2d: avg Skip shrinks denominator correctly ────────────────────────
//
// [{"x":10},{"x":null}], avg(x), Skip → mean of ONE value = 10.0.
// A buggy impl that increments `contributing_count` for skipped values
// would return 5.0 (10/2). This test catches that regression.
#[tokio::test]
async fn avg_skip_shrinks_denominator_correctly() {
    let input = AggregateInput {
        data: Some(json!([{"x": 10}, {"x": null}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Avg {
            field: "x".into(),
            out: "avg".into(),
        }],
        on_error: OnError::Skip,
    };
    let output = extract_output(run(input).await.unwrap());
    let avg_value = output[0]["avg"]
        .as_f64()
        .expect("avg must be a float Number");
    assert!(
        (avg_value - 10.0_f64).abs() < 1e-9,
        "avg under Skip must use only non-null values; expected 10.0, got: {avg_value}"
    );
}

// ── FIX 3a: count_distinct distinguishes types ────────────────────────────
//
// `1` (integer), `"1"` (string), and `1.0` (float) must be counted as
// three distinct values. serde_json's canonical `to_string()` produces
// `"1"`, `"\"1\""`, and `"1.0"` respectively — all distinct.
//
// Guards against a future refactor to `as_str()` which would conflate
// non-string values (they return None and would all be dropped).
#[tokio::test]
async fn count_distinct_distinguishes_types() {
    let input = AggregateInput {
        data: Some(json!([{"v": 1}, {"v": "1"}, {"v": 1.0_f64}])),
        group_by: vec![],
        aggregations: vec![Aggregation::CountDistinct {
            field: "v".into(),
            out: "distinct_count".into(),
        }],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    assert_eq!(
        output[0]["distinct_count"].as_u64(),
        Some(3),
        "count_distinct must distinguish integer 1, string '1', and float 1.0"
    );
}

// ── FIX 3b: group_by with multiple keys ───────────────────────────────────
//
// Exercises the multi-key zip/reconstruct path: two group-by fields,
// three input elements producing two distinct (dept, level) groups in
// first-seen order.
//
// RED witness: a single-key impl would group by only `dept`, producing
// one row instead of two.
#[tokio::test]
async fn group_by_multiple_keys() {
    let input = AggregateInput {
        data: Some(json!([
            {"dept": "eng", "level": "sr", "v": 1},
            {"dept": "eng", "level": "jr", "v": 2},
            {"dept": "eng", "level": "sr", "v": 3}
        ])),
        group_by: vec!["dept".into(), "level".into()],
        aggregations: vec![Aggregation::Sum {
            field: "v".into(),
            out: "s".into(),
        }],
        on_error: OnError::Fail,
    };
    let output = extract_output(run(input).await.unwrap());
    // Two groups: (eng, sr) first-seen, (eng, jr) second.
    assert_eq!(
        output,
        json!([
            {"dept": "eng", "level": "sr", "s": 4},
            {"dept": "eng", "level": "jr", "s": 2}
        ]),
        "multi-key group_by must produce one row per unique key tuple in first-seen order"
    );
}

// ── FIX B: u64 above i64::MAX is accepted by sum ─────────────────────────
//
// serde_json represents u64::MAX as a Number that returns None from as_i64()
// but Some from as_f64(). The old `is_i64()/is_f64()` match fell through
// to the dirty-value arm and returned Fatal for a valid JSON number.
//
// RED witness: the old impl returned Err(Fatal) causing `unwrap()` to panic.
// The new `is_number()` → `as_i64()` → `as_f64()` path routes u64-only
// values through the float upgrade, returning a finite f64 approximation.
#[tokio::test]
async fn sum_handles_u64_above_i64_max() {
    // u64::MAX = 18446744073709551615; serde_json encodes this as a u64 Number.
    let input = AggregateInput {
        data: Some(json!([{"x": u64::MAX}])),
        group_by: vec![],
        aggregations: vec![Aggregation::Sum {
            field: "x".into(),
            out: "total".into(),
        }],
        on_error: OnError::Fail,
    };
    // Must NOT be Fatal; must return a finite numeric result.
    let output = extract_output(run(input).await.unwrap());
    let total = output[0]["total"]
        .as_f64()
        .expect("sum of u64::MAX must be a float Number");
    assert!(
        total.is_finite(),
        "sum of u64::MAX must be a finite float, not Inf/NaN; got: {total}"
    );
}

// ── FIX C: aggregation `out` colliding with group_by field is Fatal ───────
//
// Without this check, the aggregation result would silently overwrite the
// group key value in the output row — data corruption invisible to the author.
//
// RED witness: without the collision check, sum(amount) out="region" would
// emit `[{"region": 30}]` (the sum value overwrites the group key), causing
// `unwrap_err()` to panic on the Ok result.
#[tokio::test]
async fn out_colliding_with_group_by_is_fatal() {
    let input = AggregateInput {
        data: Some(json!([{"region": "west", "amount": 10}])),
        group_by: vec!["region".into()],
        aggregations: vec![Aggregation::Sum {
            field: "amount".into(),
            out: "region".into(), // collides with the group_by field
        }],
        on_error: OnError::Fail,
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "aggregation out key colliding with group_by field must be Fatal; got: {err:?}"
    );
}

// ── Authoring-error precedence and messages ──────────────────────────────

/// The message a `Fatal` rejection carries.
///
/// Every authoring rejection is a `Fatal` whose message is its whole value —
/// it names the offending key or field for the author — so these tests
/// assert the message, not just the variant.
fn fatal_message(err: &ActionError) -> String {
    std::error::Error::source(err)
        .expect("a Fatal ActionError must retain its message as its source")
        .to_string()
}

fn count(out: &str) -> Aggregation {
    Aggregation::Count { out: out.into() }
}

fn input(data: Option<Value>, group_by: &[&str], aggregations: Vec<Aggregation>) -> AggregateInput {
    AggregateInput {
        data,
        group_by: group_by.iter().map(|field| (*field).to_owned()).collect(),
        aggregations,
        on_error: OnError::Fail,
    }
}

/// Which authoring mistake an input with several is reported for, and the
/// message it carries.
///
/// Precedence is load-bearing and invisible in the happy path: an input with
/// more than one mistake must still be reported for the same one it was
/// reported for before the grouping code was restructured. In particular the
/// duplicate-`out` pass sweeps every aggregation before the collision pass
/// sweeps any, so an input that is both duplicate and colliding reports the
/// duplicate — merging those two passes would silently change the message.
///
/// RED witness: swapping the two sweeps, or reading group keys before
/// guarding that an element is an object, reports a different message here.
#[tokio::test]
async fn authoring_errors_report_the_first_failure_in_order() {
    let cases: Vec<(&str, AggregateInput, &str)> = vec![
        (
            "typed data outranks empty aggregations",
            input(Some(json!({"x": 1})), &[], vec![]),
            "aggregate: `data` must be a JSON array, got object",
        ),
        (
            "null data outranks empty aggregations",
            input(Some(Value::Null), &[], vec![]),
            "aggregate: `data` must be a JSON array, got null",
        ),
        (
            "absent data outranks empty aggregations",
            input(None, &[], vec![]),
            "aggregate: `data` must be a JSON array, got null",
        ),
        (
            "empty aggregations",
            input(Some(json!([])), &[], vec![]),
            "aggregate: at least one aggregation is required",
        ),
        (
            "duplicate out key outranks a group_by collision",
            input(
                Some(json!([{"n": 1}])),
                &["n"],
                vec![count("n"), count("n")],
            ),
            "aggregate: duplicate out key `n` in aggregations",
        ),
        (
            "duplicate out key",
            input(
                Some(json!([{"n": 1}])),
                &[],
                vec![count("n"), count("total"), count("n")],
            ),
            "aggregate: duplicate out key `n` in aggregations",
        ),
        (
            "out key colliding with a group_by field",
            input(
                Some(json!([{"region": "west"}])),
                &["region"],
                vec![count("region")],
            ),
            "aggregate: aggregation output `region` collides with a group_by field",
        ),
        (
            "non-object element outranks a missing group_by field",
            input(Some(json!([1])), &["region"], vec![count("n")]),
            "aggregate: every array element must be a JSON object, got number",
        ),
        (
            "missing group_by field",
            input(Some(json!([{"x": 1}])), &["region"], vec![count("n")]),
            "aggregate: group_by field `region` missing on an element",
        ),
    ];

    for (label, case_input, expected) in cases {
        let err = run(case_input)
            .await
            .expect_err("every case must be rejected before a row is emitted");
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "{label}: expected Fatal; got: {err:?}"
        );
        assert_eq!(fatal_message(&err), expected, "{label}");
    }
}

/// Group identity is the canonical JSON of the group-by values, so JSON type
/// survives: `1`, `"1"`, and `1.0` are three groups, not one.
///
/// This is the property the whole grouping design rests on — rows are keyed
/// by canonical bytes and then report the original values — and it has no
/// other pin in the suite.
///
/// RED witness: keying groups on a display string, or comparing the values
/// loosely, merges `1` into `"1"` and reports four rows with `n: 3` here.
#[tokio::test]
async fn group_keys_distinguish_json_types() {
    let input = input(
        Some(json!([
            {"k": 1},
            {"k": "1"},
            {"k": 1.0},
            {"k": true},
            {"k": null},
            {"k": "1"}
        ])),
        &["k"],
        vec![count("n")],
    );

    let rows = extract_output(run(input).await.unwrap());

    assert_eq!(
        rows,
        json!([
            {"k": 1, "n": 1},
            {"k": "1", "n": 2},
            {"k": 1.0, "n": 1},
            {"k": true, "n": 1},
            {"k": null, "n": 1}
        ]),
        "each JSON type keeps its own group, in first-seen order"
    );
}
