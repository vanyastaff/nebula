//! `core.filter` — filter an array of JSON objects by a `Condition`.
//!
//! Iterates the input array in order, evaluates `condition` against each
//! element, and returns a new array containing only the elements for which
//! the condition holds. Order is preserved.
//!
//! This fills the predicate-filter gap in the `{{ }}` expression language,
//! whose `filter`/`map`/`reduce` builtins require lambda support that is not
//! yet implemented.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data":      [ { "x": 1 }, { "x": 2 }, { "x": 3 } ],
//!   "condition": { "field": "x", "op": "gt", "value": 1 }
//! }
//! ```
//!
//! ## Output
//!
//! ```json
//! [ { "x": 2 }, { "x": 3 } ]
//! ```
//!
//! ## Error semantics
//!
//! - `data` absent / null → **Fatal** (filtering a non-array is an authoring
//!   error; there is no empty-array default for the input).
//! - `data` present but not a JSON array → **Fatal** naming the actual type.
//! - Any array element that is not a JSON object → **Fatal**, enforced by an
//!   explicit `is_object()` guard before `evaluate_condition` is called.
//!   This is uniform across ALL operators: `Value::get` on a non-object returns
//!   `None` rather than an error, so operators like `Ne`/`NotExists` would
//!   silently include a non-object element without the guard.
//! - Empty array input → output `[]` (valid; not an error).
//! - Empty result (no elements match) → output `[]` (valid; not an error).
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::{HasSchema, Schema, ValidSchema, ValidationReport, field_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::condition::{Condition, evaluate_condition};
use crate::util::ValueTypeNameStr;

// ── Input ─────────────────────────────────────────────────────────────────────

/// Input for `core.filter`.
///
/// `data` must be a JSON array when present. `null` / absent values are
/// rejected with a Fatal error — there is no default empty array, because
/// filtering a non-array is always an authoring mistake.
///
/// ## Wire shape
///
/// ```json
/// {
///   "data":      [ { "status": "active", "score": 10 } ],
///   "condition": { "field": "status", "op": "eq", "value": "active" }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterInput {
    /// Array of JSON objects to filter. Must be a JSON array when present.
    #[serde(default)]
    pub data: Option<Value>,
    /// Predicate applied to each element. Supports leaf predicates and
    /// `all` / `any` / `not` combinators — see `crate::condition::Condition`.
    pub condition: Condition,
}

impl HasSchema for FilterInput {
    #[instrument(name = "core.filter.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA
            .get_or_init(|| {
                Schema::builder()
                    .add(super::input_schema::record_data())
                    .add(super::input_schema::condition(field_key!("condition")))
                    .root_rule(super::input_schema::array_present(field_key!("data"))?)
                    .build()
            })
            .clone()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that filters a JSON array of objects by a `Condition`.
///
/// Keyed `core.filter`. No I/O, no credentials, no resources.
///
/// ## Example wire input / output
///
/// ```json
/// {
///   "data": [
///     { "role": "admin",  "active": true },
///     { "role": "viewer", "active": true },
///     { "role": "admin",  "active": false }
///   ],
///   "condition": { "all": [
///     { "field": "role",   "op": "eq",     "value": "admin" },
///     { "field": "active", "op": "truthy"                   }
///   ] }
/// }
/// ```
///
/// Output: `[ { "role": "admin", "active": true } ]`
#[derive(Debug)]
pub struct Filter;

impl nebula_action::action::Action for Filter {
    type Input = FilterInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.filter"),
            nebula_action::metadata_name!("Filter"),
            "Filter an array of JSON objects by a condition",
        )
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for Filter {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Filter)
    }
}

