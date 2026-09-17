//! `core.datetime` — offset-aware timestamp arithmetic and formatting.
//!
//! All timestamp inputs and outputs are **offset-aware RFC3339** strings.
//! Naive timestamps (no UTC-offset) are rejected with a `Fatal` error.
//! All arithmetic is performed in UTC after `.to_utc()` so there is
//! **no DST ambiguity** — `Days` means exactly 86 400 seconds, `Weeks`
//! exactly 604 800 seconds. Calendar units (months, years) are excluded
//! because their duration is ambiguous.
//!
//! ## Operations
//!
//! | `op`       | Description |
//! |------------|-------------|
//! | `format`   | Re-render a timestamp using a strftime format string. Optionally shift to a UTC offset before formatting. strftime specifiers are not eagerly validated — bad specifiers render literally (chrono's documented behaviour). |
//! | `parse`    | Normalise an RFC3339 timestamp to the canonical UTC form (`2026-06-19T00:00:00Z`). In v1 the optional `format` field is reserved and unused. |
//! | `add`      | Advance a timestamp by `amount` (≥ 0) units. |
//! | `subtract` | Retreat a timestamp by `amount` (≥ 0) units. |
//! | `diff`     | Return the signed integer number of whole `unit`s between `from` and `to`. `to < from` produces a negative result. |
//!
//! ## Purity note
//!
//! `now` / clock access is intentionally excluded — the `clock` feature of
//! chrono is present in the workspace pin but this action never calls it.
//! Purity is behavioural: every input is deterministic; nothing is read from
//! the system clock.
//!
//! ## Input
//!
//! ```json
//! {
//!   "data": { /* optional — non-object → Fatal; null/absent → ignored */ },
//!   "op": "format",
//!   "input": "2026-06-19T00:00:00Z",
//!   "format": "%Y-%m-%d"
//! }
//! ```
//!
//! ## Output
//!
//! A single JSON value: a `String` for `format`/`parse`/`add`/`subtract`,
//! or a `Number` (i64) for `diff`.

use std::sync::OnceLock;

