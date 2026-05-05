//! Explicit availability markers for probe data.
//!
//! Host probes frequently encounter data that is unsupported on a platform,
//! unavailable because the backend could not read it, not sampled yet, stale, or
//! permission-denied. This module provides a small wrapper so those states are
//! not represented as misleading zeros, `None`, or empty collections.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Availability state for a probe field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum AvailabilityStatus {
    /// The value was measured or derived successfully.
    Available,
    /// The platform/backend does not support this field.
    Unsupported,
    /// The field is supported but the value was unavailable during this probe.
    Unavailable,
    /// The probe was denied by OS permissions.
    PermissionDenied,
    /// The field has not been implemented by `nebula-system`.
    NotImplemented,
    /// The backend requires sampling state and no valid sample exists yet.
    NotSampled,
    /// The last known value exists but was not refreshed for this observation.
    Stale,
}

/// A probe value with explicit availability status.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Availability<T> {
    /// Availability status for this field.
    pub status: AvailabilityStatus,
    /// Measured value, present only when the status carries usable data.
    pub value: Option<T>,
    /// Human-readable reason for unavailable, unsupported, stale, or partial data.
    pub reason: Option<String>,
}

impl<T> Availability<T> {
    /// Build an available value.
    pub fn available(value: T) -> Self {
        Self {
            status: AvailabilityStatus::Available,
            value: Some(value),
            reason: None,
        }
    }

    /// Build an unsupported value.
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::without_value(AvailabilityStatus::Unsupported, reason)
    }

    /// Build an unavailable value.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::without_value(AvailabilityStatus::Unavailable, reason)
    }

    /// Build a permission-denied value.
    pub fn permission_denied(reason: impl Into<String>) -> Self {
        Self::without_value(AvailabilityStatus::PermissionDenied, reason)
    }

    /// Build a not-implemented value.
    pub fn not_implemented(reason: impl Into<String>) -> Self {
        Self::without_value(AvailabilityStatus::NotImplemented, reason)
    }

    /// Build a not-sampled-yet value.
    pub fn not_sampled(reason: impl Into<String>) -> Self {
        Self::without_value(AvailabilityStatus::NotSampled, reason)
    }

    /// Build a stale value with the last known reading.
    pub fn stale(value: Option<T>, reason: impl Into<String>) -> Self {
        Self {
            status: AvailabilityStatus::Stale,
            value,
            reason: Some(reason.into()),
        }
    }

    /// Return true when the value is available.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.status == AvailabilityStatus::Available
    }

    /// Borrow the measured value if one is present.
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Convert into the measured value if one is present.
    #[must_use]
    pub fn into_value(self) -> Option<T> {
        self.value
    }

    /// Map the available value while preserving non-available status and reason.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Availability<U> {
        Availability {
            status: self.status,
            value: self.value.map(f),
            reason: self.reason,
        }
    }

    fn without_value(status: AvailabilityStatus, reason: impl Into<String>) -> Self {
        Self {
            status,
            value: None,
            reason: Some(reason.into()),
        }
    }
}

impl<T> From<T> for Availability<T> {
    fn from(value: T) -> Self {
        Self::available(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{Availability, AvailabilityStatus};

    #[test]
    fn available_value_is_distinguishable_from_unavailable() {
        let available = Availability::available(42);
        assert!(available.is_available());
        assert_eq!(available.value(), Some(&42));
        assert_eq!(available.status, AvailabilityStatus::Available);
        assert!(available.reason.is_none());

        let unavailable = Availability::<u32>::unavailable("backend did not return a value");
        assert!(!unavailable.is_available());
        assert_eq!(unavailable.value(), None);
        assert_eq!(unavailable.status, AvailabilityStatus::Unavailable);
        assert_eq!(
            unavailable.reason.as_deref(),
            Some("backend did not return a value")
        );
    }

    #[test]
    fn stale_values_can_carry_last_known_reading() {
        let stale = Availability::stale(Some(17), "sample interval was too short");
        assert!(!stale.is_available());
        assert_eq!(stale.status, AvailabilityStatus::Stale);
        assert_eq!(stale.value(), Some(&17));
    }

    #[test]
    fn map_preserves_status_and_reason() {
        let mapped = Availability::not_sampled("first sample").map(|value: u32| value + 1);
        assert_eq!(mapped.status, AvailabilityStatus::NotSampled);
        assert_eq!(mapped.value(), None);
        assert_eq!(mapped.reason.as_deref(), Some("first sample"));

        let mapped = Availability::available(2).map(|value| value * 10);
        assert_eq!(mapped.value(), Some(&20));
        assert!(mapped.is_available());
    }
}
