//! Deferred rules — require runtime context beyond the value + predicate
//! map. Skipped at schema-validation time.

use serde::{Deserialize, Serialize};

use crate::{
    foundation::{FieldPath, ValidationError},
    rule::context::PredicateContext,
};

/// Rule requiring runtime evaluation beyond static context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeferredRule {
    /// Custom expression string. Typing via `nebula-expression` is Refactor 2.
    Custom(String),
    /// Each array item must have a unique value at the given sub-path.
    UniqueBy(FieldPath),
}

impl DeferredRule {
    /// Validates deferred. Without a ctx bridge to the runtime evaluator
    /// these rules return `Ok(())` — they'll be picked up by the workflow
    /// engine when it has a real context.
    pub fn validate(
        &self,
        _input: &serde_json::Value,
        _ctx: Option<&PredicateContext>,
    ) -> Result<(), ValidationError> {
        Ok(())
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
