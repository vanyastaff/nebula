//! Proptest: data and authored-wire roundtrips preserve their distinct identities.

use nebula_schema::{AuthoredValue, Expression, ProgramSyntax};
use proptest::prelude::*;
use serde_json::json;

proptest! {
    /// A JSON object with string/number/bool leaf values survives a
    /// AuthoredValue::from_data → to_json roundtrip without interpolation.
    #[test]
    fn literal_value_roundtrip(
        s in any::<String>(),
        n in any::<i32>(),
        b in any::<bool>()
    ) {
        // Build a JSON object with three typed fields.
        let v = json!({
            "s": s,
            "n": n,
            "b": b
        });
        let fv = AuthoredValue::from_data(v.clone()).unwrap();
        prop_assert_eq!(fv.to_json(), v);
        let decoded: AuthoredValue = serde_json::from_slice(&serde_json::to_vec(&fv).unwrap()).unwrap();
        prop_assert_eq!(decoded, fv);
    }

    /// Every JSON property key roundtrips through data ingress and authored wire.
    #[test]
    fn authored_value_json_roundtrip(
        key1 in any::<String>(),
        val1 in prop::num::i64::ANY
    ) {
        let v = json!({ key1: val1 });
        let authored = AuthoredValue::from_data(v.clone()).unwrap();
        prop_assert_eq!(authored.to_json(), v);
        let decoded: AuthoredValue =
            serde_json::from_slice(&serde_json::to_vec(&authored).unwrap()).unwrap();
        prop_assert_eq!(decoded, authored);
    }

    #[test]
    fn expression_source_and_literal_identity_roundtrip(
        source in any::<String>(),
        syntax in prop::sample::select(vec![ProgramSyntax::Auto, ProgramSyntax::Expression, ProgramSyntax::Template]),
    ) {
        let expression = AuthoredValue::Expression(Expression::with_syntax(source.clone(), syntax));
        let literal = AuthoredValue::from_data(json!(source)).unwrap();
        let encoded_expression = serde_json::to_vec(&expression).unwrap();
        let encoded_literal = serde_json::to_vec(&literal).unwrap();
        prop_assert_ne!(&encoded_expression, &encoded_literal);
        let decoded_expression: AuthoredValue = serde_json::from_slice(&encoded_expression).unwrap();
        let decoded_literal: AuthoredValue = serde_json::from_slice(&encoded_literal).unwrap();
        prop_assert_eq!(decoded_expression, expression);
        prop_assert_eq!(decoded_literal, literal);
    }
}
