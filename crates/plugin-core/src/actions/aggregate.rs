//! `core.aggregate` — reduce a JSON array of objects to grouped summaries.
//!
//! Iterates the input array, optionally partitions elements into groups by
//! one or more field keys, and computes one or more aggregation functions
//! per group. The output is always a JSON array of summary objects — one row
//! per group (or one row when no grouping is requested).
//!
//! This fills the aggregation gap in the `{{ }}` expression language, which
//! has no live array aggregators (`sum`/`avg`/`count`/`min`/`max` over an
//! array) and whose `group_by`/`reduce` builtins require lambda support that
//! is not yet implemented.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": [
//!     { "region": "west", "amount": 10 },
//!     { "region": "east", "amount": 20 },
//!     { "region": "west", "amount": 30 }
//!   ],
//!   "group_by":     ["region"],
//!   "aggregations": [
//!     { "fn": "count", "out": "n" },
//!     { "fn": "sum",   "field": "amount", "out": "total" }
//!   ]
//! }
//! ```
//!
//! ## Output
//!
//! ```json
//! [
//!   { "region": "west", "n": 2, "total": 40 },
//!   { "region": "east", "n": 1, "total": 20 }
//! ]
//! ```
//!
//! (Row order = first-seen group order, not sorted.)
//!
//! ## Error semantics
//!
//! - `data` absent / null / non-array → **Fatal**.
//! - Any array element that is not a JSON object → **Fatal** (explicit
//!   `is_object()` guard — `Value::get` on a non-object returns `None`
//!   silently, which would cause group-key and field reads to misfire).
//! - `aggregations` empty → **Fatal** (authoring error: nothing to compute).
//! - Duplicate `out` key across aggregations → **Fatal** (authoring error).
//! - Any aggregation `out` key that matches a `group_by` field name → **Fatal**
//!   (would silently overwrite the group key with the aggregation result).
//! - `group_by` field absent on any element → **Fatal** (cannot determine
//!   the group; fail-closed is safer than treating absent as a synthetic key).
//! - `on_error: fail` (default) and a numeric aggregation encounters a
//!   missing, null, or non-numeric field value → **Fatal**.  A `Skip` policy
//!   ignores that single value instead.
//! - `count` always counts the row regardless of `on_error`.
//! - `count_distinct` and `collect` silently skip null/missing values.
//! - `join` silently skips null/missing; a non-null, non-string value is a dirty
//!   value subject to `on_error` (Fatal under `fail`, skipped under `skip`).
//!
//! The action is **pure** — no I/O, no credentials, no resources.

use std::collections::HashMap;
use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::HasSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use tracing::instrument;

use crate::condition::compare_ordered;
use crate::util::ValueTypeNameStr;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a `Value` to `f64` only when it is already a JSON number.
///
/// Returns `None` for every non-number variant (including strings), so callers
/// can treat a `None` as a "dirty value" subject to the `on_error` policy.
/// String-to-number coercion is intentionally absent: it would be a hidden
/// type mutation invisible to the workflow author.
pub(crate) fn as_f64_strict(v: &Value) -> Option<f64> {
    // `serde_json::Value::as_f64` only succeeds for `Value::Number`.
    v.as_f64()
}

// ── Input types ───────────────────────────────────────────────────────────────

/// Input for `core.aggregate`.
///
/// `data` must be a JSON array of objects. `null` / absent values are
/// rejected with a Fatal error — there is no default empty array, because
/// aggregating a non-array is always an authoring mistake.
///
/// ## Wire shape
///
/// ```json
/// {
///   "data":         [ { "region": "west", "amount": 10 } ],
///   "group_by":     ["region"],
///   "aggregations": [{ "fn": "sum", "field": "amount", "out": "total" }],
///   "on_error":     "fail"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateInput {
    /// Array of JSON objects to aggregate. Must be a JSON array when present.
    #[serde(default)]
    pub data: Option<Value>,
    /// Zero or more field names to group by. An empty list produces one global
    /// summary row. Each named field must be present on every element.
    #[serde(default)]
    pub group_by: Vec<String>,
    /// One or more aggregation functions to apply per group. Must be non-empty.
    pub aggregations: Vec<Aggregation>,
    /// How to handle missing, null, or non-numeric values for numeric
    /// aggregations (`sum`/`avg`/`min`/`max`). Default: `Fail`.
    #[serde(default)]
    pub on_error: OnError,
}

