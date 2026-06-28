//! [`PortKey`] — validated newtype for action port identifiers.
//!
//! A `PortKey` names a specific input or output connection point on an action
//! (e.g. `"in"`, `"out"`, `"error"`, `"tools"`). It enforces the charset and
//! structural rules shared with [`BranchKey`](crate::BranchKey) at construction
//! time, so an invalid port name can never reach the engine's routing tables.
//!
//! # Serde security contract
//!
//! `#[serde(try_from = "String", into = "String")]` routes deserialization
//! through [`TryFrom<String>`] — forged port names in JSON (e.g.
//! `"bad port!"`) are rejected at the serde boundary rather than accepted as
//! raw strings. `#[serde(transparent)]` would bypass this check.
//!
//! # Compile-time literals
//!
//! Use the [`port_key!`](crate::port_key!) macro for hard-coded literals;
//! invalid literals are rejected at compile time.
//!
//! ```rust
//! use nebula_core::port_key;
//! let k = port_key!("out");
//! assert_eq!(k.as_str(), "out");
//! ```

use std::borrow::Borrow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::key_validation::{KeyValidationError, is_valid_key, validate_key};

// ── PortKey ──────────────────────────────────────────────────────────────────

/// A validated port key identifying an input or output connection point.
///
/// Valid keys satisfy:
/// - Non-empty, ≤ 64 bytes.
/// - Characters `[a-zA-Z0-9_-]` only (case-sensitive).
/// - No leading or trailing `_`/`-`.
/// - No consecutive `--` or `__`.
///
/// Construct from a literal with [`port_key!`](crate::port_key!), or at runtime
/// via [`PortKey::new`] / [`TryFrom`].
///
/// The [`Borrow<str>`] and [`AsRef<str>`] impls allow `HashMap<PortKey, _>` to
/// be queried with a bare `&str` key (e.g.
/// `map.contains_key("out")`).
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PortKey(String);

impl PortKey {
    /// Construct a `PortKey`, validating the supplied string.
    ///
    /// # Errors
    ///
    /// Returns [`KeyValidationError`] when the string violates any key rule.
    pub fn new(s: impl Into<String>) -> Result<Self, KeyValidationError> {
        let s = s.into();
        validate_key(&s).map_err(|kind| KeyValidationError::new(s.clone(), kind))?;
        Ok(Self(s))
    }

    /// Borrow the key as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` when `s` satisfies all key rules.
    ///
    /// Used by the [`port_key!`](crate::port_key!) macro's compile-time
    /// `const assert!`.
    #[must_use]
    pub const fn is_valid_key_const(s: &str) -> bool {
        is_valid_key(s)
    }
}

// ── Standard trait impls ─────────────────────────────────────────────────────

impl fmt::Debug for PortKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PortKey({:?})", self.0)
    }
}

impl fmt::Display for PortKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for PortKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// `Borrow<str>` is required so `HashMap<PortKey, _>::get("out")` works
/// without an owned `PortKey`. The `Hash` and `PartialEq` impls delegate to
/// `str` (via the derived impls on the inner `String`), satisfying the
/// `Borrow` contract: `hash(key) == hash(key.borrow())`.
impl Borrow<str> for PortKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PortKey {
    type Error = KeyValidationError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        validate_key(&s).map_err(|kind| KeyValidationError::new(s.clone(), kind))?;
        Ok(Self(s))
    }
}

impl TryFrom<&str> for PortKey {
    type Error = KeyValidationError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        validate_key(s).map_err(|kind| KeyValidationError::new(s, kind))?;
        Ok(Self(s.to_owned()))
    }
}

