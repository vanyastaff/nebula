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
                    .property(super::input_schema::record_data())
                    .property(super::input_schema::condition(field_key!("condition")))
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
#[path = "filter_tests.rs"]
mod tests;