/// Behavior when a numeric aggregation (`sum`/`avg`/`min`/`max`) encounters a
/// missing, null, or non-numeric field value.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    /// Treat the dirty value as a Fatal error (default).
    #[default]
    Fail,
    /// Silently ignore the dirty value and continue.
    Skip,
}

/// A single aggregation function to apply per group.
///
/// Each variant writes exactly one key (the `out` field) into every summary
/// row. The `fn` tag selects the function; other fields depend on the variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "fn", rename_all = "snake_case")]
pub enum Aggregation {
    /// Count every row in the group (COUNT(*) semantics; ignores `on_error`).
    Count {
        /// Output key written into each summary row.
        out: String,
    },
    /// Count distinct non-null values of `field` in the group.
    /// Null/missing values are silently skipped.
    CountDistinct {
        /// Source field whose distinct values are counted.
        field: String,
        /// Output key written into each summary row.
        out: String,
    },
    /// Sum numeric values of `field` across the group.
    /// Integer precision is preserved when all values are integers;
    /// overflow → Fatal. Mixed int+float → returns f64.
    Sum {
        /// Source field whose numeric values are summed.
        field: String,
        /// Output key written into each summary row.
        out: String,
    },
    /// Arithmetic mean of numeric values of `field` across the group.
    /// Always returns `f64`. Returns `null` when the group is empty or
    /// all values are skipped (under `on_error: skip`).
    Avg {
        /// Source field whose numeric values are averaged.
        field: String,
        /// Output key written into each summary row.
        out: String,
    },
    /// Minimum numeric value of `field` across the group.
    /// Returns the actual element value (type preserved). Returns `null`
    /// when the group is empty or all values are skipped.
    Min {
        /// Source field whose minimum numeric value is selected.
        field: String,
        /// Output key written into each summary row.
        out: String,
    },
    /// Maximum numeric value of `field` across the group.
    /// Returns the actual element value (type preserved). Returns `null`
    /// when the group is empty or all values are skipped.
    Max {
        /// Source field whose maximum numeric value is selected.
        field: String,
        /// Output key written into each summary row.
        out: String,
    },
    /// Collect non-null values of `field` across the group into a JSON array.
    /// Null/missing values are silently skipped.
    Collect {
        /// Source field whose non-null values are collected into an array.
        field: String,
        /// Output key written into each summary row.
        out: String,
    },
    /// Join the string values of `field` with `sep` (defaults to `","`).
    /// Null/missing values are silently skipped; a non-null, non-string value
    /// is a dirty value subject to `on_error` (numbers are not coerced).
    Join {
        /// Source field whose string values are joined.
        field: String,
        /// Output key written into each summary row.
        out: String,
        /// Separator inserted between joined values. Defaults to `","`.
        #[serde(default = "default_join_sep")]
        sep: String,
    },
}

fn default_join_sep() -> String {
    ",".to_string()
}

impl Aggregation {
    /// The output key written into each summary row.
    fn out_key(&self) -> &str {
        match self {
            Aggregation::Count { out }
            | Aggregation::CountDistinct { out, .. }
            | Aggregation::Sum { out, .. }
            | Aggregation::Avg { out, .. }
            | Aggregation::Min { out, .. }
            | Aggregation::Max { out, .. }
            | Aggregation::Collect { out, .. }
            | Aggregation::Join { out, .. } => out.as_str(),
        }
    }
}

// `data` is fully dynamic; the module doc describes expected structure.
impl HasSchema for AggregateInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
    }
}

// ── Per-group accumulator ─────────────────────────────────────────────────────

/// Running state for a single aggregation function over a single group.
///
/// Each variant corresponds one-to-one with an `Aggregation` variant.
/// `Accumulator::new` constructs the matching variant; `feed` advances it;
/// `finalize` converts the running state to the output `Value`.
#[derive(Debug)]
enum Accumulator {
    Count {
        row_count: u64,
    },
    CountDistinct {
        distinct_serialized: std::collections::HashSet<String>,
    },
    /// Preserves integer type when all values are integers. Upgrades to
    /// `SumFloat` on the first floating-point value encountered.
    SumInt {
        integer_total: i64,
    },
    SumFloat {
        float_total: f64,
    },
    Avg {
        running_sum: f64,
        contributing_count: u64,
    },
    Min {
        // The original `Value` (not a cached f64): min/max are decided by exact
        // comparison so large 64-bit integers don't collapse via f64.
        current_min: Option<Value>,
    },
    Max {
        current_max: Option<Value>,
    },
    Collect {
        collected_values: Vec<Value>,
    },
    Join {
        joined_parts: Vec<String>,
        separator: String,
    },
}

