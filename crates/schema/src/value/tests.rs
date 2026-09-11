use std::assert_matches;

use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::*;
use crate::{Expression, SecretValue, ValidSchema};

fn literal(value: Value) -> AuthoredValue {
    AuthoredValue::Literal(ScalarValue::try_from(value).expect("fixture is a JSON scalar"))
}

#[test]
fn from_data_flat_literal() {
    let value = AuthoredValue::from_data(json!(42)).unwrap();
    let AuthoredValue::Literal(scalar) = value else {
        panic!("scalar JSON must become a literal leaf");
    };
    assert_eq!(scalar.as_json(), &json!(42));
}

#[test]
fn from_data_object_becomes_tree() {
    let value = AuthoredValue::from_data(json!({"a": 1, "b": "x"})).unwrap();
    let AuthoredValue::Object(properties) = value else {
        panic!("JSON objects must become object nodes");
    };
    assert_eq!(properties.len(), 2);
    assert_eq!(properties["a"].as_literal(), Some(&json!(1)));
    assert_eq!(properties["b"].as_literal(), Some(&json!("x")));
}

#[test]
fn detects_expression_wrapper() {
    let value = AuthoredValue::from_template_json(json!({"$expr": "{{ $x }}"})).unwrap();
    let AuthoredValue::Expression(expression) = value else {
        panic!("explicit shorthand must recognize an expression wrapper");
    };
    assert_eq!(expression.source(), "{{ $x }}");
}

#[test]
fn detects_inline_expression_marker() {
    let value = AuthoredValue::from_template_json(json!("hello {{ $y }}")).unwrap();
    let AuthoredValue::Expression(expression) = value else {
        panic!("explicit shorthand must recognize a mixed template");
    };
    assert_eq!(expression.source(), "hello {{ $y }}");
}

#[test]
fn data_braces_without_dollar_stay_literal() {
    let input = json!("hello {{ world }}");
    let value = AuthoredValue::from_data(input.clone()).unwrap();
    assert_eq!(value.as_literal(), Some(&input));
}

#[test]
fn multi_dollar_expr_is_expression() {
    let input = "{{ $a }} and {{ $b }}";
    let value = AuthoredValue::from_template_json(json!(input)).unwrap();
    let AuthoredValue::Expression(expression) = value else {
        panic!("explicit shorthand must retain both template segments");
    };
    assert_eq!(expression.parse().unwrap().source(), input);
}

#[test]
fn unclosed_braces_are_data_or_an_authored_parse_error() {
    let input = json!("text with {{ but no close");
    let data = AuthoredValue::from_data(input.clone()).unwrap();
    assert_eq!(data.as_literal(), Some(&input));
    let authored = AuthoredValue::from_template_json(input).unwrap();
    let AuthoredValue::Expression(expression) = authored else {
        panic!("an unescaped opener must be classified before parsing");
    };
    assert_eq!(expression.parse().unwrap_err().code(), "expression.parse");
}

#[test]
fn plain_text_stays_literal() {
    let input = json!("plain text");
    for value in [
        AuthoredValue::from_data(input.clone()).unwrap(),
        AuthoredValue::from_template_json(input.clone()).unwrap(),
    ] {
        assert_eq!(value.as_literal(), Some(&input));
    }
}

#[test]
fn template_without_dollar_is_an_expression() {
    let input = "{{ no_dollar }}";
    let value = AuthoredValue::from_template_json(json!(input)).unwrap();
    let AuthoredValue::Expression(expression) = value else {
        panic!("template classification must not depend on a dollar sign");
    };
    assert_eq!(expression.parse().unwrap().source(), input);
}

#[test]
fn expr_wrapper_requires_explicit_template_ingress() {
    let input = json!({"$expr": "anything"});
    let authored = AuthoredValue::from_template_json(input.clone()).unwrap();
    assert_matches!(authored, AuthoredValue::Expression(_));
    let data = AuthoredValue::from_data(input.clone()).unwrap();
    assert_matches!(data, AuthoredValue::Object(_));
    assert_eq!(data.to_json(), input);
}

