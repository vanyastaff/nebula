//! Generic redacted wrapper for any secret type.
//!
//! [`RedactedSecret`] wraps a [`secrecy::SecretBox`] and adds a `Serialize`
//! impl that always writes `"[REDACTED]"` plus a redacted `Debug`.

use std::fmt;

use secrecy::SecretBox;
use serde::{Serialize, Serializer};
use zeroize::Zeroize;

/// A wrapper around [`SecretBox<S>`] that serializes as `"[REDACTED]"` and
/// redacts its `Debug` output.
///
/// Use this when you need a `Serialize`-capable secret field for a type
/// that is not `SecretString`.
pub struct RedactedSecret<S: Zeroize>(pub SecretBox<S>);

impl<S: Zeroize> Serialize for RedactedSecret<S> {
    fn serialize<Ser: Serializer>(&self, s: Ser) -> Result<Ser::Ok, Ser::Error> {
        s.serialize_str("[REDACTED]")
    }
}

impl<S: Zeroize> fmt::Debug for RedactedSecret<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl<S: Zeroize> std::ops::Deref for RedactedSecret<S> {
    type Target = SecretBox<S>;

    fn deref(&self) -> &SecretBox<S> {
        &self.0
    }
}