impl Accumulator {
    fn new(aggregation: &Aggregation) -> Self {
        match aggregation {
            Aggregation::Count { .. } => Accumulator::Count { row_count: 0 },
            Aggregation::CountDistinct { .. } => Accumulator::CountDistinct {
                distinct_serialized: std::collections::HashSet::new(),
            },
            Aggregation::Sum { .. } => Accumulator::SumInt { integer_total: 0 },
            Aggregation::Avg { .. } => Accumulator::Avg {
                running_sum: 0.0,
                contributing_count: 0,
            },
            Aggregation::Min { .. } => Accumulator::Min { current_min: None },
            Aggregation::Max { .. } => Accumulator::Max { current_max: None },
            Aggregation::Collect { .. } => Accumulator::Collect {
                collected_values: vec![],
            },
            Aggregation::Join { sep, .. } => Accumulator::Join {
                joined_parts: vec![],
                separator: sep.clone(),
            },
        }
    }

    /// Advance the accumulator with one element from the input array.
    ///
    /// Returns `Err(ActionError::Fatal)` when the field value is dirty (missing,
    /// null, or non-numeric) and `dirty_value_policy` is `Fail`.
    fn feed(
        &mut self,
        element: &Value,
        aggregation: &Aggregation,
        dirty_value_policy: OnError,
    ) -> Result<(), ActionError> {
        match (self, aggregation) {
            // COUNT(*) — always increments; not subject to dirty-value policy.
            (Accumulator::Count { row_count }, Aggregation::Count { .. }) => {
                *row_count += 1;
            },

            // COUNT DISTINCT — skip null/missing (documented behavior).
            (
                Accumulator::CountDistinct {
                    distinct_serialized,
                },
                Aggregation::CountDistinct { field, .. },
            ) => {
                if let Some(field_value) = element.get(field.as_str())
                    && !field_value.is_null()
                {
                    // Serialize to string for set membership; serde_json
                    // produces a canonical representation that distinguishes
                    // types (1 != "1" != 1.0).
                    distinct_serialized.insert(field_value.to_string());
                }
            },

            // SUM (integer path) — preserves i64 when all values are integers;
            // upgrades to SumFloat on the first u64-only or floating-point value.
            //
            // Matching on `v.is_number()` then trying i64 first (covers both i64
            // and all integers ≤ i64::MAX) then falling through to as_f64 (covers
            // u64 > i64::MAX and genuine floats — serde_json's as_f64 returns Some
            // for any Number) avoids any expect/unreachable in library code.
            (acc @ Accumulator::SumInt { .. }, Aggregation::Sum { field, out }) => {
                match element.get(field.as_str()) {
                    Some(v) if v.is_number() => {
                        if let Some(addend) = v.as_i64() {
                            // Fast path: i64-representable integer — stay integer.
                            if let Accumulator::SumInt { integer_total } = acc {
                                *integer_total = integer_total
                                    .checked_add(addend)
                                    .ok_or_else(|| ActionError::fatal("aggregate: sum overflow"))?;
                            }
                        } else if let Some(addend) = v.as_f64() {
                            // Upgrade path: u64 above i64::MAX, or a float literal.
                            // i64 → f64: precision may degrade for very large integers,
                            // but JSON numbers with that many digits are already f64-lossy
                            // at parse time, so no additional precision is lost here.
                            let prior = if let Accumulator::SumInt { integer_total } = &*acc {
                                *integer_total as f64
                            } else {
                                0.0 // unreachable: arm guard `acc @ SumInt` holds
                            };
                            *acc = Accumulator::SumFloat {
                                float_total: prior + addend,
                            };
                        } else {
                            // is_number() true but neither i64 nor f64 representable —
                            // treat as a dirty value (subject to on_error policy).
                            return apply_dirty_value_policy(
                                field,
                                out,
                                v.type_name_str(),
                                dirty_value_policy,
                            );
                        }
                    },
                    Some(v) if v.is_null() => {
                        return apply_dirty_value_policy(field, out, "null", dirty_value_policy);
                    },
                    None => {
                        return apply_dirty_value_policy(field, out, "missing", dirty_value_policy);
                    },
                    Some(v) => {
                        return apply_dirty_value_policy(
                            field,
                            out,
                            v.type_name_str(),
                            dirty_value_policy,
                        );
                    },
                }
            },

            // SUM (float path) — reached after the first float caused an upgrade.
            (Accumulator::SumFloat { float_total }, Aggregation::Sum { field, out }) => {
                match element.get(field.as_str()) {
                    Some(field_value) => match as_f64_strict(field_value) {
                        Some(addend) => *float_total += addend,
                        None if field_value.is_null() => {
                            return apply_dirty_value_policy(
                                field,
                                out,
                                "null",
                                dirty_value_policy,
                            );
                        },
                        None => {
                            return apply_dirty_value_policy(
                                field,
                                out,
                                field_value.type_name_str(),
                                dirty_value_policy,
                            );
                        },
                    },
                    None => {
                        return apply_dirty_value_policy(field, out, "missing", dirty_value_policy);
                    },
                }
            },

            // AVG
            (
                Accumulator::Avg {
                    running_sum,
                    contributing_count,
                },
                Aggregation::Avg { field, out },
            ) => match element.get(field.as_str()) {
                Some(field_value) => match as_f64_strict(field_value) {
                    Some(addend) => {
                        *running_sum += addend;
                        *contributing_count += 1;
                    },
                    None if field_value.is_null() => {
                        return apply_dirty_value_policy(field, out, "null", dirty_value_policy);
                    },
                    None => {
                        return apply_dirty_value_policy(
                            field,
                            out,
                            field_value.type_name_str(),
                            dirty_value_policy,
                        );
                    },
                },
                None => return apply_dirty_value_policy(field, out, "missing", dirty_value_policy),
            },

            // MIN
            (Accumulator::Min { current_min }, Aggregation::Min { field, out }) => {
                match element.get(field.as_str()) {
                    // `as_f64_strict` is the numeric-type guard only; the actual
                    // min is decided by exact comparison so large integers survive.
                    Some(field_value) => match as_f64_strict(field_value) {
                        Some(_) => {
                            let is_new_minimum = match current_min.as_ref() {
                                None => true,
                                Some(prior) => compare_ordered(field_value, prior)?.is_lt(),
                            };
                            if is_new_minimum {
                                *current_min = Some(field_value.clone());
                            }
                        },
                        None if field_value.is_null() => {
                            return apply_dirty_value_policy(
                                field,
                                out,
                                "null",
                                dirty_value_policy,
                            );
                        },
                        None => {
                            return apply_dirty_value_policy(
                                field,
                                out,
                                field_value.type_name_str(),
                                dirty_value_policy,
                            );
                        },
                    },
                    None => {
                        return apply_dirty_value_policy(field, out, "missing", dirty_value_policy);
                    },
                }
            },

            // MAX
            (Accumulator::Max { current_max }, Aggregation::Max { field, out }) => {
                match element.get(field.as_str()) {
                    // `as_f64_strict` is the numeric-type guard only; the actual
                    // max is decided by exact comparison so large integers survive.
                    Some(field_value) => match as_f64_strict(field_value) {
                        Some(_) => {
                            let is_new_maximum = match current_max.as_ref() {
                                None => true,
                                Some(prior) => compare_ordered(field_value, prior)?.is_gt(),
                            };
                            if is_new_maximum {
                                *current_max = Some(field_value.clone());
                            }
                        },
                        None if field_value.is_null() => {
                            return apply_dirty_value_policy(
                                field,
                                out,
                                "null",
                                dirty_value_policy,
                            );
                        },
                        None => {
                            return apply_dirty_value_policy(
                                field,
                                out,
                                field_value.type_name_str(),
                                dirty_value_policy,
                            );
                        },
                    },
                    None => {
                        return apply_dirty_value_policy(field, out, "missing", dirty_value_policy);
                    },
                }
            },

            // COLLECT — skip null/missing (documented).
            (Accumulator::Collect { collected_values }, Aggregation::Collect { field, .. }) => {
                if let Some(field_value) = element.get(field.as_str())
                    && !field_value.is_null()
                {
                    collected_values.push(field_value.clone());
                }
            },

            // JOIN — skip null/missing (documented). A non-null, non-string
            // value is a *dirty value* subject to `on_error` (Fatal under
            // `fail`, skipped under `skip`), exactly like the numeric
            // aggregations — it is NOT silently dropped, and numbers are NOT
            // coerced to strings (that would be a hidden type mutation; see
            // `as_f64_strict`).
            (Accumulator::Join { joined_parts, .. }, Aggregation::Join { field, out, .. }) => {
                match element.get(field.as_str()) {
                    None | Some(Value::Null) => {},
                    Some(Value::String(string_value)) => joined_parts.push(string_value.clone()),
                    Some(other) => {
                        return apply_dirty_value_policy(
                            field,
                            out,
                            other.type_name_str(),
                            dirty_value_policy,
                        );
                    },
                }
            },

            // Every (Accumulator, Aggregation) pair is constructed in
            // `Accumulator::new` to match by variant. A mismatch here means the
            // caller zipped a different aggregation list than was used to build
            // the accumulators — a logic error in the caller, not in user data.
            _ => {
                return Err(ActionError::fatal(
                    "aggregate: internal — accumulator variant does not match aggregation variant; \
                     callers must zip the same aggregation list used in Accumulator::new",
                ));
            },
        }
        Ok(())
    }