#[test]
fn escaped_double_braces_stay_literal() {
    for source in ["{{{{ x }}}}", r"\{{ x }}", r"\\\{{ x }}"] {
        let value = AuthoredValue::from_template_json(json!(source)).unwrap();
        assert_eq!(value.as_literal(), Some(&json!(source)));
    }
    for source in [r"\\{{ x }}", r"\\\\{{ x }}"] {
        let value = AuthoredValue::from_template_json(json!(source)).unwrap();
        assert_matches!(value, AuthoredValue::Expression(_));
    }
}

#[test]
fn mode_like_object_stays_object() {
    let input = json!({"mode": "oauth2", "value": {"scope": "r"}});
    let value = AuthoredValue::from_data(input.clone()).unwrap();
    assert_matches!(value, AuthoredValue::Object(_));
    assert_eq!(value.to_json(), input);
}

#[test]
fn mode_with_extra_keys_stays_object() {
    let input = json!({"mode": "x", "value": null, "extra": 1});
    let value = AuthoredValue::from_data(input.clone()).unwrap();
    assert_matches!(value, AuthoredValue::Object(_));
    assert_eq!(value.to_json(), input);
}

#[test]
fn values_insert_get_path() {
    let mut values = AuthoredValue::object();
    values.insert_data("user", json!({"email": "a@b"})).unwrap();
    let path = ValuePath::parse("/user/email").unwrap();
    assert_eq!(
        values.get_path(&path).and_then(ValueTree::as_literal),
        Some(&json!("a@b"))
    );
}

#[test]
fn values_get_path_through_mode_value() {
    let mut values = AuthoredValue::object();
    values
        .insert_data(
            "auth",
            json!({
                "mode": "oauth",
                "value": {"token": "secret"}
            }),
        )
        .unwrap();
    let path = ValuePath::parse("/auth/value/token").unwrap();
    assert_eq!(
        values.get_path(&path).and_then(ValueTree::as_literal),
        Some(&json!("secret"))
    );
}

#[test]
fn from_data_accepts_arbitrary_nested_property_keys() {
    let input = json!({"user": {"bad-key": "x", "": 1, "a/b~c": 2, "0": 3}});
    let values = AuthoredValue::from_data(input.clone()).unwrap();
    assert_eq!(values.to_json(), input);
    for (pointer, expected) in [
        ("/user/bad-key", json!("x")),
        ("/user/", json!(1)),
        ("/user/a~1b~0c", json!(2)),
        ("/user/0", json!(3)),
    ] {
        assert_eq!(
            values
                .get_path(&ValuePath::parse(pointer).unwrap())
                .and_then(ValueTree::as_literal),
            Some(&expected)
        );
    }
}

#[test]
fn from_data_does_not_drop_or_hide_arbitrary_object_keys() {
    let input = json!({"bad-key": 1, "ok_key": 2});
    let parsed = AuthoredValue::from_data(input.clone()).unwrap();
    assert_matches!(parsed, AuthoredValue::Object(_));
    assert_eq!(parsed.to_json(), input);
    assert_eq!(
        ScalarValue::try_from(input).unwrap_err().code(),
        "type_mismatch"
    );
}

#[test]
fn insert_data_preserves_nested_keys_and_rejects_scalar_receivers() {
    let mut values = AuthoredValue::object();
    values
        .insert_data("config", json!({"bad-key": "x"}))
        .unwrap();
    assert_eq!(values.to_json(), json!({"config": {"bad-key": "x"}}));

    let mut scalar = literal(json!(0));
    let error = scalar
        .insert_data("config", json!({"bad-key": "x"}))
        .unwrap_err();
    assert_eq!(error.code(), "type_mismatch");
    assert_eq!(scalar.as_literal(), Some(&json!(0)));
}

