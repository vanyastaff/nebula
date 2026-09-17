use std::future::Future;

use nebula_action::testing::TestContextBuilder;
use nebula_action::{ActionError, ActionResult, StatelessAction};
use serde_json::{Value, json};

use super::{NullsOrder, Sort, SortInput, SortKey, SortOrder};

fn run(input: SortInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
    let action = Sort;
    let ctx = TestContextBuilder::new().build();
    async move { action.execute(input, &ctx).await }
}

fn extract_output(result: ActionResult<Value>) -> Value {
    result
        .into_primary_output()
        .and_then(nebula_action::ActionOutput::into_value)
        .expect("ActionResult must carry a primary output value")
}

fn asc(field: &str) -> SortKey {
    SortKey {
        field: field.into(),
        order: SortOrder::Asc,
        nulls: NullsOrder::Greatest,
        case_insensitive: false,
    }
}

fn desc(field: &str) -> SortKey {
    SortKey {
        field: field.into(),
        order: SortOrder::Desc,
        nulls: NullsOrder::Greatest,
        case_insensitive: false,
    }
}

/// A key with explicit `nulls` placement (ascending).
fn asc_nulls(field: &str, nulls: NullsOrder) -> SortKey {
    SortKey {
        field: field.into(),
        order: SortOrder::Asc,
        nulls,
        case_insensitive: false,
    }
}

/// A case-insensitive ascending key.
fn asc_ci(field: &str) -> SortKey {
    SortKey {
        field: field.into(),
        order: SortOrder::Asc,
        nulls: NullsOrder::Greatest,
        case_insensitive: true,
    }
}

// ── 1: non-array data is Fatal ────────────────────────────────────────────
//
// RED witness: without the type-guard arm, the object would not be rejected
// and `unwrap_err()` would panic.
#[tokio::test]
async fn non_array_data_is_fatal() {
    let input = SortInput {
        data: Some(json!({"n": 1})),
        keys: vec![asc("n")],
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
    let input = SortInput {
        data: Some(json!(null)),
        keys: vec![asc("n")],
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for null data; got: {err:?}"
    );
}

// ── 3: empty keys is Fatal ────────────────────────────────────────────────
//
// RED witness: without the `keys.is_empty()` guard the sort would succeed
// with all elements considered Equal (original order preserved), returning
// Ok instead of Err.
#[tokio::test]
async fn empty_keys_is_fatal() {
    let input = SortInput {
        data: Some(json!([{"n": 1}])),
        keys: vec![],
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for empty keys; got: {err:?}"
    );
}

// ── 4: non-object element is Fatal ────────────────────────────────────────
//
// `Value::get` on a non-object returns `None` silently. Without the
// explicit `is_object()` guard, field reads during comparison would misfire
// rather than producing an error.
//
// RED witness: without the guard, the number `5` would be treated as an
// element with all-null fields (sorts last) rather than producing Fatal.
#[tokio::test]
async fn non_object_element_is_fatal() {
    let input = SortInput {
        data: Some(json!([{"n": 1}, 5])),
        keys: vec![asc("n")],
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "expected Fatal for non-object element; got: {err:?}"
    );
}

// ── 5: sort single key ascending (numeric) ────────────────────────────────
//
// RED witness: without the sort impl, the elements remain in input order
// [3,1,2] — the assert would fail.
#[tokio::test]
async fn sort_single_key_ascending_numeric() {
    let input = SortInput {
        data: Some(json!([{"n": 3}, {"n": 1}, {"n": 2}])),
        keys: vec![asc("n")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"n": 1}, {"n": 2}, {"n": 3}]),
        "ascending numeric sort must produce [1,2,3]"
    );
}

// ── 5b: sort by large 64-bit integer IDs (beyond f64 precision) ───────────
//
// 2^53, 2^53+1, 2^53+2 are distinct i64 but all round to (at most two) f64
// values. RED witness: with the old f64 comparison they compared Equal and
// the stable sort left them in input order [+1, +2, +0] — this assert fails.
#[tokio::test]
async fn sort_large_integer_ids_exact() {
    let input = SortInput {
        data: Some(json!([
            { "id": 9_007_199_254_740_993_i64 },
            { "id": 9_007_199_254_740_994_i64 },
            { "id": 9_007_199_254_740_992_i64 },
        ])),
        keys: vec![asc("id")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([
            { "id": 9_007_199_254_740_992_i64 },
            { "id": 9_007_199_254_740_993_i64 },
            { "id": 9_007_199_254_740_994_i64 },
        ]),
        "large integer IDs must sort by exact value, not collapse via f64"
    );
}

// ── 6: sort single key descending (numeric) ───────────────────────────────
#[tokio::test]
async fn sort_single_key_descending() {
    let input = SortInput {
        data: Some(json!([{"n": 3}, {"n": 1}, {"n": 2}])),
        keys: vec![desc("n")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"n": 3}, {"n": 2}, {"n": 1}]),
        "descending numeric sort must produce [3,2,1]"
    );
}