    /// Convert the running accumulator state to the final output `Value`.
    ///
    /// Returns `Err(Fatal)` when a float accumulator overflowed to non-finite
    /// during accumulation (e.g. `1e308 + 1e308 = +Infinity`). This matches
    /// the integer path which uses `checked_add` → Fatal on overflow; silent
    /// corruption via a `0` fallback is never acceptable.
    ///
    /// `Number::from_f64` returns `None` exactly for NaN and Infinity, so the
    /// `.ok_or_else(…)?` IS the finiteness guard — no separate `is_finite` call
    /// is needed.
    fn finalize(self) -> Result<Value, ActionError> {
        match self {
            Accumulator::Count { row_count } => Ok(Value::Number(row_count.into())),
            Accumulator::CountDistinct {
                distinct_serialized,
            } => {
                // `usize` fits in `u64` on all supported platforms (max usize ≤ u64::MAX).
                Ok(Value::Number((distinct_serialized.len() as u64).into()))
            },
            Accumulator::SumInt { integer_total } => Ok(Value::Number(integer_total.into())),
            Accumulator::SumFloat { float_total } => {
                // Summing large finite f64 values can overflow to +Infinity at runtime
                // (e.g. 1e308 + 1e308 = inf). `from_f64` returns None for NaN/Inf,
                // so `ok_or_else?` is the finiteness guard.
                Ok(Value::Number(Number::from_f64(float_total).ok_or_else(
                    || ActionError::fatal("aggregate: sum overflow (non-finite result)"),
                )?))
            },
            Accumulator::Avg {
                running_sum,
                contributing_count,
            } => {
                if contributing_count == 0 {
                    return Ok(Value::Null);
                }
                // `running_sum` is f64 and can overflow to +Infinity for very large inputs.
                let mean = running_sum / contributing_count as f64;
                Ok(Value::Number(Number::from_f64(mean).ok_or_else(|| {
                    ActionError::fatal("aggregate: avg overflow (non-finite result)")
                })?))
            },
            Accumulator::Min { current_min } => Ok(current_min.unwrap_or(Value::Null)),
            Accumulator::Max { current_max } => Ok(current_max.unwrap_or(Value::Null)),
            Accumulator::Collect { collected_values } => Ok(Value::Array(collected_values)),
            Accumulator::Join {
                joined_parts,
                separator,
            } => Ok(Value::String(joined_parts.join(&separator))),
        }
    }
}