#[test]
fn insert_expression_is_explicit_and_insert_data_never_interpolates() {
    let mut values = AuthoredValue::object();
    let input = json!({"$expr": "{{ $x }}"});
    values
        .insert(
            "expr",
            AuthoredValue::from_template_json(input.clone()).unwrap(),
        )
        .unwrap();
    values.insert_data("data", input.clone()).unwrap();
    assert_matches!(values.get("expr"), Some(AuthoredValue::Expression(_)));
    assert_matches!(values.get("data"), Some(AuthoredValue::Object(_)));
    assert_eq!(values.get("data").unwrap().to_json(), input);
}

fn nested_object(depth: usize, inner_key: &str) -> Value {
    let mut current = json!({"leaf": 1});
    for _ in 0..depth {
        let mut wrapped = Map::with_capacity(1);
        wrapped.insert(inner_key.to_owned(), current);
        current = Value::Object(wrapped);
    }
    current
}

#[test]
fn from_data_rejects_deeply_nested_object_with_recursion_limit() {
    let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
    let error = AuthoredValue::from_data(json!({"top": deep})).expect_err("must reject deep data");
    assert_eq!(error.code(), "recursion_limit");
}

#[test]
fn from_template_json_rejects_deeply_nested_input() {
    let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
    let error =
        AuthoredValue::from_template_json(deep).expect_err("must reject deep authored data");
    assert_eq!(error.code(), "recursion_limit");
}

#[test]
fn authored_wire_deserialize_rejects_deeply_nested_input() {
    assert_eq!(MAX_VALUE_DEPTH, 64);
    let mut data = Value::Null;
    for _ in 0..usize::from(MAX_VALUE_DEPTH) {
        data = json!([data]);
    }
    let wire = json!({"version": 2, "data": data, "expressions": []});
    let admitted: AuthoredValue = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(admitted.to_json(), data);
    assert_eq!(admitted.canonical_bytes().unwrap()[18], 0x06);
    let decoded: AuthoredValue =
        serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap();
    assert_eq!(decoded, admitted);

    let too_deep = json!({"version": 2, "data": [data], "expressions": []});
    let error = serde_json::from_slice::<AuthoredValue>(&serde_json::to_vec(&too_deep).unwrap())
        .unwrap_err();
    assert!(error.to_string().contains("recursion_limit"), "{error}");
    let error = serde_json::from_value::<AuthoredValue>(too_deep).unwrap_err();
    assert!(error.to_string().contains("recursion_limit"), "{error}");
}

#[test]
fn authored_wire_rejects_excessive_data_nodes_from_materialized_json() {
    let wire = json!({
        "version": 2,
        "data": vec![Value::Null; MAX_VALUE_NODES],
        "expressions": [],
    });
    let error = serde_json::from_value::<AuthoredValue>(wire).unwrap_err();
    assert!(error.to_string().contains("data nodes"), "{error}");
}

#[test]
fn authored_wire_rejects_excessive_data_text_from_materialized_json() {
    let wire = json!({
        "version": 2,
        "data": "x".repeat(MAX_VALUE_TEXT_BYTES + 1),
        "expressions": [],
    });
    let error = serde_json::from_value::<AuthoredValue>(wire).unwrap_err();
    assert!(error.to_string().contains("data text bytes"), "{error}");
}

