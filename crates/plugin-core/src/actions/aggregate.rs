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

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use nebula_action::{ActionContext, ActionError, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::{HasSchema, Property, Schema, ValidSchema, ValidationReport, field_key};
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

impl HasSchema for AggregateInput {
    #[instrument(name = "core.aggregate.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA.get_or_init(|| {
            Schema::builder()
                .property(super::input_schema::record_data())
                .property(super::input_schema::strings(field_key!("group_by")))
                .property(Property::list(field_key!("aggregations")).required().item(
                    Property::object(field_key!("item"))
                        .description("Tagged aggregation; variant-specific field presence is checked by serde.")
                        .property(Property::select(field_key!("fn"))
                            .option("count", "Count")
                            .option("count_distinct", "Count distinct")
                            .option("sum", "Sum").option("avg", "Average")
                            .option("min", "Minimum").option("max", "Maximum")
                            .option("collect", "Collect").option("join", "Join").required())
                        .property(Property::string(field_key!("field")))
                        .property(Property::string(field_key!("out")))
                        .property(Property::string(field_key!("sep"))),
                ))
                .property(Property::select(field_key!("on_error"))
                    .option("fail", "Fail").option("skip", "Skip"))
                .root_rule(super::input_schema::array_present(field_key!("data"))?)
                .build()
        }).clone()
    }
}

// ── Input validation ──────────────────────────────────────────────────────────

/// The input array, or a Fatal error naming what was given instead.
///
/// Absent, `null`, and non-array `data` are all authoring mistakes — there is no
/// implicit empty array, because aggregating a non-array is never intended.
fn validated_elements(data: Option<&Value>) -> Result<&[Value], ActionError> {
    match data {
        Some(Value::Array(elements)) => Ok(elements),
        Some(Value::Null) | None => Err(ActionError::fatal(
            "aggregate: `data` must be a JSON array, got null",
        )),
        Some(other) => Err(ActionError::fatal(format!(
            "aggregate: `data` must be a JSON array, got {}",
            other.type_name_str()
        ))),
    }
}