use chrono::{DateTime, Duration, FixedOffset, SecondsFormat};
use nebula_action::{ActionContext, ActionError, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::{
    HasSchema, Predicate, Property, Rule, Schema, ValidSchema, ValidationReport, ValuePath,
    field_key,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::util::ValueTypeNameStr;

// ── Config types ──────────────────────────────────────────────────────────────

/// Unit of duration for arithmetic and diff operations.
///
/// The base unit is the **millisecond**, so sub-second durations are
/// representable. `Days` and `Weeks` are defined in terms of fixed milliseconds
/// (86 400 000 and 604 800 000 respectively) — not calendar days. `Months` and
/// `Years` are excluded because their length varies and would require calendar
/// awareness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationUnit {
    /// 1 millisecond — the finest representable unit.
    Milliseconds,
    /// 1 000 milliseconds.
    Seconds,
    /// 60 000 milliseconds.
    Minutes,
    /// 3 600 000 milliseconds.
    Hours,
    /// 86 400 000 milliseconds (not a calendar day).
    Days,
    /// 604 800 000 milliseconds (not a calendar week).
    Weeks,
}

impl DurationUnit {
    /// Returns the number of milliseconds in one unit.
    pub(crate) fn millis_per_unit(self) -> i64 {
        match self {
            DurationUnit::Milliseconds => 1,
            DurationUnit::Seconds => 1_000,
            DurationUnit::Minutes => 60_000,
            DurationUnit::Hours => 3_600_000,
            DurationUnit::Days => 86_400_000,
            DurationUnit::Weeks => 604_800_000,
        }
    }
}

/// Internally tagged operation flattened into [`DateTimeInput`].
///
/// The `"op"` field drives deserialization to the correct variant. These
/// types are deserialized from workflow JSON, not literal-constructed by
/// external Rust code, so forward-compatibility is handled via
/// `#[serde(default)]` on optional fields rather than `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DateTimeOp {
    /// Re-render `input` using a strftime `format` string.
    ///
    /// If `tz_offset_seconds` is provided the timestamp is first converted
    /// to that offset. Invalid offset values (e.g. outside ±86 399 s) are
    /// Fatal. Absent or `null` ⇒ UTC is used.
    ///
    /// strftime specifiers are not eagerly validated — unrecognised specifiers
    /// render literally (chrono's documented behaviour).
    Format {
        /// Offset-aware RFC3339 timestamp to format.
        input: String,
        /// strftime format string (e.g. `"%Y-%m-%d"`).
        format: String,
        /// Optional UTC offset in seconds applied before formatting
        /// (e.g. `19800` for +05:30). Must be in `[-86399, 86399]`.
        #[serde(default)]
        tz_offset_seconds: Option<i32>,
    },

    /// Normalise an offset-aware RFC3339 timestamp to canonical UTC form.
    ///
    /// In v1 the optional `format` field is reserved and unused. The canonical
    /// output carries a `Z` suffix and whole-second instants render without a
    /// fractional part (e.g. `"2026-06-19T00:00:00Z"`); any sub-second component
    /// in the input is preserved (e.g. `"...00.250Z"`), not truncated.
    Parse {
        /// Offset-aware RFC3339 string to normalise.
        input: String,
        /// Reserved for future use. Currently ignored.
        #[serde(default)]
        format: Option<String>,
    },

    /// Advance `input` by `amount` × `unit`.
    ///
    /// `amount` must be ≥ 0; the direction is encoded in the op name.
    /// Duration overflow (e.g. adding i64::MAX milliseconds) is Fatal.
    /// Sub-second results (from the `milliseconds` unit) are preserved in the
    /// output; whole-second results render without a fractional part.
    Add {
        /// Offset-aware RFC3339 timestamp.
        input: String,
        /// Non-negative number of units to add.
        amount: i64,
        /// Unit of the duration.
        unit: DurationUnit,
    },

    /// Retreat `input` by `amount` × `unit`.
    ///
    /// `amount` must be ≥ 0; the direction is encoded in the op name.
    /// Duration overflow is Fatal. Sub-second results (from the `milliseconds`
    /// unit) are preserved in the output; whole-second results render without a
    /// fractional part.
    Subtract {
        /// Offset-aware RFC3339 timestamp.
        input: String,
        /// Non-negative number of units to subtract.
        amount: i64,
        /// Unit of the duration.
        unit: DurationUnit,
    },

    /// Compute the signed integer number of whole `unit`s between `from` and `to`.
    ///
    /// `(to − from)` is computed in UTC. If `to < from` the result is negative
    /// (not an error). Fractional units are truncated toward zero.
    Diff {
        /// Earlier (or reference) timestamp.
        from: String,
        /// Later (or comparison) timestamp.
        to: String,
        /// Unit for the result.
        unit: DurationUnit,
    },
}

/// Resolved input for `core.datetime`.
///
/// `data` is optional metadata; when `Some` it must be a JSON object or null
/// (non-object → `Fatal`). It is otherwise ignored in v1. `op` drives the
/// action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateTimeInput {
    /// Optional context object. Must be a JSON object or null/absent.
    #[serde(default)]
    pub data: Option<Value>,
    /// The datetime operation to perform.
    #[serde(flatten)]
    pub op: DateTimeOp,
}

