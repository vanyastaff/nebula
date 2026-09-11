//! Proptest: schema paths and RFC6901 data paths retain their distinct grammars.

use nebula_schema::{FieldPath, ValuePath};
use proptest::prelude::*;

fn arb_segment() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,10}"
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

    #[test]
    fn data_pointer_preserves_arbitrary_and_empty_segments(
        segments in prop::collection::vec(".{0,10}", 0..6)
    ) {
        let path = ValuePath::from_segments(&segments);
        let decoded = ValuePath::from_pointer(path.as_str()).unwrap();
        let recovered: Vec<_> = decoded.segments().map(std::borrow::Cow::into_owned).collect();
        prop_assert_eq!(recovered, segments);
        prop_assert_eq!(decoded, path);
    }
}
