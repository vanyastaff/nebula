//! Date and time functions

use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::ExpressionResult,
    eval::{Argument, BuiltinView},
    value::RuntimeValue,
};

use super::{check_arg_count, check_min_arg_count, get_value_arg};

fn preflight_string_output(
    view: BuiltinView<'_>,
    context: &EvaluationContext,
    output_bytes: usize,
) -> ExpressionResult<()> {
    view.check_output_bytes(output_bytes)?;
    let output = view.output_builder(context);
    output.ensure_string_bytes(output_bytes)?;
    output.ensure_total_bytes(output_bytes)
}

/// Parse an IANA timezone name into a `chrono_tz::Tz`.
///
/// Used by every datetime builtin that accepts an optional `tz` argument.
/// Returns a typed error without echoing the runtime value.
fn parse_timezone(function: &str, name: &str) -> ExpressionResult<Tz> {
    name.parse::<Tz>().map_err(|_| {
        ExpressionError::invalid_argument(
            function,
            "Unknown timezone; expected an IANA name like 'Europe/Moscow' or 'UTC'",
        )
    })
}

/// Optional 0-based timezone arg shared by `format_date` / `parse_date`.
///
/// Returns `Ok(None)` if the slot doesn't exist; `Err` if it exists but
/// isn't a string or names an unknown zone.
fn optional_tz_arg(
    function: &str,
    args: &[Argument<'_>],
    index: usize,
) -> ExpressionResult<Option<Tz>> {
    let Some(argument) = args.get(index) else {
        return Ok(None);
    };
    let Some(raw) = argument.as_value() else {
        return Err(ExpressionError::invalid_argument(
            function,
            "timezone argument must be a value",
        ));
    };
    let name = raw.as_str().ok_or_else(|| {
        ExpressionError::type_error("string", crate::value_utils::value_type_name(raw))
    })?;
    parse_timezone(function, name).map(Some)
}

/// Get current timestamp as Unix seconds
pub(crate) fn now(
    _args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    let now = Utc::now().timestamp();
    Ok(RuntimeValue::Integer(now))
}

/// Get current date/time as ISO 8601 string
pub(crate) fn now_iso(
    _args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    let now = Utc::now();
    Ok(RuntimeValue::string(now.to_rfc3339()))
}

/// Format a timestamp or date value.
///
/// Signature: `format_date(value, format_or_tz?, tz?)`
/// - `value`: date value, Unix timestamp (integer), or ISO/common date string.
/// - 2-arg form: `format_date(value, x)` first tries `x` as an IANA timezone name (so
///   `format_date(0, "Europe/Moscow")` does what most callers mean — render `value` in Moscow time
///   as RFC 3339). If `x` doesn't parse as a known timezone, it is treated as a format string. This
///   avoids the previous "unreachable tz-only" trap where passing only a timezone silently fell
///   through to the format-string path.
/// - 3-arg form: `format_date(value, format, tz)` is unambiguous — `format` is the strftime-style
///   template, `tz` is the timezone.
/// - When `tz` is omitted output is in UTC; when `format` is omitted RFC 3339 is used. Unknown
///   timezone names in the explicit-tz slot yield a typed error.
pub(crate) fn format_date(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("format_date", args, 1)?;
    if args.len() > 3 {
        return Err(ExpressionError::invalid_argument(
            "format_date",
            format!("expected 1-3 arguments, got {}", args.len()),
        ));
    }

    let utc_dt = parse_datetime(get_value_arg("format_date", args, 0, "value")?)?;

    let (format_str, tz) = match args.len() {
        1 => (None, None),
        2 => {
            let argument = get_value_arg("format_date", args, 1, "format_or_tz")?;
            let arg1 = argument.as_str().ok_or_else(|| {
                ExpressionError::type_error("string", crate::value_utils::value_type_name(argument))
            })?;
            // Probe-parse as IANA timezone. Success → tz-only call;
            // failure → treat as format string (legacy 2-arg shape).
            if let Ok(tz) = arg1.parse::<Tz>() {
                (None, Some(tz))
            } else {
                (Some(arg1), None)
            }
        },
        _ => {
            let fmt = get_value_arg("format_date", args, 1, "format")?
                .as_str()
                .ok_or_else(|| {
                    ExpressionError::type_error(
                        "string",
                        crate::value_utils::value_type_name(
                            get_value_arg("format_date", args, 1, "format")
                                .map_or(&RuntimeValue::Null, |value| value),
                        ),
                    )
                })?;
            let tz = optional_tz_arg("format_date", args, 2)?;
            (Some(fmt), tz)
        },
    };

    let rendered = match (format_str, tz) {
        (None, None) => utc_dt.to_rfc3339(),
        (None, Some(tz)) => utc_dt.with_timezone(&tz).to_rfc3339(),
        (Some(fmt), None) => format_datetime(&utc_dt, fmt, view, ctx)?,
        (Some(fmt), Some(tz)) => format_datetime(&utc_dt.with_timezone(&tz), fmt, view, ctx)?,
    };
    preflight_string_output(view, ctx, rendered.len())?;

    Ok(RuntimeValue::string(rendered))
}

/// Parse a date string to a date value.
///
/// Signature: `parse_date(value, tz?)`
/// - `value`: date value, timestamp (integer), or date string.
/// - `tz`: optional IANA timezone name. When the input string has no embedded offset, it is
///   interpreted as wall time in `tz` (UTC by default). Strings that already carry a `+HH:MM` / `Z`
///   suffix ignore `tz` and round-trip exactly.
pub(crate) fn parse_date(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("parse_date", args, 1)?;
    if args.len() > 2 {
        return Err(ExpressionError::invalid_argument(
            "parse_date",
            format!("expected 1-2 arguments, got {}", args.len()),
        ));
    }

    let tz = optional_tz_arg("parse_date", args, 1)?;
    let value = get_value_arg("parse_date", args, 0, "value")?;
    let dt = match tz {
        Some(tz) => parse_datetime_in_tz(value, tz)?,
        None => parse_datetime(value)?,
    };
    Ok(RuntimeValue::DateTime(dt))
}

/// A date shift, split into the two kinds chrono can express.
///
/// `months` carries calendar months and years (which are not fixed-length
/// durations), `duration` carries every fixed-length unit.
struct DateShift {
    months: i64,
    duration: chrono::Duration,
}

/// Build a [`DateShift`] for a unit string and an (untrusted) `amount`,
/// rejecting an unknown unit or an out-of-range magnitude with a typed error.
///
/// Uses the non-panicking `try_*` constructors: `Duration::weeks`/`days`/… panic
/// on overflow, and `amount` comes from workflow/template input.
fn shift_for_unit(fn_name: &str, unit: &str, amount: i64) -> ExpressionResult<DateShift> {
    let (months, duration) = match unit.to_lowercase().as_str() {
        "seconds" | "second" | "s" => (0, chrono::Duration::try_seconds(amount)),
        "minutes" | "minute" | "m" => (0, chrono::Duration::try_minutes(amount)),
        "hours" | "hour" | "h" => (0, chrono::Duration::try_hours(amount)),
        "days" | "day" | "d" => (0, chrono::Duration::try_days(amount)),
        "weeks" | "week" | "w" => (0, chrono::Duration::try_weeks(amount)),
        // Calendar units: a month is not a fixed number of seconds, so these
        // must go through `checked_add_months` / `checked_sub_months`.
        "months" | "month" => (amount, Some(chrono::Duration::zero())),
        "years" | "year" | "y" => (
            amount.checked_mul(12).ok_or_else(|| {
                ExpressionError::invalid_argument(fn_name, "Duration is out of range")
            })?,
            Some(chrono::Duration::zero()),
        ),
        _ => {
            return Err(ExpressionError::invalid_argument(
                fn_name,
                "Invalid duration unit",
            ));
        },
    };
    let duration = duration
        .ok_or_else(|| ExpressionError::invalid_argument(fn_name, "Duration is out of range"))?;
    Ok(DateShift { months, duration })
}

impl DateShift {
    /// Apply the shift forward, clamping to the last valid day of the target
    /// month the way `chrono` does (`Jan 31 + 1 month = Feb 29`).
    fn add(
        self,
        dt: DateTime<FixedOffset>,
        fn_name: &str,
    ) -> ExpressionResult<DateTime<FixedOffset>> {
        let shifted = dt
            .checked_add_months(chrono::Months::new(u32::try_from(self.months).map_err(
                |_| ExpressionError::invalid_argument(fn_name, "Duration is out of range"),
            )?))
            .and_then(|dt| dt.checked_add_signed(self.duration))
            .ok_or_else(|| {
                ExpressionError::invalid_argument(
                    fn_name,
                    "Date addition overflows the representable date range",
                )
            })?;
        Ok(shifted)
    }

    /// Apply the shift backward.
    fn subtract(
        self,
        dt: DateTime<FixedOffset>,
        fn_name: &str,
    ) -> ExpressionResult<DateTime<FixedOffset>> {
        let shifted = dt
            .checked_sub_months(chrono::Months::new(u32::try_from(self.months).map_err(
                |_| ExpressionError::invalid_argument(fn_name, "Duration is out of range"),
            )?))
            .and_then(|dt| dt.checked_sub_signed(self.duration))
            .ok_or_else(|| {
                ExpressionError::invalid_argument(
                    fn_name,
                    "Date subtraction overflows the representable date range",
                )
            })?;
        Ok(shifted)
    }
}

/// Add a calendar or fixed-length shift to a date
pub(crate) fn date_add(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_add", args, 3)?;

    let dt = parse_datetime(get_value_arg("date_add", args, 0, "value")?)?;
    let amount_value = get_value_arg("date_add", args, 1, "amount")?;
    let amount = amount_value.as_i64().ok_or_else(|| {
        ExpressionError::type_error("integer", crate::value_utils::value_type_name(amount_value))
    })?;
    let unit_value = get_value_arg("date_add", args, 2, "unit")?;
    let unit = unit_value.as_str().ok_or_else(|| {
        ExpressionError::type_error("string", crate::value_utils::value_type_name(unit_value))
    })?;

    let shift = shift_for_unit("date_add", unit, amount)?;
    Ok(RuntimeValue::DateTime(shift.add(dt, "date_add")?))
}

/// Subtract duration from a date
pub(crate) fn date_subtract(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_subtract", args, 3)?;

    let dt = parse_datetime(get_value_arg("date_subtract", args, 0, "value")?)?;
    let amount_value = get_value_arg("date_subtract", args, 1, "amount")?;
    let amount = amount_value.as_i64().ok_or_else(|| {
        ExpressionError::type_error("integer", crate::value_utils::value_type_name(amount_value))
    })?;
    let unit_value = get_value_arg("date_subtract", args, 2, "unit")?;
    let unit = unit_value.as_str().ok_or_else(|| {
        ExpressionError::type_error("string", crate::value_utils::value_type_name(unit_value))
    })?;

    let shift = shift_for_unit("date_subtract", unit, amount)?;
    Ok(RuntimeValue::DateTime(shift.subtract(dt, "date_subtract")?))
}

/// Get difference between two dates in specified unit
pub(crate) fn date_diff(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_diff", args, 3)?;

    let dt1 = parse_datetime(get_value_arg("date_diff", args, 0, "value")?)?;
    let dt2 = parse_datetime(get_value_arg("date_diff", args, 1, "value")?)?;
    let unit_value = get_value_arg("date_diff", args, 2, "unit")?;
    let unit = unit_value.as_str().ok_or_else(|| {
        ExpressionError::type_error("string", crate::value_utils::value_type_name(unit_value))
    })?;

    let duration = dt1.signed_duration_since(dt2);

    let result = match unit.to_lowercase().as_str() {
        "seconds" | "second" | "s" => duration.num_seconds(),
        "minutes" | "minute" | "m" => duration.num_minutes(),
        "hours" | "hour" | "h" => duration.num_hours(),
        "days" | "day" | "d" => duration.num_days(),
        "weeks" | "week" | "w" => duration.num_weeks(),
        _ => {
            return Err(ExpressionError::invalid_argument(
                "date_diff",
                "Invalid duration unit",
            ));
        },
    };

    Ok(RuntimeValue::Integer(result))
}

/// Extract year from date
pub(crate) fn date_year(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_year", args, 1)?;
    let dt = parse_datetime(get_value_arg("date_year", args, 0, "value")?)?;
    Ok(RuntimeValue::Integer(i64::from(dt.year())))
}

/// Extract month from date (1-12)
pub(crate) fn date_month(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_month", args, 1)?;
    let dt = parse_datetime(get_value_arg("date_month", args, 0, "value")?)?;
    Ok(RuntimeValue::Integer(i64::from(dt.month())))
}

/// Extract day from date (1-31)
pub(crate) fn date_day(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_day", args, 1)?;
    let dt = parse_datetime(get_value_arg("date_day", args, 0, "value")?)?;
    Ok(RuntimeValue::Integer(i64::from(dt.day())))
}

/// Extract hour from date (0-23)
pub(crate) fn date_hour(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_hour", args, 1)?;
    let dt = parse_datetime(get_value_arg("date_hour", args, 0, "value")?)?;
    Ok(RuntimeValue::Integer(i64::from(dt.hour())))
}

/// Extract minute from date (0-59)
pub(crate) fn date_minute(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_minute", args, 1)?;
    let dt = parse_datetime(get_value_arg("date_minute", args, 0, "value")?)?;
    Ok(RuntimeValue::Integer(i64::from(dt.minute())))
}

/// Extract second from date (0-59)
pub(crate) fn date_second(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_second", args, 1)?;
    let dt = parse_datetime(get_value_arg("date_second", args, 0, "value")?)?;
    Ok(RuntimeValue::Integer(i64::from(dt.second())))
}

/// Get day of week (0=Sunday, 6=Saturday)
pub(crate) fn date_day_of_week(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("date_day_of_week", args, 1)?;
    let dt = parse_datetime(get_value_arg("date_day_of_week", args, 0, "value")?)?;
    let weekday = dt.weekday().num_days_from_sunday();
    Ok(RuntimeValue::Integer(i64::from(weekday)))
}

// Helper functions

/// Format strings tried in order when no explicit format is given.
/// Strings that fail RFC 3339 fall through to these naive (no offset)
/// patterns; midnight is assumed for date-only forms.
const NAIVE_DATE_FORMATS: &[&str] = &[
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%d",
    "%Y/%m/%d %H:%M:%S",
    "%Y/%m/%d",
    "%d.%m.%Y %H:%M:%S",
    "%d.%m.%Y",
];

/// Try to parse a string as a `NaiveDateTime` using the common formats.
///
/// Date-only inputs (e.g. `2024-01-01`) are extended to midnight of that
/// day via `NaiveDate::and_time(NaiveTime::MIN)` — an infallible
/// constructor that avoids the `expect`/`unwrap` panic-path that
/// `and_hms_opt(0, 0, 0)` would have introduced in library code.
fn parse_naive(s: &str) -> Option<NaiveDateTime> {
    for format in NAIVE_DATE_FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, format) {
            return Some(naive);
        }
        if let Ok(date) = chrono::NaiveDate::parse_from_str(s, format) {
            return Some(date.and_time(NaiveTime::MIN));
        }
    }
    None
}

