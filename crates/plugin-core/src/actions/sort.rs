//! `core.sort` — sort a JSON array of objects by one or more fields.
//!
//! Iterates the input array, validates that every element is a JSON object,
//! then performs a **stable** multi-key sort according to `keys`. The output
//! is a new array with the same elements in sorted order.
//!
//! This fills the sort-by-field gap in the `{{ }}` expression language, whose
//! `array.sort` builtin sorts by whole-value natural order and cannot sort an
//! array of objects by a named field.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": [
//!     { "name": "Charlie", "score": 80 },
//!     { "name": "Alice",   "score": 95 },
//!     { "name": "Bob",     "score": 80 }
//!   ],
//!   "keys": [
//!     { "field": "score", "order": "desc" },
//!     { "field": "name",  "order": "asc"  }
//!   ]
//! }
//! ```
//!
//! ## Output
//!
//! ```json
//! [
//!   { "name": "Alice",   "score": 95 },
//!   { "name": "Bob",     "score": 80 },
//!   { "name": "Charlie", "score": 80 }
//! ]
//! ```
//!
//! ## Null / missing field semantics
//!
//! By default a field value that is absent or `null` sorts as GREATEST: in
//! ascending order it appears last; in descending order it appears first. Each
//! key can override this with `nulls`: `"first"` or `"last"` place null/missing
//! at an **absolute** position regardless of `order` (`"greatest"` is the
//! default). If both elements are missing or null for a key, they are `Equal`
//! for that key and the next key is consulted (or original order preserved for
//! stability).
//!
//! ## Case-insensitive strings
//!
//! Set `case_insensitive: true` on a key to compare string values without
//! regard to case (Unicode-aware). It has no effect on non-string values.
//!
//! ## Error semantics
//!
//! - `data` absent / null / non-array → **Fatal**.
//! - `keys` empty → **Fatal**.
//! - Any array element that is not a JSON object → **Fatal** (explicit
//!   `is_object()` guard before sorting — `Value::get` on a non-object
//!   returns `None` silently which would corrupt the sort).
//! - Comparing fields of different scalable types (e.g. number vs. string)
//!   → **Fatal** propagated from `compare_ordered`.
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::cmp::Ordering;
use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::HasSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::condition::compare_ordered;
use crate::util::ValueTypeNameStr;

// ── Input types ───────────────────────────────────────────────────────────────

/// Sort direction for a single key.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// Smallest value first (default).
    #[default]
    Asc,
    /// Largest value first.
    Desc,
}

/// Where null/missing field values sort for a key.
///
/// `Greatest` (the default) treats null as the greatest *value*, so it
/// participates in the direction: last in `asc`, first in `desc`. `First` and
/// `Last` are **absolute** positions, independent of `order`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullsOrder {
    /// Null/missing sorts as the greatest value (default): last in `asc`,
    /// first in `desc`.
    #[default]
    Greatest,
    /// Null/missing always sorts first, regardless of `order`.
    First,
    /// Null/missing always sorts last, regardless of `order`.
    Last,
}

/// A single field-based sort key, with optional direction, null placement, and
/// case-insensitive string comparison.
///
/// ## Wire shape
///
/// ```json
/// { "field": "name", "order": "asc", "nulls": "last", "case_insensitive": true }
/// ```
///
/// `order` defaults to `"asc"`, `nulls` to `"greatest"`, and `case_insensitive`
/// to `false` when omitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortKey {
    /// Top-level field name to sort by.
    pub field: String,
    /// Sort direction. Defaults to ascending.
    #[serde(default)]
    pub order: SortOrder,
    /// Where null/missing values sort. Defaults to `Greatest`.
    #[serde(default)]
    pub nulls: NullsOrder,
    /// Compare string values case-insensitively (Unicode-aware). Defaults to
    /// `false`. Has no effect on non-string values.
    #[serde(default)]
    pub case_insensitive: bool,
}