#[test]
fn authored_wire_rejects_excessive_borrowed_key_from_bytes() {
    let oversized_key = "x".repeat(MAX_VALUE_TEXT_BYTES + 1);
    let wire = format!(r#"{{"version":2,"data":{{"{oversized_key}":null}},"expressions":[]}}"#);
    let error = serde_json::from_slice::<AuthoredValue>(wire.as_bytes()).unwrap_err();
    assert!(error.to_string().contains("data text bytes"), "{error}");
}

#[test]
fn authored_wire_rejects_excessive_expression_entries() {
    let entry = json!({"path": "", "syntax": "auto", "source": "1"});
    let wire = json!({
        "version": 2,
        "data": null,
        "expressions": vec![entry; MAX_EXPRESSION_ENTRIES + 1],
    });
    let error = serde_json::from_value::<AuthoredValue>(wire).unwrap_err();
    assert!(error.to_string().contains("expression entries"), "{error}");
}

#[test]
fn authored_wire_rejects_excessive_expression_text() {
    let wire = json!({
        "version": 2,
        "data": null,
        "expressions": [{
            "path": "",
            "syntax": "auto",
            "source": "x".repeat(MAX_EXPRESSION_TEXT_BYTES + 1),
        }],
    });
    let error = serde_json::from_value::<AuthoredValue>(wire).unwrap_err();
    assert!(
        error.to_string().contains("expression text bytes"),
        "{error}"
    );
}

#[test]
fn authored_wire_rejects_excessive_expression_text_from_bytes() {
    let wire = serde_json::to_vec(&json!({
        "version": 2,
        "data": null,
        "expressions": [{
            "path": "",
            "syntax": "auto",
            "source": "x".repeat(MAX_EXPRESSION_TEXT_BYTES + 1),
        }],
    }))
    .unwrap();
    let error = serde_json::from_slice::<AuthoredValue>(&wire).unwrap_err();
    assert!(
        error.to_string().contains("expression text bytes"),
        "{error}"
    );
}

#[test]
fn data_constructor_enforces_value_node_budget() {
    let error = AuthoredValue::from_data(Value::Array(vec![Value::Null; MAX_VALUE_NODES]))
        .expect_err("the root plus maximum child nodes must exceed the budget");
    assert_eq!(error.code(), "value.limit_exceeded");
    assert_eq!(
        error
            .params()
            .iter()
            .find_map(|(key, value)| (key == "resource").then(|| value.as_str()).flatten()),
        Some("data nodes")
    );
}

#[test]
fn data_constructor_enforces_value_text_budget() {
    let error = AuthoredValue::from_data(Value::String("x".repeat(MAX_VALUE_TEXT_BYTES + 1)))
        .expect_err("oversized text must be rejected before tree construction");
    assert_eq!(error.code(), "value.limit_exceeded");
    assert_eq!(
        error
            .params()
            .iter()
            .find_map(|(key, value)| (key == "resource").then(|| value.as_str()).flatten()),
        Some("data text bytes")
    );
}

#[test]
fn template_constructor_enforces_expression_entry_budget() {
    let input = Value::Array(vec![json!("{{ 1 }}"); MAX_EXPRESSION_ENTRIES + 1]);
    let error = AuthoredValue::from_template_json(input)
        .expect_err("too many template expressions must be rejected");
    assert_eq!(error.code(), "value.limit_exceeded");
    assert_eq!(
        error
            .params()
            .iter()
            .find_map(|(key, value)| (key == "resource").then(|| value.as_str()).flatten()),
        Some("expression entries")
    );
}

#[test]
fn authored_serializer_enforces_all_value_budgets_before_writing() {
    let authored = AuthoredValue::List(vec![
        AuthoredValue::Literal(
            ScalarValue::try_from(Value::Null).unwrap()
        );
        MAX_VALUE_NODES
    ]);
    let mut bytes = Vec::new();
    let error = serde_json::to_writer(&mut bytes, &authored)
        .expect_err("an oversized in-memory tree must not produce invalid wire data");
    assert!(error.to_string().contains("data nodes"), "{error}");
    assert!(bytes.is_empty());

    let authored =
        AuthoredValue::Expression(Expression::new("x".repeat(MAX_EXPRESSION_TEXT_BYTES + 1)));
    let mut bytes = Vec::new();
    let error = serde_json::to_writer(&mut bytes, &authored)
        .expect_err("an oversized expression must not produce invalid wire data");
    assert!(
        error.to_string().contains("expression text bytes"),
        "{error}"
    );
    assert!(bytes.is_empty());
}

#[test]
fn authored_wire_rejects_unsupported_versions_and_legacy_variants() {
    for wire in [
        json!({"version": 1, "data": null, "expressions": []}),
        json!({"version": 3, "data": null, "expressions": []}),
        json!({"kind": "literal", "value": null}),
        json!({"kind": "unknown_variant", "value": null}),
    ] {
        assert!(serde_json::from_value::<AuthoredValue>(wire.clone()).is_err());
        assert!(
            serde_json::from_slice::<AuthoredValue>(&serde_json::to_vec(&wire).unwrap()).is_err()
        );
    }
}

#[test]
fn from_data_checks_depth_inside_arbitrary_key_objects() {
    let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
    let error =
        AuthoredValue::from_data(json!({"bad-key": deep})).expect_err("must reject deep data");
    assert_eq!(error.code(), "recursion_limit");
}

#[test]
fn over_deep_subtrees_cannot_be_hidden_in_composite_scalars() {
    let deep = nested_object(usize::from(MAX_VALUE_DEPTH) + 5, "n");
    let error = ScalarValue::try_from(deep.clone()).unwrap_err();
    assert_eq!(error.code(), "type_mismatch");
    let error = AuthoredValue::from_data(deep).unwrap_err();
    assert_eq!(error.code(), "recursion_limit");
}

#[test]
fn from_data_accepts_exact_recursion_limit() {
    let input = nested_object(usize::from(MAX_VALUE_DEPTH) - 1, "n");
    let admitted = AuthoredValue::from_data(input.clone()).unwrap();
    assert_eq!(admitted.to_json(), input);
    let error = AuthoredValue::from_data(json!({"top": input})).unwrap_err();
    assert_eq!(error.code(), "recursion_limit");
}

#[test]
fn roundtrip_preserves_structure() {
    let input = json!({
        "a": 1,
        "b": [1, 2, {"x": true}],
        "c": {"$expr": "{{ $x }}"},
        "d": {"mode": "m", "value": "v"}
    });
    let parsed = AuthoredValue::from_template_json(input.clone()).unwrap();
    assert_eq!(parsed.to_json(), input);
    let wire = serde_json::to_value(&parsed).unwrap();
    let decoded: AuthoredValue = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded, parsed);
}