/// Required by `#[serde(into = "String")]`.
impl From<PortKey> for String {
    fn from(k: PortKey) -> Self {
        k.0
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use crate::key_validation::KeyValidationErrorKind;

    // ── Construction ─────────────────────────────────────────────────────────

    #[test]
    fn valid_single_char() {
        assert_eq!(PortKey::new("a").unwrap().as_str(), "a");
    }

    #[test]
    fn valid_standard_keys() {
        for key in ["in", "out", "error", "main", "default", "true", "false"] {
            assert!(PortKey::new(key).is_ok(), "{key:?} should be valid");
        }
    }

    #[test]
    fn valid_max_length() {
        let s = "a".repeat(64);
        assert!(PortKey::new(s).is_ok());
    }

    #[test]
    fn valid_mixed_case_preserved() {
        // Keys are case-sensitive; "Main" and "main" are distinct.
        let upper = PortKey::new("Main").unwrap();
        let lower = PortKey::new("main").unwrap();
        assert_ne!(upper, lower, "case must be preserved");
        assert_eq!(upper.as_str(), "Main");
    }

    #[test]
    fn invalid_empty() {
        assert!(matches!(
            PortKey::new("").unwrap_err().kind,
            KeyValidationErrorKind::Empty
        ));
    }

    #[test]
    fn invalid_too_long() {
        let s = "a".repeat(65);
        assert!(matches!(
            PortKey::new(s).unwrap_err().kind,
            KeyValidationErrorKind::TooLong { actual: 65, .. }
        ));
    }

    #[test]
    fn invalid_leading_underscore() {
        assert!(matches!(
            PortKey::new("_x").unwrap_err().kind,
            KeyValidationErrorKind::LeadingOrTrailingSeparator
        ));
    }

    #[test]
    fn invalid_consecutive_dash() {
        assert!(matches!(
            PortKey::new("a--b").unwrap_err().kind,
            KeyValidationErrorKind::ConsecutiveSeparator
        ));
    }

    #[test]
    fn invalid_space() {
        assert!(matches!(
            PortKey::new("bad port").unwrap_err().kind,
            KeyValidationErrorKind::InvalidChar { ch: ' ', .. }
        ));
    }

    // ── is_valid_key_const ───────────────────────────────────────────────────

    #[test]
    fn const_check_accepts_valid() {
        assert!(PortKey::is_valid_key_const("out"));
        assert!(PortKey::is_valid_key_const("in"));
        assert!(PortKey::is_valid_key_const("error"));
    }

    #[test]
    fn const_check_rejects_invalid() {
        assert!(!PortKey::is_valid_key_const(""));
        assert!(!PortKey::is_valid_key_const("_x"));
        assert!(!PortKey::is_valid_key_const("a--b"));
        assert!(!PortKey::is_valid_key_const("bad port!"));
    }

    // ── HashMap Borrow<str> ──────────────────────────────────────────────────

    #[test]
    fn hashmap_get_by_str_via_borrow() {
        let mut map: HashMap<PortKey, u32> = HashMap::new();
        map.insert(PortKey::new("out").unwrap(), 42);
        map.insert(PortKey::new("error").unwrap(), 7);

        // Queried by &str — works via Borrow<str>.
        assert_eq!(map.get("out"), Some(&42));
        assert_eq!(map.get("error"), Some(&7));
        assert_eq!(map.get("missing"), None);
        assert!(map.contains_key("out"));
    }

    // ── Serde round-trip ─────────────────────────────────────────────────────

    #[test]
    fn serde_round_trip_valid() {
        let key = PortKey::new("out").unwrap();
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, r#""out""#);
        let back: PortKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key, back);
    }

    // ── SECURITY: forged port key rejected at serde boundary ─────────────────

    #[test]
    fn serde_rejects_forged_invalid_char() {
        // A forged key with an invalid character must be rejected on deserialization.
        // This would have been accepted when PortKey was `type PortKey = String`.
        let result: Result<PortKey, _> = serde_json::from_str(r#""bad port!""#);
        assert!(
            result.is_err(),
            "PortKey deserialization must reject keys with invalid characters"
        );
    }

    #[test]
    fn serde_rejects_empty_key() {
        let result: Result<PortKey, _> = serde_json::from_str(r#""""#);
        assert!(
            result.is_err(),
            "PortKey deserialization must reject empty string"
        );
    }

    // ── Display / Debug ──────────────────────────────────────────────────────

    #[test]
    fn display_shows_key_string() {
        let key = PortKey::new("out").unwrap();
        assert_eq!(key.to_string(), "out");
    }

    #[test]
    fn debug_is_readable() {
        let key = PortKey::new("out").unwrap();
        assert_eq!(format!("{key:?}"), r#"PortKey("out")"#);
    }

    // ── From<PortKey> for String ─────────────────────────────────────────────

    #[test]
    fn into_string_conversion() {
        let key = PortKey::new("out").unwrap();
        let s: String = key.into();
        assert_eq!(s, "out");
    }

    // ── TryFrom<&str> ────────────────────────────────────────────────────────

    #[test]
    fn try_from_str_valid() {
        let key = PortKey::try_from("out").unwrap();
        assert_eq!(key.as_str(), "out");
    }

    #[test]
    fn try_from_str_invalid() {
        assert!(PortKey::try_from("bad!").is_err());
    }

    // ── as_value in JSON object ──────────────────────────────────────────────

    #[test]
    fn port_key_in_json_object_serde() {
        let map: HashMap<PortKey, i32> = [
            (PortKey::new("out").unwrap(), 1),
            (PortKey::new("error").unwrap(), 2),
        ]
        .into_iter()
        .collect();

        // Serializes to a JSON object with string keys.
        let val = serde_json::to_value(&map).unwrap();
        assert_eq!(val["out"], json!(1));
        assert_eq!(val["error"], json!(2));

        // Round-trips back cleanly.
        let back: HashMap<PortKey, i32> = serde_json::from_value(val).unwrap();
        assert_eq!(back.get("out"), Some(&1));
        assert_eq!(back.get("error"), Some(&2));
    }
}
