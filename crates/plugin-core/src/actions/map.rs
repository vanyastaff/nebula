//! `core.map` — reshape every element of a JSON array of objects.
//!
//! Applies a sequence of [`TransformOperation`]s to each element in order,
//! returning a new array of the same length with each element reshaped.
//! This fills the gap left by the `{{ }}` expression language, whose
//! `array.map` builtin requires lambda support that is not yet implemented.
//!
//! ## Scope
//!
//! The operation vocabulary is identical to `core.json_transform` — `pick`,
//! `omit`, `rename`, and `flatten` — applied to each element. `pick`/`omit`/
//! `rename` act on the element's top-level keys; `flatten` collapses an element's
//! nested objects into dotted top-level keys. See [`TransformOperation`] for
//! per-operation semantics.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": [
//!     { "first_name": "Alice", "last": "Smith",  "secret": "x" },
//!     { "first_name": "Bob",   "last": "Jones",  "secret": "y" }
//!   ],
//!   "operations": [
//!     { "op": "omit",   "fields": ["secret"] },
//!     { "op": "rename", "from": "first_name", "to": "name" }
//!   ]
//! }
//! ```
//!
//! ## Output
//!
//! ```json
//! [
//!   { "last": "Smith", "name": "Alice" },
//!   { "last": "Jones", "name": "Bob"   }
//! ]
//! ```
//!
//! ## Error semantics
//!
//! - `data` absent / null / non-array → **Fatal** naming the actual type.
//! - `operations` empty → **Fatal** (a no-op map is always an authoring mistake;
//!   consistent with the other sibling array nodes that require their config).
//! - Any array element that is not a JSON object → **Fatal** (explicit
//!   `is_object()` guard; `Value::get` on a non-object returns `None` silently,
//!   which would produce wrong results without the guard).
//! - `Rename` source key absent on an element → **Fatal** (propagated from the
//!   shared `apply_operations` contract; identical to `core.json_transform`).
//! - `Pick` / `Omit` missing keys → silent skip (per `apply_operations` contract).
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::{HasSchema, Schema, ValidSchema, ValidationReport, field_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::actions::json_transform::{TransformOperation, apply_operations};
use crate::util::ValueTypeNameStr;

// ── Input ─────────────────────────────────────────────────────────────────────

/// Input for `core.map`.
///
/// `data` must be a JSON array of objects. `null` / absent values are rejected
/// with a Fatal error — mapping over a non-array is always an authoring mistake.
///
/// ## Wire shape
///
/// ```json
/// {
///   "data": [
///     { "a": 1, "b": 2 },
///     { "a": 3, "b": 4 }
///   ],
///   "operations": [
///     { "op": "pick", "fields": ["a"] }
///   ]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapInput {
    /// Array of JSON objects to reshape. Must be a JSON array when present.
    #[serde(default)]
    pub data: Option<Value>,
    /// Ordered list of transform operations applied left-to-right to each element.
    pub operations: Vec<TransformOperation>,
}

impl HasSchema for MapInput {
    #[instrument(name = "core.map.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA
            .get_or_init(|| {
                Schema::builder()
                    .property(super::input_schema::record_data())
                    .property(
                        super::json_transform::operations_schema()
                            .description("Required operation array; an empty array is a no-op."),
                    )
                    .root_rule(super::input_schema::array_present(field_key!("data"))?)
                    .root_rule(super::input_schema::array_present(field_key!(
                        "operations"
                    ))?)
                    .build()
            })
            .clone()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that reshapes every element of a JSON array of objects by
/// applying a sequence of `pick`/`omit`/`rename` operations to each one.
///
/// Keyed `core.map`. Count-preserving: N elements in → N elements out, original
/// order maintained. No I/O, no credentials, no resources.
///
/// ## Example wire input / output
///
/// ```json
/// {
///   "data": [
///     { "id": 1, "name": "Alice", "secret": "x" },
///     { "id": 2, "name": "Bob",   "secret": "y" }
///   ],
///   "operations": [
///     { "op": "omit",   "fields": ["secret"] },
///     { "op": "rename", "from": "name", "to": "label" }
///   ]
/// }
/// ```
///
/// Output:
///
/// ```json
/// [
///   { "id": 1, "label": "Alice" },
///   { "id": 2, "label": "Bob"   }
/// ]
/// ```
#[derive(Debug)]
pub struct MapAction;

impl nebula_action::action::Action for MapAction {
    type Input = MapInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.map"),
            nebula_action::metadata_name!("Map"),
            "Reshape each element of a JSON array of objects (per-element \
             pick/omit/rename/flatten)",
        )
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for MapAction {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(MapAction)
    }
}

impl StatelessAction for MapAction {
    #[instrument(name = "core.map", skip_all, fields(element_count))]
    async fn execute(
        &self,
        input: MapInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // ── 1. Validate data ──────────────────────────────────────────────────
        let elements: Vec<Value> = match input.data {
            Some(Value::Array(arr)) => arr,
            Some(Value::Null) | None => {
                return Err(ActionError::fatal(
                    "map: `data` must be a JSON array, got null",
                ));
            },
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "map: `data` must be a JSON array, got {}",
                    other.type_name_str()
                )));
            },
        };

        tracing::Span::current().record("element_count", elements.len());

        // ── 2. Validate operations non-empty ──────────────────────────────────
        //
        // A map with zero operations leaves every element unchanged, which is
        // identical to copying the array. This is always an authoring mistake
        // (use the input directly), so we fail-fast here — consistent with
        // the sibling array nodes (filter/aggregate/sort/dedupe all require
        // their config fields to be non-trivial).
        if input.operations.is_empty() {
            return Err(ActionError::fatal(
                "map: at least one operation is required",
            ));
        }

        // ── 3. Apply operations to each element ───────────────────────────────
        //
        // Count-preserving: the output vec is pre-allocated to exactly the
        // same capacity as the input. Each element is consumed from `elements`
        // so we never clone the original values.
        let mut reshaped: Vec<Value> = Vec::with_capacity(elements.len());

        for element in elements {
            // Every element must be a JSON object; destructure to its inner Map
            // for in-place mutation. `Value::get` on a non-object is silently
            // None, so Pick/Omit would pass a non-object through and Rename would
            // fire a misleading "key not found" — validating and extracting in one
            // step keeps rejection uniform across all operations (no dead branch).
            let Value::Object(mut fields) = element else {
                return Err(ActionError::fatal(format!(
                    "map: every array element must be a JSON object, got {}",
                    element.type_name_str()
                )));
            };

            apply_operations(&mut fields, &input.operations, "map")?;
            reshaped.push(Value::Object(fields));
        }

        Ok(ActionResult::success(Value::Array(reshaped)))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "map_tests.rs"]
mod tests;
