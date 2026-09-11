//! Deferred rules — require runtime context beyond the value + predicate
//! map. Skipped at schema-validation time.

use serde::{Deserialize, Serialize};

use crate::{
    foundation::{FieldPath, ValidationError},
    rule::context::PredicateContext,
};

/// Rule requiring runtime evaluation beyond static context.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeferredRule {
    /// Opaque custom expression whose evaluation is owned by the runtime.
    Custom(String),
    /// Each array item must have a unique value at the given sub-path.
    UniqueBy(FieldPath),
}

impl std::fmt::Debug for DeferredRule {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Custom(_) => "DeferredRule::Custom(<protected>)",
            Self::UniqueBy(_) => "DeferredRule::UniqueBy(<protected>)",
        })
    }
}

impl DeferredRule {
    /// Reports that the required runtime evaluator is unavailable.
    ///
    /// The static rule pass defers these rules before reaching this method.
    /// Full evaluation cannot claim satisfaction without an evaluator.
    pub fn validate(
        &self,
        _input: &serde_json::Value,
        _ctx: Option<&PredicateContext>,
    ) -> Result<(), ValidationError> {
        Err(ValidationError::unavailable(
            "deferred rule requires a runtime evaluator",
        ))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn custom_wire_form() {
        let r = DeferredRule::Custom("check()".into());
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j, json!({"custom": "check()"}));
    }

    #[test]
    fn unique_by_roundtrip() {
        let r = DeferredRule::UniqueBy(FieldPath::parse("name").unwrap());
        let back: DeferredRule = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }
}
