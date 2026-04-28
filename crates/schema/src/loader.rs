//! Runtime loader registry and async loader types.
//!
//! All fallible paths now return [`ValidationError`] (unified error type).
//! Codes emitted:
//!
//! | Code | When |
//! |------|------|
//! | `loader.not_registered` | Named loader key not found in registry |
//! | `loader.failed` | Loader invocation returned an error |
//!
//! Lint-time warnings (`missing_loader`, `loader_without_dynamic`) are emitted
//! by the lint pass in `lint.rs`, not here.

use std::{future::Future, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    FieldValues, SelectOption,
    error::ValidationError,
    field::Field,
    key::FieldKey,
    path::{FieldPath, PathSegment},
    secret::SECRET_REDACTED,
    value::FieldValue,
};

/// Boxed future used by async loader functions.
pub type LoaderFuture<T> =
    Pin<Box<dyn Future<Output = Result<LoaderResult<T>, ValidationError>> + Send>>;

/// Context passed to runtime loaders.
#[derive(Debug, Clone)]
pub struct LoaderContext {
    /// Key or schema path of the field currently requesting dynamic data.
    pub field_key: String,
    /// Current runtime values at call time.
    pub values: FieldValues,
    /// Optional free-text query from searchable UI controls.
    pub filter: Option<String>,
    /// Optional pagination cursor from previous response.
    pub cursor: Option<String>,
    /// Loader-specific metadata.
    pub metadata: Option<Value>,
}

impl LoaderContext {
    /// Construct context for a specific field key.
    pub fn new(field_key: impl Into<String>, values: FieldValues) -> Self {
        Self {
            field_key: field_key.into(),
            values,
            filter: None,
            cursor: None,
            metadata: None,
        }
    }

    /// Redact all [`Field::Secret`]-backed values (including any nested
    /// secrets) before exposing `values` to a loader. Does **not** evaluate
    /// expressions — [`crate::value::FieldValue::Expression`] leaves under
    /// secret fields are also collapsed to a redacted string literal to avoid
    /// surfacing the expression source.
    #[must_use]
    pub fn with_secrets_redacted(mut self, schema: &crate::validated::ValidSchema) -> Self {
        for field in schema.fields() {
            if let Some(v) = self.values.get_mut(field.key()) {
                redact_secrets_in_value_for_loader(field, v);
            }
        }
        self
    }

    /// Attach text filter.
    #[must_use]
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Attach pagination cursor.
    #[must_use]
    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    /// Attach arbitrary metadata payload.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

fn redact_secrets_in_value_for_loader(field: &Field, value: &mut FieldValue) {
    use serde_json::Value as Json;
    match (field, &mut *value) {
        (Field::Secret(_), _) => {
            *value = FieldValue::Literal(Json::String(SECRET_REDACTED.to_owned()));
        },
        (Field::Object(obj), FieldValue::Object(map)) => {
            for ch in &obj.fields {
                if let Some(v) = map.get_mut(ch.key()) {
                    redact_secrets_in_value_for_loader(ch, v);
                }
            }
        },
        (Field::List(list), FieldValue::List(items)) => {
            if let Some(item_field) = list.item.as_deref() {
                for v in &mut *items {
                    redact_secrets_in_value_for_loader(item_field, v);
                }
            }
        },
        (
            Field::Mode(mode),
            FieldValue::Mode {
                mode: mode_key,
                value: Some(mv),
            },
        ) => {
            let Some(var) = mode.variants.iter().find(|v| v.key == mode_key.as_str()) else {
                return;
            };
            redact_secrets_in_value_for_loader(&var.field, mv.as_mut());
        },
        (Field::Mode(mode), FieldValue::Object(map)) => {
            let Ok(mode_selector_key) = FieldKey::new("mode") else {
                return;
            };
            let Ok(payload_key) = FieldKey::new("value") else {
                return;
            };
            let resolved_key = match map.get(&mode_selector_key) {
                Some(FieldValue::Literal(Json::String(mode_key))) => Some(mode_key.clone()),
                Some(_) => None,
                None => mode.default_variant.clone(),
            };
            let Some(mv) = map.get_mut(&payload_key) else {
                return;
            };
            let Some(var) = resolved_key
                .as_deref()
                .and_then(|mode_key| mode.variants.iter().find(|v| v.key == mode_key))
            else {
                // If the active variant cannot be determined, over-redact the payload rather
                // than risk exposing nested secret material to loader implementations.
                *mv = FieldValue::Literal(Json::String(SECRET_REDACTED.to_owned()));
                return;
            };
            redact_secrets_in_value_for_loader(&var.field, mv);
        },
        _ => {},
    }
}

/// Generic async loader wrapper.
pub struct Loader<T: Send + 'static>(
    Arc<dyn Fn(LoaderContext) -> LoaderFuture<T> + Send + Sync + 'static>,
);