/// Either return a Fatal error or silently continue, according to `policy`,
/// when a numeric aggregation encounters a dirty field value.
fn apply_dirty_value_policy(
    field_name: &str,
    output_key: &str,
    dirty_reason: &str,
    policy: OnError,
) -> Result<(), ActionError> {
    match policy {
        OnError::Fail => Err(ActionError::fatal(format!(
            "aggregate: {output_key}({field_name}) hit a {dirty_reason} value; \
             set on_error=skip to ignore"
        ))),
        OnError::Skip => Ok(()),
    }
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action that reduces a JSON array of objects to grouped summaries.
///
/// Keyed `core.aggregate`. No I/O, no credentials, no resources.
///
/// ## Example wire input / output
///
/// ```json
/// {
///   "data": [
///     { "dept": "eng",  "salary": 120000 },
///     { "dept": "eng",  "salary": 150000 },
///     { "dept": "mktg", "salary": 90000  }
///   ],
///   "group_by":     ["dept"],
///   "aggregations": [
///     { "fn": "count", "out": "headcount" },
///     { "fn": "avg",   "field": "salary", "out": "avg_salary" }
///   ]
/// }
/// ```
///
/// Output:
/// ```json
/// [
///   { "dept": "eng",  "headcount": 2, "avg_salary": 135000.0 },
///   { "dept": "mktg", "headcount": 1, "avg_salary": 90000.0  }
/// ]
/// ```
#[derive(Debug)]
pub struct Aggregate;

impl nebula_action::action::Action for Aggregate {
    type Input = AggregateInput;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.aggregate"),
            "Aggregate",
            "Reduce an array of objects to grouped/scalar summaries \
             (sum/count/avg/min/max/collect/join)",
        )
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for Aggregate {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(Aggregate)
    }
}

