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

use nebula_action::{ActionContext, ActionError, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::{HasSchema, Property, Schema, ValidSchema, ValidationReport, field_key};
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

impl HasSchema for SortInput {
    #[instrument(name = "core.sort.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA.get_or_init(|| {
            Schema::builder()
                .property(super::input_schema::record_data())
                .property(Property::list(field_key!("keys")).required().item(
                    Property::object(field_key!("item"))
                        .description("Sort key; serde requires field while allowing the empty JSON key.")
                        .property(Property::string(field_key!("field")))
                        .property(Property::select(field_key!("order"))
                            .option("asc", "Ascending").option("desc", "Descending"))
                        .property(Property::select(field_key!("nulls"))
                            .option("greatest", "Greatest").option("first", "First").option("last", "Last"))
                        .property(Property::boolean(field_key!("case_insensitive"))),
                ))
                .root_rule(super::input_schema::array_present(field_key!("data"))?)
                .build()
        }).clone()
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

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.sort"),
            nebula_action::metadata_name!("Sort"),
            "Sort an array of objects by one or more fields (asc/desc)",
        )
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
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
#[path = "sort_tests.rs"]
mod tests;