// ── 7: sort string field lexicographically ────────────────────────────────
#[tokio::test]
async fn sort_string_field() {
    let input = SortInput {
        data: Some(json!([{"s": "banana"}, {"s": "apple"}, {"s": "cherry"}])),
        keys: vec![asc("s")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"s": "apple"}, {"s": "banana"}, {"s": "cherry"}]),
        "string sort must be lexicographic ascending"
    );
}

// ── 7b: case-insensitive string sort ──────────────────────────────────────
//
// Byte-order (case-sensitive) would put the capitalized "Cherry" (C=0x43)
// before the lowercase "apple"/"banana" → ["Cherry","apple","banana"].
// Case-insensitive folds case first → ["apple","banana","Cherry"]. The two
// assertions together prove the flag actually changes the ordering.
#[tokio::test]
async fn sort_case_insensitive_strings() {
    let data = json!([{"s": "banana"}, {"s": "apple"}, {"s": "Cherry"}]);

    let ci = extract_output(
        run(SortInput {
            data: Some(data.clone()),
            keys: vec![asc_ci("s")],
        })
        .await
        .unwrap(),
    );
    assert_eq!(
        ci,
        json!([{"s": "apple"}, {"s": "banana"}, {"s": "Cherry"}]),
        "case-insensitive sort must order apple < banana < Cherry"
    );

    // Case-sensitive (default) puts the capitalized value first by byte order.
    let cs = extract_output(
        run(SortInput {
            data: Some(data),
            keys: vec![asc("s")],
        })
        .await
        .unwrap(),
    );
    assert_eq!(
        cs,
        json!([{"s": "Cherry"}, {"s": "apple"}, {"s": "banana"}]),
        "case-sensitive byte order puts 'Cherry' first"
    );
}

// ── 7c: nulls = First places null/missing first regardless of value ───────
#[tokio::test]
async fn sort_nulls_first_ascending() {
    let input = SortInput {
        data: Some(json!([{"n": 2}, {"x": "no n here"}, {"n": 1}])),
        keys: vec![asc_nulls("n", NullsOrder::First)],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"x": "no n here"}, {"n": 1}, {"n": 2}]),
        "nulls=First must put the missing-field row first, then 1, 2"
    );
}

// ── 7d: nulls = Last is ABSOLUTE — last even in descending order ──────────
//
// Default (`Greatest`) in desc would put null FIRST (null = greatest value,
// reversed). `Last` overrides that: null stays last regardless of direction.
#[tokio::test]
async fn sort_nulls_last_is_absolute_in_desc() {
    let input = SortInput {
        data: Some(json!([{"n": 2}, {"n": null}, {"n": 1}])),
        keys: vec![SortKey {
            field: "n".into(),
            order: SortOrder::Desc,
            nulls: NullsOrder::Last,
            case_insensitive: false,
        }],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"n": 2}, {"n": 1}, {"n": null}]),
        "nulls=Last must keep null last even when sorting descending"
    );
}

// ── 7e: default nulls = Greatest is unchanged (null last in asc) ──────────
#[tokio::test]
async fn sort_nulls_default_greatest_last_in_asc() {
    let input = SortInput {
        data: Some(json!([{"n": 2}, {"n": null}, {"n": 1}])),
        keys: vec![asc("n")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"n": 1}, {"n": 2}, {"n": null}]),
        "default (Greatest) must keep null last in ascending order"
    );
}

// ── 8: sort multi-key (primary asc + secondary desc tie-breaker) ──────────
//
// Input:  [{a:1,b:1},{a:1,b:2},{a:0,b:9}]
// Sort:   [a asc, b desc]
// Step 1: a asc → {a:0,b:9} first; then the two {a:1,...} elements.
// Step 2: among the {a:1,...} elements, b desc → b=2 before b=1.
// Expected: [{a:0,b:9},{a:1,b:2},{a:1,b:1}].
//
// RED witness: a single-key sort on `a` alone would produce
// [{a:0,...},{a:1,b:1},{a:1,b:2}] (or either order for the b=1/b=2 pair),
// failing the concrete-equality assertion.
#[tokio::test]
async fn sort_multi_key() {
    let input = SortInput {
        data: Some(json!([
            {"a": 1, "b": 1},
            {"a": 1, "b": 2},
            {"a": 0, "b": 9}
        ])),
        keys: vec![asc("a"), desc("b")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"a": 0, "b": 9}, {"a": 1, "b": 2}, {"a": 1, "b": 1}]),
        "multi-key sort [a asc, b desc] must order correctly"
    );
}

