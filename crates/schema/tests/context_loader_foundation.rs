use std::convert::Infallible;

use nebula_expression::CompiledProgram;
use nebula_schema::{
    AuthoredValue, Expression, Field, LoaderContext, LoaderRegistry, LoaderResult, ResolvedValue,
    Schema, SecretValue, ValidSchema, ValuePath, ValueTree,
    context::{predicate_context_for, root_predicate_context_for},
    field_key,
    secret::SECRET_REDACTED,
    value::MAX_VALUE_DEPTH,
};
use nebula_validator::PredicateContext;
use proptest::prelude::*;
use serde_json::{Value, json};

fn context_value<'a>(context: &'a PredicateContext, pointer: &str) -> Option<&'a Value> {
    context.get(&ValuePath::parse(pointer).unwrap())
}

fn secret_object() -> nebula_schema::ObjectField {
    Field::object(field_key!("credentials"))
        .add(
            Field::secret(field_key!("token"))
                .read_alias("old_token")
                .unwrap()
                .read_alias("older_token")
                .unwrap(),
        )
        .add(
            Field::string(field_key!("region"))
                .read_alias("area")
                .unwrap(),
        )
}

fn aliased_schema() -> ValidSchema {
    Schema::builder()
        .add(
            Field::object(field_key!("config"))
                .read_alias("legacy_config")
                .unwrap()
                .add(secret_object().read_alias("legacy_credentials").unwrap()),
        )
        .add(Field::list(field_key!("rows")).item(secret_object()))
        .add(
            Field::mode(field_key!("auth"))
                .read_alias("legacy_auth")
                .unwrap()
                .variant("oauth", "OAuth", secret_object())
                .default_variant("oauth"),
        )
        .build()
        .unwrap()
}

#[test]
fn raw_contexts_fold_all_aliases_through_objects_lists_and_modes() {
    let schema = aliased_schema();
    let values = AuthoredValue::from_data(json!({
        "legacy_config": {
            "legacy_credentials": {
                "token": "canonical-secret",
                "old_token": {"arbitrary": "losing-secret"},
                "older_token": "another-losing-secret",
                "area": "eu",
                "undeclared": "smuggled-secret"
            }
        },
        "rows": [{"old_token": "row-secret", "older_token": "loser", "area": "us"}],
        "legacy_auth": {"value": {"older_token": "mode-secret", "area": "au"}}
    }))
    .unwrap();

    for context in [
        predicate_context_for(schema.fields(), &values).unwrap(),
        root_predicate_context_for(schema.fields(), &values).unwrap(),
    ] {
        assert_eq!(
            context_value(&context, "/config"),
            Some(&json!({
                "credentials": {"region": "eu"}
            }))
        );
        assert_eq!(
            context_value(&context, "/rows"),
            Some(&json!([{"region": "us"}]))
        );
        assert_eq!(
            context_value(&context, "/auth"),
            Some(&json!({"value": {"region": "au"}}))
        );
        for pointer in [
            "/legacy_config",
            "/legacy_auth",
            "/config/legacy_credentials",
            "/config/credentials/token",
            "/config/credentials/old_token",
            "/config/credentials/older_token",
            "/config/credentials/area",
            "/config/credentials/undeclared",
            "/auth/value/older_token",
        ] {
            assert_eq!(context_value(&context, pointer), None, "{pointer}");
        }
    }
}

#[test]
fn canonical_input_wins_and_first_declared_alias_wins_otherwise() {
    let fields = vec![Field::from(
        Field::string(field_key!("region"))
            .read_alias("first")
            .unwrap()
            .read_alias("second")
            .unwrap(),
    )];
    for (json, expected) in [
        (json!({"second": "us", "first": "eu"}), "eu"),
        (json!({"second": "us", "first": "eu", "region": "au"}), "au"),
    ] {
        let values = AuthoredValue::from_data(json).unwrap();
        let context = predicate_context_for(&fields, &values).unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context_value(&context, "/region"), Some(&json!(expected)));
    }
}

