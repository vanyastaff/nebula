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
use nebula_action::{ActionContext, ActionError, ActionMetadata, ActionResult, StatelessAction};
use nebula_core::action_key;
use nebula_schema::HasSchema;
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

/// Externally-tagged operation carried by [`DateTimeInput`].
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
    /// output uses whole-second precision with a `Z` suffix
    /// (e.g. `"2026-06-19T00:00:00Z"`).
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
    /// Duration overflow (e.g. adding i64::MAX seconds) is Fatal.
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
    /// Duration overflow is Fatal.
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

// The input is dynamically typed (timestamp strings, op enum, strftime
// strings) — no closed-form JSON Schema can be emitted. Empty schema is the
// honest declaration; the module doc describes the expected structure.
impl HasSchema for DateTimeInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
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

/// Canonical UTC RFC3339 with whole-second precision and `Z` suffix.
fn to_utc_rfc3339(dt: DateTime<FixedOffset>) -> String {
    dt.to_utc().to_rfc3339_opts(SecondsFormat::Secs, true)
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

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.datetime"),
            "DateTime",
            "Offset-aware RFC3339 timestamp formatting, parsing, arithmetic, and diff",
        )
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
                Value::String(result.to_rfc3339_opts(SecondsFormat::Secs, true))
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
                Value::String(result.to_rfc3339_opts(SecondsFormat::Secs, true))
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
mod tests {
    use std::future::Future;

    use nebula_action::testing::TestContextBuilder;
    use serde_json::json;

    use super::*;

    // ── Test harness ──────────────────────────────────────────────────────────

    fn run(input: DateTimeInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
        let action = DateTimeAction;
        let ctx = TestContextBuilder::new().build();
        async move { action.execute(input, &ctx).await }
    }

    fn extract_output(result: ActionResult<Value>) -> Value {
        result
            .into_primary_output()
            .and_then(nebula_action::ActionOutput::into_value)
            .expect("ActionResult must carry a primary output value")
    }

    fn fmt_input(ts: &str, fmt: &str) -> DateTimeInput {
        DateTimeInput {
            data: None,
            op: DateTimeOp::Format {
                input: ts.into(),
                format: fmt.into(),
                tz_offset_seconds: None,
            },
        }
    }

    fn add_input(ts: &str, amount: i64, unit: DurationUnit) -> DateTimeInput {
        DateTimeInput {
            data: None,
            op: DateTimeOp::Add {
                input: ts.into(),
                amount,
                unit,
            },
        }
    }

    fn sub_input(ts: &str, amount: i64, unit: DurationUnit) -> DateTimeInput {
        DateTimeInput {
            data: None,
            op: DateTimeOp::Subtract {
                input: ts.into(),
                amount,
                unit,
            },
        }
    }

    fn diff_input(from: &str, to: &str, unit: DurationUnit) -> DateTimeInput {
        DateTimeInput {
            data: None,
            op: DateTimeOp::Diff {
                from: from.into(),
                to: to.into(),
                unit,
            },
        }
    }

    fn parse_input(ts: &str) -> DateTimeInput {
        DateTimeInput {
            data: None,
            op: DateTimeOp::Parse {
                input: ts.into(),
                format: None,
            },
        }
    }

    // ── Format ────────────────────────────────────────────────────────────────

    /// RED witness: if format() were not wired, this would panic on unwrap of
    /// a Failed result.
    #[tokio::test]
    async fn format_known_instant_date_only() {
        let out = extract_output(
            run(fmt_input("2026-06-19T00:00:00Z", "%Y-%m-%d"))
                .await
                .unwrap(),
        );
        assert_eq!(out, json!("2026-06-19"));
    }

    #[tokio::test]
    async fn format_day_name() {
        // 2026-06-19 is a Friday.
        let out = extract_output(run(fmt_input("2026-06-19T12:00:00Z", "%A")).await.unwrap());
        assert_eq!(out, json!("Friday"));
    }

    #[tokio::test]
    async fn format_with_tz_offset() {
        // UTC 2026-06-19T00:00:00Z → +05:30 offset = 2026-06-19T05:30:00+05:30
        // %H:%M should render as "05:30".
        let input = DateTimeInput {
            data: None,
            op: DateTimeOp::Format {
                input: "2026-06-19T00:00:00Z".into(),
                format: "%H:%M".into(),
                tz_offset_seconds: Some(19_800), // +05:30
            },
        };
        let out = extract_output(run(input).await.unwrap());
        assert_eq!(out, json!("05:30"));
    }

