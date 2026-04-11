//! Advisory retry metadata.

use std::{fmt, time::Duration};

/// Advisory metadata suggesting how a failed operation might be retried.
///
/// This is a *hint*, not an obligation. Callers (e.g. the resilience layer)
/// may ignore it or merge it with their own policies.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use nebula_error::RetryHint;
///
/// let hint = RetryHint::after(Duration::from_secs(5)).with_max_attempts(3);
///
/// assert_eq!(hint.after, Some(Duration::from_secs(5)));
/// assert_eq!(hint.max_attempts, Some(3));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryHint {
    /// Minimum duration to wait before retrying.
    pub after: Option<Duration>,
    /// Suggested maximum number of retry attempts.
    pub max_attempts: Option<u32>,
}

impl RetryHint {
    /// Creates a hint with only a backoff duration.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use nebula_error::RetryHint;
    ///
    /// let hint = RetryHint::after(Duration::from_millis(500));
    /// assert_eq!(hint.after, Some(Duration::from_millis(500)));
    /// assert_eq!(hint.max_attempts, None);
    /// ```
    pub fn after(duration: Duration) -> Self {
        Self {
            after: Some(duration),
            max_attempts: None,
        }
    }

    /// Creates a hint with only a max-attempts suggestion.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::RetryHint;
    ///
    /// let hint = RetryHint::max_attempts(5);
    /// assert_eq!(hint.after, None);
    /// assert_eq!(hint.max_attempts, Some(5));
    /// ```
    pub fn max_attempts(n: u32) -> Self {
        Self {
            after: None,
            max_attempts: Some(n),
        }
    }

    /// Adds a max-attempts limit to an existing hint.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use nebula_error::RetryHint;
    ///
    /// let hint = RetryHint::after(Duration::from_secs(1)).with_max_attempts(3);
    /// assert_eq!(hint.max_attempts, Some(3));
    /// ```
    #[must_use]
    pub fn with_max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = Some(n);
        self
    }
}

impl fmt::Display for RetryHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.after, self.max_attempts) {
            (Some(d), Some(n)) => write!(f, "retry after {}ms (max {} attempts)", d.as_millis(), n),
            (Some(d), None) => write!(f, "retry after {}ms", d.as_millis()),
            (None, Some(n)) => write!(f, "retry (max {} attempts)", n),
            (None, None) => write!(f, "retry"),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for RetryHint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let field_count =
            usize::from(self.after.is_some()) + usize::from(self.max_attempts.is_some());
        let mut s = serializer.serialize_struct("RetryHint", field_count)?;
        if let Some(d) = self.after {
            s.serialize_field("after_ms", &d.as_millis())?;
        }
        if let Some(n) = self.max_attempts {
            s.serialize_field("max_attempts", &n)?;
        }
        s.end()
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for RetryHint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Helper {
            after_ms: Option<u64>,
            max_attempts: Option<u32>,
        }
        let h = Helper::deserialize(deserializer)?;
        Ok(Self {
            after: h.after_ms.map(Duration::from_millis),
            max_attempts: h.max_attempts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_only() {
        let hint = RetryHint::after(Duration::from_secs(2));
        assert_eq!(hint.after, Some(Duration::from_secs(2)));
        assert_eq!(hint.max_attempts, None);
    }

    #[test]
    fn max_attempts_only() {
        let hint = RetryHint::max_attempts(5);
        assert_eq!(hint.after, None);
        assert_eq!(hint.max_attempts, Some(5));
    }

    #[test]
    fn with_max_attempts_chains() {
        let hint = RetryHint::after(Duration::from_millis(100)).with_max_attempts(3);
        assert_eq!(hint.after, Some(Duration::from_millis(100)));
        assert_eq!(hint.max_attempts, Some(3));
    }

    #[test]
    fn display_after_and_max() {
        let hint = RetryHint::after(Duration::from_secs(5)).with_max_attempts(3);
        assert_eq!(hint.to_string(), "retry after 5000ms (max 3 attempts)");
    }

    #[test]
    fn display_after_only() {
        let hint = RetryHint::after(Duration::from_millis(250));
        assert_eq!(hint.to_string(), "retry after 250ms");
    }

    #[test]
    fn display_max_only() {
        let hint = RetryHint::max_attempts(10);
        assert_eq!(hint.to_string(), "retry (max 10 attempts)");
    }

    #[test]
    fn display_neither() {
        let hint = RetryHint {
            after: None,
            max_attempts: None,
        };
        assert_eq!(hint.to_string(), "retry");
    }
}
