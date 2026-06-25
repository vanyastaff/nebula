//! [`BranchKey`] — validated newtype for workflow branch identifiers.
//!
//! A `BranchKey` names a specific branch selected by an action (e.g. `"true"`,
//! `"false"`, `"case_1"`). It shares the same validation rules as
//! [`PortKey`](crate::PortKey) but is a **distinct type**: assigning a
//! `BranchKey` where a `PortKey` is required (or vice-versa) is a compile-time
//! type error.
//!
//! # Serde security contract
//!
//! `#[serde(try_from = "String", into = "String")]` routes deserialization
//! through [`TryFrom<String>`] so invalid branch names are rejected at the
//! serde boundary.
//!
//! # Compile-time literals
//!
//! Use the [`branch_key!`](crate::branch_key!) macro for hard-coded literals.
//!
//! ```rust
//! use nebula_action::branch_key;
//! let k = branch_key!("true");
//! assert_eq!(k.as_str(), "true");
//! ```

use std::borrow::Borrow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::key_validation::{KeyValidationError, is_valid_key, validate_key};

// ── BranchKey ────────────────────────────────────────────────────────────────

/// A validated branch key identifying a selected workflow branch.
///
/// Valid keys satisfy:
/// - Non-empty, ≤ 64 bytes.
/// - Characters `[a-zA-Z0-9_-]` only (case-sensitive).
/// - No leading or trailing `_`/`-`.
/// - No consecutive `--` or `__`.
///
/// Construct from a literal with [`branch_key!`](crate::branch_key!), or at
/// runtime via [`BranchKey::new`] / [`TryFrom`].
///
/// The [`Borrow<str>`] and [`AsRef<str>`] impls allow `HashMap<BranchKey, _>`
/// to be queried with a bare `&str` key.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BranchKey(String);

impl BranchKey {
    /// Construct a `BranchKey`, validating the supplied string.
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
    /// Used by the [`branch_key!`](crate::branch_key!) macro's compile-time
    /// `const assert!`.
    #[must_use]
    pub const fn is_valid_key_const(s: &str) -> bool {
        is_valid_key(s)
    }
}

// ── Standard trait impls ─────────────────────────────────────────────────────

impl fmt::Debug for BranchKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BranchKey({:?})", self.0)
    }
}

impl fmt::Display for BranchKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for BranchKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// `Borrow<str>` is required so `HashMap<BranchKey, _>::get("true")` works
/// without an owned `BranchKey`. The `Hash` and `PartialEq` impls delegate to
/// `str` (via the derived impls on the inner `String`), satisfying the
/// `Borrow` contract: `hash(key) == hash(key.borrow())`.
impl Borrow<str> for BranchKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BranchKey {
    type Error = KeyValidationError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        validate_key(&s).map_err(|kind| KeyValidationError::new(s.clone(), kind))?;
        Ok(Self(s))
    }
}

impl TryFrom<&str> for BranchKey {
    type Error = KeyValidationError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        validate_key(s).map_err(|kind| KeyValidationError::new(s, kind))?;
        Ok(Self(s.to_owned()))
    }
}

/// Required by `#[serde(into = "String")]`.
impl From<BranchKey> for String {
    fn from(k: BranchKey) -> Self {
        k.0
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::key_validation::KeyValidationErrorKind;

    // ── Construction ─────────────────────────────────────────────────────────

    #[test]
    fn valid_branch_keys() {
        for key in ["true", "false", "default", "case-1", "case_1", "a"] {
            assert!(BranchKey::new(key).is_ok(), "{key:?} should be valid");
        }
    }

    #[test]
    fn invalid_empty() {
        assert!(matches!(
            BranchKey::new("").unwrap_err().kind,
            KeyValidationErrorKind::Empty
        ));
    }

    #[test]
    fn invalid_leading_underscore() {
        assert!(matches!(
            BranchKey::new("_x").unwrap_err().kind,
            KeyValidationErrorKind::LeadingOrTrailingSeparator
        ));
    }

    #[test]
    fn invalid_consecutive_dash() {
        assert!(matches!(
            BranchKey::new("a--b").unwrap_err().kind,
            KeyValidationErrorKind::ConsecutiveSeparator
        ));
    }

    #[test]
    fn invalid_space() {
        assert!(matches!(
            BranchKey::new("bad branch").unwrap_err().kind,
            KeyValidationErrorKind::InvalidChar { ch: ' ', .. }
        ));
    }

    // ── Type distinctness: BranchKey ≠ PortKey ───────────────────────────────

    // Enforced at compile time: the following would be a type error:
    //   let _: PortKey = BranchKey::new("out").unwrap();
    // No runtime test is needed; the absence of From<BranchKey> for PortKey
    // makes the wrong assignment a compiler error.

    // ── HashMap Borrow<str> ──────────────────────────────────────────────────

    #[test]
    fn hashmap_get_by_str_via_borrow() {
        let mut map: HashMap<BranchKey, &str> = HashMap::new();
        map.insert(BranchKey::new("true").unwrap(), "yes");
        map.insert(BranchKey::new("false").unwrap(), "no");

        assert_eq!(map.get("true"), Some(&"yes"));
        assert_eq!(map.get("false"), Some(&"no"));
        assert_eq!(map.get("missing"), None);
        assert!(map.contains_key("true"));
    }

    // ── Serde round-trip ─────────────────────────────────────────────────────

    #[test]
    fn serde_round_trip_valid() {
        let key = BranchKey::new("true").unwrap();
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, r#""true""#);
        let back: BranchKey = serde_json::from_str(&json).unwrap();
        assert_eq!(key, back);
    }

    #[test]
    fn serde_rejects_forged_invalid_char() {
        let result: Result<BranchKey, _> = serde_json::from_str(r#""bad branch!""#);
        assert!(
            result.is_err(),
            "BranchKey deserialization must reject keys with invalid characters"
        );
    }

    #[test]
    fn serde_rejects_empty_key() {
        let result: Result<BranchKey, _> = serde_json::from_str(r#""""#);
        assert!(
            result.is_err(),
            "BranchKey deserialization must reject empty string"
        );
    }

    // ── Display / Debug ──────────────────────────────────────────────────────

    #[test]
    fn display_shows_key_string() {
        let key = BranchKey::new("true").unwrap();
        assert_eq!(key.to_string(), "true");
    }

    #[test]
    fn debug_is_readable() {
        let key = BranchKey::new("true").unwrap();
        assert_eq!(format!("{key:?}"), r#"BranchKey("true")"#);
    }

    // ── From<BranchKey> for String ────────────────────────────────────────────

    #[test]
    fn into_string_conversion() {
        let key = BranchKey::new("false").unwrap();
        let s: String = key.into();
        assert_eq!(s, "false");
    }

    // ── TryFrom<&str> ────────────────────────────────────────────────────────

    #[test]
    fn try_from_str_valid() {
        let key = BranchKey::try_from("true").unwrap();
        assert_eq!(key.as_str(), "true");
    }

    #[test]
    fn try_from_str_invalid() {
        assert!(BranchKey::try_from("bad!").is_err());
    }

    // ── is_valid_key_const ───────────────────────────────────────────────────

    #[test]
    fn const_check_valid() {
        assert!(BranchKey::is_valid_key_const("true"));
        assert!(BranchKey::is_valid_key_const("false"));
        assert!(BranchKey::is_valid_key_const("default"));
    }

    #[test]
    fn const_check_invalid() {
        assert!(!BranchKey::is_valid_key_const(""));
        assert!(!BranchKey::is_valid_key_const("_x"));
        assert!(!BranchKey::is_valid_key_const("a--b"));
    }
}