#[test]
fn from_data_accepts_every_json_root_without_interpolation() {
    for input in [
        Value::Null,
        json!(true),
        json!(42),
        json!("{{ $input.secret }}"),
        json!([null, {"": "{{ malformed"}]),
        json!({"$expr": "{{ $x }}"}),
    ] {
        let value = AuthoredValue::from_data(input.clone()).unwrap();
        assert_eq!(value.to_json(), input);
    }
}

#[test]
fn root_empty_property_is_distinct_from_missing_and_null() {
    let values = AuthoredValue::from_data(json!({"": null})).unwrap();
    assert_eq!(values.get_path(&ValuePath::root()), Some(&values));
    assert_eq!(
        values
            .get_path(&ValuePath::parse("/").unwrap())
            .and_then(ValueTree::as_literal),
        Some(&Value::Null)
    );
    assert!(
        values
            .get_path(&ValuePath::parse("/missing").unwrap())
            .is_none()
    );
}

// ── canonical_bytes / ContentId ──────────────────────────────────────────

fn canon(value: &AuthoredValue) -> Vec<u8> {
    value.canonical_bytes().expect("canonicalizable")
}

fn key(value: &str) -> String {
    value.to_owned()
}

/// Equal integer/float spellings share an encoding; negative zero normalizes to zero.
#[test]
fn canon_normalizes_integral_numbers() {
    let int = literal(json!(1));
    let float = literal(json!(1.0));
    let neg_zero = literal(Value::from(-0.0_f64));
    let zero = literal(json!(0));
    assert_eq!(canon(&int), canon(&float), "1 and 1.0 share a canon");
    assert_eq!(canon(&neg_zero), canon(&zero), "-0.0 and 0 share a canon");

    let frac = literal(json!(1.5));
    assert_ne!(canon(&int), canon(&frac), "1 and 1.5 differ");
}

