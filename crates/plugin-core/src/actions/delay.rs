//! `core.delay` — park the execution on a timer, then resume.
//!
//! `core.delay` is the first first-party action that returns
//! [`ActionResult::Wait`]. It parks the execution either for a fixed duration
//! (`for`) or until an absolute offset-aware RFC3339 timestamp (`until`), then
//! the engine's timer-wake machinery resumes the node and completes it. The
//! node is a **pass-through**: after resume, the original input `data` is sent
//! downstream unchanged (it does not branch).
//!
//! ## Kind
//!
//! Delay is registered as [`ActionKind::Stateless`](nebula_action::metadata::ActionKind::Stateless).
//! The timer-park capability is **orthogonal** to the kind axis: parking is
//! expressed by the `ActionResult::Wait` return value, not by the kind. (The
//! `Control` family's [`ControlOutcome`](nebula_action::control::ControlOutcome)
//! structurally cannot emit `Wait`, so a control-flavoured Delay is not
//! representable.)
//!
//! ## Input
//!
//! ```json
//! { "data": { /* optional pass-through payload */ }, "mode": "for", "amount": 30, "unit": "seconds" }
//! ```
//! or
//! ```json
//! { "data": null, "mode": "until", "datetime": "2026-06-19T00:00:00Z" }
//! ```
//!
//! ## Validation
//!
//! - `for`: `amount` must be `> 0` (a zero delay is no Delay node; negative is
//!   nonsensical) and `amount × unit` must not overflow. The computed wait is
//!   clamped to a 24-hour ceiling ([`MAX_DELAY_SECS`]).
//! - `until`: the timestamp must be offset-aware RFC3339 (naive strings are
//!   rejected). A timestamp in the past is **not** an error — the engine wakes
//!   the node on the next scheduler tick.
//!
//! ## Output
//!
//! The original input `data` (or `null` when absent), delivered on the default
//! flow-out port after the timer fires.

use std::sync::OnceLock;
use std::time::Duration as StdDuration;

use nebula_action::{
    ActionContext, ActionError, ActionMetadata, ActionOutput, ActionResult, StatelessAction,
    result::WaitCondition,
};
use nebula_core::action_key;
use nebula_schema::HasSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::actions::datetime::DurationUnit;

/// Maximum duration `core.delay` will park for: 24 hours, in seconds.
///
/// A `for` delay whose computed duration exceeds this ceiling is clamped to it
/// (with a `warn`). This bounds a single timer-park so a mis-specified workflow
/// cannot pin a node in `Waiting` for an unbounded span. It is the wait ceiling
/// for this action specifically — unrelated to any storage TTL constant.
pub const MAX_DELAY_SECS: u64 = 86_400;

// ── Config types ──────────────────────────────────────────────────────────────

/// How long `core.delay` parks: a relative duration or an absolute instant.
///
/// Externally tagged by `"mode"`. Deserialized from workflow JSON, so
/// forward-compatibility is handled per-field rather than via
/// `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DelaySpec {
    /// Park for `amount × unit`.
    ///
    /// `amount` must be `> 0`. The computed wait is clamped to
    /// [`MAX_DELAY_SECS`].
    For {
        /// Number of `unit`s to wait. Must be strictly positive.
        amount: i64,
        /// Unit of the duration (reused from `core.datetime`).
        unit: DurationUnit,
    },

    /// Park until an absolute offset-aware RFC3339 timestamp.
    ///
    /// A past timestamp is not an error — the engine wakes on the next tick.
    Until {
        /// Offset-aware RFC3339 instant to resume at (naive strings rejected).
        datetime: String,
    },
}

/// Resolved input for `core.delay`.
///
/// `data` is an optional pass-through payload echoed downstream after the
/// timer fires; `spec` selects the wait mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelayInput {
    /// Optional payload passed through unchanged after resume.
    #[serde(default)]
    pub data: Option<Value>,
    /// The wait specification.
    #[serde(flatten)]
    pub spec: DelaySpec,
}

