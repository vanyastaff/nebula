//! Serde helpers for config value types that need custom deserialization.
//!
//! Use these with `#[serde(with = "nebula_config::serde::duration")]` in your structs.
//!
//! # What works natively (no helper needed)
//!
//! These types implement `serde::Deserialize` from a string automatically:
//!
//! ```rust,ignore
//! use serde::Deserialize;
//! use std::net::{IpAddr, SocketAddr};
//! use std::path::PathBuf;
//! use url::Url;
//! use chrono::{DateTime, Utc};
//!
//! #[derive(Deserialize)]
//! struct Config {
//!     endpoint: Url,           // "https://api.example.com"
//!     bind: SocketAddr,        // "0.0.0.0:8080"
//!     ip: IpAddr,              // "192.168.0.1"
//!     data_dir: PathBuf,       // "/var/data"
//!     created_at: DateTime<Utc>, // "2024-01-15T10:30:00Z"
//! }
//! ```
//!
//! # What needs a helper
//!
//! `std::time::Duration` has no standard string format, so we provide helpers
//! for the most common cases:
//!
//! ```rust,ignore
//! use serde::Deserialize;
//! use std::time::Duration;
//!
//! #[derive(Deserialize)]
//! struct Config {
//!     // From integer seconds: timeout = 30
//!     #[serde(with = "nebula_config::serde::duration_secs")]
//!     timeout: Duration,
//!
//!     // From integer milliseconds: poll = 500
//!     #[serde(with = "nebula_config::serde::duration_millis")]
//!     poll: Duration,
//!
//!     // From human string: ttl = "1h30m" or "30s" or "500ms"
//!     #[serde(with = "nebula_config::serde::duration_human")]
//!     ttl: Duration,
//! }
//! ```

use std::time::Duration;

// ── duration_secs ─────────────────────────────────────────────────────────────

/// Serialize/deserialize `Duration` as whole seconds (integer).
///
/// Config: `timeout = 30`
pub mod duration_secs {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}

// ── duration_millis ───────────────────────────────────────────────────────────

/// Serialize/deserialize `Duration` as whole milliseconds (integer).
///
/// Config: `poll = 500`
pub mod duration_millis {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_millis().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

// ── duration_human ────────────────────────────────────────────────────────────

/// Serialize/deserialize `Duration` from a human-readable string or integer seconds.
///
/// Accepted string formats: `"30s"`, `"5m"`, `"1h"`, `"100ms"`, `"1h30m"`, `"2h30m45s500ms"`
///
/// Supported units (must appear in descending order, each at most once):
/// - `h` — hours
/// - `m` — minutes
/// - `s` — seconds
/// - `ms` — milliseconds
///
/// Config examples:
/// ```toml
/// ttl = "1h30m"
/// retry_delay = "500ms"
/// session_timeout = "24h"
/// ```
pub mod duration_human {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        use serde::Serialize;
        // Serialize back as seconds integer (canonical form)
        d.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        d.deserialize_any(HumanDurationVisitor)
    }
}

struct HumanDurationVisitor;