/// Length-prefixing prevents concatenation aliases; shape tags separate containers.
#[test]
fn canon_is_injective_across_shapes() {
    let ab = AuthoredValue::List(vec![literal(json!("a")), literal(json!("b"))]);
    let concat = AuthoredValue::List(vec![literal(json!("ab"))]);
    assert_ne!(
        canon(&ab),
        canon(&concat),
        "length-prefix blocks concatenation alias"
    );

    let empty_list = AuthoredValue::List(vec![]);
    let empty_obj = AuthoredValue::Object(IndexMap::new());
    assert_ne!(
        canon(&empty_list),
        canon(&empty_obj),
        "list tag != object tag"
    );

    let input = json!({"k": 1});
    assert_eq!(
        ScalarValue::try_from(input.clone()).unwrap_err().code(),
        "type_mismatch"
    );
    let parsed = AuthoredValue::from_data(input).unwrap();
    let mut typed = IndexMap::new();
    typed.insert(key("k"), literal(json!(1)));
    let typed_obj = AuthoredValue::Object(typed);
    assert_eq!(
        canon(&parsed),
        canon(&typed_obj),
        "an object has only one canonical tree representation"
    );
}

/// Object canon is independent of key insertion order.
#[test]
fn canon_object_key_order_invariant() {
    let mut forward = IndexMap::new();
    forward.insert(key("alpha"), literal(json!(1)));
    forward.insert(key("beta"), literal(json!(2)));
    forward.insert(key("gamma"), literal(json!(3)));

    let mut reversed = IndexMap::new();
    reversed.insert(key("gamma"), literal(json!(3)));
    reversed.insert(key("beta"), literal(json!(2)));
    reversed.insert(key("alpha"), literal(json!(1)));

    assert_eq!(
        canon(&AuthoredValue::Object(forward)),
        canon(&AuthoredValue::Object(reversed)),
        "key order must not affect the canon"
    );
}

/// A secret value has no canonical form — it must not enter a content hash.
#[test]
fn canon_rejects_secret() {
    let secret = AuthoredValue::Secret(SecretValue::String(crate::secret::SecretString::new(
        "hunter2".to_owned(),
    )));
    let err = secret
        .canonical_bytes()
        .expect_err("secrets are not hashable");
    assert_eq!(err.code(), "secret.not_hashable");
    assert!(
        secret.content_id().is_err(),
        "content_id propagates the rejection"
    );
}

#[test]
fn content_id_is_deterministic_and_hex() {
    let value = AuthoredValue::Object({
        let mut map = IndexMap::new();
        map.insert(key("k"), literal(json!("v")));
        map
    });
    let id_a = value.content_id().unwrap();
    let id_b = value.content_id().unwrap();
    assert_eq!(id_a, id_b, "same value, same id");
    let hex = id_a.to_string();
    assert_eq!(hex.len(), 64, "32 bytes render as 64 hex chars");
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
}

/// Ingested objects and directly constructed objects share one canonical encoding.
#[test]
fn data_canon_matches_equivalent_object() {
    let values = AuthoredValue::from_data(json!({"a": 1, "b": "x"})).unwrap();
    let mut map = IndexMap::new();
    map.insert(key("a"), literal(json!(1)));
    map.insert(key("b"), literal(json!("x")));
    let object = AuthoredValue::Object(map);
    assert_eq!(
        values.canonical_bytes().unwrap(),
        canon(&object),
        "data ingress and explicit object construction share a canon"
    );
}

/// The normalization invariant must hold across the WHOLE i64/u64 range, not
/// only |x| < 2^53: an integer near 2^63 spelled as int vs as float must
/// still share a canon. (Guards against the off-by-`<` at the 2^53 bound.)
#[test]
fn canon_normalizes_large_integers() {
    for n in [
        9_007_199_254_740_992_i64,     // 2^53 (the old boundary, exclusive)
        9_007_199_254_740_994_i64,     // 2^53 + 2 (next exact f64 integer above 2^53)
        4_503_599_627_370_497_i64,     // odd, within 2^53
        1_152_921_504_606_846_976_i64, // 2^60
    ] {
        // Only test values that round-trip exactly through f64 (so the float
        // spelling denotes the same integer).
        #[expect(clippy::cast_precision_loss, reason = "checked exact below")]
        let as_float = n as f64;
        #[expect(clippy::cast_possible_truncation, reason = "checked exact below")]
        let back = as_float as i64;
        assert_eq!(back, n, "fixture must be exactly representable as a float");
        let int = literal(json!(n));
        let float = literal(Value::from(as_float));
        assert_eq!(
            canon(&int),
            canon(&float),
            "{n} as int and as float must share a canon"
        );
    }
}