// ── 9: sort is stable — equal elements preserve original order ────────────
//
// Input:  [{k:1,id:"x"},{k:1,id:"y"}]  — both have k=1.
// Sort:   k asc.
// Expected: x before y (original order preserved; stable sort).
//
// RED witness: an unstable sort could produce y before x; also proves the
// test is sensitive to order (json equality of the full array).
#[tokio::test]
async fn sort_is_stable() {
    let input = SortInput {
        data: Some(json!([
            {"k": 1, "id": "x"},
            {"k": 1, "id": "y"}
        ])),
        keys: vec![asc("k")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"k": 1, "id": "x"}, {"k": 1, "id": "y"}]),
        "sort must be stable: equal-key elements retain original relative order (x before y)"
    );
}

// ── 10: missing field sorts last ascending ────────────────────────────────
//
// Input:  [{n:2},{},{n:1}]  (middle element has no `n` field)
// Sort:   n asc → present-value elements first, then the missing-field one.
// Expected: [{n:1},{n:2},{}].
//
// RED witness: treating null/missing as 0 (less than any positive value)
// would place {} first instead of last.
#[tokio::test]
async fn missing_field_sorts_last_ascending() {
    let input = SortInput {
        data: Some(json!([{"n": 2}, {}, {"n": 1}])),
        keys: vec![asc("n")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"n": 1}, {"n": 2}, {}]),
        "missing-field element must sort last in ascending order"
    );
}

// ── 11: missing field sorts first descending ──────────────────────────────
//
// Same input: [{n:2},{},{n:1}]
// Sort: n desc → missing is GREATEST, goes first.
// Expected: [{},{n:2},{n:1}].
//
// RED witness: treating null/missing as 0 (less than positives) would place
// {} last instead of first in descending order.
#[tokio::test]
async fn missing_field_sorts_first_descending() {
    let input = SortInput {
        data: Some(json!([{"n": 2}, {}, {"n": 1}])),
        keys: vec![desc("n")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{}, {"n": 2}, {"n": 1}]),
        "missing-field element must sort first in descending order"
    );
}

// ── 11b: explicit null field sorts as GREATEST (same as a missing key) ────
//
// A present field whose value is `null` is treated identically to an absent
// key — GREATEST — so it sorts last in asc / first in desc. This is a
// distinct code path (`Some(Value::Null)`) from the missing-key (`None`)
// case above; assert it directly so the doc claim can't silently regress.
#[tokio::test]
async fn null_field_sorts_last_ascending() {
    let input = SortInput {
        data: Some(json!([{"n": 2}, {"n": null}, {"n": 1}])),
        keys: vec![asc("n")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"n": 1}, {"n": 2}, {"n": null}]),
        "explicit-null field must sort last in ascending order"
    );
}

#[tokio::test]
async fn null_field_sorts_first_descending() {
    let input = SortInput {
        data: Some(json!([{"n": 2}, {"n": null}, {"n": 1}])),
        keys: vec![desc("n")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(
        out,
        json!([{"n": null}, {"n": 2}, {"n": 1}]),
        "explicit-null field must sort first in descending order"
    );
}

// ── 12: type-mismatch field is Fatal ─────────────────────────────────────
//
// Input:  [{n:1},{n:"x"}]  — `n` is a number in element 0, a string in
// element 1.  `compare_ordered` will Fatal on this mismatch; the latched-
// error pattern propagates it after the sort.
//
// RED witness: without error propagation, the comparator would return
// `Ordering::Equal` for the bad pair and the sort would silently succeed
// with the error swallowed — `unwrap_err()` would panic.
#[tokio::test]
async fn type_mismatch_field_is_fatal() {
    let input = SortInput {
        data: Some(json!([{"n": 1}, {"n": "x"}])),
        keys: vec![asc("n")],
    };
    let err = run(input).await.unwrap_err();
    assert!(
        matches!(err, ActionError::Fatal { .. }),
        "type mismatch between number and string must be Fatal; got: {err:?}"
    );
}

// ── 13: empty input array → [] (not Fatal) ────────────────────────────────
#[tokio::test]
async fn empty_input_array_returns_empty_array() {
    let input = SortInput {
        data: Some(json!([])),
        keys: vec![asc("n")],
    };
    let out = extract_output(run(input).await.unwrap());
    assert_eq!(out, json!([]), "empty input must return empty array");
}

// ── 14: action key is "core.sort" ────────────────────────────────────────
#[test]
fn action_key_is_core_dot_sort() {
    let factory =
        nebula_action::GenericStatelessFactory::<Sort>::new().expect("sort metadata must admit");
    assert_eq!(
        nebula_action::ActionFactory::metadata(&factory)
            .base()
            .key()
            .as_str(),
        "core.sort"
    );
}
