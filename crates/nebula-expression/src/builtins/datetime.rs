//! Date and time functions

use super::{check_arg_count, check_min_arg_count};
use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Timelike, Utc};
use nebula_value::Value;

/// Get current timestamp as Unix seconds
pub fn now(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    let now = Utc::now().timestamp();
    Ok(Value::integer(now))
}

/// Get current date/time as ISO 8601 string
pub fn now_iso(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    let now = Utc::now();
    Ok(Value::text(now.to_rfc3339()))
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
            ExpressionError::expression_type_error("string", args[1].kind().name())
        })?;

        let formatted = format_datetime(&dt, format_str)?;
        Ok(Value::text(formatted))
    } else {
        // Default: ISO 8601
        Ok(Value::text(dt.to_rfc3339()))
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
    Ok(Value::integer(dt.timestamp()))
}

/// Add duration to a date
pub fn date_add(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_add", args, 3)?;

    let dt = parse_datetime(&args[0])?;
    let amount = args[1].to_integer()?;
    let unit = args[2]
        .as_str()
        .ok_or_else(|| ExpressionError::expression_type_error("string", args[2].kind().name()))?;

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

    Ok(Value::integer(new_dt.timestamp()))
}

/// Subtract duration from a date
pub fn date_subtract(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_subtract", args, 3)?;

    let dt = parse_datetime(&args[0])?;
    let amount = args[1].to_integer()?;
    let unit = args[2]
        .as_str()
        .ok_or_else(|| ExpressionError::expression_type_error("string", args[2].kind().name()))?;

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

    Ok(Value::integer(new_dt.timestamp()))
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
    let unit = args[2]
        .as_str()
        .ok_or_else(|| ExpressionError::expression_type_error("string", args[2].kind().name()))?;

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

    Ok(Value::integer(result))
}

/// Extract year from date
pub fn date_year(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_year", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::integer(dt.year() as i64))
}

/// Extract month from date (1-12)
pub fn date_month(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_month", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::integer(dt.month() as i64))
}

/// Extract day from date (1-31)
pub fn date_day(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_day", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::integer(dt.day() as i64))
}

/// Extract hour from date (0-23)
pub fn date_hour(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_hour", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::integer(dt.hour() as i64))
}

/// Extract minute from date (0-59)
pub fn date_minute(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_minute", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::integer(dt.minute() as i64))
}

/// Extract second from date (0-59)
pub fn date_second(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("date_second", args, 1)?;
    let dt = parse_datetime(&args[0])?;
    Ok(Value::integer(dt.second() as i64))
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
    Ok(Value::integer(weekday as i64))
}

// Helper functions

/// Parse datetime from Value (can be timestamp or string)
fn parse_datetime(value: &Value) -> ExpressionResult<DateTime<Utc>> {
    match value {
        Value::Integer(i) => {
            let timestamp = i.value();
            let dt = Utc
                .timestamp_opt(timestamp, 0)
                .single()
                .ok_or_else(|| ExpressionError::expression_eval_error("Invalid timestamp"))?;
            Ok(dt)
        }
        Value::Text(s) => {
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
            value.kind().name(),
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
    let mut result = format.to_string();

    // Replace in order from longest to shortest to avoid partial replacements
    result = result.replace("YYYY", &format!("{:04}", dt.year()));
    result = result.replace("YY", &format!("{:02}", dt.year() % 100));
    result = result.replace("MM", &format!("{:02}", dt.month()));
    result = result.replace("DD", &format!("{:02}", dt.day()));
    result = result.replace("HH", &format!("{:02}", dt.hour()));
    result = result.replace("mm", &format!("{:02}", dt.minute()));
    result = result.replace("ss", &format!("{:02}", dt.second()));

    // Single letter variants (after double-letter to avoid conflicts)
    result = result.replace("M", &format!("{}", dt.month()));
    result = result.replace("D", &format!("{}", dt.day()));
    result = result.replace("H", &format!("{}", dt.hour()));
    result = result.replace("m", &format!("{}", dt.minute()));
    result = result.replace("s", &format!("{}", dt.second()));

    Ok(result)
}