/// A secret nested inside a container still rejects (recursion propagates it).
#[test]
fn canon_rejects_nested_secret() {
    let secret = || {
        AuthoredValue::Secret(SecretValue::String(crate::secret::SecretString::new(
            "s".to_owned(),
        )))
    };
    let in_list = AuthoredValue::List(vec![literal(json!(1)), secret()]);
    let mut map = IndexMap::new();
    map.insert(key("k"), secret());
    let in_object = AuthoredValue::Object(map);
    let in_mode = AuthoredValue::Object(indexmap::indexmap! {
        key("mode") => literal(json!("m")),
        key("value") => secret(),
    });
    for value in [in_list, in_object, in_mode] {
        assert_eq!(
            value
                .canonical_bytes()
                .expect_err("nested secret rejects")
                .code(),
            "secret.not_hashable"
        );
    }
}

/// A missing payload, explicit null, a value, and another mode remain distinct.
#[test]
fn canon_mode_variants_are_distinct() {
    let none = AuthoredValue::from_data(json!({"mode": "m"})).unwrap();
    let null = AuthoredValue::from_data(json!({"mode": "m", "value": null})).unwrap();
    let some = AuthoredValue::from_data(json!({"mode": "m", "value": 0})).unwrap();
    let other_key = AuthoredValue::from_data(json!({"mode": "n"})).unwrap();
    assert_ne!(canon(&none), canon(&some));
    assert_ne!(canon(&none), canon(&other_key));
    assert_ne!(canon(&some), canon(&other_key));
    assert_ne!(canon(&none), canon(&null));
    assert_ne!(canon(&some), canon(&null));
    assert_ne!(canon(&other_key), canon(&null));
}

/// Empty string, list, and object are mutually distinct and non-degenerate.
#[test]
fn canon_empty_containers_are_distinct() {
    let empty_string = literal(json!(""));
    let empty_list = AuthoredValue::List(vec![]);
    let empty_object = AuthoredValue::Object(IndexMap::new());
    let canons = [
        canon(&empty_string),
        canon(&empty_list),
        canon(&empty_object),
    ];
    for (i, a) in canons.iter().enumerate() {
        for b in &canons[i + 1..] {
            assert_ne!(a, b, "empty containers must not alias");
        }
    }
}

/// Arrays have no scalar escape hatch and always share the list encoding.
#[test]
fn canon_array_has_one_tree_representation() {
    let input = json!([1, 2]);
    assert_eq!(
        ScalarValue::try_from(input.clone()).unwrap_err().code(),
        "type_mismatch"
    );
    let parsed = AuthoredValue::from_data(input).unwrap();
    let typed = AuthoredValue::List(vec![literal(json!(1)), literal(json!(2))]);
    assert_eq!(canon(&parsed), canon(&typed));
}

#[test]
fn canon_expression_is_distinct_from_lookalike_data() {
    let source = "{{ $input.value }}";
    let expression = AuthoredValue::Expression(Expression::new(source));
    let string = literal(json!(source));
    let wrapper = AuthoredValue::from_data(json!({"$expr": source})).unwrap();
    assert_ne!(canon(&expression), canon(&string));
    assert_ne!(canon(&expression), canon(&wrapper));
    assert_ne!(canon(&string), canon(&wrapper));
}

#[test]
fn content_id_separates_distinct_values() {
    let a = AuthoredValue::from_data(json!({"n": 1})).unwrap();
    let b = AuthoredValue::from_data(json!({"n": 2})).unwrap();
    assert_ne!(
        a.content_id().unwrap(),
        b.content_id().unwrap(),
        "distinct values must have distinct content ids"
    );
}

