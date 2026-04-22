//! [`RuleContext`] implementations backed by [`FieldValues`] subtrees.
//!
//! These adapters let predicate rules (visibility/required `When(rule)`) run
//! directly against the current value tree without allocating a `HashMap`.

use indexmap::IndexMap;
use nebula_validator::RuleContext;
use serde_json::Value;

use crate::{
    key::FieldKey,
    value::{FieldValue, FieldValues},
};

/// Root context — borrowed view over top-level [`FieldValues`].
pub(crate) struct RootContext<'a>(pub &'a FieldValues);

impl RuleContext for RootContext<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        // FieldKey: Borrow<str> — get_by_str queries the inner IndexMap without
        // constructing a FieldKey (no Arc allocation for the lookup).
        match self.0.get_by_str(key)? {
            FieldValue::Literal(v) => Some(v),
            // Do not leak a sentinel value into predicate evaluation.
            // Secret fields are treated as non-addressable by `RuleContext`.
            FieldValue::SecretLiteral(_) => None,
            _ => None,
        }
    }
}

/// Sub-context — borrowed view over a nested object's field map.
pub(crate) struct ObjectContext<'a>(pub &'a IndexMap<FieldKey, FieldValue>);

impl RuleContext for ObjectContext<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        // IndexMap<FieldKey, _>::get accepts Q: Hash + Eq where FieldKey: Borrow<Q>.
        // Passing &str directly avoids constructing a FieldKey.
        match self.0.get(key)? {
            FieldValue::Literal(v) => Some(v),
            // Do not leak a sentinel value into predicate evaluation.
            // Secret fields are treated as non-addressable by `RuleContext`.
            FieldValue::SecretLiteral(_) => None,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn root_context_returns_literal() {
        let mut values = FieldValues::new();
        let fk = FieldKey::new("x").unwrap();
        values.set(fk, FieldValue::Literal(json!(42)));
        let ctx = RootContext(&values);
        assert_eq!(ctx.get("x"), Some(&json!(42)));
    }

    #[test]
    fn root_context_returns_none_for_expression() {
        let mut values = FieldValues::new();
        let fk = FieldKey::new("x").unwrap();
        values.set(
            fk,
            FieldValue::Expression(crate::expression::Expression::new("{{ $x }}")),
        );
        let ctx = RootContext(&values);
        // Expressions don't resolve at this layer.
        assert_eq!(ctx.get("x"), None);
    }

    #[test]
    fn root_context_returns_none_for_missing() {
        let values = FieldValues::new();
        let ctx = RootContext(&values);
        assert_eq!(ctx.get("missing"), None);
    }

    #[test]
    fn root_context_hides_secret_literals_from_rules() {
        let mut values = FieldValues::new();
        let fk = FieldKey::new("api_key").unwrap();
        values.set(
            fk,
            FieldValue::SecretLiteral(crate::secret::SecretValue::string("s3cr3t".to_owned())),
        );
        let ctx = RootContext(&values);
        assert_eq!(ctx.get("api_key"), None);
    }

    #[test]
    fn object_context_hides_secret_literals_from_rules() {
        let mut map = IndexMap::new();
        map.insert(
            FieldKey::new("token").unwrap(),
            FieldValue::SecretLiteral(crate::secret::SecretValue::string("abc".to_owned())),
        );
        let ctx = ObjectContext(&map);
        assert_eq!(ctx.get("token"), None);
    }
}
