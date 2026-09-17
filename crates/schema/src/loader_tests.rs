use serde_json::json;

use super::*;
use crate::{
    AuthoredValue, Property, ScalarValue, Schema, ValueTree, expression::Expression, field_key,
    secret::SECRET_REDACTED,
};

fn scrub(context: LoaderContext) -> RedactedLoaderContext {
    context.redacted(&[]).unwrap()
}

#[tokio::test]
async fn load_options_unregistered_returns_not_registered() {
    let registry = LoaderRegistry::new();
    let ctx = LoaderContext::new("field", AuthoredValue::object());
    let err = registry
        .load_options("missing", scrub(ctx))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "loader.not_registered");
    assert!(
        err.params()
            .iter()
            .any(|(k, v)| k == "loader" && v == "missing")
    );
    // Error path should reflect the requesting field key.
    assert_eq!(err.path().to_string(), "/field");
}

#[tokio::test]
async fn load_records_unregistered_returns_not_registered() {
    let registry = LoaderRegistry::new();
    let ctx = LoaderContext::new("field", AuthoredValue::object());
    let err = registry
        .load_records("missing", scrub(ctx))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "loader.not_registered");
    // Error path should reflect the requesting field key.
    assert_eq!(err.path().to_string(), "/field");
}

#[tokio::test]
async fn load_options_registered_returns_result() {
    let registry = LoaderRegistry::new().register_option("opts", |_ctx| async {
        Ok(LoaderResult::done(vec![SelectOption::new(
            json!("a"),
            "Option A",
        )]))
    });
    let ctx = LoaderContext::new("field", AuthoredValue::object());
    let result = registry.load_options("opts", scrub(ctx)).await.unwrap();
    assert_eq!(result.items.len(), 1);
}

#[tokio::test]
async fn loader_failure_wraps_as_loader_failed() {
    const PRIVATE_DETAIL: &str = "PRIVATE downstream loader detail";
    let registry = LoaderRegistry::new().register_option("fail", |_ctx| async {
        Err(ValidationError::builder("loader.failed")
            .message(PRIVATE_DETAIL)
            .build())
    });
    let ctx = LoaderContext::new("field", AuthoredValue::object());
    let err = registry.load_options("fail", scrub(ctx)).await.unwrap_err();
    assert_eq!(err.code(), "loader.failed");
    assert_eq!(err.message(), "option loader failed");
    assert!(!format!("{err:?} {err}").contains(PRIVATE_DETAIL));
    assert_eq!(
        std::error::Error::source(&err).map(ToString::to_string),
        Some("diagnostic details contain private input".to_owned())
    );
}

#[tokio::test]
async fn record_loader_failure_keeps_source_private() {
    const PRIVATE_DETAIL: &str = "PRIVATE record loader detail";
    let registry = LoaderRegistry::new().register_record("fail", |_ctx| async {
        Err(ValidationError::builder("loader.failed")
            .message(PRIVATE_DETAIL)
            .build())
    });
    let context = LoaderContext::new("field", AuthoredValue::object());
    let error = registry
        .load_records("fail", scrub(context))
        .await
        .unwrap_err();
    assert_eq!(error.message(), "record loader failed");
    assert!(!format!("{error:?} {error}").contains(PRIVATE_DETAIL));
    assert_eq!(
        std::error::Error::source(&error).map(ToString::to_string),
        Some("diagnostic details contain private input".to_owned())
    );
}

#[tokio::test]
async fn loader_failure_root_path_maps_back_to_request_field() {
    let registry = LoaderRegistry::new().register_option("regions_loader", |_ctx| async {
        Err(ValidationError::builder("loader.failed")
            .at(ValuePath::root())
            .message("downstream error")
            .build())
    });
    let ctx = LoaderContext::new("region", AuthoredValue::object());
    let err = registry
        .load_options("regions_loader", scrub(ctx))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "loader.failed");
    assert_eq!(err.path().to_string(), "/region");
}

