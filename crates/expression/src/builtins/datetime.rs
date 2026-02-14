//! Date and time functions

use super::{check_arg_count, check_min_arg_count};
use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Timelike, Utc};
use serde_json::Value;

/// Get current timestamp as Unix seconds
pub fn now(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    let now = Utc::now().timestamp();
    Ok(Value::Number(now.into()))
}

/// Get current date/time as ISO 8601 string
pub fn now_iso(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    let now = Utc::now();
    Ok(Value::String(now.to_rfc3339()))
}

/// Format a timestamp or date string
pub fn format_date(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("format_date", args, 1)?;

    let dt = parse_datetime(&args[0])?;

    if args.len() >= 2 {
        let format_str = args[1].as_str().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "string",
                crate::value_utils::value_type_name(&args[1]),
            )
        })?;

        let formatted = format_datetime(&dt, format_str)?;
        Ok(Value::String(formatted))
    } else {
        // Default: ISO 8601
        Ok(Value::String(dt.to_rfc3339()))
    }
}

/// Parse a date string to Unix timestamp
pub fn parse_date(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("parse_date", args, 1)?;

    let dt = parse_datetime(&args[0])?;
    Ok(Value::Number(dt.timestamp().into()))
}

/// Add duration to a date
pub fn date_add(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_add", args, 3)?;

    let dt = parse_datetime(&args[0])?;
    let amount = args[1].as_i64().ok_or_else(|| {
        ExpressionError::type_error("integer", crate::value_utils::value_type_name(&args[1]))
    })?;
    let unit = args[2].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[2]),
        )
    })?;

    let new_dt = match unit.to_lowercase().as_str() {
        "seconds" | "second" | "s" => dt + chrono::Duration::seconds(amount),
        "minutes" | "minute" | "m" => dt + chrono::Duration::minutes(amount),
        "hours" | "hour" | "h" => dt + chrono::Duration::hours(amount),
        "days" | "day" | "d" => dt + chrono::Duration::days(amount),
        "weeks" | "week" | "w" => dt + chrono::Duration::weeks(amount),
        _ => {
            return Err(ExpressionError::expression_invalid_argument(
                "date_add",
                format!("Invalid unit: {}", unit),
            ));
        }
    };

    Ok(Value::Number(new_dt.timestamp().into()))
}

/// Subtract duration from a date
pub fn date_subtract(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_subtract", args, 3)?;

    let dt = parse_datetime(&args[0])?;
    let amount = args[1].as_i64().ok_or_else(|| {
        ExpressionError::type_error("integer", crate::value_utils::value_type_name(&args[1]))
    })?;
    let unit = args[2].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[2]),
        )
    })?;

    let new_dt = match unit.to_lowercase().as_str() {
        "seconds" | "second" | "s" => dt - chrono::Duration::seconds(amount),
        "minutes" | "minute" | "m" => dt - chrono::Duration::minutes(amount),
        "hours" | "hour" | "h" => dt - chrono::Duration::hours(amount),
        "days" | "day" | "d" => dt - chrono::Duration::days(amount),
        "weeks" | "week" | "w" => dt - chrono::Duration::weeks(amount),
        _ => {
            return Err(ExpressionError::expression_invalid_argument(
                "date_subtract",
                format!("Invalid unit: {}", unit),
            ));
        }
    };

    Ok(Value::Number(new_dt.timestamp().into()))
}

/// Get difference between two dates in specified unit
pub fn date_diff(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_diff", args, 3)?;

    let dt1 = parse_datetime(&args[0])?;
    let dt2 = parse_datetime(&args[1])?;
    let unit = args[2].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[2]),
        )
    })?;

    let duration = dt1.signed_duration_since(dt2);

    let result = match unit.to_lowercase().as_str() {
        "seconds" | "second" | "s" => duration.num_seconds(),
        "minutes" | "minute" | "m" => duration.num_minutes(),
        "hours" | "hour" | "h" => duration.num_hours(),
        "days" | "day" | "d" => duration.num_days(),
        "weeks" | "week" | "w" => duration.num_weeks(),
        _ => {
            return Err(ExpressionError::expression_invalid_argument(
                "date_diff",
                format!("Invalid unit: {}", unit),
            ));
        }
    };

    Ok(Value::Number(result.into()))
}

/// Extract year from date
pub fn date_year(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_year", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::Number((dt.year() as i64).into()))
}

/// Extract month from date (1-12)
pub fn date_month(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_month", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::Number((dt.month() as i64).into()))
}

/// Extract day from date (1-31)
pub fn date_day(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_day", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::Number((dt.day() as i64).into()))
}