impl<T: Send + 'static> Loader<T> {
    /// Wrap an async closure into a reusable loader object.
    pub fn new<F, Fut>(loader: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<T>, ValidationError>> + Send + 'static,
    {
        Self(Arc::new(move |context| Box::pin(loader(context))))
    }

    /// Execute loader for the provided context.
    ///
    /// # Errors
    ///
    /// Returns any [`ValidationError`] produced by the wrapped loader.
    pub async fn call(&self, context: LoaderContext) -> Result<LoaderResult<T>, ValidationError> {
        (self.0)(context).await
    }
}

impl<T: Send + 'static> Clone for Loader<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

// `Loader<T>` intentionally does NOT implement `PartialEq`. Two loaders backed
// by different closures cannot be compared by value, and a previous "always
// `true`" impl violated the contract — see the T05 entry of the
// nebula-schema-quality-fixes plan.

impl<T: Send + 'static> std::fmt::Debug for Loader<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Loader(<async fn>)")
    }
}

/// Paginated result returned from runtime loaders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoaderResult<T> {
    /// Page of resolved items.
    pub items: Vec<T>,
    /// Cursor for fetching next page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Optional total item count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl<T> LoaderResult<T> {
    /// Build a non-paginated result.
    #[must_use]
    pub const fn done(items: Vec<T>) -> Self {
        Self {
            items,
            next_cursor: None,
            total: None,
        }
    }

    /// Build a paginated result with next cursor.
    pub fn page(items: Vec<T>, cursor: impl Into<String>) -> Self {
        Self {
            items,
            next_cursor: Some(cursor.into()),
            total: None,
        }
    }

    /// Attach total count.
    #[must_use]
    pub const fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }
}

impl<T> From<Vec<T>> for LoaderResult<T> {
    fn from(items: Vec<T>) -> Self {
        Self::done(items)
    }
}

/// Loader returning select options.
pub type OptionLoader = Loader<SelectOption>;
/// Loader returning dynamic record payloads.
pub type RecordLoader = Loader<Value>;

/// Build a `FieldPath` from a `LoaderContext::field_key` string.
///
/// Falls back to root if the string is not a valid schema path.
fn field_path_from_key(key: &str) -> FieldPath {
    FieldPath::parse(key).unwrap_or_else(|_| {
        FieldKey::new(key).map_or_else(
            |_| FieldPath::root(),
            |fk| FieldPath::root().join(PathSegment::Key(fk)),
        )
    })
}

/// Use the path from a loader-returned error if it's non-root, otherwise fall
/// back to the request field path.
fn field_path_from_err_or(fallback: &FieldPath, err: &ValidationError) -> FieldPath {
    if err.path.is_root() {
        fallback.clone()
    } else {
        err.path.clone()
    }
}

/// Runtime registry for named loader functions.
#[derive(Debug, Clone, Default)]
pub struct LoaderRegistry {
    option_loaders: std::collections::HashMap<String, OptionLoader>,
    record_loaders: std::collections::HashMap<String, RecordLoader>,
}