#[tokio::test]
async fn load_options_rejects_oversized_result() {
    // A loader page above the item ceiling fails closed (it must paginate).
    let registry = LoaderRegistry::new().register_option("big", |_ctx| async {
        let items = (0..=MAX_LOADER_ITEMS)
            .map(|i| SelectOption::new(json!(i), format!("o{i}")))
            .collect();
        Ok(LoaderResult::done(items))
    });
    let ctx = LoaderContext::new("region", AuthoredValue::object());
    let err = registry.load_options("big", scrub(ctx)).await.unwrap_err();
    assert_eq!(err.code(), "loader.result_too_large");
    assert_eq!(
        err.path().to_string(),
        "/region",
        "carries the requesting field"
    );
    assert!(
        err.params()
            .iter()
            .any(|(k, v)| k == "limit" && v.as_u64() == Some(MAX_LOADER_ITEMS as u64)),
        "reports the limit: {:?}",
        err.params()
    );
}

#[tokio::test]
async fn load_options_accepts_max_items_boundary() {
    // Exactly the ceiling is allowed; only strictly above it is rejected.
    let registry = LoaderRegistry::new().register_option("atlimit", |_ctx| async {
        let items = (0..MAX_LOADER_ITEMS)
            .map(|i| SelectOption::new(json!(i), format!("o{i}")))
            .collect();
        Ok(LoaderResult::done(items))
    });
    let ctx = LoaderContext::new("region", AuthoredValue::object());
    let result = registry.load_options("atlimit", scrub(ctx)).await.unwrap();
    assert_eq!(result.items.len(), MAX_LOADER_ITEMS);
}

#[tokio::test]
async fn load_records_rejects_oversized_result() {
    let registry = LoaderRegistry::new().register_record("big", |_ctx| async {
        let items = (0..=MAX_LOADER_ITEMS).map(|i| json!({ "i": i })).collect();
        Ok(LoaderResult::done(items))
    });
    let ctx = LoaderContext::new("rows", AuthoredValue::object());
    let err = registry.load_records("big", scrub(ctx)).await.unwrap_err();
    assert_eq!(err.code(), "loader.result_too_large");
}

#[tokio::test]
async fn load_records_rejects_cumulative_page_bytes() {
    let registry = LoaderRegistry::new().register_record("wide", |_ctx| async {
        Ok(LoaderResult::done(vec![
            Value::String("a".repeat(600_000)),
            Value::String("b".repeat(600_000)),
        ]))
    });
    let ctx = LoaderContext::new("rows", AuthoredValue::object());
    let error = registry.load_records("wide", scrub(ctx)).await.unwrap_err();
    assert_eq!(error.code(), "loader.result_too_large");
    assert!(
        error
            .params()
            .iter()
            .any(|(key, value)| key == "resource" && value == "serialized item bytes")
    );
}

#[tokio::test]
async fn load_records_rejects_excessive_item_depth() {
    let registry = LoaderRegistry::new().register_record("deep", |_ctx| async {
        let mut value = Value::Null;
        for _ in 0..=64 {
            value = Value::Array(vec![value]);
        }
        Ok(LoaderResult::done(vec![value]))
    });
    let ctx = LoaderContext::new("rows", AuthoredValue::object());
    let error = registry.load_records("deep", scrub(ctx)).await.unwrap_err();
    assert_eq!(error.code(), "loader.result_too_large");
    assert!(
        error
            .params()
            .iter()
            .any(|(key, value)| key == "resource" && value == "item depth")
    );
}

#[test]
fn loader_context_builder() {
    let ctx = LoaderContext::new("my_field", AuthoredValue::object())
        .with_filter("query")
        .with_cursor("tok")
        .with_metadata(json!({"page": 1}));
    assert_eq!(ctx.field_key, "my_field");
    assert_eq!(ctx.filter.as_deref(), Some("query"));
    assert_eq!(ctx.cursor.as_deref(), Some("tok"));
    assert!(ctx.metadata.is_some());
}

