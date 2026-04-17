//! Proptest: FieldPath parse/display roundtrip.

use nebula_schema::FieldPath;
use proptest::prelude::*;

fn arb_segment() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,10}".prop_map(|s| s)
}

fn arb_path() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_segment(), 1..5).prop_map(|v| v.join("."))
}

proptest! {
    #[test]
    fn parse_display_roundtrip(p in arb_path()) {
        let parsed = FieldPath::parse(&p).expect("arb_path always valid");
        prop_assert_eq!(parsed.to_string(), p);
    }
}
