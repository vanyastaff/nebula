use nebula_schema::{__private::LiteralFieldKey, FieldKey, field_key};

#[test]
fn checked_literal_and_runtime_keys_share_identity() {
    const LITERAL: Option<LiteralFieldKey> = LiteralFieldKey::parse("Property_1");
    let literal = nebula_schema::__private::field_key_from_validated_literal(LITERAL.unwrap());
    let runtime = FieldKey::new("Property_1").unwrap();
    assert_eq!(literal, runtime);
    assert_eq!(field_key!("Property_1"), runtime);
    assert_eq!(serde_json::to_string(&literal).unwrap(), "\"Property_1\"");
}

#[test]
fn invalid_static_input_cannot_obtain_literal_token() {
    for invalid in [
        "",
        "1starts_with_digit",
        "has-dash",
        "has space",
        "a/b",
        "a~b",
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789___",
        "caf\u{e9}",
    ] {
        assert!(LiteralFieldKey::parse(invalid).is_none(), "{invalid}");
        assert!(FieldKey::new(invalid).is_err(), "{invalid}");
    }
}