#[test]
fn loader_result_constructors() {
    let r: LoaderResult<i32> = LoaderResult::done(vec![1, 2]);
    assert!(r.next_cursor.is_none());

    let p: LoaderResult<i32> = LoaderResult::page(vec![1], "next");
    assert_eq!(p.next_cursor.as_deref(), Some("next"));

    let t = p.with_total(100);
    assert_eq!(t.total, Some(100));
}

fn redacted_literal() -> AuthoredValue {
    AuthoredValue::from_data(json!(SECRET_REDACTED)).unwrap()
}

#[test]
fn with_secrets_redacted_object_nested_and_non_secret_unchanged() {
    let schema = Schema::builder()
        .property(
            Property::object(field_key!("config"))
                .property(Property::secret(field_key!("api_key")))
                .property(Property::string(field_key!("label"))),
        )
        .build()
        .expect("valid schema");
    let values = AuthoredValue::from_data(json!({
        "config": {
            "api_key": "hunter2",
            "label": "visible"
        }
    }))
    .expect("values");
    let ctx = LoaderContext::new("k", values)
        .redacted(schema.properties())
        .unwrap();
    let config = ctx.0.values.get("config").expect("config");
    let ValueTree::Object(map) = config else {
        panic!("expected object, got {config:?}");
    };
    assert_eq!(map.get("api_key"), Some(&redacted_literal()));
    let label = map.get("label").expect("label");
    assert_eq!(label.as_str(), Some("visible"));
}

#[test]
fn with_secrets_redacted_list_of_secrets() {
    let schema = Schema::builder()
        .property(Property::list(field_key!("tokens")).item(Property::secret(field_key!("t"))))
        .build()
        .expect("valid schema");
    let values = AuthoredValue::from_data(json!({ "tokens": ["a", "b"] })).expect("values");
    let ctx = LoaderContext::new("k", values)
        .redacted(schema.properties())
        .unwrap();
    let list = ctx.0.values.get("tokens").expect("tokens");
    let ValueTree::List(items) = list else {
        panic!("expected list, got {list:?}");
    };
    assert_eq!(items.as_slice(), &[redacted_literal(), redacted_literal()]);
}

/// Mode variant payload is an `Object` with a secret leaf and a non-secret sibling
/// (exercises the `Property::Mode` + nested `Object` path, not a bare `Property::Secret`
/// that replaces the entire mode `value` tree with one redacted literal).
#[test]
fn with_secrets_redacted_mode_variant_object_with_nested_secret() {
    let schema = Schema::builder()
        .property(
            Property::mode(field_key!("auth"))
                .variant(
                    "oauth",
                    "OAuth",
                    Property::object(field_key!("creds"))
                        .property(Property::secret(field_key!("client_secret")))
                        .property(Property::string(field_key!("client_id"))),
                )
                .variant("plain", "Plain", Property::string(field_key!("name"))),
        )
        .build()
        .expect("valid schema");
    // The mode `value` is the unwrapped object payload: same shape as a top-level
    // `Object` field's value (child keys), not `{"creds": { ... }}` — see `redact` +
    // `Property::Object` matching in `redact_secrets_in_value_for_loader`.
    let values = AuthoredValue::from_data(json!({
        "auth": {
            "mode": "oauth",
            "value": {
                "client_secret": "top",
                "client_id": "visible"
            }
        }
    }))
    .expect("values");
    let ctx = LoaderContext::new("k", values)
        .redacted(schema.properties())
        .unwrap();
    let auth = ctx.0.values.get("auth").expect("auth");
    let ValueTree::Object(map) = auth else {
        panic!("expected object envelope, got {auth:?}");
    };
    let mode = map.get("mode").expect("mode");
    assert_eq!(mode.as_str(), Some("oauth"));
    let payload = map.get("value").expect("payload");
    let ValueTree::Object(m) = payload else {
        panic!("expected object payload, got {payload:?}");
    };
    assert_eq!(m.get("client_secret"), Some(&redacted_literal()));
    let id = m.get("client_id").expect("id");
    assert_eq!(id.as_str(), Some("visible"));
}