/// Extract hour from date (0-23)
pub fn date_hour(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_hour", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::Number((dt.hour() as i64).into()))
}

/// Extract minute from date (0-59)
pub fn date_minute(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_minute", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::Number((dt.minute() as i64).into()))
}

/// Extract second from date (0-59)
pub fn date_second(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_second", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::Number((dt.second() as i64).into()))
}

/// Get day of week (0=Sunday, 6=Saturday)
pub fn date_day_of_week(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_day_of_week", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    let weekday = dt.weekday().num_days_from_sunday();
    Ok(Value::Number((weekday as i64).into()))
}

// Helper functions

/// Parse datetime from Value (can be timestamp or string)
fn parse_datetime(value: &Value) -> ExpressionResult<DateTime<Utc>> {
    match value {
        Value::Number(i) => {
            let timestamp = crate::value_utils::number_as_i64(i).ok_or_else(|| {
                ExpressionError::expression_eval_error("Invalid timestamp: not an integer")
            })?;
            let dt = Utc
                .timestamp_opt(timestamp, 0)
                .single()
                .ok_or_else(|| ExpressionError::expression_eval_error("Invalid timestamp"))?;
            Ok(dt)
        }
        Value::String(s) => {
            let s = s.as_str();

            // Try ISO 8601 / RFC 3339 first
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Ok(dt.with_timezone(&Utc));
            }

            // Try common formats
            let formats = [
                "%Y-%m-%d %H:%M:%S",
                "%Y-%m-%d",
                "%Y/%m/%d %H:%M:%S",
                "%Y/%m/%d",
                "%d.%m.%Y %H:%M:%S",
                "%d.%m.%Y",
            ];

            for format in &formats {
                if let Ok(naive) = NaiveDateTime::parse_from_str(s, format) {
                    return Ok(Utc.from_utc_datetime(&naive));
                }
                if let Ok(date) = chrono::NaiveDate::parse_from_str(s, format) {
                    let naive = date.and_hms_opt(0, 0, 0).unwrap();
                    return Ok(Utc.from_utc_datetime(&naive));
                }
            }

            Err(ExpressionError::expression_eval_error(format!(
                "Cannot parse date: {}",
                s
            )))
        }
        _ => Err(ExpressionError::expression_type_error(
            "integer or string",
            crate::value_utils::value_type_name(value),
        )),
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
fn format_datetime(dt: &DateTime<Utc>, format: &str) -> ExpressionResult<String> {
    use std::borrow::Cow;
    use std::fmt::Write;

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
        let _ = write!(buf, "{:04}", year);
        result = Cow::Owned(result.replace("YYYY", &buf));
    }
    if result.contains("YY") {
        buf.clear();
        let _ = write!(buf, "{:02}", year % 100);
        result = Cow::Owned(result.replace("YY", &buf));
    }
    if result.contains("MM") {
        buf.clear();
        let _ = write!(buf, "{:02}", month);
        result = Cow::Owned(result.replace("MM", &buf));
    }
    if result.contains("DD") {
        buf.clear();
        let _ = write!(buf, "{:02}", day);
        result = Cow::Owned(result.replace("DD", &buf));
    }
    if result.contains("HH") {
        buf.clear();
        let _ = write!(buf, "{:02}", hour);
        result = Cow::Owned(result.replace("HH", &buf));
    }
    if result.contains("mm") {
        buf.clear();
        let _ = write!(buf, "{:02}", minute);
        result = Cow::Owned(result.replace("mm", &buf));
    }
    if result.contains("ss") {
        buf.clear();
        let _ = write!(buf, "{:02}", second);
        result = Cow::Owned(result.replace("ss", &buf));
    }

    // Single letter variants (after double-letter to avoid conflicts)
    // These use itoa-style formatting for efficiency
    if result.contains('M') {
        buf.clear();
        let _ = write!(buf, "{}", month);
        result = Cow::Owned(result.replace('M', &buf));
    }
    if result.contains('D') {
        buf.clear();
        let _ = write!(buf, "{}", day);
        result = Cow::Owned(result.replace('D', &buf));
    }
    if result.contains('H') {
        buf.clear();
        let _ = write!(buf, "{}", hour);
        result = Cow::Owned(result.replace('H', &buf));
    }
    if result.contains('m') {
        buf.clear();
        let _ = write!(buf, "{}", minute);
        result = Cow::Owned(result.replace('m', &buf));
    }
    if result.contains('s') {
        buf.clear();
        let _ = write!(buf, "{}", second);
        result = Cow::Owned(result.replace('s', &buf));
    }

    Ok(result.into_owned())
}