fn assert_generic_context<E>(expression: Option<E>) {
    let fields = vec![Field::from(
        Field::secret(field_key!("token"))
            .read_alias("old_token")
            .unwrap(),
    )];
    let mut values = ValueTree::<E>::from_data(json!({
        "old_token": "schema-secret",
        "data": {"": "empty-key", "a/b": {"~": "tilde"}, "01": "numeric", "a.b": 2},
        "literal": {"$expr": "{{ this.is.data }}"}
    }))
    .unwrap();
    let mut nested = ValueTree::object();
    nested
        .insert(
            "a/~",
            ValueTree::Secret(SecretValue::string("explicit-secret".to_owned())),
        )
        .unwrap();
    nested
        .insert(
            "list",
            ValueTree::List(vec![
                ValueTree::Secret(SecretValue::string("list-secret".to_owned())),
                ValueTree::from_data(json!(42)).unwrap(),
            ]),
        )
        .unwrap();
    if let Some(expression) = expression {
        nested
            .insert("code", ValueTree::Expression(expression))
            .unwrap();
    }
    values.insert("opaque", nested).unwrap();
    for context in [
        predicate_context_for(&fields, &values).unwrap(),
        root_predicate_context_for(&fields, &values).unwrap(),
    ] {
        assert_eq!(context_value(&context, "/token"), None);
        assert_eq!(context_value(&context, "/old_token"), None);
        assert_eq!(
            context_value(&context, "/opaque"),
            Some(&json!({"list": [null, 42]}))
        );
        assert_eq!(context_value(&context, "/opaque/code"), None);
        assert_eq!(context_value(&context, "/data/"), Some(&json!("empty-key")));
        assert_eq!(
            context_value(&context, "/data/a~1b/~0"),
            Some(&json!("tilde"))
        );
        assert_eq!(context_value(&context, "/data/01"), Some(&json!("numeric")));
        assert_eq!(context_value(&context, "/data/a.b"), Some(&json!(2)));
        assert_eq!(
            context_value(&context, "/literal"),
            Some(&json!({"$expr": "{{ this.is.data }}"}))
        );
    }
}

#[test]
fn authored_context_excludes_expression_sources_at_arbitrary_paths() {
    assert_generic_context(Some(Expression::new("expression-source-marker")));
}

#[test]
fn compiled_context_excludes_retained_program_sources() {
    assert_generic_context(Some(
        CompiledProgram::compile_expression("'expression-source-marker'").unwrap(),
    ));
}

#[test]
fn resolved_context_scrubs_explicit_and_aliased_secrets_without_parsing_data() {
    assert_generic_context::<Infallible>(None);
}

#[test]
fn context_projection_does_not_require_expression_clone_or_debug() {
    struct UnavailableExpression;
    assert_generic_context(Some(UnavailableExpression));
}

#[test]
fn malformed_secret_containers_and_unknown_modes_are_unavailable() {
    let schema = aliased_schema();
    for selector in [
        json!("unknown"),
        json!({"token": "selector-secret"}),
        json!(17),
    ] {
        let values = AuthoredValue::from_data(json!({
            "legacy_config": [{"old_token": "wrong-object-secret"}],
            "rows": {"old_token": "wrong-list-secret"},
            "legacy_auth": {
                "mode": selector,
                "value": {"old_token": "unknown-mode-secret"}
            }
        }))
        .unwrap();
        let context = predicate_context_for(schema.fields(), &values).unwrap();
        assert_eq!(context_value(&context, "/config"), None);
        assert_eq!(context_value(&context, "/rows"), None);
        assert_eq!(context_value(&context, "/auth/value"), None);
        let expected = if selector.is_string() {
            json!({"mode": "unknown"})
        } else {
            json!({})
        };
        assert_eq!(context_value(&context, "/auth"), Some(&expected));
        let loader = LoaderContext::new("field", values)
            .with_secrets_redacted(&schema)
            .unwrap();
        let snapshot = loader.values().to_json();
        assert_eq!(snapshot["config"], json!(SECRET_REDACTED));
        assert_eq!(snapshot["rows"], json!(SECRET_REDACTED));
        assert_eq!(snapshot["auth"]["value"], json!(SECRET_REDACTED));
        assert_eq!(
            snapshot["auth"]["mode"],
            if selector.is_string() {
                json!("unknown")
            } else {
                json!(SECRET_REDACTED)
            }
        );
    }
}

