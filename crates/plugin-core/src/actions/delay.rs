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
//! Sub-second parks use the `milliseconds` unit:
//! ```json
//! { "data": null, "mode": "for", "amount": 250, "unit": "milliseconds" }
//! ```
//! or park until an absolute instant:
//! ```json
//! { "data": null, "mode": "until", "datetime": "2026-06-19T00:00:00Z" }
//! ```
//!
//! ## Validation
//!
//! - `for`: `amount` must be `> 0` (a zero delay is no Delay node; negative is
//!   nonsensical) and `amount × unit` must not overflow. The computed wait is
//!   clamped to a 24-hour ceiling ([`MAX_DELAY_MILLIS`]).
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
    ActionContext, ActionError, ActionOutput, ActionResult, StatelessAction, result::WaitCondition,
};
use nebula_core::action_key;
use nebula_schema::{
    HasSchema, Predicate, Property, Rule, Schema, ValidSchema, ValidationReport, ValuePath,
    field_key,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

use crate::actions::datetime::DurationUnit;

/// Maximum duration `core.delay` will park for: 24 hours, in milliseconds.
///
/// A `for` delay whose computed duration exceeds this ceiling is clamped to it
/// (with a `warn`). This bounds a single timer-park so a mis-specified workflow
/// cannot pin a node in `Waiting` for an unbounded span. It is the wait ceiling
/// for this action specifically — unrelated to any storage TTL constant.
pub const MAX_DELAY_MILLIS: u64 = 86_400_000;

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
    /// [`MAX_DELAY_MILLIS`].
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

impl HasSchema for DelayInput {
    fn schema() -> Result<ValidSchema, ValidationReport> {
        static SCHEMA: OnceLock<Result<ValidSchema, ValidationReport>> = OnceLock::new();
        SCHEMA.get_or_init(build_delay_input_schema).clone()
    }
}

#[instrument(name = "core.delay.schema", skip_all, err)]
fn build_delay_input_schema() -> Result<ValidSchema, ValidationReport> {
    let for_mode = Predicate::Eq(ValuePath::root().push("mode"), "for".into());
    let until_mode = Predicate::Eq(ValuePath::root().push("mode"), "until".into());
    let requires_for = super::input_schema::admit_rule(Rule::predicate(for_mode.clone()))?;
    let requires_for_unit = super::input_schema::admit_rule(Rule::predicate(for_mode))?;
    let requires_until = super::input_schema::admit_rule(Rule::predicate(until_mode))?;

    Schema::builder()
        .property(
            Property::select(field_key!("mode"))
                .option("for", "For a duration")
                .option("until", "Until an instant")
                .required(),
        )
        .property(
            Property::integer(field_key!("amount"))
                .min_int(1)
                .required_when(requires_for),
        )
        .property(
            Property::select(field_key!("unit"))
                .option("milliseconds", "Milliseconds")
                .option("seconds", "Seconds")
                .option("minutes", "Minutes")
                .option("hours", "Hours")
                .option("days", "Days")
                .option("weeks", "Weeks")
                .required_when(requires_for_unit),
        )
        .property(Property::string(field_key!("datetime")).required_when(requires_until))
        .property(Property::dynamic(field_key!("data")))
        .build()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build the wait duration (in milliseconds) for a `for` spec, applying
/// validation and the 24h clamp.
fn build_delay_millis(amount: i64, unit: DurationUnit) -> Result<u64, ActionError> {
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
    let millis_per = unit.millis_per_unit();
    let total_millis = amount.checked_mul(millis_per).ok_or_else(|| {
        ActionError::fatal(format!(
            "core.delay: duration overflow computing {amount} × {millis_per} milliseconds"
        ))
    })?;
    // `total_millis > 0` here (amount > 0, millis_per ≥ 1), so the cast is safe.
    let requested_millis = u64::try_from(total_millis).map_err(|_| {
        ActionError::fatal(format!(
            "core.delay: duration overflow: {total_millis} milliseconds is out of range"
        ))
    })?;
    if requested_millis > MAX_DELAY_MILLIS {
        tracing::warn!(
            target = "core.delay",
            requested_millis,
            clamped_to = MAX_DELAY_MILLIS,
            "delay exceeds 24h ceiling; clamping"
        );
        return Ok(MAX_DELAY_MILLIS);
    }
    Ok(requested_millis)
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

    fn metadata() -> nebula_action::ActionMetadataDraft {
        nebula_action::ActionMetadataDraft::new(
            action_key!("core.delay"),
            nebula_action::metadata_name!("Delay"),
            "Parks the execution for a fixed duration or until a timestamp, then resumes",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
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
                let millis = build_delay_millis(amount, unit)?;
                WaitCondition::Duration {
                    duration: StdDuration::from_millis(millis),
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
#[path = "delay_tests.rs"]
mod tests;
