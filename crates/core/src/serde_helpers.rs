//! Shared serde helpers for Nebula crates.
//!
//! Use these via the `#[serde(with = "nebula_core::serde_helpers::duration_opt_ms")]`
//! attribute to avoid duplicating the same helpers across crates.

/// Serde support for `Option<Duration>` as an optional u64 of milliseconds.
///
/// ```rust
/// use std::time::Duration;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct Config {
///     #[serde(default, with = "nebula_core::serde_helpers::duration_opt_ms")]
///     timeout: Option<Duration>,
/// }
///
/// let c = Config { timeout: Some(Duration::from_millis(5000)) };
/// let json = serde_json::to_string(&c).unwrap();
/// assert_eq!(json, r#"{"timeout":5000}"#);
///
/// let c2: Config = serde_json::from_str(&json).unwrap();
/// assert_eq!(c2.timeout, Some(Duration::from_millis(5000)));
/// ```
pub mod duration_opt_ms {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    /// Serialize `Option<Duration>` as an optional `u64` of milliseconds.
    pub fn serialize<S: Serializer>(
        duration: &Option<Duration>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(d) => (d.as_millis() as u64).serialize(s),
            None => s.serialize_none(),
        }
    }

    /// Deserialize an optional `u64` of milliseconds into `Option<Duration>`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let opt: Option<u64> = Option::deserialize(d)?;
        Ok(opt.map(Duration::from_millis))
    }
}