impl HasSchema for DateTimeInput {
    #[instrument(name = "core.datetime.schema", skip_all, err)]
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA
            .get_or_init(|| {
                let selected = |operations: &[&str]| {
                    super::input_schema::admit_rule(Rule::predicate(Predicate::In(
                        ValuePath::root().push("op"),
                        operations.iter().map(|op| Value::from(*op)).collect(),
                    )))
                };
                let nullable_format = super::input_schema::admit_rule(Rule::any([
                    super::input_schema::admit_rule(Rule::one_of([Value::Null]))?,
                    Rule::min_length(0),
                ]))?;
                let offset_bounds = super::input_schema::admit_rule(Rule::all([
                    Rule::min_value(i64::from(i32::MIN)),
                    Rule::max_value(i64::from(i32::MAX)),
                ]))?;
                let nullable_offset = super::input_schema::admit_rule(Rule::any([
                    super::input_schema::admit_rule(Rule::one_of([Value::Null]))?,
                    offset_bounds,
                ]))?;
                Schema::builder()
                    .property(super::input_schema::nullable_object_data())
                    .property(
                        Property::select(field_key!("op"))
                            .option("format", "Format")
                            .option("parse", "Parse")
                            .option("add", "Add")
                            .option("subtract", "Subtract")
                            .option("diff", "Difference")
                            .required(),
                    )
                    .property(
                        Property::string(field_key!("input"))
                            .required_when(selected(&["format", "parse", "add", "subtract"])?),
                    )
                    .property(
                        Property::dynamic(field_key!("format"))
                            .description(
                                "String format; parse also accepts null. An empty string is valid.",
                            )
                            .with_rule(nullable_format)
                            .required_when(selected(&["format"])?),
                    )
                    .property(
                        Property::dynamic(field_key!("tz_offset_seconds"))
                            .description(
                                "Nullable i32 UTC offset; serde checks the integer representation.",
                            )
                            .with_rule(nullable_offset),
                    )
                    .property(
                        Property::integer(field_key!("amount"))
                            .min_int(0)
                            .max_int(i64::MAX)
                            .required_when(selected(&["add", "subtract"])?),
                    )
                    .property(
                        Property::select(field_key!("unit"))
                            .option("milliseconds", "Milliseconds")
                            .option("seconds", "Seconds")
                            .option("minutes", "Minutes")
                            .option("hours", "Hours")
                            .option("days", "Days")
                            .option("weeks", "Weeks")
                            .required_when(selected(&["add", "subtract", "diff"])?),
                    )
                    .property(
                        Property::string(field_key!("from")).required_when(selected(&["diff"])?),
                    )
                    .property(
                        Property::string(field_key!("to")).required_when(selected(&["diff"])?),
                    )
                    .build()
            })
            .clone()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse an offset-aware RFC3339 string; reject naive strings.
fn parse_rfc3339(field: &str, s: &str) -> Result<DateTime<FixedOffset>, ActionError> {
    DateTime::parse_from_rfc3339(s).map_err(|e| {
        ActionError::fatal(format!(
            "core.datetime: `{field}` is not a valid RFC3339 timestamp: {e}"
        ))
    })
}

/// Canonical UTC RFC3339 with a `Z` suffix. Uses `AutoSi`: whole-second
/// instants render without a fractional part (byte-identical to second
/// precision), while any sub-second component present in the input is preserved
/// rather than truncated away.
fn to_utc_rfc3339(dt: DateTime<FixedOffset>) -> String {
    dt.to_utc().to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// Build a `chrono::Duration` from `amount` (≥ 0) × `unit`, guarding overflow.
fn build_duration(amount: i64, unit: DurationUnit) -> Result<Duration, ActionError> {
    if amount < 0 {
        return Err(ActionError::fatal(format!(
            "core.datetime: amount must be non-negative (got {amount}); \
             use the opposite op to go backwards"
        )));
    }
    let millis_per = unit.millis_per_unit();
    let total_millis = amount.checked_mul(millis_per).ok_or_else(|| {
        ActionError::fatal(format!(
            "core.datetime: duration overflow computing {amount} × {millis_per} milliseconds"
        ))
    })?;
    Duration::try_milliseconds(total_millis).ok_or_else(|| {
        ActionError::fatal(format!(
            "core.datetime: duration overflow: {total_millis} milliseconds is out of range"
        ))
    })
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Pure action for offset-aware timestamp operations.
///
/// Keyed `core.datetime`. No I/O, no credentials, no resources.
///
/// # Example
///
/// ```rust
/// use nebula_plugin_core::actions::datetime::{DateTimeOp, DurationUnit};
/// use serde_json::json;
///
/// let op = DateTimeOp::Add {
///     input: "2026-06-30T00:00:00Z".into(),
///     amount: 1,
///     unit: DurationUnit::Days,
/// };
///
/// // Wire shape: {"op":"add","input":"2026-06-30T00:00:00Z","amount":1,"unit":"days"}
/// let wire = serde_json::to_value(&op).unwrap();
/// assert_eq!(wire["op"], json!("add"));
/// assert_eq!(wire["unit"], json!("days"));
/// ```
#[derive(Debug)]
pub struct DateTimeAction;

impl nebula_action::action::Action for DateTimeAction {
    type Input = DateTimeInput;
    type Output = Value;

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.datetime"),
            nebula_action::metadata_name!("DateTime"),
            "Offset-aware RFC3339 timestamp formatting, parsing, arithmetic, and diff",
        )
        .with_version(nebula_action::MetadataVersion::new(2, 0, 0))
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for DateTimeAction {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(DateTimeAction)
    }
}

impl StatelessAction for DateTimeAction {
    #[instrument(
        name = "core.datetime",
        skip_all,
        fields(op = tracing::field::debug(std::mem::discriminant(&input.op)))
    )]
    async fn execute(
        &self,
        input: DateTimeInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        // Validate `data` shape — non-object is Fatal; null/absent is fine.
        match &input.data {
            Some(Value::Object(_) | Value::Null) | None => {},
            Some(other) => {
                return Err(ActionError::fatal(format!(
                    "core.datetime: `data` must be a JSON object or null, got {}",
                    other.type_name_str()
                )));
            },
        }

        let output = match input.op {
            DateTimeOp::Format {
                input: ts,
                format,
                tz_offset_seconds,
            } => {
                let dt = parse_rfc3339("input", &ts)?;
                let formatted = match tz_offset_seconds {
                    None => dt.to_utc().format(&format).to_string(),
                    Some(offset_secs) => {
                        let offset = FixedOffset::east_opt(offset_secs).ok_or_else(|| {
                            ActionError::fatal(format!(
                                "core.datetime: `tz_offset_seconds` {offset_secs} is \
                                     out of range; must be in [-86399, 86399]"
                            ))
                        })?;
                        dt.with_timezone(&offset).format(&format).to_string()
                    },
                };
                Value::String(formatted)
            },

            DateTimeOp::Parse {
                input: ts,
                format: _,
            } => {
                // v1: RFC3339 only; `format` is reserved/unused.
                let dt = parse_rfc3339("input", &ts)?;
                Value::String(to_utc_rfc3339(dt))
            },

            DateTimeOp::Add {
                input: ts,
                amount,
                unit,
            } => {
                let dt = parse_rfc3339("input", &ts)?;
                let dur = build_duration(amount, unit)?;
                let result = dt.to_utc().checked_add_signed(dur).ok_or_else(|| {
                    ActionError::fatal("core.datetime: duration overflow".to_string())
                })?;
                // `AutoSi` keeps whole-second results byte-identical (no `.0`
                // suffix) but preserves sub-second precision from the
                // `milliseconds` unit instead of truncating it away.
                Value::String(result.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            },

            DateTimeOp::Subtract {
                input: ts,
                amount,
                unit,
            } => {
                let dt = parse_rfc3339("input", &ts)?;
                let dur = build_duration(amount, unit)?;
                let result = dt.to_utc().checked_sub_signed(dur).ok_or_else(|| {
                    ActionError::fatal("core.datetime: duration overflow".to_string())
                })?;
                // `AutoSi`: preserve sub-second precision; whole seconds stay
                // byte-identical (see the `Add` arm).
                Value::String(result.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            },

            DateTimeOp::Diff { from, to, unit } => {
                let from_dt = parse_rfc3339("from", &from)?;
                let to_dt = parse_rfc3339("to", &to)?;
                let delta: Duration = to_dt.to_utc() - from_dt.to_utc();
                let total_millis = delta.num_milliseconds();
                let millis_per = unit.millis_per_unit();
                // Integer division truncates toward zero — matches the spec.
                let count = total_millis / millis_per;
                Value::Number(count.into())
            },
        };

        Ok(ActionResult::success(output))
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "datetime_tests.rs"]
mod tests;