// The input is dynamically shaped (a tagged wait spec plus an opaque
// pass-through payload) — no closed-form JSON Schema. Empty schema is the
// honest declaration; the module doc describes the expected structure.
impl HasSchema for DelayInput {
    fn schema() -> nebula_schema::validated::ValidSchema {
        nebula_schema::validated::ValidSchema::empty()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build the wait duration for a `for` spec, applying validation and the 24h
/// clamp.
fn build_delay_secs(amount: i64, unit: DurationUnit) -> Result<u64, ActionError> {
    if amount < 0 {
        return Err(ActionError::fatal(format!(
            "core.delay: `amount` must be positive (got {amount})"
        )));
    }
    if amount == 0 {
        return Err(ActionError::fatal(
            "core.delay: `amount` must be positive (got 0); a zero delay should be no Delay node"
                .to_owned(),
        ));
    }
    let secs_per = unit.seconds_per_unit();
    let total_secs = amount.checked_mul(secs_per).ok_or_else(|| {
        ActionError::fatal(format!(
            "core.delay: duration overflow computing {amount} × {secs_per} seconds"
        ))
    })?;
    // `total_secs > 0` here (amount > 0, secs_per ≥ 1), so the cast is safe.
    let requested_secs = u64::try_from(total_secs).map_err(|_| {
        ActionError::fatal(format!(
            "core.delay: duration overflow: {total_secs} seconds is out of range"
        ))
    })?;
    if requested_secs > MAX_DELAY_SECS {
        tracing::warn!(
            target = "core.delay",
            requested_secs,
            clamped_to = MAX_DELAY_SECS,
            "delay exceeds 24h ceiling; clamping"
        );
        return Ok(MAX_DELAY_SECS);
    }
    Ok(requested_secs)
}

/// Parse an offset-aware RFC3339 string into UTC; reject naive strings.
fn parse_until_utc(s: &str) -> Result<chrono::DateTime<chrono::Utc>, ActionError> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.to_utc())
        .map_err(|e| {
            ActionError::fatal(format!(
                "core.delay: `datetime` is not a valid offset-aware RFC3339 timestamp: {e}"
            ))
        })
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Timer-park action: parks the execution for a duration or until a timestamp,
/// then resumes and passes its input `data` downstream.
///
/// Keyed `core.delay`. No I/O, no credentials, no resources. Registered as
/// [`ActionKind::Stateless`](nebula_action::metadata::ActionKind::Stateless).
///
/// # Example
///
/// ```rust
/// use nebula_plugin_core::actions::delay::DelaySpec;
/// use nebula_plugin_core::actions::datetime::DurationUnit;
/// use serde_json::json;
///
/// let spec = DelaySpec::For { amount: 30, unit: DurationUnit::Seconds };
/// // Wire shape: {"mode":"for","amount":30,"unit":"seconds"}
/// let wire = serde_json::to_value(&spec).unwrap();
/// assert_eq!(wire["mode"], json!("for"));
/// assert_eq!(wire["unit"], json!("seconds"));
/// ```
#[derive(Debug, Clone, Default)]
pub struct CoreDelay;

impl nebula_action::action::Action for CoreDelay {
    type Input = DelayInput;
    type Output = Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("core.delay"),
            "Delay",
            "Parks the execution for a fixed duration or until a timestamp, then resumes",
        )
    }

    fn dependencies() -> &'static nebula_action::Dependencies {
        static DEPS: OnceLock<nebula_action::Dependencies> = OnceLock::new();
        DEPS.get_or_init(nebula_action::Dependencies::new)
    }
}

impl nebula_action::from_workflow_node::FromWorkflowNode for CoreDelay {
    type Error = ActionError;

    async fn from_workflow_node(
        _node: &nebula_workflow::NodeDefinition,
        _ctx: &dyn ActionContext,
    ) -> Result<Self, Self::Error> {
        Ok(CoreDelay)
    }
}

