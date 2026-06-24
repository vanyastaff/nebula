//! Field alias container — extra accepted input keys (read-aliases) and
//! the optional output key remap (`emit_as`).
//!
//! This is a pure storage type; canonicalization is enforced at ingest in
//! `validated.rs` and collision checks run at lint time in `lint.rs`.

use serde::{Deserialize, Serialize};

use crate::{error::ValidationError, key::FieldKey, path::FieldPath};

/// Ordered set of read-alias keys for a single field.
///
/// # Serde
///
/// Serializes/deserializes transparently as a JSON array of strings.
/// An empty set serializes as `[]` and is skipped in field wire output via
/// `#[serde(skip_serializing_if = "FieldAliases::is_empty")]`.
///
/// # Ordering invariant
///
/// Iteration order equals insertion order (aliases are pushed sequentially).
/// The resolver tries aliases in that order; the first one present in a
/// submitted value map wins.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldAliases(Vec<FieldKey>);

impl FieldAliases {
    /// An empty alias set. Equivalent to `Default::default()`.
    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    /// Build a validated alias set from an iterable of string-like values.
    ///
    /// Each item is validated as a [`FieldKey`] (code `alias.invalid_key`).
    /// Intra-set duplicates are rejected with code `alias.duplicate`.
    ///
    /// # Errors
    ///
    /// Returns `alias.invalid_key` if any item is not a valid field key, or
    /// `alias.duplicate` if the same key appears more than once in the input.
    #[expect(
        clippy::result_large_err,
        reason = "ValidationError is intentionally large; callers are on the validation path"
    )]
    pub fn new(
        aliases: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, ValidationError> {
        let mut out = Vec::new();
        let mut seen_strs: std::collections::HashSet<String> = std::collections::HashSet::new();
        for raw in aliases {
            let raw = raw.as_ref();
            let key = FieldKey::new(raw).map_err(|_| {
                ValidationError::builder("alias.invalid_key")
                    .at(FieldPath::root())
                    .param("key", raw.to_owned())
                    .message(format!(
                        "alias `{raw}` is not a valid field key (must be ASCII alphanumeric/underscore, start with letter or underscore, max 64 chars)"
                    ))
                    .build()
            })?;
            if !seen_strs.insert(raw.to_owned()) {
                return Err(ValidationError::builder("alias.duplicate")
                    .at(FieldPath::root())
                    .param("key", raw.to_owned())
                    .message(format!(
                        "alias `{raw}` appears more than once in the alias list"
                    ))
                    .build());
            }
            out.push(key);
        }
        Ok(Self(out))
    }

    /// Returns `true` when the alias list is empty.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of aliases.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Borrow the underlying alias slice.
    #[must_use]
    #[inline]
    pub fn as_slice(&self) -> &[FieldKey] {
        &self.0
    }

    /// Append a pre-validated key without re-checking.
    ///
    /// For use by schema builders and macro-generated code that has already
    /// validated the key. Callers must ensure uniqueness themselves.
    #[inline]
    pub(crate) fn push_unchecked(&mut self, key: FieldKey) {
        self.0.push(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_default_equivalence() {
        assert_eq!(FieldAliases::empty(), FieldAliases::default());
        assert!(FieldAliases::empty().is_empty());
        assert_eq!(FieldAliases::empty().len(), 0);
    }

    #[test]
    fn new_validates_keys() {
        let err = FieldAliases::new(["has-dash"]).unwrap_err();
        assert_eq!(err.code, "alias.invalid_key");
        assert_eq!(
            err.params
                .iter()
                .find(|(k, _)| k.as_ref() == "key")
                .map(|(_, v)| v.as_str().unwrap()),
            Some("has-dash")
        );
    }

    #[test]
    fn new_rejects_intra_set_duplicate() {
        let err = FieldAliases::new(["foo", "foo"]).unwrap_err();
        assert_eq!(err.code, "alias.duplicate");
    }

    #[test]
    fn new_preserves_order() {
        let aliases = FieldAliases::new(["b", "a", "c"]).unwrap();
        let keys: Vec<&str> = aliases.as_slice().iter().map(FieldKey::as_str).collect();
        assert_eq!(keys, ["b", "a", "c"]);
    }

    #[test]
    fn serde_transparent_round_trip() {
        let aliases = FieldAliases::new(["alpha", "beta"]).unwrap();
        let wire = serde_json::to_value(&aliases).unwrap();
        assert_eq!(wire, serde_json::json!(["alpha", "beta"]));
        let back: FieldAliases = serde_json::from_value(wire).unwrap();
        assert_eq!(back, aliases);
    }

    #[test]
    fn serde_empty_round_trip() {
        let aliases = FieldAliases::empty();
        let wire = serde_json::to_value(&aliases).unwrap();
        assert_eq!(wire, serde_json::json!([]));
        let back: FieldAliases = serde_json::from_value(wire).unwrap();
        assert_eq!(back, aliases);
    }

    #[test]
    fn push_unchecked_appends() {
        let mut aliases = FieldAliases::empty();
        let key = FieldKey::new("foo").unwrap();
        aliases.push_unchecked(key.clone());
        assert_eq!(aliases.as_slice(), [key]);
    }
}
