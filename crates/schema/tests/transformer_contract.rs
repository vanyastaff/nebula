use std::error::Error;

use nebula_schema::{Field, Transformer, field_key, transformer::RegexCapture};
use proptest::prelude::*;
use serde_json::{Value, json};

#[rstest::rstest]
#[case::unclosed_group("private-pattern-marker(")]
#[case::unclosed_class("[private-pattern-marker")]
#[case::unsupported_lookahead("(?=private-pattern-marker)")]
fn malformed_regex_configuration_is_rejected_by_serde(#[case] pattern: &str) {
    let error = serde_json::from_value::<Transformer>(json!({
        "kind": "regex", "pattern": pattern, "group": 0
    }))
    .unwrap_err();
    assert!(error.to_string().contains("transformer.invalid_pattern"));
    assert!(!error.to_string().contains("private-pattern-marker"));
    assert!(!format!("{error:?}").contains("private-pattern-marker"));
}

#[test]
fn unavailable_capture_group_is_rejected_by_serde() {
    let error = serde_json::from_value::<Transformer>(json!({
        "kind": "regex", "pattern": "(private-pattern-marker)", "group": 2
    }))
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("transformer.invalid_capture_group")
    );
    assert!(!error.to_string().contains("private-pattern-marker"));
}

#[test]
fn regex_wire_roundtrip_preserves_configuration_and_default_group() {
    let source = json!({"kind": "regex", "pattern": "([0-9]+)", "group": 1});
    let transformer: Transformer = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(serde_json::to_value(&transformer).unwrap(), source);
    let restored: Transformer =
        serde_json::from_value(serde_json::to_value(&transformer).unwrap()).unwrap();
    assert_eq!(restored, transformer);
    assert_eq!(restored.apply(&json!("prefix 42 suffix")), json!("42"));

    let default: Transformer = serde_json::from_value(json!({
        "kind": "regex", "pattern": "item=([0-9]+)"
    }))
    .unwrap();
    assert_eq!(default.apply(&json!("item=42")), json!("item=42"));
    assert_eq!(
        serde_json::to_value(default).unwrap(),
        json!({
            "kind": "regex", "pattern": "item=([0-9]+)", "group": 0
        })
    );
}

#[test]
fn no_match_and_unmatched_optional_capture_keep_original_string() {
    let transformer: Transformer = serde_json::from_value(json!({
        "kind": "regex", "pattern": "^item(?:=([0-9]+))?$", "group": 1
    }))
    .unwrap();
    for (input, expected) in [("other", "other"), ("item", "item"), ("item=42", "42")] {
        assert_eq!(transformer.apply(&json!(input)), json!(expected));
    }
}

#[test]
fn string_transformers_preserve_non_string_values() {
    let transformers = vec![
        Transformer::Trim,
        Transformer::Lowercase,
        Transformer::Uppercase,
        Transformer::Replace {
            from: "x".to_owned(),
            to: "y".to_owned(),
        },
        serde_json::from_value(json!({"kind": "regex", "pattern": "(x)", "group": 1})).unwrap(),
    ];
    for transformer in transformers {
        for input in [
            Value::Null,
            json!(true),
            json!(42),
            json!(["x"]),
            json!({"key": "x"}),
        ] {
            assert_eq!(transformer.apply(&input), input);
        }
    }
}

#[test]
fn checked_constructor_errors_keep_the_complete_source_chain_redacted() {
    let error = Transformer::regex("private-pattern-marker(", 0).unwrap_err();
    assert_eq!(error.code(), "transformer.invalid_pattern");
    assert!(error.path().is_root());
    assert!(error.params().is_empty());
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private-pattern-marker")
    );
    assert!(error.source().is_some(), "compiler cause must be retained");
    let mut cause: Option<&(dyn Error + 'static)> = Some(&error);
    while let Some(error) = cause {
        assert!(!error.to_string().contains("private-pattern-marker"));
        assert!(!format!("{error:?}").contains("private-pattern-marker"));
        assert!(error.downcast_ref::<regex::Error>().is_none());
        cause = error.source();
    }
}

