//! Proof tokens that certify a value has been validated.
//!
//! A [`Validated<T>`] wraps an owned value and guarantees it passed validation.
//! This prevents "trust boundary" bugs where unvalidated data silently flows
//! through the system.
//!
//! # Design
//!
//! - The inner value has **no public field** — it can only be constructed through
//!   [`Validate::validate_into`](crate::foundation::Validate::validate_into) or
//!   [`Validated::new`].
//! - Read access via [`Deref`], [`AsRef`], [`Borrow`], and [`inner()`](Validated::inner).
//! - Ownership recovery via [`into_inner()`](Validated::into_inner).
//! - `Serialize` is derived so validated values can be persisted.
//!   `Deserialize` is intentionally **not** derived — deserialized data must
//!   be re-validated before wrapping.
//!
//! # Examples
//!
//! ```rust
//! use nebula_validator::foundation::Validate;
//! use nebula_validator::proof::Validated;
//! use nebula_validator::validators::min_length;
//!
//! let v = min_length(3);
//! let name: Validated<String> = v.validate_into("alice".to_string()).unwrap();
//!
//! // Access the validated value
//! assert_eq!(name.as_ref(), "alice");
//!
//! // Move it out when done
//! let raw: String = name.into_inner();
//! assert_eq!(raw, "alice");
//! ```

use serde::Serialize;
use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;

/// A proof token certifying that the wrapped value has passed validation.
///
/// `Validated<T>` is a thin wrapper that carries a compile-time guarantee:
/// the value inside was accepted by a [`Validate`](crate::foundation::Validate)
/// implementation before being wrapped.
///
/// # Invariants
///
/// - Can only be constructed through validated paths
///   ([`Validate::validate_into`](crate::foundation::Validate::validate_into),
///   [`Validated::new`]).
/// - Immutable: no `&mut T` access is exposed, preserving the validation
///   guarantee.
///
/// # Zero-cost
///
/// `Validated<T>` has the same size as `T` — no extra allocation.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Validated<T> {
    value: T,
}

impl<T> Validated<T> {
    /// Creates a new proof token by validating `value` with `validator`.
    ///
    /// # Errors
    ///
    /// Returns `Err(ValidatorError::ValidationFailed(..))` if validation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_validator::proof::Validated;
    /// use nebula_validator::validators::min_length;
    ///
    /// let name = Validated::new("alice".to_string(), &min_length(3)).unwrap();
    /// assert_eq!(name.as_ref(), "alice");
    /// ```
    pub fn new<V, U: ?Sized>(value: T, validator: &V) -> crate::error::ValidatorResult<Self>
    where
        T: Borrow<U>,
        V: crate::foundation::Validate<U>,
    {
        validator.validate(value.borrow())?;
        Ok(Self { value })
    }

    /// Returns a reference to the validated value.
    #[inline]
    pub fn inner(&self) -> &T {
        &self.value
    }

    /// Consumes the proof token and returns the inner value.
    #[inline]
    pub fn into_inner(self) -> T {
        self.value
    }

    /// Maps the inner value through a function, producing a new `Validated<U>`.
    ///
    /// The caller asserts that `f` preserves the validation invariant.
    /// Use sparingly — prefer re-validating when in doubt.
    #[inline]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Validated<U> {
        Validated {
            value: f(self.value),
        }
    }

    /// Constructs a proof token without validation.
    ///
    /// # Safety (logical)
    ///
    /// The caller **must** guarantee that `value` satisfies whatever
    /// validation contract this proof is expected to represent.
    /// Misuse breaks the trust boundary that `Validated<T>` exists to enforce.
    ///
    /// Prefer [`Validated::new`] or [`Validate::validate_into`](crate::foundation::Validate::validate_into).
    #[inline]
    pub fn new_unchecked(value: T) -> Self {
        Self { value }
    }
}

// ── Trait implementations ───────────────────────────────────────────────────

impl<T> Deref for Validated<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> AsRef<T> for Validated<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        &self.value
    }
}

impl<T> Borrow<T> for Validated<T> {
    #[inline]
    fn borrow(&self) -> &T {
        &self.value
    }
}

impl<T: fmt::Debug> fmt::Debug for Validated<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Validated").field(&self.value).finish()
    }
}

impl<T: fmt::Display> fmt::Display for Validated<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;
    use crate::validators::{max_length, min_length};

    #[test]
    fn validated_new_passes() {
        let v = Validated::new("hello".to_string(), &min_length(3));
        assert!(v.is_ok());
        assert_eq!(v.unwrap().inner(), "hello");
    }

    #[test]
    fn validated_new_fails() {
        let v = Validated::new("hi".to_string(), &min_length(5));
        assert!(v.is_err());
    }

    #[test]
    fn validate_into_works() {
        let v = min_length(3);
        let name: Validated<String> = v.validate_into("alice".to_string()).unwrap();
        assert_eq!(*name, "alice");
    }

    #[test]
    fn validate_into_fails() {
        let v = min_length(10);
        assert!(v.validate_into("hi".to_string()).is_err());
    }

    #[test]
    fn deref_works() {
        let v = Validated::new("hello".to_string(), &min_length(1)).unwrap();
        // Deref to &String, then auto-deref to &str
        assert!(v.starts_with("hel"));
    }

    #[test]
    fn into_inner_works() {
        let v = Validated::new("hello".to_string(), &min_length(1)).unwrap();
        let s: String = v.into_inner();
        assert_eq!(s, "hello");
    }

    #[test]
    fn map_works() {
        let v = Validated::new(42u32, &crate::validators::min(10u32)).unwrap();
        let doubled = v.map(|n| n * 2);
        assert_eq!(*doubled, 84);
    }

    #[test]
    fn zero_cost_size() {
        assert_eq!(
            std::mem::size_of::<Validated<u64>>(),
            std::mem::size_of::<u64>()
        );
        assert_eq!(
            std::mem::size_of::<Validated<String>>(),
            std::mem::size_of::<String>()
        );
    }

    #[test]
    fn display_delegates() {
        let v = Validated::new("hello".to_string(), &min_length(1)).unwrap();
        assert_eq!(format!("{v}"), "hello");
    }

    #[test]
    fn debug_shows_wrapper() {
        let v = Validated::new(42u32, &crate::validators::min(1u32)).unwrap();
        assert_eq!(format!("{v:?}"), "Validated(42)");
    }

    #[test]
    fn serialize_transparent() {
        let v = Validated::new("hello".to_string(), &min_length(1)).unwrap();
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"hello\"");
    }

    #[test]
    fn combined_validator_proof() {
        use crate::foundation::ValidateExt;

        let v = min_length(3).and(max_length(10));
        let name = v.validate_into("alice".to_string()).unwrap();
        assert_eq!(name.as_ref(), "alice");

        assert!(v.validate_into("hi".to_string()).is_err());
        assert!(
            v.validate_into("a very long string indeed".to_string())
                .is_err()
        );
    }
}