impl StatelessAction for CoreDelay {
    #[instrument(
        name = "core.delay",
        skip_all,
        fields(mode = tracing::field::debug(std::mem::discriminant(&input.spec)))
    )]
    async fn execute(
        &self,
        input: DelayInput,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        let passthrough = input.data.unwrap_or(Value::Null);

        let condition = match input.spec {
            DelaySpec::For { amount, unit } => {
                let secs = build_delay_secs(amount, unit)?;
                WaitCondition::Duration {
                    duration: StdDuration::from_secs(secs),
                }
            },
            DelaySpec::Until { datetime } => {
                let when = parse_until_utc(&datetime)?;
                WaitCondition::Until { datetime: when }
            },
        };

        Ok(ActionResult::Wait {
            condition,
            // A `Some` timeout on a timer wait is engine-rejected
            // (`WaitConditionNotSupported`): two competing deadlines.
            timeout: None,
            partial_output: Some(ActionOutput::Value(passthrough)),
        })
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::future::Future;

    use nebula_action::testing::TestContextBuilder;
    use serde_json::json;

    use super::*;

    fn run(input: DelayInput) -> impl Future<Output = Result<ActionResult<Value>, ActionError>> {
        let action = CoreDelay;
        let ctx = TestContextBuilder::new().build();
        async move { action.execute(input, &ctx).await }
    }

    fn for_input(amount: i64, unit: DurationUnit) -> DelayInput {
        DelayInput {
            data: None,
            spec: DelaySpec::For { amount, unit },
        }
    }

    fn until_input(datetime: &str) -> DelayInput {
        DelayInput {
            data: None,
            spec: DelaySpec::Until {
                datetime: datetime.into(),
            },
        }
    }

    /// Pull the `WaitCondition` out of a `Wait` result, or fail the test.
    fn expect_wait(result: ActionResult<Value>) -> (WaitCondition, Option<ActionOutput<Value>>) {
        match result {
            ActionResult::Wait {
                condition,
                timeout,
                partial_output,
            } => {
                assert!(timeout.is_none(), "timer wait must carry timeout: None");
                (condition, partial_output)
            },
            other => panic!("expected ActionResult::Wait, got: {other:?}"),
        }
    }

    // ── For ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn for_seconds_parks_with_duration() {
        let (condition, _) = expect_wait(run(for_input(30, DurationUnit::Seconds)).await.unwrap());
        match condition {
            WaitCondition::Duration { duration } => {
                assert_eq!(duration, StdDuration::from_secs(30));
            },
            other => panic!("expected Duration(30s), got: {other:?}"),
        }
    }

    /// Pass-through payload is carried on `partial_output` so downstream sees
    /// the original `data` after resume.
    #[tokio::test]
    async fn for_carries_data_passthrough() {
        let input = DelayInput {
            data: Some(json!({ "k": "v" })),
            spec: DelaySpec::For {
                amount: 5,
                unit: DurationUnit::Seconds,
            },
        };
        let (_, partial) = expect_wait(run(input).await.unwrap());
        let out = partial
            .and_then(ActionOutput::into_value)
            .expect("partial_output must be present");
        assert_eq!(out, json!({ "k": "v" }));
    }

    /// Absent `data` → `Value::Null` pass-through.
    #[tokio::test]
    async fn for_absent_data_is_null_passthrough() {
        let (_, partial) = expect_wait(run(for_input(5, DurationUnit::Seconds)).await.unwrap());
        let out = partial
            .and_then(ActionOutput::into_value)
            .expect("partial_output must be present");
        assert_eq!(out, Value::Null);
    }

    /// > 24h clamps to the 86 400 s ceiling.
    ///
    /// RED witness: drop the clamp branch in `build_delay_secs` → the duration
    /// would be 172 800 s and this assertion would fail.
    #[tokio::test]
    async fn for_over_24h_clamps_to_ceiling() {
        let (condition, _) = expect_wait(run(for_input(2, DurationUnit::Days)).await.unwrap());
        match condition {
            WaitCondition::Duration { duration } => {
                assert_eq!(duration, StdDuration::from_secs(MAX_DELAY_SECS));
            },
            other => panic!("expected clamped Duration, got: {other:?}"),
        }
    }

    /// Exactly 24h is at the ceiling and is NOT clamped down.
    #[tokio::test]
    async fn for_exactly_24h_is_not_clamped() {
        let (condition, _) = expect_wait(run(for_input(24, DurationUnit::Hours)).await.unwrap());
        match condition {
            WaitCondition::Duration { duration } => {
                assert_eq!(duration, StdDuration::from_secs(MAX_DELAY_SECS));
            },
            other => panic!("expected Duration(24h), got: {other:?}"),
        }
    }

    /// Negative `amount` → Fatal.
    ///
    /// Defense in depth: the explicit `amount < 0` guard yields a clear
    /// field-named message, and the later `u64::try_from` floor independently
    /// rejects any negative value — a negative delay can never reach `from_secs`.
    #[tokio::test]
    async fn for_negative_amount_is_fatal() {
        let err = run(for_input(-1, DurationUnit::Seconds)).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for negative amount; got: {err:?}"
        );
    }

    /// Zero `amount` → Fatal.
    #[tokio::test]
    async fn for_zero_amount_is_fatal() {
        let err = run(for_input(0, DurationUnit::Seconds)).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for zero amount; got: {err:?}"
        );
    }

    /// Overflow (`i64::MAX` weeks) → Fatal.
    #[tokio::test]
    async fn for_overflow_is_fatal() {
        let err = run(for_input(i64::MAX, DurationUnit::Weeks))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for overflow; got: {err:?}"
        );
    }

    // ── Until ──────────────────────────────────────────────────────────────────

    /// Valid offset-aware RFC3339 → `Wait{Until}` with the UTC-normalized instant.
    #[tokio::test]
    async fn until_valid_offset_parks_with_until() {
        let (condition, _) =
            expect_wait(run(until_input("2026-06-19T11:30:00+05:30")).await.unwrap());
        match condition {
            WaitCondition::Until { datetime } => {
                // +05:30 of 11:30 == 06:00 UTC.
                assert_eq!(datetime.to_rfc3339(), "2026-06-19T06:00:00+00:00");
            },
            other => panic!("expected Until, got: {other:?}"),
        }
    }

    /// A past timestamp is NOT an error — it parks and the engine wakes on the
    /// next tick.
    #[tokio::test]
    async fn until_past_timestamp_is_not_an_error() {
        let (condition, _) = expect_wait(run(until_input("2000-01-01T00:00:00Z")).await.unwrap());
        assert!(matches!(condition, WaitCondition::Until { .. }));
    }

    /// Naive (no-offset) timestamp → Fatal.
    ///
    /// RED witness: `2026-06-19T00:00:00` has no offset. If the parse accepted
    /// naive strings, this `unwrap_err` would panic on an `Ok` value.
    #[tokio::test]
    async fn until_naive_no_offset_is_fatal() {
        let err = run(until_input("2026-06-19T00:00:00")).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for naive (no-offset) timestamp; got: {err:?}"
        );
    }

    /// Malformed timestamp → Fatal.
    #[tokio::test]
    async fn until_malformed_is_fatal() {
        let err = run(until_input("not-a-timestamp")).await.unwrap_err();
        assert!(
            matches!(err, ActionError::Fatal { .. }),
            "expected Fatal for malformed timestamp; got: {err:?}"
        );
    }

    // ── Serde round-trips ──────────────────────────────────────────────────────

    #[test]
    fn serde_roundtrip_for() {
        let spec = DelaySpec::For {
            amount: 30,
            unit: DurationUnit::Seconds,
        };
        // Wire shape: {"mode":"for","amount":30,"unit":"seconds"}
        let wire = serde_json::to_value(&spec).unwrap();
        assert_eq!(wire["mode"], json!("for"));
        assert_eq!(wire["amount"], json!(30));
        assert_eq!(wire["unit"], json!("seconds"));
        let back: DelaySpec = serde_json::from_value(wire).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn serde_roundtrip_until() {
        let spec = DelaySpec::Until {
            datetime: "2026-06-19T00:00:00Z".into(),
        };
        let wire = serde_json::to_value(&spec).unwrap();
        assert_eq!(wire["mode"], json!("until"));
        let back: DelaySpec = serde_json::from_value(wire).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn deserialize_full_for_input_wire_shape() {
        let input: DelayInput =
            serde_json::from_value(json!({ "mode": "for", "amount": 30, "unit": "seconds" }))
                .unwrap();
        assert!(input.data.is_none());
        assert_eq!(
            input.spec,
            DelaySpec::For {
                amount: 30,
                unit: DurationUnit::Seconds,
            }
        );
    }

    // ── Metadata ───────────────────────────────────────────────────────────────

    #[test]
    fn action_key_is_core_dot_delay() {
        use nebula_action::action::Action;
        assert_eq!(CoreDelay::metadata().base.key.as_str(), "core.delay");
    }

    #[test]
    fn action_display_name_is_delay() {
        use nebula_action::action::Action;
        assert_eq!(CoreDelay::metadata().base.name, "Delay");
    }
}