#[test]
fn loader_snapshot_redacts_aliased_expression_sources_and_preserves_literal_data() {
    let schema = aliased_schema();
    let mut values = AuthoredValue::from_template_json(json!({
        "legacy_config": {"legacy_credentials": {
            "old_token": {"$expr": "secret.source"},
            "older_token": "losing-secret",
            "area": "eu"
        }},
        "rows": [{"old_token": "row-secret", "area": "us"}],
        "legacy_auth": {"value": {"old_token": "mode-secret", "area": "au"}},
        "unknown": {"expression": {"$expr": "unknown.source"}}
    }))
    .unwrap();
    values
        .insert_data("literal", json!({"$expr": "{{ literal.source }}"}))
        .unwrap();
    values
        .insert(
            "explicit/~",
            ValueTree::Secret(SecretValue::string("explicit-secret".to_owned())),
        )
        .unwrap();
    let snapshot = LoaderContext::new("field", values)
        .with_secrets_redacted(&schema)
        .unwrap();
    assert_eq!(
        snapshot.values().to_json(),
        json!({
            "config": {"credentials": {"token": SECRET_REDACTED, "region": "eu"}},
            "rows": [{"token": SECRET_REDACTED, "region": "us"}],
            "auth": {"value": {"token": SECRET_REDACTED, "region": "au"}},
            "unknown": {"expression": SECRET_REDACTED},
            "literal": {"$expr": "{{ literal.source }}"},
            "explicit/~": SECRET_REDACTED
        })
    );
    assert_eq!(
        LoaderContext::new("field", snapshot.values().clone())
            .with_secrets_redacted(&schema)
            .unwrap()
            .values(),
        snapshot.values(),
        "redaction must be idempotent"
    );
}

#[tokio::test]
async fn schema_dispatch_never_receives_code_or_explicit_secret_nodes() {
    let schema = Schema::builder()
        .add(Field::dynamic(field_key!("field")).loader("capture"))
        .build()
        .unwrap();
    let registry = LoaderRegistry::new().register_record("capture", |context| async move {
        Ok(LoaderResult::done(vec![context.values.to_json()]))
    });
    let mut values = AuthoredValue::object();
    values
        .insert(
            "code",
            ValueTree::Expression(Expression::new("source-marker")),
        )
        .unwrap();
    values
        .insert(
            "token",
            ValueTree::Secret(SecretValue::string("secret-marker".to_owned())),
        )
        .unwrap();
    let result = schema
        .load_dynamic_records("field", &registry, LoaderContext::new("field", values))
        .await
        .unwrap();
    assert_eq!(
        result.items,
        vec![json!({"code": SECRET_REDACTED, "token": SECRET_REDACTED})]
    );
}

#[rstest::rstest]
#[case::draft_options(false, true)]
#[case::draft_records(false, false)]
#[case::valid_options(true, true)]
#[case::valid_records(true, false)]
#[tokio::test]
async fn schema_loader_entrypoints_bind_secret_redaction(
    #[case] validated: bool,
    #[case] options: bool,
) {
    let fields = vec![
        Field::from(
            Field::secret(field_key!("token"))
                .read_alias("old_token")
                .unwrap(),
        ),
        Field::from(
            Field::select(field_key!("choice"))
                .dynamic()
                .loader("options"),
        ),
        Field::from(Field::dynamic(field_key!("record")).loader("records")),
    ];
    let draft: Schema = serde_json::from_value(json!({"fields": fields})).unwrap();
    let schema = fields
        .into_iter()
        .fold(Schema::builder(), nebula_schema::SchemaBuilder::add)
        .build()
        .unwrap();
    let registry = LoaderRegistry::new()
        .register_option("options", |context| async move {
            assert_eq!(context.values.to_json(), json!({"token": SECRET_REDACTED}));
            Ok(LoaderResult::done(vec![nebula_schema::SelectOption::new(
                json!("safe"),
                "Safe",
            )]))
        })
        .register_record("records", |context| async move {
            assert_eq!(context.values.to_json(), json!({"token": SECRET_REDACTED}));
            Ok(LoaderResult::done(vec![json!("safe")]))
        });
    let key = if options { "choice" } else { "record" };
    let context = LoaderContext::new(
        key,
        AuthoredValue::from_data(json!({
            "old_token": "bound-secret"
        }))
        .unwrap(),
    );
    let path = nebula_schema::FieldPath::parse(key).unwrap();
    match (validated, options) {
        (false, true) => {
            assert_eq!(
                draft
                    .load_select_options(key, &registry, context.clone())
                    .await
                    .unwrap()
                    .items
                    .len(),
                1
            );
            assert_eq!(
                draft
                    .load_select_options_at(&path, &registry, context)
                    .await
                    .unwrap()
                    .items
                    .len(),
                1
            );
        },
        (false, false) => {
            assert_eq!(
                draft
                    .load_dynamic_records(key, &registry, context.clone())
                    .await
                    .unwrap()
                    .items,
                vec![json!("safe")]
            );
            assert_eq!(
                draft
                    .load_dynamic_records_at(&path, &registry, context)
                    .await
                    .unwrap()
                    .items,
                vec![json!("safe")]
            );
        },
        (true, true) => {
            assert_eq!(
                schema
                    .load_select_options(key, &registry, context.clone())
                    .await
                    .unwrap()
                    .items
                    .len(),
                1
            );
            assert_eq!(
                schema
                    .load_select_options_at(&path, &registry, context)
                    .await
                    .unwrap()
                    .items
                    .len(),
                1
            );
        },
        (true, false) => {
            assert_eq!(
                schema
                    .load_dynamic_records(key, &registry, context.clone())
                    .await
                    .unwrap()
                    .items,
                vec![json!("safe")]
            );
            assert_eq!(
                schema
                    .load_dynamic_records_at(&path, &registry, context)
                    .await
                    .unwrap()
                    .items,
                vec![json!("safe")]
            );
        },
    }
}