/// Input for `core.sort`.
///
/// `data` must be a JSON array of objects when present. `null` / absent values
/// are rejected with a Fatal error — sorting a non-array is always an authoring
/// mistake.
///
/// ## Wire shape
///
/// ```json
/// {
///   "data": [ { "n": 3 }, { "n": 1 }, { "n": 2 } ],
///   "keys": [ { "field": "n", "order": "asc" } ]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortInput {
    /// Array of JSON objects to sort. Must be a JSON array when present.
    #[serde(default)]
    pub data: Option<Value>,
    /// Ordered sort keys: primary first, then tie-breakers. At least one required.
    pub keys: Vec<SortKey>,
}

// `data` is fully dynamic; the module doc describes expected structure.
impl HasSchema for SortInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that sorts a JSON array of objects by one or more named fields.
///
/// Keyed `core.sort`. No I/O, no credentials, no resources.
///
/// ## Example wire input / output
///
/// ```json
/// {
///   "data": [
///     { "priority": 2, "name": "beta"  },
///     { "priority": 1, "name": "alpha" },
///     { "priority": 2, "name": "alpha" }
///   ],
///   "keys": [
///     { "field": "priority", "order": "asc"  },
///     { "field": "name",     "order": "asc"  }
///   ]
/// }
/// ```
///
/// Output:
/// ```json
/// [
///   { "priority": 1, "name": "alpha" },
///   { "priority": 2, "name": "alpha" },
///   { "priority": 2, "name": "beta"  }
/// ]
/// ```
#[derive(Debug)]
pub struct Sort;

impl nebula_action::action::Action for Sort {
    type Input = SortInput;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.sort"),
            "Sort",
            "Sort an array of objects by one or more fields (asc/desc)",
        )
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for Sort {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Sort)
    }
}