/// Parse a runtime value into an absolute instant, interpreting any
/// naive (no-offset) string as UTC.
///
/// Date values pass through unchanged; integers are Unix seconds; strings are
/// RFC 3339 first, then the common naive formats.
fn parse_datetime(value: &RuntimeValue) -> ExpressionResult<DateTime<FixedOffset>> {
    match value {
        RuntimeValue::DateTime(dt) => Ok(*dt),
        RuntimeValue::Integer(timestamp) => Utc
            .timestamp_opt(*timestamp, 0)
            .single()
            .map(|dt| dt.fixed_offset())
            .ok_or_else(|| ExpressionError::eval_error("Invalid timestamp")),
        RuntimeValue::Unsigned(timestamp) => i64::try_from(*timestamp)
            .ok()
            .and_then(|timestamp| Utc.timestamp_opt(timestamp, 0).single())
            .map(|dt| dt.fixed_offset())
            .ok_or_else(|| ExpressionError::eval_error("Invalid timestamp")),
        RuntimeValue::String(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Ok(dt);
            }
            let naive = parse_naive(s)
                .ok_or_else(|| ExpressionError::eval_error("Cannot parse date string"))?;
            Ok(Utc.from_utc_datetime(&naive).fixed_offset())
        },
        _ => Err(ExpressionError::type_error(
            "date, integer, or string",
            crate::value_utils::value_type_name(value),
        )),
    }
}

