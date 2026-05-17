//! Public-schema projection (ADR-0052 P4, design-spec hole #6).
//!
//! `ValidSchema::json_schema()` emits the credential type's internal rule
//! graph as Nebula vendor extensions: `x-nebula-root-rules`
//! (`crates/schema/src/json_schema.rs:115`) serializes the full
//! cross-field `Rule` operands, and `x-nebula-required-mode` /
//! `x-nebula-visibility-mode` mark fields as rule-conditioned (the
//! exporter emits these two as a bare discriminant string — `"when"` —
//! not the operand; they are still rule-derived metadata, not part of the
//! public JSON-Schema contract). Exposing this family to catalog clients
//! leaks rule-graph internals. This api-owned mapper strips the whole
//! family **recursively** (defence-in-depth: `x-nebula-root-rules` is the
//! genuinely-sensitive operand carrier; the two mode keys are stripped as
//! non-contract rule-derived metadata)
//! while keeping the standard JSON-Schema contract (`type`, `properties`,
//! `required`, `minLength`, `pattern`, `enum`, `additionalProperties`, …)
//! and non-predicate structural hints (`x-nebula-field-kind`,
//! `x-nebula-expression-mode`). It is **not** a raw `json_schema()`
//! passthrough — the spec mandates an api-owned projection.

use serde_json::Value;

/// Vendor-extension keys that carry rule / cross-field predicate logic
/// and MUST NOT reach an unauthenticated client.
const STRIPPED_KEYS: &[&str] = &[
    "x-nebula-root-rules",
    "x-nebula-required-mode",
    "x-nebula-visibility-mode",
];

/// Recursively remove the rule/predicate vendor-extension family from a
/// JSON-Schema `Value`. Pure; allocation-light; total (no panics).
#[must_use]
pub fn project_public_schema(mut schema: Value) -> Value {
    strip(&mut schema);
    schema
}

fn strip(node: &mut Value) {
    match node {
        Value::Object(map) => {
            for k in STRIPPED_KEYS {
                map.remove(*k);
            }
            for v in map.values_mut() {
                strip(v);
            }
        },
        Value::Array(items) => {
            for v in items {
                strip(v);
            }
        },
        _ => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_root_rules_and_predicate_operands_keeps_standard_keywords() {
        let raw = serde_json::json!({
            "type": "object",
            "properties": {
                "api_key": {
                    "type": "string",
                    "minLength": 2,
                    "pattern": "^k-",
                    "x-nebula-field-kind": "secret",
                    "x-nebula-required-mode": { "when": { "predicate": "X" } }
                }
            },
            "x-nebula-root-rules": [ { "predicate": "cross-field" } ],
            "additionalProperties": false
        });
        let p = project_public_schema(raw);
        assert!(p.get("x-nebula-root-rules").is_none());
        let ak = &p["properties"]["api_key"];
        assert!(ak.get("x-nebula-required-mode").is_none());
        assert_eq!(ak["minLength"], 2, "standard keyword kept");
        assert_eq!(ak["pattern"], "^k-", "standard keyword kept");
        assert_eq!(
            ak["x-nebula-field-kind"], "secret",
            "non-predicate structural hint kept"
        );
        assert_eq!(p["additionalProperties"], false);
    }

    #[test]
    fn idempotent_and_total_on_scalars() {
        assert_eq!(project_public_schema(Value::Null), Value::Null);
        let once = project_public_schema(serde_json::json!({"a":{"x-nebula-root-rules":[]}}));
        assert_eq!(project_public_schema(once.clone()), once);
        assert!(once["a"].get("x-nebula-root-rules").is_none());
    }
}
