//! Time on both hosts: RFC 3339 instants written the way the server writes them, and a pause that
//! works on the native runtime and in the browser.

use web_time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch. A clock set before 1970 reads as the epoch.
pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// `2026-10-08T20:34:40.939000Z`: UTC with microsecond precision, as execution history reports it.
pub(crate) fn rfc3339(unix_millis: i64) -> String {
    let seconds = unix_millis.div_euclid(1000);
    let millis = unix_millis.rem_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:06}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60,
        millis * 1000
    )
}

/// How long ago `then` was, as a person says it: `just now`, `5 min ago`, `3 h ago`, `2 days ago`.
pub(crate) fn ago(now: i64, then: i64) -> String {
    let seconds = (now - then).max(0) / 1000;
    match seconds {
        0..=44 => "just now".to_owned(),
        45..=3_599 => format!("{} min ago", (seconds + 30) / 60),
        3_600..=86_399 => format!("{} h ago", seconds / 3600),
        86_400..=172_799 => "yesterday".to_owned(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// A duration between two instants, as runs report it: `840 ms`, `3.2 s`, `2 min 05 s`.
pub(crate) fn duration(millis: i64) -> String {
    let millis = millis.max(0);
    match millis {
        0..=999 => format!("{millis} ms"),
        1_000..=59_999 => format!("{:.1} s", millis as f64 / 1000.0),
        _ => format!("{} min {:02} s", millis / 60_000, millis % 60_000 / 1000),
    }
}

/// Milliseconds since the epoch of `2026-10-08T20:34:40.939Z` or `...+02:00`; fractions beyond
/// milliseconds are dropped. `None` for anything else.
pub(crate) fn parse_rfc3339(text: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || !matches!(bytes[10], b'T' | b't')
    {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let mut rest = &text[19..];
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        let padded = format!("{:0<3}", &fraction[..digits.min(3)]);
        millis = padded.parse::<i64>().ok()?;
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        zone if zone.len() == 6 => {
            let sign = match zone.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let hours = zone.get(1..3)?.parse::<i64>().ok()?;
            let minutes = zone.get(4..6)?.parse::<i64>().ok()?;
            sign * (hours * 3600 + minutes * 60)
        },
        _ => return None,
    };
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    Some(seconds * 1000 + millis)
}

/// Days since 1970-01-01 of a calendar date (Howard Hinnant's `days_from_civil`).
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The calendar date of a day count since 1970-01-01 (Howard Hinnant's `civil_from_days`).
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Waits `millis` without blocking the host: a Tokio timer natively, `setTimeout` in the browser.
pub(crate) async fn sleep(millis: u32) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(std::time::Duration::from_millis(u64::from(millis))).await;
    #[cfg(target_arch = "wasm32")]
    {
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            let delay = i32::try_from(millis).unwrap_or(i32::MAX);
            if let Some(window) = web_sys::window() {
                // A refused timer resolves nothing; the caller's next read still happens on its turn.
                let _scheduled =
                    window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, delay);
            }
        });
        let _elapsed = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_read_as_the_server_writes_them() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000000Z");
        // 2026-10-08T20:34:40.939Z, an execution's creation time from the server.
        assert_eq!(rfc3339(1_791_491_680_939), "2026-10-08T20:34:40.939000Z");
        // A leap day and the last millisecond of a year.
        assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00.000000Z");
        assert_eq!(rfc3339(1_735_689_599_999), "2024-12-31T23:59:59.999000Z");
    }

    #[test]
    fn instants_parse_back_including_offsets() {
        for millis in [0, 951_782_400_000, 1_735_689_599_999, 1_791_491_680_939] {
            assert_eq!(parse_rfc3339(&rfc3339(millis)), Some(millis));
        }
        assert_eq!(
            parse_rfc3339("2026-10-08T22:34:40+02:00"),
            parse_rfc3339("2026-10-08T20:34:40Z")
        );
        assert_eq!(parse_rfc3339("8 Oct 2026"), None);
        assert_eq!(ago(100_000, 90_000), "just now");
        assert_eq!(ago(10 * 60_000, 0), "10 min ago");
        assert_eq!(ago(5 * 3_600_000, 0), "5 h ago");
        assert_eq!(ago(30 * 3_600_000, 0), "yesterday");
        assert_eq!(duration(840), "840 ms");
        assert_eq!(duration(3_240), "3.2 s");
        assert_eq!(duration(125_000), "2 min 05 s");
        assert_eq!(parse_rfc3339("2026-13-08T20:34:40Z"), None);
    }
}