#[test]
fn capture_index_bounds_include_the_full_match_and_reject_usize_max() {
    let pattern = "prefix=([0-9]+)";
    let full = RegexCapture::new(pattern, 0).unwrap();
    let group = RegexCapture::new(pattern, 1).unwrap();
    assert_eq!(full.pattern(), pattern);
    assert_eq!(full.group(), 0);
    assert_eq!(group.group(), 1);
    assert_eq!(
        Transformer::Regex(full).apply(&json!("prefix=42")),
        json!("prefix=42")
    );
    assert_eq!(
        Transformer::Regex(group).apply(&json!("prefix=42")),
        json!("42")
    );
    for group in [2, usize::MAX] {
        let error = RegexCapture::new(pattern, group).unwrap_err();
        assert_eq!(error.code(), "transformer.invalid_capture_group");
        assert!(
            error
                .params()
                .iter()
                .any(|(name, value)| name == "group" && value == &json!(group))
        );
        assert!(
            error
                .params()
                .iter()
                .any(|(name, value)| name == "capture_count" && value == &json!(2))
        );
    }
}

#[test]
fn regex_debug_omits_pattern_but_wire_retains_it() {
    let transformer = Transformer::regex("(private-pattern-marker)", 1).unwrap();
    assert!(!format!("{transformer:?}").contains("private-pattern-marker"));
    let clone = transformer.clone();
    assert_eq!(clone, transformer);
    assert_eq!(
        serde_json::to_value(&clone).unwrap(),
        json!({
            "kind": "regex", "pattern": "(private-pattern-marker)", "group": 1
        })
    );
    assert_eq!(
        clone.apply(&json!("private-pattern-marker")),
        json!("private-pattern-marker")
    );
}

#[test]
fn schema_field_wire_preserves_checked_transformer_metadata() {
    let transformer = Transformer::regex("^prefix=([0-9]+)$", 1).unwrap();
    let field = Field::from(
        Field::string(field_key!("identifier"))
            .with_transformer(Transformer::Trim)
            .with_transformer(transformer.clone()),
    );
    let wire = serde_json::to_value(&field).unwrap();
    assert_eq!(
        wire["transformers"],
        json!([
            {"kind": "trim"},
            {"kind": "regex", "pattern": "^prefix=([0-9]+)$", "group": 1}
        ])
    );
    let restored: Field = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(restored.transformers(), &[Transformer::Trim, transformer]);
    assert_eq!(serde_json::to_value(restored).unwrap(), wire);
}

#[test]
fn schema_field_cannot_deserialize_an_invalid_regex_transformer() {
    let field = Field::from(Field::string(field_key!("identifier")));
    let mut wire = serde_json::to_value(field).unwrap();
    wire["transformers"] = json!([{
        "kind": "regex", "pattern": "private-pattern-marker(", "group": 0
    }]);
    let error = serde_json::from_value::<Field>(wire).unwrap_err();
    assert!(error.to_string().contains("transformer.invalid_pattern"));
    assert!(!error.to_string().contains("private-pattern-marker"));
}

#[rstest::rstest]
#[case::trim(Transformer::Trim, "  hello  ", "hello", json!({"kind": "trim"}))]
#[case::lowercase(Transformer::Lowercase, "HeLLo", "hello", json!({"kind": "lowercase"}))]
#[case::uppercase(Transformer::Uppercase, "HeLLo", "HELLO", json!({"kind": "uppercase"}))]
#[case::replace(
    Transformer::Replace { from: "foo".to_owned(), to: "bar".to_owned() },
    "foo-foo", "bar-bar", json!({"kind": "replace", "from": "foo", "to": "bar"})
)]
fn existing_transformer_behaviors_and_wire_shapes_remain(
    #[case] transformer: Transformer,
    #[case] input: &str,
    #[case] expected: &str,
    #[case] wire: Value,
) {
    assert_eq!(transformer.apply(&json!(input)), json!(expected));
    assert_eq!(serde_json::to_value(&transformer).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<Transformer>(wire).unwrap(),
        transformer
    );
}

proptest! {
    #[test]
    fn regex_capture_roundtrip_keeps_extraction_behavior(value in 0_u64..=u64::MAX) {
        let transformer = Transformer::regex(r"^prefix=([0-9]+)$", 1).unwrap();
        let restored: Transformer =
            serde_json::from_value(serde_json::to_value(&transformer).unwrap()).unwrap();
        let input = json!(format!("prefix={value}"));
        prop_assert_eq!(transformer.apply(&input), json!(value.to_string()));
        prop_assert_eq!(restored.apply(&input), json!(value.to_string()));
        prop_assert_eq!(transformer, restored);
    }
}