/// Parse a runtime value into an absolute instant, interpreting naive strings
/// as wall time in the given timezone.
///
/// Numeric timestamps are absolute and ignore `tz`. Strings with embedded
/// offsets (`Z`, `+HH:MM`) also bypass `tz` — that information already
/// fully determines the instant.
fn parse_datetime_in_tz(value: &RuntimeValue, tz: Tz) -> ExpressionResult<DateTime<FixedOffset>> {
    match value {
        RuntimeValue::String(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Ok(dt);
            }
            let naive = parse_naive(s)
                .ok_or_else(|| ExpressionError::eval_error("Cannot parse date string"))?;
            // Naive wall time → tz → UTC. For ambiguous instants (DST
            // fall-back), pick the earliest interpretation; for skipped
            // instants (DST spring-forward), surface a typed error.
            tz.from_local_datetime(&naive)
                .earliest()
                .map(|dt| dt.fixed_offset())
                .ok_or_else(|| {
                    ExpressionError::eval_error(
                        "Local datetime does not exist in the requested timezone",
                    )
                })
        },
        _ => parse_datetime(value),
    }
}

/// Format datetime using a format string
/// Supports common format patterns:
/// - YYYY: 4-digit year
/// - YY: 2-digit year
/// - MM: 2-digit month
/// - M: month
/// - DD: 2-digit day
/// - D: day
/// - HH: 2-digit hour (24h)
/// - H: hour
/// - mm: 2-digit minute
/// - m: minute
/// - ss: 2-digit second
/// - s: second
fn format_datetime<TZ: TimeZone>(
    dt: &DateTime<TZ>,
    format: &str,
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<String>
where
    TZ::Offset: std::fmt::Display,
{
    use std::{borrow::Cow, fmt::Write};

    // Pre-compute all formatted values once to avoid repeated formatting
    let year = dt.year();
    let month = dt.month();
    let day = dt.day();
    let hour = dt.hour();
    let minute = dt.minute();
    let second = dt.second();

    // Use Cow to avoid allocation if no replacements are needed
    let mut result: Cow<'_, str> = Cow::Borrowed(format);

    // Pre-format numeric values with stack-allocated buffers
    let mut buf = String::with_capacity(4);

    // Replace in order from longest to shortest to avoid partial replacements
    if result.contains("YYYY") {
        buf.clear();
        let _ = write!(buf, "{year:04}");
        replace_format_token(&mut result, "YYYY", &buf, view, context)?;
    }
    if result.contains("YY") {
        buf.clear();
        let _ = write!(buf, "{:02}", year % 100);
        replace_format_token(&mut result, "YY", &buf, view, context)?;
    }
    if result.contains("MM") {
        buf.clear();
        let _ = write!(buf, "{month:02}");
        replace_format_token(&mut result, "MM", &buf, view, context)?;
    }
    if result.contains("DD") {
        buf.clear();
        let _ = write!(buf, "{day:02}");
        replace_format_token(&mut result, "DD", &buf, view, context)?;
    }
    if result.contains("HH") {
        buf.clear();
        let _ = write!(buf, "{hour:02}");
        replace_format_token(&mut result, "HH", &buf, view, context)?;
    }
    if result.contains("mm") {
        buf.clear();
        let _ = write!(buf, "{minute:02}");
        replace_format_token(&mut result, "mm", &buf, view, context)?;
    }
    if result.contains("ss") {
        buf.clear();
        let _ = write!(buf, "{second:02}");
        replace_format_token(&mut result, "ss", &buf, view, context)?;
    }

    // Single letter variants (after double-letter to avoid conflicts)
    // These use itoa-style formatting for efficiency
    if result.contains('M') {
        buf.clear();
        let _ = write!(buf, "{month}");
        replace_format_token(&mut result, "M", &buf, view, context)?;
    }
    if result.contains('D') {
        buf.clear();
        let _ = write!(buf, "{day}");
        replace_format_token(&mut result, "D", &buf, view, context)?;
    }
    if result.contains('H') {
        buf.clear();
        let _ = write!(buf, "{hour}");
        replace_format_token(&mut result, "H", &buf, view, context)?;
    }
    if result.contains('m') {
        buf.clear();
        let _ = write!(buf, "{minute}");
        replace_format_token(&mut result, "m", &buf, view, context)?;
    }
    if result.contains('s') {
        buf.clear();
        let _ = write!(buf, "{second}");
        replace_format_token(&mut result, "s", &buf, view, context)?;
    }

    Ok(result.into_owned())
}

fn replace_format_token(
    value: &mut std::borrow::Cow<'_, str>,
    token: &str,
    replacement: &str,
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<()> {
    let occurrences = value.matches(token).count();
    let removed = token.len().saturating_mul(occurrences);
    let added = replacement.len().saturating_mul(occurrences);
    let output_bytes = value.len().saturating_sub(removed).saturating_add(added);
    preflight_string_output(view, context, output_bytes)?;
    *value = std::borrow::Cow::Owned(value.replace(token, replacement));
    Ok(())
}
