//! Classification methods for [`Rule`]: categorizing variants as
//! value-validation, context predicate, deferred, or combinator; and
//! collecting field references for dependency analysis.

use super::Rule;

impl Rule {
    /// Returns `true` if this rule validates a single value
    /// (as opposed to evaluating context predicates).
    ///
    /// Deferred rules (`Custom`, `UniqueBy`) are **not** classified as value rules;
    /// use [`is_deferred`](Self::is_deferred) to check for those.
    #[must_use]
    pub fn is_value_rule(&self) -> bool {
        matches!(
            self,
            Self::Pattern { .. }
                | Self::MinLength { .. }
                | Self::MaxLength { .. }
                | Self::Min { .. }
                | Self::Max { .. }
                | Self::GreaterThan { .. }
                | Self::LessThan { .. }
                | Self::OneOf { .. }
                | Self::MinItems { .. }
                | Self::MaxItems { .. }
                | Self::Email { .. }
                | Self::Url { .. }
        )
    }

    /// Returns `true` if this rule evaluates context predicates
    /// (checks a sibling field value).
    #[must_use]
    pub fn is_predicate(&self) -> bool {
        matches!(
            self,
            Self::Eq { .. }
                | Self::Ne { .. }
                | Self::Gt { .. }
                | Self::Gte { .. }
                | Self::Lt { .. }
                | Self::Lte { .. }
                | Self::IsTrue { .. }
                | Self::IsFalse { .. }
                | Self::Set { .. }
                | Self::Empty { .. }
                | Self::Contains { .. }
                | Self::Matches { .. }
                | Self::In { .. }
        )
    }

    /// Returns `true` if this rule requires runtime expression context.
    ///
    /// Deferred rules are skipped during static schema validation.
    #[inline]
    #[must_use]
    pub fn is_deferred(&self) -> bool {
        matches!(self, Self::UniqueBy { .. } | Self::Custom { .. })
    }

    /// Collects all field IDs referenced by context predicates in this rule.
    ///
    /// Recurses into logical combinators (`All`, `Any`, `Not`).
    /// Value-only rules and deferred rules return no references.
    pub fn field_references<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Eq { field, .. }
            | Self::Ne { field, .. }
            | Self::Gt { field, .. }
            | Self::Gte { field, .. }
            | Self::Lt { field, .. }
            | Self::Lte { field, .. }
            | Self::IsTrue { field }
            | Self::IsFalse { field }
            | Self::Set { field }
            | Self::Empty { field }
            | Self::Contains { field, .. }
            | Self::Matches { field, .. }
            | Self::In { field, .. } => out.push(field),
            Self::All { rules } | Self::Any { rules } => {
                for rule in rules {
                    rule.field_references(out);
                }
            },
            Self::Not { inner } => inner.field_references(out),
            _ => {},
        }
    }
}