    #[tokio::test]
    async fn format_invalid_tz_offset_is_fatal() {
        let input = DateTimeInput {
            data: None,
            op: DateTimeOp::Format {
                input: "2026-06-19T00:00:00Z".into(),
                format: "%H".into(),
                tz_offset_seconds: Some(100_000), // > 86399 → invalid
            },
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for invalid tz_offset_seconds; got: {err:?}"
        );
    }

    // ── Parse ─────────────────────────────────────────────────────────────────

    /// Parse +offset → canonical UTC.
    ///
    /// RED witness: if the UTC-normalization were removed, `2026-06-19T11:30:00+05:30`
    /// would not equal `2026-06-19T06:00:00Z`.
    #[tokio::test]
    async fn parse_offset_to_utc_canonical() {
        // 2026-06-19T11:30:00+05:30 = 2026-06-19T06:00:00Z
        let out = extract_output(run(parse_input("2026-06-19T11:30:00+05:30")).await.unwrap());
        assert_eq!(out, json!("2026-06-19T06:00:00Z"));
    }

    #[tokio::test]
    async fn parse_round_trip_utc() {
        let out = extract_output(run(parse_input("2026-06-19T00:00:00Z")).await.unwrap());
        assert_eq!(out, json!("2026-06-19T00:00:00Z"));
    }