impl StatelessAction for Filter {
    #[instrument(name = "core.filter", skip_all, fields(element_count))]
    async fn execute(
        &self,
        input: FilterInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        let elements: Vec<Value> = match input.data {
            Some(Value::Array(arr)) => arr,
            Some(Value::Null) | None => {
                return Err(ActionError::fatal(
                    "filter: `data` must be a JSON array, got null",
                ));
            },
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "filter: `data` must be a JSON array, got {}",
                    other.type_name_str()
                )));
            },
        };

        // Record element count now that we have the slice.
        tracing::Span::current().record("element_count", elements.len());

        // Output is a subset of the input; Vec::new() avoids over-allocating
        // for the common case where only a fraction of elements match.
        let mut matching_elements: Vec<Value> = Vec::new();

        for element in elements {
            // Guard: every element must be a JSON object.
            //
            // `Value::get` on a non-object returns `None`, NOT an error, so
            // operators like `Ne`/`NotExists` would silently INCLUDE a
            // non-object element if we delegated the check to `evaluate_condition`.
            // The explicit guard makes the rejection uniform across all operators.
            if !element.is_object() {
                return Err(ActionError::fatal(format!(
                    "filter: every array element must be a JSON object, got {}",
                    element.type_name_str()
                )));
            }
            if evaluate_condition(&element, &input.condition)? {
                matching_elements.push(element);
            }
        }

        Ok(ActionResult::success(Value::Array(matching_elements)))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::future::Future;

    use nebula_action::testing::TestContextBuilder;
    use nebula_action::{ActionError, ActionResult, StatelessAction};
    use serde_json::{Value, json};

    use crate::condition::{Condition, ConditionOp};

    use super::{Filter, FilterInput};

    fn run(input: FilterInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
        let action = Filter;
        let ctx = TestContextBuilder::new().build();
        async move { action.execute(input, &ctx).await }
    }

    fn extract_output(result: ActionResult<Value>) -> Value {
        result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .expect("ActionResult must carry a primary output value")
    }

    fn leaf_gt(field: &str, threshold: i64) -> Condition {
        Condition::Leaf {
            field: field.into(),
            op: ConditionOp::Gt,
            value: Some(json!(threshold)),
        }
    }

    fn leaf_eq(field: &str, val: &str) -> Condition {
        Condition::Leaf {
            field: field.into(),
            op: ConditionOp::Eq,
            value: Some(json!(val)),
        }
    }

    fn leaf_exists(field: &str) -> Condition {
        Condition::Leaf {
            field: field.into(),
            op: ConditionOp::Exists,
            value: None,
        }
    }

    // ── 1: non-array data is Fatal ────────────────────────────────────────────
    //
    // RED witness: without the `Some(other) => Err(Fatal)` arm the input object
    // would not be rejected, so no Err is returned and `unwrap_err()` panics.
    #[tokio::test]
    async fn non_array_data_is_fatal() {
        let input = FilterInput {
            data: Some(json!({"x": 1})), // object, not an array
            condition: leaf_gt("x", 0),
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for object data; got: {err:?}"
        );
    }

    // ── 2: non-object element is Fatal regardless of operator ────────────────
    //
    // An explicit `is_object()` guard fires BEFORE `evaluate_condition`.
    // `Value::get` on a non-object returns `None`, so without the guard
    // operators like `Ne`/`NotExists` would silently INCLUDE the non-object
    // element instead of erroring. The guard makes rejection uniform.
    //
    // Uses `Eq` (a non-ordered operator) so the test cannot pass by accident
    // via the ordered-comparison path in `evaluate_condition`.
    //
    // RED witness: without the `is_object()` guard, `Eq("x","y")` on a number
    // element returns `Ok(false)` (field missing → not equal), causing the loop
    // to skip the element silently — no Err is returned and `unwrap_err()` panics.
    #[tokio::test]
    async fn non_object_element_is_fatal_regardless_of_operator() {
        let input = FilterInput {
            data: Some(json!([1, 2])), // numbers are not JSON objects
            condition: leaf_eq("x", "y"),
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for non-object element with Eq operator; got: {err:?}"
        );
    }

    // ── 2b: silent-include regression — NotExists on non-object is Fatal ──────
    //
    // WITHOUT the guard, `NotExists("missing")` on the number `42` returns
    // `Ok(true)` (field is absent → not exists), so `42` leaks into the output:
    // `Ok([{"ok":1}, 42])`. WITH the guard the number triggers Fatal before
    // the condition is evaluated.
    //
    // RED witness: remove the `is_object()` guard and this test returns
    // `Ok([{"ok":1}, 42])` — a non-object value in the filtered output.
    #[tokio::test]
    async fn non_object_element_with_not_exists_is_fatal_not_silently_included() {
        let condition = Condition::Leaf {
            field: "missing".into(),
            op: ConditionOp::NotExists,
            value: None,
        };
        let input = FilterInput {
            data: Some(json!([{"ok": 1}, 42])),
            condition,
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for non-object element under NotExists; got: {err:?}"
        );
    }

    // ── 3: matching subset returned in original order ─────────────────────────
    //
    // RED witness: without the impl, `run(input).await` returns Err so
    // `unwrap()` panics; without order preservation the full-array assert fails.
    #[tokio::test]
    async fn filter_selects_matching_subset() {
        let input = FilterInput {
            data: Some(json!([{"x": 1}, {"x": 2}, {"x": 3}])),
            condition: leaf_gt("x", 1),
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"x": 2}, {"x": 3}]),
            "filter must return only elements where x > 1, in original order"
        );
    }

    // ── 3b: order is original (non-ascending input) ───────────────────────────
    //
    // Test 3 (`filter_selects_matching_subset`) only checks an ascending input,
    // which a sort-then-filter impl would also satisfy. This test uses a
    // non-ascending input so the expected output cannot be produced by sorting.
    //
    // Input:  [{x:3},{x:1},{x:2}]  (3 comes before 2)
    // Filter: x > 1  → keeps {x:3} and {x:2}
    // Expected: [{x:3},{x:2}]  — 3 before 2, original order, NOT sorted.
    //
    // RED witness: a sort-then-filter impl would return [{x:2},{x:3}], which
    // does not equal [{x:3},{x:2}] and causes the assert to fail.
    #[tokio::test]
    async fn filter_preserves_original_order() {
        let input = FilterInput {
            data: Some(json!([{"x": 3}, {"x": 1}, {"x": 2}])),
            condition: leaf_gt("x", 1),
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"x": 3}, {"x": 2}]),
            "filter must preserve original element order (3 before 2, not sorted)"
        );
    }

    // ── 4: no match → empty array, not Fatal ─────────────────────────────────
    #[tokio::test]
    async fn filter_empty_result_is_empty_array() {
        let input = FilterInput {
            data: Some(json!([{"x": 0}, {"x": 1}])),
            condition: leaf_gt("x", 99),
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([]),
            "empty result must be an empty array, not Fatal"
        );
    }

    // ── 5: combinator condition (All) correctly prunes the array ─────────────
    //
    // Proves that recursive `Condition` reuse works end-to-end through Filter.
    #[tokio::test]
    async fn filter_with_combinator_condition() {
        // Keep elements where role == "admin" AND active field exists.
        let condition = Condition::All(vec![leaf_eq("role", "admin"), leaf_exists("active")]);
        let input = FilterInput {
            data: Some(json!([
                {"role": "admin",  "active": true},
                {"role": "viewer", "active": true},
                {"role": "admin"}                   // no "active" field → All fails
            ])),
            condition,
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([{"role": "admin", "active": true}]),
            "All combinator must keep only elements satisfying BOTH sub-conditions"
        );
    }

    // ── 6: empty input array → output [] (not Fatal) ─────────────────────────
    #[tokio::test]
    async fn filter_empty_input_array_returns_empty_array() {
        let input = FilterInput {
            data: Some(json!([])),
            condition: leaf_gt("x", 0),
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(
            out,
            json!([]),
            "empty input array must yield empty output array"
        );
    }

    // ── 7: null data is Fatal (no default empty-array) ───────────────────────
    #[tokio::test]
    async fn null_data_is_fatal() {
        let input = FilterInput {
            data: Some(json!(null)),
            condition: leaf_gt("x", 0),
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for null data; got: {err:?}"
        );
    }

    // ── 8: absent data is Fatal (no default empty-array) ─────────────────────
    #[tokio::test]
    async fn absent_data_is_fatal() {
        let input = FilterInput {
            data: None,
            condition: leaf_gt("x", 0),
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for absent data; got: {err:?}"
        );
    }

    // ── 9: action key is "core.filter" ───────────────────────────────────────
    #[test]
    fn action_key_is_core_dot_filter() {
        let factory = nebula_action::GenericStatelessFactory::<Filter>::new()
            .expect("filter metadata must admit");
        assert_eq!(
            nebula_action::ActionFactory::metadata(&factory)
                .base()
                .key()
                .as_str(),
            "core.filter"
        );
    }
}