#[test]
fn raw_loader_debug_never_formats_unvalidated_values() {
    let values = AuthoredValue::from_data(json!({"legacy_secret": "plaintext-marker"})).unwrap();
    let context = LoaderContext::new("field", values);
    assert_eq!(format!("{context:?}"), "LoaderContext { .. }");
}

#[test]
fn raw_boundaries_reject_excess_depth_before_projection() {
    let mut values = AuthoredValue::from_data(json!("deep-secret")).unwrap();
    for _ in 0..=MAX_VALUE_DEPTH {
        values = ValueTree::List(vec![values]);
    }
    for result in [
        predicate_context_for(&[], &values),
        root_predicate_context_for(&[], &values),
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.code(), "recursion_limit");
        assert_eq!(error.path().depth(), usize::from(MAX_VALUE_DEPTH) + 1);
        assert!(!error.to_string().contains("deep-secret"));
    }
    let schema = Schema::builder().build().unwrap();
    let error = LoaderContext::new("field", values)
        .with_secrets_redacted(&schema)
        .unwrap_err();
    assert_eq!(error.code(), "recursion_limit");
}

#[test]
fn raw_boundaries_accept_exact_depth_limit() {
    let mut values = AuthoredValue::from_data(json!(42)).unwrap();
    for _ in 0..MAX_VALUE_DEPTH {
        values = ValueTree::Object(indexmap::indexmap! {"child".to_owned() => values});
    }
    let pointer =
        ValuePath::from_segments(std::iter::repeat_n("child", usize::from(MAX_VALUE_DEPTH)));
    let context = predicate_context_for(&[], &values).unwrap();
    assert_eq!(context.get(&pointer), Some(&json!(42)));
    let schema = Schema::builder().build().unwrap();
    let snapshot = LoaderContext::new("field", values)
        .with_secrets_redacted(&schema)
        .unwrap();
    assert_eq!(
        snapshot
            .values()
            .get_path(&pointer)
            .and_then(ValueTree::as_literal),
        Some(&json!(42))
    );
}

#[tokio::test]
async fn registry_errors_preserve_arbitrary_rfc6901_paths() {
    let schema = Schema::builder()
        .add(Field::dynamic(field_key!("record")).loader("missing"))
        .build()
        .unwrap();
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("/a~1b/~0/01/", AuthoredValue::object());
    let error = schema
        .load_dynamic_records("record", &registry, context)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "loader.not_registered");
    assert_eq!(
        error.path(),
        &ValuePath::from_segments(["a/b", "~", "01", ""])
    );
}

proptest! {
    #[test]
    fn arbitrary_object_keys_keep_identity_while_secret_subtrees_are_removed(key in ".{0,32}") {
        let mut data = ResolvedValue::object();
        data.insert(&key, ValueTree::from_data(json!(123)).unwrap()).unwrap();
        let mut hidden = ResolvedValue::object();
        hidden.insert(&key, ValueTree::Secret(SecretValue::string("hidden".to_owned()))).unwrap();
        let mut values = ResolvedValue::object();
        values.insert("data", data).unwrap();
        values.insert("hidden", hidden).unwrap();
        let context = predicate_context_for(&[], &values).unwrap();
        prop_assert_eq!(context.get(&ValuePath::from_segments(["data", key.as_str()])), Some(&json!(123)));
        prop_assert_eq!(context_value(&context, "/hidden"), Some(&json!({})));
    }
}
