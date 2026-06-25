//! Shared validation logic for port and branch key newtypes.
//!
//! Both [`PortKey`](crate::PortKey) and [`BranchKey`](crate::BranchKey) enforce
//! the same rules — only the type identity differs. This private module is the
//! single source of truth so the rules cannot drift between the two types.
//!
//! Validation rules (charset is **case-sensitive**):
//! - Non-empty, ≤ 64 characters.
//! - Characters: `[a-zA-Z0-9_-]` only.
//! - No leading or trailing `_` or `-`.
//! - No consecutive `--` or `__`.

/// Maximum allowed byte length of a port/branch key.
pub(crate) const MAX_KEY_LEN: usize = 64;

/// Error produced when constructing a [`PortKey`](crate::PortKey) or
/// [`BranchKey`](crate::BranchKey) from an invalid string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid key {key:?}: {kind}")]
pub struct KeyValidationError {
    /// The string that failed validation.
    pub key: String,
    /// The specific reason validation failed.
    pub kind: KeyValidationErrorKind,
}

impl KeyValidationError {
    pub(crate) fn new(key: impl Into<String>, kind: KeyValidationErrorKind) -> Self {
        Self {
            key: key.into(),
            kind,
        }
    }
}

/// Classification of a [`KeyValidationError`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KeyValidationErrorKind {
    /// Key has zero characters.
    #[error("key must not be empty")]
    Empty,
    /// Key exceeds the maximum allowed length.
    #[error("key is too long: {actual} bytes (max {max})")]
    TooLong {
        /// Maximum allowed length.
        max: usize,
        /// Actual length of the rejected key.
        actual: usize,
    },
    /// Key contains a character outside `[a-zA-Z0-9_-]`.
    #[error("key contains invalid character {ch:?} at position {pos}")]
    InvalidChar {
        /// The disallowed character.
        ch: char,
        /// Byte position in the key string.
        pos: usize,
    },
    /// Key starts or ends with `_` or `-`.
    #[error("key must not start or end with '_' or '-'")]
    LeadingOrTrailingSeparator,
    /// Key contains `--` or `__`.
    #[error("key must not contain consecutive separators ('--' or '__')")]
    ConsecutiveSeparator,
}

/// Validate `s` against the shared port/branch key rules.
///
/// Returns `Ok(())` on success, `Err(kind)` on the first rule violation.
pub(crate) fn validate_key(s: &str) -> Result<(), KeyValidationErrorKind> {
    if s.is_empty() {
        return Err(KeyValidationErrorKind::Empty);
    }
    if s.len() > MAX_KEY_LEN {
        return Err(KeyValidationErrorKind::TooLong {
            max: MAX_KEY_LEN,
            actual: s.len(),
        });
    }

    // Charset check.
    for (pos, ch) in s.char_indices() {
        if !matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-') {
            return Err(KeyValidationErrorKind::InvalidChar { ch, pos });
        }
    }

    // Leading/trailing separator.
    let first = s.chars().next().expect("non-empty string has a first char");
    let last = s
        .chars()
        .next_back()
        .expect("non-empty string has a last char");
    if matches!(first, '_' | '-') || matches!(last, '_' | '-') {
        return Err(KeyValidationErrorKind::LeadingOrTrailingSeparator);
    }

    // Consecutive separator.
    if s.contains("--") || s.contains("__") {
        return Err(KeyValidationErrorKind::ConsecutiveSeparator);
    }

    Ok(())
}

/// `const fn` validator used by the compile-time macro assertions.
///
/// Returns `true` when `s` satisfies all key rules, `false` otherwise.
/// Mirrors [`validate_key`] but operates at compile time.
pub(crate) const fn is_valid_key(s: &str) -> bool {
    let bytes = s.as_bytes();
    let len = bytes.len();

    if len == 0 || len > MAX_KEY_LEN {
        return false;
    }

    let first = bytes[0];
    let last = bytes[len - 1];
    if matches!(first, b'_' | b'-') || matches!(last, b'_' | b'-') {
        return false;
    }

    let mut i = 0;
    while i < len {
        let b = bytes[i];
        let valid_char = matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-');
        if !valid_char {
            return false;
        }
        // Consecutive separator check.
        if i + 1 < len && (b == b'-' || b == b'_') && bytes[i + 1] == b {
            return false;
        }
        i += 1;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Runtime validator ────────────────────────────────────────────────────

    #[test]
    fn empty_is_invalid() {
        assert_eq!(validate_key(""), Err(KeyValidationErrorKind::Empty));
    }

    #[test]
    fn too_long_is_invalid() {
        let long = "a".repeat(MAX_KEY_LEN + 1);
        assert!(matches!(
            validate_key(&long),
            Err(KeyValidationErrorKind::TooLong { actual, .. }) if actual == MAX_KEY_LEN + 1
        ));
    }

    #[test]
    fn exactly_max_length_is_valid() {
        let s = "a".repeat(MAX_KEY_LEN);
        assert!(validate_key(&s).is_ok());
    }

    #[test]
    fn invalid_char_space() {
        assert!(matches!(
            validate_key("bad key"),
            Err(KeyValidationErrorKind::InvalidChar { ch: ' ', .. })
        ));
    }

    #[test]
    fn invalid_char_dot() {
        assert!(matches!(
            validate_key("bad.key"),
            Err(KeyValidationErrorKind::InvalidChar { ch: '.', .. })
        ));
    }

    #[test]
    fn invalid_char_bang() {
        assert!(matches!(
            validate_key("bad!"),
            Err(KeyValidationErrorKind::InvalidChar { ch: '!', .. })
        ));
    }

    #[test]
    fn leading_underscore_is_invalid() {
        assert_eq!(
            validate_key("_x"),
            Err(KeyValidationErrorKind::LeadingOrTrailingSeparator)
        );
    }

    #[test]
    fn trailing_dash_is_invalid() {
        assert_eq!(
            validate_key("x-"),
            Err(KeyValidationErrorKind::LeadingOrTrailingSeparator)
        );
    }

    #[test]
    fn consecutive_dash_is_invalid() {
        assert_eq!(
            validate_key("a--b"),
            Err(KeyValidationErrorKind::ConsecutiveSeparator)
        );
    }

    #[test]
    fn consecutive_underscore_is_invalid() {
        assert_eq!(
            validate_key("a__b"),
            Err(KeyValidationErrorKind::ConsecutiveSeparator)
        );
    }

    #[test]
    fn valid_keys() {
        for s in [
            "in",
            "out",
            "error",
            "true",
            "false",
            "main",
            "default",
            "a",
            "rule-0",
            "rule_1",
            "CamelCase",
        ] {
            assert!(validate_key(s).is_ok(), "{s:?} should be valid");
        }
    }

    // ── Const validator ──────────────────────────────────────────────────────

    #[test]
    fn const_validator_matches_runtime_for_valid() {
        for s in ["in", "out", "error", "a", "rule-0", "Main", "default"] {
            assert!(is_valid_key(s), "{s:?}: const said invalid");
            assert!(validate_key(s).is_ok(), "{s:?}: runtime said invalid");
        }
    }

    #[test]
    fn const_validator_matches_runtime_for_invalid() {
        for s in ["", "_x", "a--b", "a__b", "bad key", "bad.key", "x-"] {
            assert!(!is_valid_key(s), "{s:?}: const said valid");
            assert!(validate_key(s).is_err(), "{s:?}: runtime said valid");
        }
    }
}