/// Multi-byte varint: a 200-element string length encodes as LEB128
/// `[0xC8, 0x01]` (200 = 0x48 | continuation, then 1).
#[test]
fn canon_varint_is_multibyte_for_large_lengths() {
    let long = "a".repeat(200);
    let bytes = canon(&literal(json!(long)));
    // After domain (16) + version (2) + TAG_STRING (1) comes the varint length.
    assert_eq!(
        &bytes[19..21],
        &[0xC8, 0x01],
        "200 encodes as a 2-byte varint"
    );
}

/// Golden bytes — freeze the exact on-the-wire content-address format so any
/// silent re-keying (tag value, framing order, version) is a loud failure.
#[test]
fn canon_golden_bytes() {
    let value = AuthoredValue::from_data(json!({"a": 1})).unwrap();
    let bytes = value.canonical_bytes().unwrap();

    let mut expected = b"nbschema-value-v".to_vec();
    expected.extend_from_slice(&[0x00, 0x02]); // VALUE_CANON_VERSION = 2
    expected.push(0x07); // TAG_OBJECT
    expected.push(0x01); // entry count (varint 1)
    expected.extend_from_slice(&[0x01, b'a']); // key "a" (len 1 + bytes)
    expected.push(0x03); // TAG_INT
    expected.extend_from_slice(&1_i128.to_be_bytes()); // value 1 (i128 BE)

    assert_eq!(bytes, expected, "canonical format must not drift");
    assert_eq!(
        &bytes[16..18],
        &VALUE_CANON_VERSION.to_be_bytes(),
        "the version prefix is pinned"
    );
}

/// Direct construction cannot bypass the canonical encoder's depth guard.
#[test]
fn canon_rejects_over_deep_value() {
    let mut value = literal(Value::Null);
    for _ in 0..(usize::from(MAX_VALUE_DEPTH) + 5) {
        value = AuthoredValue::List(vec![value]);
    }
    assert_eq!(
        value
            .canonical_bytes()
            .expect_err("over-deep value must error, not overflow")
            .code(),
        "recursion_limit"
    );
}

#[test]
fn proof_boundary_rejects_manually_constructed_excessive_node_count() {
    let values = AuthoredValue::List(vec![literal(Value::Null); MAX_VALUE_NODES]);

    let report = ValidSchema::any()
        .validate(values)
        .expect_err("the root plus the maximum child count exceeds the cumulative node budget");
    assert_eq!(
        report.errors().next().unwrap().code(),
        "value.limit_exceeded"
    );
}

#[test]
fn proof_boundary_rejects_manually_constructed_excessive_text() {
    let values = literal(Value::String("x".repeat(MAX_VALUE_TEXT_BYTES + 1)));

    let report = ValidSchema::any()
        .validate(values)
        .expect_err("manual scalar construction must not bypass cumulative text limits");
    assert_eq!(
        report.errors().next().unwrap().code(),
        "value.limit_exceeded"
    );
}

#[test]
fn proof_boundary_rejects_manually_constructed_excessive_expressions() {
    let expression = AuthoredValue::Expression(Expression::new("1"));
    let values = AuthoredValue::List(vec![expression; MAX_EXPRESSION_ENTRIES + 1]);
    let report = ValidSchema::any()
        .validate(values)
        .expect_err("manual expression variants must not bypass the entry budget");
    assert_eq!(
        report.errors().next().unwrap().code(),
        "value.limit_exceeded"
    );

    let values =
        AuthoredValue::Expression(Expression::new("x".repeat(MAX_EXPRESSION_TEXT_BYTES + 1)));
    let report = ValidSchema::any()
        .validate(values)
        .expect_err("manual expression variants must not bypass the source-text budget");
    assert_eq!(
        report.errors().next().unwrap().code(),
        "value.limit_exceeded"
    );
}
