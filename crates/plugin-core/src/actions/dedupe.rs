//! `core.dedupe` — remove duplicate elements from a JSON array of objects,
//! keyed by one or more fields.
//!
//! Iterates the input array in order and keeps the **first** element for each
//! unique key-tuple; later elements with the same key-tuple are dropped. The
//! original order of kept elements is preserved.
//!
//! This fills a gap in the `{{ }}` expression language: `array.unique` dedupes
//! by whole-value equality and cannot dedupe by a named field subset. An empty
//! `keys` list that would replicate `array.unique` is rejected as redundant.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": [
//!     { "id": 1, "value": "a" },
//!     { "id": 2, "value": "b" },
//!     { "id": 1, "value": "c" }
//!   ],
//!   "keys": ["id"]
//! }
//! ```
//!
//! ## Output
//!
//! ```json
//! [
//!   { "id": 1, "value": "a" },
//!   { "id": 2, "value": "b" }
//! ]
//! ```
//!
//! (First occurrence of `id=1` is kept; the later duplicate is dropped.)
//!
//! ## Error semantics
//!
//! - `data` absent / null / non-array → **Fatal**.
//! - `keys` empty → **Fatal** (with a pointer to `array.unique` for whole-value
//!   dedup).
//! - Any array element that is not a JSON object → **Fatal** (explicit
//!   `is_object()` guard — `Value::get` on a non-object returns `None`
//!   silently, which would cause key reads to misfire).
//! - A `keys` field **absent** on an element → **Fatal** (cannot determine
//!   identity without the key; consistent with `core.aggregate`'s group_by
//!   missing-key rule). A `null` key value is allowed — null is a valid
//!   identity component.
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::collections::HashSet;
use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::{HasSchema, Schema, ValidSchema, ValidationReport, field_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::util::ValueTypeNameStr;

// ── Input types ───────────────────────────────────────────────────────────────

/// Input for `core.dedupe`.
///
/// `data` must be a JSON array of objects when present. `null` / absent values
/// are rejected with a Fatal error — deduping a non-array is always an
/// authoring mistake.
///
/// ## Wire shape
///
/// ```json
/// {
///   "data": [ { "id": 1, "v": "a" }, { "id": 1, "v": "b" } ],
///   "keys": ["id"]
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupeInput {
    /// Array of JSON objects to deduplicate. Must be a JSON array when present.
    #[serde(default)]
    pub data: Option<Value>,
    /// Property names whose value-tuple defines element identity. At least one
    /// required; empty `keys` is rejected — use `array.unique` for
    /// whole-value dedup.
    pub keys: Vec<String>,
}

impl HasSchema for DedupeInput {
    #[instrument(name = "core.dedupe.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA
            .get_or_init(|| {
                Schema::builder()
                    .property(super::input_schema::record_data())
                    .property(super::input_schema::strings(field_key!("keys")).required())
                    .root_rule(super::input_schema::array_present(field_key!("data"))?)
                    .build()
            })
            .clone()
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that removes duplicate elements from a JSON array of objects
/// by one or more key fields. First occurrence wins; original order preserved.
///
/// Keyed `core.dedupe`. No I/O, no credentials, no resources.
///
/// ## Example wire input / output
///
/// ```json
/// {
///   "data": [
///     { "user": "alice", "event": "login"  },
///     { "user": "bob",   "event": "login"  },
///     { "user": "alice", "event": "logout" }
///   ],
///   "keys": ["user"]
/// }
/// ```
///
/// Output:
/// ```json
/// [
///   { "user": "alice", "event": "login" },
///   { "user": "bob",   "event": "login" }
/// ]
/// ```
#[derive(Debug)]
pub struct Dedupe;

impl nebula_action::action::Action for Dedupe {
    type Input = DedupeInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.dedupe"),
            nebula_action::metadata_name!("Dedupe"),
            "Remove duplicate array elements by one or more key fields (first occurrence wins)",
        )
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for Dedupe {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Dedupe)
    }
}

impl StatelessAction for Dedupe {
    #[instrument(name = "core.dedupe", skip_all, fields(element_count))]
    async fn execute(
        &self,
        input: DedupeInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // ── 1. Validate data ──────────────────────────────────────────────────
        let elements: Vec<Value> = match input.data {
            Some(Value::Array(arr)) => arr,
            Some(Value::Null) | None => {
                return Err(ActionError::fatal(
                    "dedupe: `data` must be a JSON array, got null",
                ));
            },
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "dedupe: `data` must be a JSON array, got {}",
                    other.type_name_str()
                )));
            },
        };

        tracing::Span::current().record("element_count", elements.len());

        // ── 2. Validate keys non-empty ────────────────────────────────────────
        if input.keys.is_empty() {
            return Err(ActionError::fatal(
                "dedupe: at least one key field is required \
                 (use the array.unique expression for whole-value dedup)",
            ));
        }

        // ── 3. Iterate in order; keep first-seen key-tuples ───────────────────
        //
        // `seen_key_tuples` tracks serialized identity tuples so duplicates are
        // detected in O(1) per element. serde_json's `Value::Number` stores
        // integer and float representations separately, so serialization keeps
        // `1`, `"1"`, and `1.0` distinct. Object key-fields serialize with their
        // keys in sorted order (serde_json's default `BTreeMap`-backed `Map`; the
        // `preserve_order` feature is not enabled), so `{"a":1,"b":2}` and
        // `{"b":2,"a":1}` are the SAME identity; array key-fields keep element
        // order, so `[1,2]` and `[2,1]` are DISTINCT identities.
        let mut seen_key_tuples: HashSet<String> = HashSet::new();
        let mut kept_elements: Vec<Value> = Vec::new();

        for element in elements {
            // Guard: every element must be a JSON object.
            // `Value::get` on a non-object returns `None` silently, so key reads
            // would misfire without this explicit check.
            if !element.is_object() {
                return Err(ActionError::fatal(format!(
                    "dedupe: every array element must be a JSON object, got {}",
                    element.type_name_str()
                )));
            }

            // Build the canonical identity tuple for this element.
            //
            // A field that is ABSENT is Fatal (can't determine identity).
            // A field that is present but has a `null` value is allowed —
            // null is a valid identity component.
            let mut key_values: Vec<Value> = Vec::with_capacity(input.keys.len());
            for key_field in &input.keys {
                match element.get(key_field.as_str()) {
                    Some(field_value) => key_values.push(field_value.clone()),
                    None => {
                        return Err(ActionError::fatal(format!(
                            "dedupe: key field `{key_field}` missing on an element"
                        )));
                    },
                }
            }

            // Serialize the key-tuple to a canonical string for HashSet membership.
            // `key_values` contains only cloned JSON Values — serialization should
            // not fail, but we propagate any error rather than panic.
            let serialized_tuple = serde_json::to_string(&Value::Array(key_values))
                .map_err(|e| ActionError::fatal(format!("dedupe: failed to serialize key: {e}")))?;

            // First-occurrence wins: insert returns false when the key was already
            // present, indicating a duplicate that should be dropped.
            if seen_key_tuples.insert(serialized_tuple) {
                kept_elements.push(element);
            }
        }

        Ok(ActionResult::success(Value::Array(kept_elements)))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "dedupe_tests.rs"]
mod tests;