    // ── Add ───────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_one_day() {
        let out = extract_output(
            run(add_input("2026-06-19T00:00:00Z", 1, DurationUnit::Days))
                .await
                .unwrap(),
        );
        assert_eq!(out, json!("2026-06-20T00:00:00Z"));
    }

    /// Cross-month boundary: June 30 + 1 day = July 1.
    ///
    /// RED witness: without UTC-arithmetic, calendar-aware ops might give a
    /// different result on month boundaries.
    #[tokio::test]
    async fn add_crosses_month_boundary() {
        let out = extract_output(
            run(add_input("2026-06-30T00:00:00Z", 1, DurationUnit::Days))
                .await
                .unwrap(),
        );
        assert_eq!(out, json!("2026-07-01T00:00:00Z"));
    }

    #[tokio::test]
    async fn add_hours() {
        let out = extract_output(
            run(add_input("2026-06-19T22:00:00Z", 3, DurationUnit::Hours))
                .await
                .unwrap(),
        );
        assert_eq!(out, json!("2026-06-20T01:00:00Z"));
    }

    /// Add milliseconds: 1 000 ms advances exactly one second. Proves the
    /// `Milliseconds` unit drives `chrono::Duration` on the millisecond base.
    ///
    /// (Output stays whole-second precision per the module's documented
    /// invariant; this asserts the arithmetic, not sub-second rendering.)
    #[tokio::test]
    async fn add_milliseconds_whole_second() {
        let out = extract_output(
            run(add_input(
                "2026-06-19T00:00:00Z",
                1_000,
                DurationUnit::Milliseconds,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out, json!("2026-06-19T00:00:01Z"));
    }

    // ── Subtract ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn subtract_two_hours() {
        let out = extract_output(
            run(sub_input("2026-06-19T04:00:00Z", 2, DurationUnit::Hours))
                .await
                .unwrap(),
        );
        assert_eq!(out, json!("2026-06-19T02:00:00Z"));
    }

    /// Subtract crosses midnight.
    #[tokio::test]
    async fn subtract_crosses_midnight() {
        let out = extract_output(
            run(sub_input("2026-06-19T01:00:00Z", 2, DurationUnit::Hours))
                .await
                .unwrap(),
        );
        assert_eq!(out, json!("2026-06-18T23:00:00Z"));
    }

    // ── Diff ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn diff_seconds_one_hour() {
        // 3 600 seconds apart.
        let out = extract_output(
            run(diff_input(
                "2026-06-19T00:00:00Z",
                "2026-06-19T01:00:00Z",
                DurationUnit::Seconds,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out, json!(3600_i64));
    }

    #[tokio::test]
    async fn diff_days_one() {
        let out = extract_output(
            run(diff_input(
                "2026-06-19T00:00:00Z",
                "2026-06-20T00:00:00Z",
                DurationUnit::Days,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out, json!(1_i64));
    }

    /// `to < from` → negative diff. This is NOT an error.
    ///
    /// RED witness: if the action returned an error for negative diffs, this
    /// test would fail on `.unwrap()`.
    #[tokio::test]
    async fn diff_negative_when_to_before_from() {
        let out = extract_output(
            run(diff_input(
                "2026-06-19T01:00:00Z",
                "2026-06-19T00:00:00Z",
                DurationUnit::Seconds,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out, json!(-3600_i64));
    }

    /// Diff with a non-exact unit truncates toward zero (partial unit = 0 whole units).
    ///
    /// 25 h gap with Days unit → 1 (not 2).
    /// 23 h gap with Days unit → 0 (not 1).
    ///
    /// RED witness: if the implementation rounded instead of truncating, the
    /// 23 h case would return 1 (wrong) — the second assertion would fail.
    #[tokio::test]
    async fn diff_days_truncates_partial() {
        // 25 hours → 1 full day (integer-division of 90000 / 86400 = 1)
        let out = extract_output(
            run(diff_input(
                "2026-06-19T00:00:00Z",
                "2026-06-20T01:00:00Z",
                DurationUnit::Days,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out, json!(1_i64), "25 h must truncate to 1 full day");

        // 23 hours → 0 full days (integer-division of 82800 / 86400 = 0)
        let out2 = extract_output(
            run(diff_input(
                "2026-06-19T00:00:00Z",
                "2026-06-19T23:00:00Z",
                DurationUnit::Days,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out2, json!(0_i64), "23 h must truncate to 0 full days");
    }

    /// Diff with Weeks unit: a 14-day gap returns 2 whole weeks.
    #[tokio::test]
    async fn diff_weeks() {
        let out = extract_output(
            run(diff_input(
                "2026-06-19T00:00:00Z",
                "2026-07-03T00:00:00Z",
                DurationUnit::Weeks,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out, json!(2_i64), "14-day gap must equal 2 whole weeks");
    }

    /// Diff with Minutes unit: a 90-minute gap returns 90.
    #[tokio::test]
    async fn diff_minutes() {
        let out = extract_output(
            run(diff_input(
                "2026-06-19T00:00:00Z",
                "2026-06-19T01:30:00Z",
                DurationUnit::Minutes,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out, json!(90_i64), "90-minute gap must equal 90 minutes");
    }

    /// Diff with Milliseconds unit: a 1-second gap returns 1 000.
    ///
    /// RED witness: on the old seconds base there was no `Milliseconds` unit, so
    /// sub-second-resolution diffs could not be requested.
    #[tokio::test]
    async fn diff_milliseconds() {
        let out = extract_output(
            run(diff_input(
                "2026-06-19T00:00:00Z",
                "2026-06-19T00:00:01Z",
                DurationUnit::Milliseconds,
            ))
            .await
            .unwrap(),
        );
        assert_eq!(out, json!(1_000_i64), "1-second gap must equal 1000 ms");
    }

    // ── Error paths (RED witnesses) ───────────────────────────────────────────

    /// Malformed timestamp → Fatal.
    #[tokio::test]
    async fn malformed_timestamp_is_fatal() {
        let err = run(fmt_input("not-a-timestamp", "%Y")).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for malformed timestamp; got: {err:?}"
        );
    }

    /// Naive timestamp (no UTC offset) → Fatal.
    ///
    /// RED witness: `2026-06-19T00:00:00` has no offset component. If the
    /// action accepted naive strings, this test would pass the `unwrap()` and
    /// the `assert!` would be reached — but it should never get there.
    #[tokio::test]
    async fn naive_timestamp_no_offset_is_fatal() {
        let err = run(fmt_input("2026-06-19T00:00:00", "%Y-%m-%d"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for naive (no-offset) timestamp; got: {err:?}"
        );
    }

    /// Negative `amount` on Add → Fatal.
    ///
    /// RED witness: if the guard were removed, `build_duration(-1, Seconds)`
    /// would construct a negative Duration and silently go backwards instead of
    /// returning an error.
    #[tokio::test]
    async fn negative_amount_on_add_is_fatal() {
        let err = run(add_input("2026-06-19T00:00:00Z", -1, DurationUnit::Seconds))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for negative amount; got: {err:?}"
        );
    }

    /// Negative `amount` on Subtract → Fatal.
    #[tokio::test]
    async fn negative_amount_on_subtract_is_fatal() {
        let err = run(sub_input("2026-06-19T00:00:00Z", -5, DurationUnit::Minutes))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for negative amount on Subtract; got: {err:?}"
        );
    }

    /// Duration overflow (i64::MAX seconds) → Fatal.
    #[tokio::test]
    async fn duration_overflow_is_fatal() {
        let err = run(add_input(
            "2026-06-19T00:00:00Z",
            i64::MAX,
            DurationUnit::Seconds,
        ))
        .await
        .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for overflow; got: {err:?}"
        );
    }

    /// Non-object `data` → Fatal.
    #[tokio::test]
    async fn non_object_data_is_fatal() {
        let input = DateTimeInput {
            data: Some(json!([1, 2, 3])),
            op: DateTimeOp::Parse {
                input: "2026-06-19T00:00:00Z".into(),
                format: None,
            },
        };
        let err = run(input).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for array data; got: {err:?}"
        );
    }

    // ── Serde round-trips ─────────────────────────────────────────────────────

    #[test]
    fn serde_roundtrip_format() {
        let op = DateTimeOp::Format {
            input: "2026-06-19T00:00:00Z".into(),
            format: "%Y-%m-%d".into(),
            tz_offset_seconds: Some(3600),
        };
        let json = serde_json::to_string(&op).unwrap();
        let back: DateTimeOp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, op);
    }

    #[test]
    fn serde_roundtrip_parse() {
        let op = DateTimeOp::Parse {
            input: "2026-06-19T00:00:00Z".into(),
            format: None,
        };
        let json = serde_json::to_string(&op).unwrap();
        let back: DateTimeOp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, op);
    }

    #[test]
    fn serde_roundtrip_add() {
        let op = DateTimeOp::Add {
            input: "2026-06-19T00:00:00Z".into(),
            amount: 7,
            unit: DurationUnit::Days,
        };
        let json = serde_json::to_string(&op).unwrap();
        let back: DateTimeOp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, op);
    }

    #[test]
    fn serde_roundtrip_subtract() {
        let op = DateTimeOp::Subtract {
            input: "2026-06-19T00:00:00Z".into(),
            amount: 2,
            unit: DurationUnit::Hours,
        };
        let json = serde_json::to_string(&op).unwrap();
        let back: DateTimeOp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, op);
    }

    #[test]
    fn serde_roundtrip_diff() {
        let op = DateTimeOp::Diff {
            from: "2026-06-19T00:00:00Z".into(),
            to: "2026-06-20T00:00:00Z".into(),
            unit: DurationUnit::Days,
        };
        let json = serde_json::to_string(&op).unwrap();
        let back: DateTimeOp = serde_json::from_str(&json).unwrap();
        assert_eq!(back, op);
    }

    #[test]
    fn serde_roundtrip_duration_unit_weeks() {
        let unit = DurationUnit::Weeks;
        let json = serde_json::to_string(&unit).unwrap();
        let back: DurationUnit = serde_json::from_str(&json).unwrap();
        assert_eq!(back, unit);
    }

    /// New `milliseconds` wire value serializes to `"milliseconds"` and round-trips.
    #[test]
    fn serde_roundtrip_duration_unit_milliseconds() {
        let unit = DurationUnit::Milliseconds;
        let json = serde_json::to_string(&unit).unwrap();
        assert_eq!(json, "\"milliseconds\"");
        let back: DurationUnit = serde_json::from_str(&json).unwrap();
        assert_eq!(back, unit);
    }

    /// Backward-compatibility: the pre-existing `seconds` wire value still
    /// deserializes unchanged after the millisecond-base rework.
    #[test]
    fn deserialize_legacy_seconds_unit_unchanged() {
        let back: DurationUnit = serde_json::from_str("\"seconds\"").unwrap();
        assert_eq!(back, DurationUnit::Seconds);
    }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn action_key_is_core_dot_datetime() {
        use nebula_action::action::Action;
        assert_eq!(
            DateTimeAction::metadata().base.key.as_str(),
            "core.datetime"
        );
    }

    #[test]
    fn action_display_name_is_datetime() {
        use nebula_action::action::Action;
        assert_eq!(DateTimeAction::metadata().base.name, "DateTime");
    }
}