impl StatelessAction for Aggregate {
    #[instrument(name = "core.aggregate", skip_all, fields(element_count))]
    async fn execute(
        &self,
        input: AggregateInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // ── 1. Validate data ──────────────────────────────────────────────────
        let elements: Vec<Value> = match input.data {
            Some(Value::Array(arr)) => arr,
            Some(Value::Null) | None => {
                return Err(ActionError::fatal(
                    "aggregate: `data` must be a JSON array, got null",
                ));
            },
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "aggregate: `data` must be a JSON array, got {}",
                    other.type_name_str()
                )));
            },
        };

        tracing::Span::current().record("element_count", elements.len());

        // ── 2. Validate aggregations non-empty ────────────────────────────────
        if input.aggregations.is_empty() {
            return Err(ActionError::fatal(
                "aggregate: at least one aggregation is required",
            ));
        }

        // ── 3. Reject duplicate `out` keys (authoring error) ─────────────────
        {
            let mut seen_output_keys: HashMap<&str, ()> = HashMap::new();
            for aggregation in &input.aggregations {
                let output_key = aggregation.out_key();
                if seen_output_keys.insert(output_key, ()).is_some() {
                    return Err(ActionError::fatal(format!(
                        "aggregate: duplicate out key `{output_key}` in aggregations"
                    )));
                }
            }
        }

        // ── 3b. Reject aggregation `out` keys that collide with group_by fields ─
        //
        // A collision silently overwrites the group key in the output row with
        // the aggregation result. Fail-closed is the correct policy here: the
        // author almost certainly made a naming mistake.
        for aggregation in &input.aggregations {
            let output_key = aggregation.out_key();
            if input.group_by.iter().any(|f| f == output_key) {
                return Err(ActionError::fatal(format!(
                    "aggregate: aggregation output `{output_key}` collides with a group_by field"
                )));
            }
        }

        // ── 4. Partition elements into groups and accumulate ──────────────────
        //
        // `group_insertion_order` tracks first-seen group keys so output rows
        // are in deterministic first-seen order, not HashMap iteration order.
        // `group_accumulators` maps serialized group key → per-aggregation state.
        //
        // When `group_by` is empty, all elements land in one global group keyed
        // by the canonical empty-array string "[]".

        // A serialized JSON array of the group-by field values, used as a
        // HashMap key. Canonical serde_json serialization preserves type
        // distinctions (1 ≠ "1").
        type SerializedGroupKey = String;

        let ungrouped_key: SerializedGroupKey = "[]".to_owned();

        let mut group_insertion_order: Vec<SerializedGroupKey> = Vec::new();
        let mut group_accumulators: HashMap<SerializedGroupKey, Vec<Accumulator>> = HashMap::new();

        if elements.is_empty() {
            // Short-circuit: no elements means no groups (grouped) or one zeroed
            // row (ungrouped).
            if input.group_by.is_empty() {
                group_insertion_order.push(ungrouped_key.clone());
                group_accumulators.insert(
                    ungrouped_key,
                    input.aggregations.iter().map(Accumulator::new).collect(),
                );
            }
            // Grouped + empty input → no groups → output is [].
        } else {
            for element in &elements {
                // Guard: every element must be a JSON object.
                // `Value::get` on a non-object returns `None` silently, so group-key
                // reads and field reads would misfire without this explicit check.
                if !element.is_object() {
                    return Err(ActionError::fatal(format!(
                        "aggregate: every array element must be a JSON object, got {}",
                        element.type_name_str()
                    )));
                }

                // Build the serialized group key for this element.
                let group_key: SerializedGroupKey = if input.group_by.is_empty() {
                    ungrouped_key.clone()
                } else {
                    let mut key_values: Vec<Value> = Vec::with_capacity(input.group_by.len());
                    for group_field in &input.group_by {
                        match element.get(group_field.as_str()) {
                            Some(field_value) => key_values.push(field_value.clone()),
                            None => {
                                return Err(ActionError::fatal(format!(
                                    "aggregate: group_by field `{group_field}` \
                                     missing on an element"
                                )));
                            },
                        }
                    }
                    // `key_values` contains only cloned JSON Values — serialization
                    // should not fail, but we propagate any error rather than panic.
                    serde_json::to_string(&Value::Array(key_values)).map_err(|e| {
                        ActionError::fatal(format!("aggregate: failed to serialize group key: {e}"))
                    })?
                };

                // Initialize accumulators on first encounter with this group key.
                if !group_accumulators.contains_key(&group_key) {
                    group_insertion_order.push(group_key.clone());
                    group_accumulators.insert(
                        group_key.clone(),
                        input.aggregations.iter().map(Accumulator::new).collect(),
                    );
                }

                // Advance each accumulator with this element.
                // `group_insertion_order` and `group_accumulators` are kept in sync:
                // every key pushed to `group_insertion_order` has a corresponding
                // entry in `group_accumulators` inserted in the same branch above.
                let accumulators = group_accumulators.get_mut(&group_key).ok_or_else(|| {
                    ActionError::fatal(
                        "aggregate: internal — group accumulator missing for inserted key",
                    )
                })?;
                for (accumulator, aggregation) in accumulators.iter_mut().zip(&input.aggregations) {
                    accumulator.feed(element, aggregation, input.on_error)?;
                }
            }
        }

        // ── 5. Emit one output row per group in first-seen order ──────────────
        let mut summary_rows: Vec<Value> = Vec::with_capacity(group_insertion_order.len());

        for group_key in group_insertion_order {
            // `group_key` came from `group_insertion_order`, which only holds keys
            // that were simultaneously inserted into `group_accumulators`.
            let accumulators = group_accumulators.remove(&group_key).ok_or_else(|| {
                ActionError::fatal("aggregate: internal — group key missing from accumulators")
            })?;

            let mut summary_row = serde_json::Map::new();

            // Inject the group-by field values first (before aggregation outputs).
            if !input.group_by.is_empty() {
                let key_values: Vec<Value> = serde_json::from_str(&group_key).map_err(|e| {
                    ActionError::fatal(format!("aggregate: failed to parse group key: {e}"))
                })?;
                for (group_field, field_value) in input.group_by.iter().zip(key_values) {
                    summary_row.insert(group_field.clone(), field_value);
                }
            }

            // Inject each aggregation's output value.
            // `finalize` returns Err on float overflow (sum/avg → non-finite).
            for (accumulator, aggregation) in accumulators.into_iter().zip(&input.aggregations) {
                summary_row.insert(aggregation.out_key().to_owned(), accumulator.finalize()?);
            }

            summary_rows.push(Value::Object(summary_row));
        }

        Ok(ActionResult::success(Value::Array(summary_rows)))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::future::Future;

    use nebula_action::testing::TestContextBuilder;
    use nebula_action::{ActionError, ActionResult, StatelessAction};
    use serde_json::{Value, json};

    use super::{Aggregate, AggregateInput, Aggregation, OnError};

    fn run(
        input: AggregateInput,
    ) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
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
        use nebula_action::action::Action;
        assert_eq!(Aggregate::metadata().base.key.as_str(), "core.aggregate");
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
}