#[test]
fn with_secrets_redacted_mode_object_without_mode_uses_default_variant() {
    let schema = Schema::builder()
        .property(
            Property::mode(field_key!("auth"))
                .variant(
                    "oauth",
                    "OAuth",
                    Property::object(field_key!("creds"))
                        .property(Property::secret(field_key!("client_secret")))
                        .property(Property::string(field_key!("client_id"))),
                )
                .default_variant("oauth"),
        )
        .build()
        .expect("valid schema");
    let values = AuthoredValue::from_data(json!({
        "auth": {
            "value": {
                "client_secret": "top",
                "client_id": "visible"
            }
        }
    }))
    .expect("values");

    let ctx = LoaderContext::new("k", values)
        .redacted(schema.properties())
        .unwrap();
    let auth = ctx.0.values.get("auth").expect("auth");
    let ValueTree::Object(map) = auth else {
        panic!("expected object envelope, got {auth:?}");
    };
    let payload = map.get("value").expect("payload");
    let ValueTree::Object(m) = payload else {
        panic!("expected object payload, got {payload:?}");
    };
    assert_eq!(m.get("client_secret"), Some(&redacted_literal()));
    let id = m.get("client_id").expect("id");
    assert_eq!(id.as_str(), Some("visible"));
}

#[test]
fn with_secrets_redacted_expression_on_secret_is_literal_token() {
    let schema = Schema::builder()
        .property(Property::secret(field_key!("api_key")))
        .build()
        .expect("valid schema");
    let mut values = AuthoredValue::object();
    values
        .insert(
            "api_key",
            ValueTree::Expression(Expression::new("would.leak()")),
        )
        .unwrap();
    let ctx = LoaderContext::new("k", values)
        .redacted(schema.properties())
        .unwrap();
    let v = ctx.0.values.get("api_key").expect("api_key");
    assert_eq!(*v, redacted_literal());
}

#[test]
fn with_secrets_redacted_object_literal_blob_is_over_redacted() {
    // Objects cannot hide inside literals. Serialized secret data supplied
    // as a scalar to a secret-bearing container still needs whole redaction.
    let error = ScalarValue::try_from(json!({"api_key": "PLAINTEXT-LEAK"})).unwrap_err();
    assert_eq!(error.code(), "type_mismatch");
    let schema = Schema::builder()
        .property(
            Property::object(field_key!("cfg"))
                .property(Property::secret(field_key!("api_key")))
                .property(Property::string(field_key!("label"))),
        )
        .build()
        .expect("valid schema");
    let values = AuthoredValue::from_data(json!({
        "cfg": json!({"api_key": "PLAINTEXT-LEAK", "label": "x"}).to_string()
    }))
    .unwrap();
    let ctx = LoaderContext::new("k", values)
        .redacted(schema.properties())
        .unwrap();
    let cfg = ctx.0.values.get("cfg").expect("cfg");
    assert_eq!(*cfg, redacted_literal());
    assert!(
        !format!("{cfg:?}").contains("PLAINTEXT-LEAK"),
        "blob plaintext reached the loader: {cfg:?}"
    );
}

#[test]
fn with_secrets_redacted_mode_unknown_variant_over_redacts() {
    // The canonical object envelope must redact an unknown variant's payload.
    let schema = Schema::builder()
        .property(
            Property::mode(field_key!("auth")).variant(
                "oauth",
                "OAuth",
                Property::object(field_key!("creds"))
                    .property(Property::secret(field_key!("client_secret"))),
            ),
        )
        .build()
        .expect("valid schema");
    let values = AuthoredValue::from_data(json!({
        "auth": {"mode": "nonexistent", "value": {"client_secret": "PLAINTEXT-LEAK"}}
    }))
    .unwrap();
    let ctx = LoaderContext::new("k", values)
        .redacted(schema.properties())
        .unwrap();
    let auth = ctx.0.values.get("auth").expect("auth");
    assert_eq!(
        auth.get("mode").and_then(ValueTree::as_str),
        Some("nonexistent")
    );
    assert_eq!(auth.get("value"), Some(&redacted_literal()));
    assert!(!format!("{auth:?}").contains("PLAINTEXT-LEAK"));
}
