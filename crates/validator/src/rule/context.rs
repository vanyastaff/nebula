//! Placeholder — replaced by full impl in Task 7.

use std::collections::HashMap;

use crate::foundation::FieldPath;

/// Typed lookup bag for sibling-field predicate evaluation.
///
/// Placeholder — full implementation replaces this in Task 7.
pub struct PredicateContext {
    fields: HashMap<FieldPath, serde_json::Value>,
}

impl PredicateContext {
    /// Returns the value for `p`, or `None` if the field is absent.
    pub fn get(&self, p: &FieldPath) -> Option<&serde_json::Value> {
        self.fields.get(p)
    }

    /// Constructs a context from the top-level keys of a JSON object.
    pub fn from_json(obj: &serde_json::Value) -> Self {
        let mut fields = HashMap::new();
        if let Some(m) = obj.as_object() {
            for (k, v) in m {
                if let Some(path) = FieldPath::parse(k) {
                    fields.insert(path, v.clone());
                }
            }
        }
        Self { fields }
    }
}
