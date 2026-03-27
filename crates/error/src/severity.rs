//! Error severity levels.

use std::fmt;

/// Indicates how severe an error is.
///
/// Ordered from least to most severe: `Info < Warning < Error`.
/// The default severity is [`Error`](ErrorSeverity::Error).
///
/// # Examples
///
/// ```
/// use nebula_error::ErrorSeverity;
///
/// assert!(ErrorSeverity::Info < ErrorSeverity::Error);
/// assert_eq!(ErrorSeverity::default(), ErrorSeverity::Error);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorSeverity {
    /// Informational — not an error, but worth noting.
    Info = 0,
    /// Warning — something may be wrong, but execution continues.
    Warning = 1,
    /// Error — something failed and must be addressed.
    #[default]
    Error = 2,
}

impl ErrorSeverity {
    /// Returns `true` if this is [`Error`](Self::Error) severity.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorSeverity;
    ///
    /// assert!(ErrorSeverity::Error.is_error());
    /// assert!(!ErrorSeverity::Info.is_error());
    /// ```
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }

    /// Returns `true` if this is [`Warning`](Self::Warning) severity.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorSeverity;
    ///
    /// assert!(ErrorSeverity::Warning.is_warning());
    /// assert!(!ErrorSeverity::Error.is_warning());
    /// ```
    pub const fn is_warning(&self) -> bool {
        matches!(self, Self::Warning)
    }

    /// Returns `true` if this is [`Info`](Self::Info) severity.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorSeverity;
    ///
    /// assert!(ErrorSeverity::Info.is_info());
    /// assert!(!ErrorSeverity::Error.is_info());
    /// ```
    pub const fn is_info(&self) -> bool {
        matches!(self, Self::Info)
    }

    /// Returns the lowercase string representation.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorSeverity;
    ///
    /// assert_eq!(ErrorSeverity::Error.as_str(), "error");
    /// ```
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for ErrorSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ErrorSeverity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ErrorSeverity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        match s {
            "info" => Ok(Self::Info),
            "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["info", "warning", "error"],
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_error() {
        assert_eq!(ErrorSeverity::default(), ErrorSeverity::Error);
    }

    #[test]
    fn ordering_info_less_than_warning_less_than_error() {
        assert!(ErrorSeverity::Info < ErrorSeverity::Warning);
        assert!(ErrorSeverity::Warning < ErrorSeverity::Error);
        assert!(ErrorSeverity::Info < ErrorSeverity::Error);
    }

    #[test]
    fn display_is_lowercase() {
        assert_eq!(ErrorSeverity::Info.to_string(), "info");
        assert_eq!(ErrorSeverity::Warning.to_string(), "warning");
        assert_eq!(ErrorSeverity::Error.to_string(), "error");
    }

    #[test]
    fn is_error_returns_true_only_for_error() {
        assert!(ErrorSeverity::Error.is_error());
        assert!(!ErrorSeverity::Warning.is_error());
        assert!(!ErrorSeverity::Info.is_error());
    }

    #[test]
    fn is_warning_returns_true_only_for_warning() {
        assert!(ErrorSeverity::Warning.is_warning());
        assert!(!ErrorSeverity::Error.is_warning());
        assert!(!ErrorSeverity::Info.is_warning());
    }

    #[test]
    fn is_info_returns_true_only_for_info() {
        assert!(ErrorSeverity::Info.is_info());
        assert!(!ErrorSeverity::Error.is_info());
        assert!(!ErrorSeverity::Warning.is_info());
    }

    #[test]
    fn max_severity_picks_highest() {
        let severities = [
            ErrorSeverity::Info,
            ErrorSeverity::Warning,
            ErrorSeverity::Error,
        ];
        assert_eq!(severities.iter().max(), Some(&ErrorSeverity::Error));
    }

    #[test]
    fn as_str_matches_display() {
        for sev in [
            ErrorSeverity::Info,
            ErrorSeverity::Warning,
            ErrorSeverity::Error,
        ] {
            assert_eq!(sev.as_str(), sev.to_string());
        }
    }
}