impl serde::de::Visitor<'_> for HumanDurationVisitor {
    type Value = Duration;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a duration as integer seconds or string like \"30s\", \"5m\", \"1h30m\", \"100ms\""
        )
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Duration, E> {
        Ok(Duration::from_secs(v))
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Duration, E> {
        if v < 0 {
            return Err(E::custom("duration cannot be negative"));
        }
        Ok(Duration::from_secs(v as u64))
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Duration, E> {
        if v < 0.0 {
            return Err(E::custom("duration cannot be negative"));
        }
        Ok(Duration::from_secs_f64(v))
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Duration, E> {
        parse_human_duration(v).ok_or_else(|| {
            E::custom(format!(
                "invalid duration {v:?}; expected e.g. \"30s\", \"5m\", \"1h30m\", \"100ms\""
            ))
        })
    }
}

/// Parse a human-readable duration string into a `Duration`.
///
/// Units in descending order: `h`, `m`, `s`, `ms`.
/// Each unit may appear at most once.
pub(crate) fn parse_human_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // (label, milliseconds per unit) — "ms" listed after "s" but matched before "m"
    // via the guard below.
    const UNITS: &[(&str, u64)] = &[("h", 3_600_000), ("m", 60_000), ("s", 1_000), ("ms", 1)];

    let mut total_ms: u64 = 0;
    let mut rest = s;
    let mut found_any = false;
    let mut unit_idx = 0;

    while !rest.is_empty() && unit_idx < UNITS.len() {
        if !rest.starts_with(|c: char| c.is_ascii_digit()) {
            break;
        }

        let digit_end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let num_str = &rest[..digit_end];
        let after_digits = &rest[digit_end..];

        let mut matched = false;
        while unit_idx < UNITS.len() {
            let (label, ms_factor) = UNITS[unit_idx];
            // Guard: "m" must not eat the start of "ms"
            if label == "m" && after_digits.starts_with("ms") {
                unit_idx += 1;
                continue;
            }
            if after_digits.starts_with(label) {
                let n: u64 = num_str.parse().ok()?;
                total_ms = total_ms.checked_add(n.checked_mul(ms_factor)?)?;
                rest = &after_digits[label.len()..];
                unit_idx += 1;
                found_any = true;
                matched = true;
                break;
            }
            unit_idx += 1;
        }

        if !matched {
            return None;
        }
    }

    if !rest.is_empty() || !found_any {
        return None;
    }

    Some(Duration::new(
        total_ms / 1_000,
        (total_ms % 1_000) as u32 * 1_000_000,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    // ── parse_human_duration ──────────────────────────────────────────────────

    #[test]
    fn parse_single_units() {
        assert_eq!(parse_human_duration("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_human_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_human_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(
            parse_human_duration("500ms"),
            Some(Duration::from_millis(500))
        );
        assert_eq!(parse_human_duration("0s"), Some(Duration::ZERO));
    }

    #[test]
    fn parse_compound() {
        assert_eq!(
            parse_human_duration("1h30m"),
            Some(Duration::from_secs(5400))
        );
        assert_eq!(
            parse_human_duration("2h30m45s"),
            Some(Duration::from_secs(9045))
        );
        assert_eq!(
            parse_human_duration("1h30m45s500ms"),
            Some(Duration::from_millis(5445500))
        );
        assert_eq!(
            parse_human_duration("30m10s"),
            Some(Duration::from_secs(1810))
        );
    }

    #[test]
    fn parse_whitespace_trimmed() {
        assert_eq!(
            parse_human_duration("  30s  "),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(parse_human_duration(""), None);
        assert_eq!(parse_human_duration("abc"), None);
        assert_eq!(parse_human_duration("30"), None); // no unit
        assert_eq!(parse_human_duration("30x"), None); // unknown unit
        assert_eq!(parse_human_duration("-5s"), None); // negative
    }

    // ── duration_secs ─────────────────────────────────────────────────────────

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Secs {
        #[serde(with = "duration_secs")]
        d: Duration,
    }

    #[test]
    fn duration_secs_roundtrip() {
        let v = Secs {
            d: Duration::from_secs(30),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"d":30}"#);
        assert_eq!(serde_json::from_str::<Secs>(&json).unwrap(), v);
    }

    #[test]
    fn duration_secs_from_integer() {
        let v: Secs = serde_json::from_str(r#"{"d":60}"#).unwrap();
        assert_eq!(v.d, Duration::from_secs(60));
    }

    // ── duration_millis ───────────────────────────────────────────────────────

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Millis {
        #[serde(with = "duration_millis")]
        d: Duration,
    }

    #[test]
    fn duration_millis_roundtrip() {
        let v = Millis {
            d: Duration::from_millis(500),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"d":500}"#);
        assert_eq!(serde_json::from_str::<Millis>(&json).unwrap(), v);
    }

    // ── duration_human ────────────────────────────────────────────────────────

    #[derive(Debug, Deserialize, PartialEq)]
    struct Human {
        #[serde(with = "duration_human")]
        d: Duration,
    }

    #[test]
    fn duration_human_from_string() {
        let v: Human = serde_json::from_str(r#"{"d":"1h30m"}"#).unwrap();
        assert_eq!(v.d, Duration::from_secs(5400));
    }

    #[test]
    fn duration_human_from_integer() {
        let v: Human = serde_json::from_str(r#"{"d":30}"#).unwrap();
        assert_eq!(v.d, Duration::from_secs(30));
    }

    #[test]
    fn duration_human_invalid_errors() {
        assert!(serde_json::from_str::<Human>(r#"{"d":"badval"}"#).is_err());
    }
}