/// Reject the authoring mistakes that make the aggregation meaningless.
///
/// A duplicate `out` key and an `out` key colliding with a `group_by` field are
/// both silent-overwrite bugs — the second overwrites the row's group key with
/// an aggregation result — so both fail closed before any element is read.
fn validate_authoring(input: &AggregateInput) -> Result<(), ActionError> {
    if input.aggregations.is_empty() {
        return Err(ActionError::fatal(
            "aggregate: at least one aggregation is required",
        ));
    }

    let mut seen_output_keys: HashSet<&str> = HashSet::with_capacity(input.aggregations.len());
    for output_key in input.aggregations.iter().map(Aggregation::out_key) {
        if !seen_output_keys.insert(output_key) {
            return Err(ActionError::fatal(format!(
                "aggregate: duplicate out key `{output_key}` in aggregations"
            )));
        }
    }

    for aggregation in &input.aggregations {
        let output_key = aggregation.out_key();
        if input.group_by.iter().any(|field| field == output_key) {
            return Err(ActionError::fatal(format!(
                "aggregate: aggregation output `{output_key}` collides with a group_by field"
            )));
        }
    }

    Ok(())
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
        distinct_serialized: HashSet<String>,
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

/// Which end of the ordering a MIN/MAX accumulator tracks.
///
/// The MIN and MAX feed paths are identical except for the comparison
/// direction; this enum names that difference instead of duplicating the body.
#[derive(Debug, Clone, Copy)]
enum ExtremeSide {
    /// MIN — a candidate wins when it orders strictly *before* the prior value.
    Min,
    /// MAX — a candidate wins when it orders strictly *after* the prior value.
    Max,
}

impl ExtremeSide {
    /// Whether a candidate ordering against the prior value makes the
    /// candidate the new extreme.
    fn wins(self, candidate_vs_prior: std::cmp::Ordering) -> bool {
        match self {
            ExtremeSide::Min => candidate_vs_prior.is_lt(),
            ExtremeSide::Max => candidate_vs_prior.is_gt(),
        }
    }
}

impl Accumulator {
    fn new(aggregation: &Aggregation) -> Self {
        match aggregation {
            Aggregation::Count { .. } => Accumulator::Count { row_count: 0 },
            Aggregation::CountDistinct { .. } => Accumulator::CountDistinct {
                distinct_serialized: HashSet::new(),
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

            (
                Accumulator::CountDistinct {
                    distinct_serialized,
                },
                Aggregation::CountDistinct { field, .. },
            ) => {
                Self::feed_count_distinct(distinct_serialized, element, field);
            },

            (acc @ Accumulator::SumInt { .. }, Aggregation::Sum { field, out }) => {
                Self::feed_sum_int(acc, element, field, out, dirty_value_policy)?;
            },

            (Accumulator::SumFloat { float_total }, Aggregation::Sum { field, out }) => {
                Self::feed_sum_float(float_total, element, field, out, dirty_value_policy)?;
            },

            (
                Accumulator::Avg {
                    running_sum,
                    contributing_count,
                },
                Aggregation::Avg { field, out },
            ) => {
                Self::feed_avg(
                    running_sum,
                    contributing_count,
                    element,
                    field,
                    out,
                    dirty_value_policy,
                )?;
            },

            (Accumulator::Min { current_min }, Aggregation::Min { field, out }) => {
                Self::feed_extreme(
                    current_min,
                    element,
                    field,
                    out,
                    ExtremeSide::Min,
                    dirty_value_policy,
                )?;
            },

            (Accumulator::Max { current_max }, Aggregation::Max { field, out }) => {
                Self::feed_extreme(
                    current_max,
                    element,
                    field,
                    out,
                    ExtremeSide::Max,
                    dirty_value_policy,
                )?;
            },

            (Accumulator::Collect { collected_values }, Aggregation::Collect { field, .. }) => {
                Self::feed_collect(collected_values, element, field);
            },

            (Accumulator::Join { joined_parts, .. }, Aggregation::Join { field, out, .. }) => {
                Self::feed_join(joined_parts, element, field, out, dirty_value_policy)?;
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

    /// COUNT DISTINCT — skip null/missing (documented behavior).
    fn feed_count_distinct(
        distinct_serialized: &mut HashSet<String>,
        element: &Value,
        field: &str,
    ) {
        if let Some(field_value) = element.get(field)
            && !field_value.is_null()
        {
            // Serialize to string for set membership; serde_json
            // produces a canonical representation that distinguishes
            // types (1 != "1" != 1.0).
            distinct_serialized.insert(field_value.to_string());
        }
    }

    /// SUM (integer path) — preserves i64 when all values are integers;
    /// upgrades to SumFloat on the first u64-only or floating-point value.
    ///
    /// Matching on `v.is_number()` then trying i64 first (covers both i64
    /// and all integers ≤ i64::MAX) then falling through to as_f64 (covers
    /// u64 > i64::MAX and genuine floats — serde_json's as_f64 returns Some
    /// for any Number) avoids any expect/unreachable in library code.
    fn feed_sum_int(
        acc: &mut Accumulator,
        element: &Value,
        field: &str,
        out: &str,
        policy: OnError,
    ) -> Result<(), ActionError> {
        match element.get(field) {
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
                    // because f64's 53-bit mantissa cannot represent every
                    // integer exactly. serde_json itself parses integer
                    // literals exactly (u64/i64) whenever they fit, and falls
                    // back to f64 for literals with a decimal point, an
                    // exponent, more digits than u64 holds, or a negative
                    // value below i64::MIN whose magnitude still fits u64
                    // (all give as_i64() == None but as_f64() == Some, so the
                    // upgrade path below handles them identically) — so this
                    // upgrade deliberately trades the i64 total's exactness
                    // for the f64 arithmetic the sum now uses.
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
                    return apply_dirty_value_policy(field, out, v.type_name_str(), policy);
                }
            },
            Some(v) if v.is_null() => {
                return apply_dirty_value_policy(field, out, "null", policy);
            },
            None => {
                return apply_dirty_value_policy(field, out, "missing", policy);
            },
            Some(v) => {
                return apply_dirty_value_policy(field, out, v.type_name_str(), policy);
            },
        }
        Ok(())
    }

    /// SUM (float path) — reached after the first float caused an upgrade.
    fn feed_sum_float(
        float_total: &mut f64,
        element: &Value,
        field: &str,
        out: &str,
        policy: OnError,
    ) -> Result<(), ActionError> {
        if let Some((_, addend)) = numeric_addend_or_dirty(element, field, out, policy)? {
            *float_total += addend;
        }
        Ok(())
    }

    /// AVG — advance the running mean state with one numeric element.
    fn feed_avg(
        running_sum: &mut f64,
        contributing_count: &mut u64,
        element: &Value,
        field: &str,
        out: &str,
        policy: OnError,
    ) -> Result<(), ActionError> {
        if let Some((_, addend)) = numeric_addend_or_dirty(element, field, out, policy)? {
            *running_sum += addend;
            *contributing_count += 1;
        }
        Ok(())
    }

    /// MIN/MAX — replace the running extreme when the element orders past it.
    ///
    /// The two aggregations are identical except for the comparison direction,
    /// which `side` names; with no prior extreme the first value is taken
    /// unconditionally.
    fn feed_extreme(
        current_extreme: &mut Option<Value>,
        element: &Value,
        field: &str,
        out: &str,
        side: ExtremeSide,
        policy: OnError,
    ) -> Result<(), ActionError> {
        // `as_f64_strict` is the numeric-type guard only; the actual
        // min/max is decided by exact comparison so large integers survive.
        if let Some((field_value, _)) = numeric_addend_or_dirty(element, field, out, policy)? {
            let is_new_extreme = match current_extreme.as_ref() {
                None => true,
                Some(prior) => side.wins(compare_ordered(field_value, prior)?),
            };
            if is_new_extreme {
                *current_extreme = Some(field_value.clone());
            }
        }
        Ok(())
    }

    /// COLLECT — skip null/missing (documented).
    fn feed_collect(collected_values: &mut Vec<Value>, element: &Value, field: &str) {
        if let Some(field_value) = element.get(field)
            && !field_value.is_null()
        {
            collected_values.push(field_value.clone());
        }
    }

    /// JOIN — skip null/missing (documented). A non-null, non-string
    /// value is a *dirty value* subject to `on_error` (Fatal under
    /// `fail`, skipped under `skip`), exactly like the numeric
    /// aggregations — it is NOT silently dropped, and numbers are NOT
    /// coerced to strings (that would be a hidden type mutation; see
    /// `as_f64_strict`).
    fn feed_join(
        joined_parts: &mut Vec<String>,
        element: &Value,
        field: &str,
        out: &str,
        policy: OnError,
    ) -> Result<(), ActionError> {
        match element.get(field) {
            None | Some(Value::Null) => {},
            Some(Value::String(string_value)) => joined_parts.push(string_value.clone()),
            Some(other) => {
                return apply_dirty_value_policy(field, out, other.type_name_str(), policy);
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

/// Extract the numeric addend for a numeric aggregation arm, or route a dirty
/// value (missing / null / non-numeric) through the `on_error` policy.
///
/// `Ok(Some((field_value, addend)))` = the element carries a number (the exact
/// `Value` is returned next to the `f64` so min/max can compare and clone it
/// type-preserving); `Ok(None)` = a dirty value was skipped under
/// `OnError::Skip` (do not advance); `Err` = it failed under `OnError::Fail`.
fn numeric_addend_or_dirty<'element>(
    element: &'element Value,
    field_name: &str,
    output_key: &str,
    policy: OnError,
) -> Result<Option<(&'element Value, f64)>, ActionError> {
    if let Some(field_value) = element.get(field_name) {
        match as_f64_strict(field_value) {
            Some(addend) => Ok(Some((field_value, addend))),
            None if field_value.is_null() => {
                apply_dirty_value_policy(field_name, output_key, "null", policy)?;
                Ok(None)
            },
            None => {
                apply_dirty_value_policy(
                    field_name,
                    output_key,
                    field_value.type_name_str(),
                    policy,
                )?;
                Ok(None)
            },
        }
    } else {
        apply_dirty_value_policy(field_name, output_key, "missing", policy)?;
        Ok(None)
    }
}

// ── Grouping ──────────────────────────────────────────────────────────────────

/// One group's identity: the canonical JSON of its `group_by` values, and the
/// values themselves.
///
/// Groups are compared by `serialized` — canonical serialization preserves JSON
/// type, so `1` and `"1"` are different groups — and a summary row reports
/// `field_values`, so rows are built from the original values rather than by
/// parsing the key back.
#[derive(Debug)]
struct GroupKey {
    serialized: String,
    field_values: Vec<Value>,
}

impl GroupKey {
    /// Build a key from a group's `group_by` values, in declaration order.
    ///
    /// No values means the one group every element belongs to when `group_by` is
    /// empty. The canonical form is derived here and nowhere else, so the two
    /// fields cannot disagree.
    fn from_values(field_values: Vec<Value>) -> Result<Self, ActionError> {
        // `field_values` holds only cloned JSON values, so serialization cannot
        // fail in practice; the error is propagated rather than unwrapped.
        let serialized = serde_json::to_string(&field_values).map_err(|e| {
            ActionError::fatal(format!("aggregate: failed to serialize group key: {e}"))
        })?;

        Ok(Self {
            serialized,
            field_values,
        })
    }

    /// Read `group_by` off `element`, failing closed when a named field is
    /// absent — a synthetic key would silently merge unrelated rows.
    fn from_element(element: &Value, group_by: &[String]) -> Result<Self, ActionError> {
        let field_values = group_by
            .iter()
            .map(|group_field| {
                element.get(group_field.as_str()).cloned().ok_or_else(|| {
                    ActionError::fatal(format!(
                        "aggregate: group_by field `{group_field}` missing on an element"
                    ))
                })
            })
            .collect::<Result<Vec<Value>, ActionError>>()?;

        Self::from_values(field_values)
    }
}

/// One group: the values of its `group_by` fields, and the running accumulators
/// of its aggregations.
#[derive(Debug)]
struct Group {
    field_values: Vec<Value>,
    accumulators: Vec<Accumulator>,
}

/// The groups of one aggregation run, in first-seen order.
///
/// First-seen order *is* the output row order. `index_by_serialized` is derived
/// state: `append_group` is its only writer and records a position in the same
/// step it appends the group, so a recorded position always indexes the group it
/// was recorded for. The layout a row needs — the `group_by` field names and
/// each accumulator's output key — is read from `input` rather than passed
/// alongside, so a row cannot be assembled from a different authoring than the
/// groups were built from.
#[derive(Debug)]
struct GroupedElements<'a> {
    input: &'a AggregateInput,
    groups: Vec<Group>,
    index_by_serialized: HashMap<String, usize>,
}

impl<'a> GroupedElements<'a> {
    fn new(input: &'a AggregateInput) -> Self {
        Self {
            input,
            groups: Vec::new(),
            index_by_serialized: HashMap::new(),
        }
    }

    /// The accumulators of `key`'s group, creating the group — with one fresh
    /// accumulator per aggregation — the first time the key is seen.
    fn accumulators_for(&mut self, key: GroupKey) -> &mut [Accumulator] {
        let index = if let Some(existing) = self.index_by_serialized.get(&key.serialized).copied() {
            existing
        } else {
            self.append_group(key)
        };

        &mut self.groups[index].accumulators
    }

    /// Append `key`'s group with fresh accumulators, and return its position.
    fn append_group(&mut self, key: GroupKey) -> usize {
        let index = self.groups.len();
        self.index_by_serialized.insert(key.serialized, index);
        self.groups.push(Group {
            field_values: key.field_values,
            accumulators: self
                .input
                .aggregations
                .iter()
                .map(Accumulator::new)
                .collect(),
        });
        index
    }

    /// One summary row per group, in first-seen order.
    ///
    /// Each group's accumulators were built one-per-aggregation from this same
    /// input, so the group-by values and the aggregated outputs together fill the
    /// row exactly — and `validate_authoring` has already rejected `out` keys
    /// that collide with a `group_by` field, so neither overwrites the other.
    fn summary_rows(self) -> Result<Vec<Value>, ActionError> {
        let mut summary_rows = Vec::with_capacity(self.groups.len());

        for group in self.groups {
            let mut summary_row = serde_json::Map::new();

            for (group_field, field_value) in self.input.group_by.iter().zip(&group.field_values) {
                summary_row.insert(group_field.clone(), field_value.clone());
            }

            // `finalize` fails on float overflow (sum/avg → non-finite).
            for (accumulator, aggregation) in
                group.accumulators.into_iter().zip(&self.input.aggregations)
            {
                summary_row.insert(aggregation.out_key().to_owned(), accumulator.finalize()?);
            }

            summary_rows.push(Value::Object(summary_row));
        }

        Ok(summary_rows)
    }
}

/// Partition `elements` into first-seen-order groups and feed each element to
/// its group's accumulators.
///
/// Empty input is not the same as no groups: with `group_by` empty it still
/// yields the one global group, whose fresh accumulators report their zero state
/// (`count` 0, `sum` 0, `avg` null, …). With `group_by` non-empty, no elements
/// means no groups, and so no rows.
fn group_and_accumulate<'a>(
    elements: &[Value],
    input: &'a AggregateInput,
) -> Result<GroupedElements<'a>, ActionError> {
    let mut grouped = GroupedElements::new(input);

    if elements.is_empty() {
        if input.group_by.is_empty() {
            // Creating the group is the whole effect; there is no element to feed.
            let _ = grouped.accumulators_for(GroupKey::from_values(Vec::new())?);
        }
        return Ok(grouped);
    }

    for element in elements {
        // `Value::get` on a non-object returns `None` silently, so the group-key
        // read and every field read would misfire without this explicit guard.
        if !element.is_object() {
            return Err(ActionError::fatal(format!(
                "aggregate: every array element must be a JSON object, got {}",
                element.type_name_str()
            )));
        }

        let key = if input.group_by.is_empty() {
            GroupKey::from_values(Vec::new())?
        } else {
            GroupKey::from_element(element, &input.group_by)?
        };

        let accumulators = grouped.accumulators_for(key);
        for (accumulator, aggregation) in accumulators.iter_mut().zip(&input.aggregations) {
            accumulator.feed(element, aggregation, input.on_error)?;
        }
    }

    Ok(grouped)
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

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.aggregate"),
            nebula_action::metadata_name!("Aggregate"),
            "Reduce an array of objects to grouped/scalar summaries \
             (sum/count/avg/min/max/collect/join)",
        )
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
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
        let elements = validated_elements(input.data.as_ref())?;
        tracing::Span::current().record("element_count", elements.len());
        validate_authoring(&input)?;

        let grouped = group_and_accumulate(elements, &input)?;
        let summary_rows = grouped.summary_rows()?;

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

    fn input(
        data: Option<Value>,
        group_by: &[&str],
        aggregations: Vec<Aggregation>,
    ) -> AggregateInput {
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
}
