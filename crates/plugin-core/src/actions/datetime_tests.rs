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

/// Parse preserves a sub-second component while normalising the offset to UTC.
///
/// 2026-06-19T11:30:00.250+05:30 = 2026-06-19T06:00:00.250Z.
///
/// RED witness: with `SecondsFormat::Secs` the `.250` was truncated and this
/// returned `2026-06-19T06:00:00Z`.
#[tokio::test]
async fn parse_preserves_sub_second() {
    let out = extract_output(
        run(parse_input("2026-06-19T11:30:00.250+05:30"))
            .await
            .unwrap(),
    );
    assert_eq!(out, json!("2026-06-19T06:00:00.250Z"));
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
/// `Milliseconds` unit drives `chrono::Duration` on the millisecond base,
/// and that a whole-second result renders with no fractional part (byte
/// identical to the seconds base — `AutoSi` adds no `.0` suffix).
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

/// Sub-second add: 250 ms is preserved in the output (not truncated to a
/// whole second).
///
/// RED witness: with `SecondsFormat::Secs` the `.250` was silently dropped
/// and this returned `2026-06-19T00:00:00Z`.
#[tokio::test]
async fn add_milliseconds_sub_second_is_preserved() {
    let out = extract_output(
        run(add_input(
            "2026-06-19T00:00:00Z",
            250,
            DurationUnit::Milliseconds,
        ))
        .await
        .unwrap(),
    );
    assert_eq!(out, json!("2026-06-19T00:00:00.250Z"));
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

/// Sub-second subtract: 250 ms is preserved in the output. Crossing a second
/// boundary downward yields a fractional second (`...01` − 250 ms = `...00.750`).
#[tokio::test]
async fn subtract_milliseconds_sub_second_is_preserved() {
    let out = extract_output(
        run(sub_input(
            "2026-06-19T00:00:01Z",
            250,
            DurationUnit::Milliseconds,
        ))
        .await
        .unwrap(),
    );
    assert_eq!(out, json!("2026-06-19T00:00:00.750Z"));
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
    let factory = nebula_action::GenericStatelessFactory::<DateTimeAction>::new()
        .expect("datetime metadata must admit");
    assert_eq!(
        nebula_action::ActionFactory::metadata(&factory)
            .base()
            .key()
            .as_str(),
        "core.datetime"
    );
}

#[test]
fn action_display_name_is_datetime() {
    let factory = nebula_action::GenericStatelessFactory::<DateTimeAction>::new()
        .expect("datetime metadata must admit");
    assert_eq!(
        nebula_action::ActionFactory::metadata(&factory)
            .base()
            .name()
            .to_owned(),
        "DateTime"
    );
}