impl LoaderRegistry {
    /// Create an empty loader registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register option loader using builder style.
    #[must_use]
    pub fn register_option<F, Fut>(mut self, key: impl Into<String>, loader: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<SelectOption>, ValidationError>> + Send + 'static,
    {
        self.option_loaders
            .insert(key.into(), OptionLoader::new(loader));
        self
    }

    /// Register record loader using builder style.
    #[must_use]
    pub fn register_record<F, Fut>(mut self, key: impl Into<String>, loader: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<Value>, ValidationError>> + Send + 'static,
    {
        self.record_loaders
            .insert(key.into(), RecordLoader::new(loader));
        self
    }

    /// Resolve and execute option loader by key.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `loader.not_registered` when `key`
    /// is not registered, or `loader.failed` if the loader returns an error.
    /// Both errors carry the requesting field's path (from `context.field_key`).
    pub async fn load_options(
        &self,
        key: &str,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let field_path = field_path_from_key(&context.field_key);
        let Some(loader) = self.option_loaders.get(key) else {
            return Err(ValidationError::builder("loader.not_registered")
                .at(field_path)
                .message(format!("option loader `{key}` is not registered"))
                .param("loader", Value::String(key.to_owned()))
                .build());
        };
        loader.call(context).await.map_err(|e| {
            ValidationError::builder("loader.failed")
                .at(field_path_from_err_or(&field_path, &e))
                .message(format!("option loader `{key}` failed: {e}"))
                .param("loader", Value::String(key.to_owned()))
                .source(e)
                .build()
        })
    }

    /// Resolve and execute record loader by key.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `loader.not_registered` when `key`
    /// is not registered, or `loader.failed` if the loader returns an error.
    /// Both errors carry the requesting field's path (from `context.field_key`).
    pub async fn load_records(
        &self,
        key: &str,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, ValidationError> {
        let field_path = field_path_from_key(&context.field_key);
        let Some(loader) = self.record_loaders.get(key) else {
            return Err(ValidationError::builder("loader.not_registered")
                .at(field_path)
                .message(format!("record loader `{key}` is not registered"))
                .param("loader", Value::String(key.to_owned()))
                .build());
        };
        loader.call(context).await.map_err(|e| {
            ValidationError::builder("loader.failed")
                .at(field_path_from_err_or(&field_path, &e))
                .message(format!("record loader `{key}` failed: {e}"))
                .param("loader", Value::String(key.to_owned()))
                .source(e)
                .build()
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value as Json, json};

    use super::*;
    use crate::{
        Field, FieldValues, Schema, expression::Expression, field_key, key::FieldKey,
        secret::SECRET_REDACTED, value::FieldValue,
    };

    fn k(name: &str) -> FieldKey {
        FieldKey::new(name).expect("test key")
    }

    #[tokio::test]
    async fn load_options_unregistered_returns_not_registered() {
        let registry = LoaderRegistry::new();
        let ctx = LoaderContext::new("field", FieldValues::new());
        let err = registry.load_options("missing", ctx).await.unwrap_err();
        assert_eq!(err.code, "loader.not_registered");
        assert!(
            err.params
                .iter()
                .any(|(k, v)| k == "loader" && v == "missing")
        );
        // Error path should reflect the requesting field key.
        assert_eq!(err.path.to_string(), "field");
    }

    #[tokio::test]
    async fn load_records_unregistered_returns_not_registered() {
        let registry = LoaderRegistry::new();
        let ctx = LoaderContext::new("field", FieldValues::new());
        let err = registry.load_records("missing", ctx).await.unwrap_err();
        assert_eq!(err.code, "loader.not_registered");
        // Error path should reflect the requesting field key.
        assert_eq!(err.path.to_string(), "field");
    }

    #[tokio::test]
    async fn load_options_registered_returns_result() {
        let registry = LoaderRegistry::new().register_option("opts", |_ctx| async {
            Ok(LoaderResult::done(vec![SelectOption::new(
                json!("a"),
                "Option A",
            )]))
        });
        let ctx = LoaderContext::new("field", FieldValues::new());
        let result = registry.load_options("opts", ctx).await.unwrap();
        assert_eq!(result.items.len(), 1);
    }

    #[tokio::test]
    async fn loader_failure_wraps_as_loader_failed() {
        let registry = LoaderRegistry::new().register_option("fail", |_ctx| async {
            Err(ValidationError::builder("loader.failed")
                .message("downstream error")
                .build())
        });
        let ctx = LoaderContext::new("field", FieldValues::new());
        let err = registry.load_options("fail", ctx).await.unwrap_err();
        assert_eq!(err.code, "loader.failed");
    }

    #[tokio::test]
    async fn loader_failure_root_path_maps_back_to_request_field() {
        let registry = LoaderRegistry::new().register_option("regions_loader", |_ctx| async {
            Err(ValidationError::builder("loader.failed")
                .at(FieldPath::root())
                .message("downstream error")
                .build())
        });
        let ctx = LoaderContext::new("region", FieldValues::new());
        let err = registry
            .load_options("regions_loader", ctx)
            .await
            .unwrap_err();
        assert_eq!(err.code, "loader.failed");
        assert_eq!(err.path.to_string(), "region");
    }

    #[test]
    fn loader_context_builder() {
        let ctx = LoaderContext::new("my_field", FieldValues::new())
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

    fn redacted_literal() -> FieldValue {
        FieldValue::Literal(Json::String(SECRET_REDACTED.to_owned()))
    }

    #[test]
    fn with_secrets_redacted_object_nested_and_non_secret_unchanged() {
        let schema = Schema::builder()
            .add(
                Field::object(field_key!("config"))
                    .add(Field::secret(field_key!("api_key")))
                    .add(Field::string(field_key!("label"))),
            )
            .build()
            .expect("valid schema");
        let values = FieldValues::from_json(json!({
            "config": {
                "api_key": "hunter2",
                "label": "visible"
            }
        }))
        .expect("values");
        let ctx = LoaderContext::new("k", values).with_secrets_redacted(&schema);
        let config = ctx.values.get_by_str("config").expect("config");
        let FieldValue::Object(map) = config else {
            panic!("expected object, got {config:?}");
        };
        assert_eq!(map.get(&k("api_key")), Some(&redacted_literal()));
        let label = map.get(&k("label")).expect("label");
        let FieldValue::Literal(Json::String(s)) = label else {
            panic!("expected string literal, got {label:?}");
        };
        assert_eq!(s, "visible");
    }

    #[test]
    fn with_secrets_redacted_list_of_secrets() {
        let schema = Schema::builder()
            .add(Field::list(field_key!("tokens")).item(Field::secret(field_key!("t"))))
            .build()
            .expect("valid schema");
        let values = FieldValues::from_json(json!({ "tokens": ["a", "b"] })).expect("values");
        let ctx = LoaderContext::new("k", values).with_secrets_redacted(&schema);
        let list = ctx.values.get_by_str("tokens").expect("tokens");
        let FieldValue::List(items) = list else {
            panic!("expected list, got {list:?}");
        };
        assert_eq!(items.as_slice(), &[redacted_literal(), redacted_literal()]);
    }

    /// Mode variant payload is an `Object` with a secret leaf and a non-secret sibling
    /// (exercises the `Field::Mode` + nested `Object` path, not a bare `Field::Secret`
    /// that replaces the entire mode `value` tree with one redacted literal).
    #[test]
    fn with_secrets_redacted_mode_variant_object_with_nested_secret() {
        let schema = Schema::builder()
            .add(
                Field::mode(field_key!("auth"))
                    .variant(
                        "oauth",
                        "OAuth",
                        Field::object(field_key!("creds"))
                            .add(Field::secret(field_key!("client_secret")))
                            .add(Field::string(field_key!("client_id"))),
                    )
                    .variant("plain", "Plain", Field::string(field_key!("name"))),
            )
            .build()
            .expect("valid schema");
        // The mode `value` is the unwrapped object payload: same shape as a top-level
        // `Object` field's value (child keys), not `{"creds": { ... }}` — see `redact` +
        // `Field::Object` matching in `redact_secrets_in_value_for_loader`.
        let values = FieldValues::from_json(json!({
            "auth": {
                "mode": "oauth",
                "value": {
                    "client_secret": "top",
                    "client_id": "visible"
                }
            }
        }))
        .expect("values");
        let ctx = LoaderContext::new("k", values).with_secrets_redacted(&schema);
        let auth = ctx.values.get_by_str("auth").expect("auth");
        let FieldValue::Object(map) = auth else {
            panic!("expected object envelope, got {auth:?}");
        };
        let mode = map.get(&k("mode")).expect("mode");
        let FieldValue::Literal(Json::String(mode_key)) = mode else {
            panic!("expected mode literal, got {mode:?}");
        };
        assert_eq!(mode_key, "oauth");
        let payload = map.get(&k("value")).expect("payload");
        let FieldValue::Object(m) = payload else {
            panic!("expected object payload, got {payload:?}");
        };
        assert_eq!(m.get(&k("client_secret")), Some(&redacted_literal()));
        let id = m.get(&k("client_id")).expect("id");
        let FieldValue::Literal(Json::String(s)) = id else {
            panic!("expected client_id literal, got {id:?}");
        };
        assert_eq!(s, "visible");
    }

    #[test]
    fn with_secrets_redacted_mode_object_without_mode_uses_default_variant() {
        let schema = Schema::builder()
            .add(
                Field::mode(field_key!("auth"))
                    .variant(
                        "oauth",
                        "OAuth",
                        Field::object(field_key!("creds"))
                            .add(Field::secret(field_key!("client_secret")))
                            .add(Field::string(field_key!("client_id"))),
                    )
                    .default_variant("oauth"),
            )
            .build()
            .expect("valid schema");
        let values = FieldValues::from_json(json!({
            "auth": {
                "value": {
                    "client_secret": "top",
                    "client_id": "visible"
                }
            }
        }))
        .expect("values");

        let ctx = LoaderContext::new("k", values).with_secrets_redacted(&schema);
        let auth = ctx.values.get_by_str("auth").expect("auth");
        let FieldValue::Object(map) = auth else {
            panic!("expected object envelope, got {auth:?}");
        };
        let payload = map.get(&k("value")).expect("payload");
        let FieldValue::Object(m) = payload else {
            panic!("expected object payload, got {payload:?}");
        };
        assert_eq!(m.get(&k("client_secret")), Some(&redacted_literal()));
        let id = m.get(&k("client_id")).expect("id");
        let FieldValue::Literal(Json::String(s)) = id else {
            panic!("expected client_id literal, got {id:?}");
        };
        assert_eq!(s, "visible");
    }

    #[test]
    fn with_secrets_redacted_expression_on_secret_is_literal_token() {
        let schema = Schema::builder()
            .add(Field::secret(field_key!("api_key")))
            .build()
            .expect("valid schema");
        let mut values = FieldValues::new();
        values.set(
            k("api_key"),
            FieldValue::Expression(Expression::new("would.leak()")),
        );
        let ctx = LoaderContext::new("k", values).with_secrets_redacted(&schema);
        let v = ctx.values.get_by_str("api_key").expect("api_key");
        assert_eq!(*v, redacted_literal());
    }
}
