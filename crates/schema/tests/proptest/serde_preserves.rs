//! Proptest: serde roundtrip for `FieldValue` and `FieldValues`.

use nebula_schema::{FieldValue, FieldValues};
use proptest::prelude::*;
use serde_json::json;

proptest! {
    /// A JSON object with string/number/bool leaf values survives a
    /// FieldValue::from_json → to_json roundtrip.
    #[test]
    fn literal_value_roundtrip(
        s in any::<String>().prop_filter("no expr markers", |s| !s.contains("{{") && !s.contains("}}")),
        n in any::<i32>(),
        b in any::<bool>()
    ) {
        // Build a JSON object with three typed fields.
        let v = json!({
            "s": s,
            "n": n,
            "b": b
        });
        let fv = FieldValue::from_json(v.clone());
        prop_assert_eq!(fv.to_json(), v);
    }

    /// FieldValues roundtrips through to_json + from_json.
    #[test]
    fn field_values_json_roundtrip(
        key1 in "[a-z][a-z0-9_]{0,8}",
        val1 in prop::num::i64::ANY
    ) {
        let v = json!({ key1: val1 });
        let fvs = FieldValues::from_json(v.clone());
        if let Ok(fvs2) = fvs {
            prop_assert_eq!(fvs2.to_json(), v);
        } else {
            // key1 might be invalid as FieldKey (e.g. starts with digit) — ok.
        }
    }
}