impl StatelessAction for Sort {
    #[instrument(name = "core.sort", skip_all, fields(element_count))]
    async fn execute(
        &self,
        input: SortInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // ── 1. Validate data ──────────────────────────────────────────────────
        let mut elements: Vec<Value> = match input.data {
            Some(Value::Array(arr)) => arr,
            Some(Value::Null) | None => {
                return Err(ActionError::fatal(
                    "sort: `data` must be a JSON array, got null",
                ));
            },
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "sort: `data` must be a JSON array, got {}",
                    other.type_name_str()
                )));
            },
        };

        tracing::Span::current().record("element_count", elements.len());

        // ── 2. Validate keys non-empty ────────────────────────────────────────
        if input.keys.is_empty() {
            return Err(ActionError::fatal(
                "sort: at least one sort key is required",
            ));
        }

        // ── 3. Validate that every element is a JSON object ───────────────────
        //
        // `Value::get` on a non-object returns `None` silently, so field reads
        // during comparison would misfire without this explicit guard.
        // Validate ALL elements before sorting to fail fast and uniformly.
        for element in &elements {
            if !element.is_object() {
                return Err(ActionError::fatal(format!(
                    "sort: every array element must be a JSON object, got {}",
                    element.type_name_str()
                )));
            }
        }

        // Early exit: nothing to sort.
        if elements.len() <= 1 {
            return Ok(ActionResult::success(Value::Array(elements)));
        }

        // ── 4. Stable sort with latched-error comparator ──────────────────────
        //
        // `slice::sort_by` requires a total `Ordering` and cannot return an
        // error. To propagate a `compare_ordered` failure (e.g. comparing a
        // number field against a string field), the comparator captures the
        // first error into a `mut Option<ActionError>`. Once an error is
        // latched the comparator returns `Ordering::Equal` for all subsequent
        // pairs (causing them to preserve input order harmlessly). After
        // `sort_by` returns the latched error is checked and propagated as
        // Fatal. `sort_by` is stable — equal elements preserve their original
        // relative order.
        let mut latched_sort_error: Option<ActionError> = None;
        let keys = &input.keys;

        elements.sort_by(|elem_a, elem_b| {
            // Once an error is latched, stop doing real comparisons.
            if latched_sort_error.is_some() {
                return Ordering::Equal;
            }

            for sort_key in keys {
                let field_a = elem_a.get(sort_key.field.as_str());
                let field_b = elem_b.get(sort_key.field.as_str());

                // Each arm yields the FINAL, direction-applied ordering for this
                // key. Value-vs-value comparisons and `nulls = Greatest` flow
                // through the `order` reversal; `nulls = First`/`Last` are
                // absolute positions independent of `order` (see
                // `null_directed_ordering`). Matching the `Option<&Value>` tuple
                // directly binds the present, non-null values without an `expect`.
                let directed_ordering = match (field_a, field_b) {
                    // Both null/missing — Equal for this key; consult the next key.
                    (None | Some(Value::Null), None | Some(Value::Null)) => Ordering::Equal,

                    // Only a is null/missing.
                    (None | Some(Value::Null), _) => null_directed_ordering(sort_key, true),

                    // Only b is null/missing.
                    (_, None | Some(Value::Null)) => null_directed_ordering(sort_key, false),

                    // Both present and non-null — compare (case-insensitive for
                    // strings when requested), then apply the direction.
                    (Some(val_a), Some(val_b)) => {
                        let base = match compare_values(val_a, val_b, sort_key.case_insensitive) {
                            Ok(ord) => ord,
                            Err(err) => {
                                latched_sort_error = Some(err);
                                return Ordering::Equal;
                            },
                        };
                        match sort_key.order {
                            SortOrder::Asc => base,
                            SortOrder::Desc => base.reverse(),
                        }
                    },
                };

                // If this key produced a non-Equal result, we are done.
                if directed_ordering != Ordering::Equal {
                    return directed_ordering;
                }
                // Otherwise fall through to the next key.
            }

            // All keys produced Equal — preserve input order (stable sort).
            Ordering::Equal
        });

        // Propagate any error that was latched during the sort.
        if let Some(sort_error) = latched_sort_error {
            return Err(sort_error);
        }

        Ok(ActionResult::success(Value::Array(elements)))
    }
}

// ── Comparison helpers ──────────────────────────────────────────────────────────

/// The final, direction-applied ordering of `a` vs `b` for a key where exactly
/// one side is null/missing. `a_is_null` selects which side.
///
/// `Greatest` treats null as the greatest value, so it participates in the
/// direction (last in `asc`, first in `desc`). `First`/`Last` are absolute and
/// ignore `order`.
fn null_directed_ordering(key: &SortKey, a_is_null: bool) -> Ordering {
    match key.nulls {
        NullsOrder::Greatest => {
            let base = if a_is_null {
                Ordering::Greater
            } else {
                Ordering::Less
            };
            match key.order {
                SortOrder::Asc => base,
                SortOrder::Desc => base.reverse(),
            }
        },
        NullsOrder::First => {
            if a_is_null {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        },
        NullsOrder::Last => {
            if a_is_null {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        },
    }
}

/// Compare two non-null values, optionally case-insensitively for strings.
///
/// Case-insensitive comparison is Unicode-aware and allocation-free: it folds
/// each side to lowercase lazily (`char::to_lowercase`) and compares the
/// resulting char streams. Non-string values ignore `case_insensitive` and fall
/// through to [`compare_ordered`].
fn compare_values(a: &Value, b: &Value, case_insensitive: bool) -> Result<Ordering, ActionError> {
    if case_insensitive && let (Value::String(lhs), Value::String(rhs)) = (a, b) {
        return Ok(lhs
            .chars()
            .flat_map(char::to_lowercase)
            .cmp(rhs.chars().flat_map(char::to_lowercase)));
    }
    compare_ordered(a, b)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        use nebula_action::action::Action;
        assert_eq!(Sort::metadata().base.key.as_str(), "core.sort");
    }
}
