//! Integration tests for `providers.rs` — `ProviderRegistry`, `OptionProvider`,
//! `DynamicRecordProvider`, envelope validation, and error paths.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_parameter::providers::{
    DynamicProviderEnvelope, DynamicRecordProvider, DynamicResponseKind, OptionProvider,
    ProviderError, ProviderRegistry, ProviderRequest,
};
use nebula_parameter::option::SelectOption;
use nebula_parameter::spec::FieldSpec;
use nebula_parameter::metadata::FieldMetadata;
use nebula_parameter::values::ParameterValues;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_request(field_id: &str) -> ProviderRequest {
    ProviderRequest {
        field_id: field_id.to_owned(),
        values: ParameterValues::new(),
        filter: None,
        cursor: None,
    }
}

fn option(value: &str, label: &str) -> SelectOption {
    SelectOption::new(serde_json::json!(value), label)
}

fn text_spec(id: &str, label: &str) -> FieldSpec {
    FieldSpec::Text {
        meta: FieldMetadata {
            id: id.to_owned(),
            label: label.to_owned(),
            ..Default::default()
        },
        multiline: false,
    }
}

// ── Mock providers ────────────────────────────────────────────────────────────

/// Returns a fixed set of options.
struct StaticOptionProvider {
    options: Vec<SelectOption>,
}

#[async_trait]
impl OptionProvider for StaticOptionProvider {
    async fn resolve(
        &self,
        _request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
        Ok(DynamicProviderEnvelope::new(
            DynamicResponseKind::Options,
            self.options.clone(),
        ))
    }
}

/// Echoes the request `field_id` back as a single text FieldSpec.
struct EchoFieldProvider;

#[async_trait]
impl DynamicRecordProvider for EchoFieldProvider {
    async fn resolve_fields(
        &self,
        request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<FieldSpec>, ProviderError> {
        let spec = text_spec(&request.field_id, &request.field_id);
        Ok(DynamicProviderEnvelope::new(DynamicResponseKind::Fields, vec![spec]))
    }
}

/// Returns the wrong kind (`Fields` instead of `Options`).
struct WrongKindOptionProvider;

#[async_trait]
impl OptionProvider for WrongKindOptionProvider {
    async fn resolve(
        &self,
        _request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
        Ok(DynamicProviderEnvelope {
            response_version: 1,
            kind: DynamicResponseKind::Fields, // wrong kind
            items: Vec::new(),
            next_cursor: None,
            schema_version: None,
        })
    }
}

/// Returns an unsupported response version.
struct BadVersionOptionProvider;

#[async_trait]
impl OptionProvider for BadVersionOptionProvider {
    async fn resolve(
        &self,
        _request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
        Ok(DynamicProviderEnvelope {
            response_version: 99,
            kind: DynamicResponseKind::Options,
            items: Vec::new(),
            next_cursor: None,
            schema_version: None,
        })
    }
}

/// Always returns a provider-specific error.
struct FailingOptionProvider;

#[async_trait]
impl OptionProvider for FailingOptionProvider {
    async fn resolve(
        &self,
        _request: &ProviderRequest,
    ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
        Err(ProviderError::ResolveFailed {
            key: "test".to_owned(),
            message: "upstream timeout".to_owned(),
        })
    }
}

// ── OptionProvider — happy path ───────────────────────────────────────────────

#[tokio::test]
async fn resolve_options_returns_items() {
    let mut registry = ProviderRegistry::new();
    let provider = Arc::new(StaticOptionProvider {
        options: vec![
            option("us-east-1", "US East"),
            option("eu-west-1", "EU West"),
        ],
    });
    registry
        .register_option_provider("cloud.regions", provider)
        .unwrap();

    let envelope = registry
        .resolve_options("cloud.regions", &make_request("region"))
        .await
        .unwrap();

    assert_eq!(envelope.items.len(), 2);
    assert_eq!(envelope.items[0].value, serde_json::json!("us-east-1"));
    assert_eq!(envelope.items[1].label, "EU West");
    assert_eq!(envelope.kind, DynamicResponseKind::Options);
    assert_eq!(envelope.response_version, 1);
}

#[tokio::test]
async fn resolve_options_passes_request_context() {
    struct InspectingProvider {
        tx: std::sync::Mutex<Option<ProviderRequest>>,
    }

    #[async_trait]
    impl OptionProvider for InspectingProvider {
        async fn resolve(
            &self,
            request: &ProviderRequest,
        ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
            *self.tx.lock().unwrap() = Some(request.clone());
            Ok(DynamicProviderEnvelope::new(DynamicResponseKind::Options, vec![]))
        }
    }

    let provider = Arc::new(InspectingProvider {
        tx: std::sync::Mutex::new(None),
    });
    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider("catalog.items", Arc::clone(&provider) as Arc<dyn OptionProvider>)
        .unwrap();

    let mut req = make_request("category");
    req.filter = Some("rust".to_owned());
    req.cursor = Some("page2".to_owned());

    registry.resolve_options("catalog.items", &req).await.unwrap();

    let captured = provider.tx.lock().unwrap().take().unwrap();
    assert_eq!(captured.field_id, "category");
    assert_eq!(captured.filter.as_deref(), Some("rust"));
    assert_eq!(captured.cursor.as_deref(), Some("page2"));
}

#[tokio::test]
async fn resolve_options_with_pagination_fields_preserved() {
    struct PaginatedProvider;

    #[async_trait]
    impl OptionProvider for PaginatedProvider {
        async fn resolve(
            &self,
            _request: &ProviderRequest,
        ) -> Result<DynamicProviderEnvelope<SelectOption>, ProviderError> {
            let mut env =
                DynamicProviderEnvelope::new(DynamicResponseKind::Options, vec![option("a", "A")]);
            env.next_cursor = Some("cursor-abc".to_owned());
            env.schema_version = Some("2026-01".to_owned());
            Ok(env)
        }
    }

    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider("cat.pages", Arc::new(PaginatedProvider))
        .unwrap();

    let envelope = registry
        .resolve_options("cat.pages", &make_request("x"))
        .await
        .unwrap();

    assert_eq!(envelope.next_cursor.as_deref(), Some("cursor-abc"));
    assert_eq!(envelope.schema_version.as_deref(), Some("2026-01"));
}

// ── OptionProvider — error paths ──────────────────────────────────────────────

#[tokio::test]
async fn resolve_options_not_found() {
    let registry = ProviderRegistry::new();
    let err = registry
        .resolve_options("missing.key", &make_request("x"))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::NotFound { .. }));
}

#[tokio::test]
async fn resolve_options_kind_mismatch_is_error() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider("bad.kind", Arc::new(WrongKindOptionProvider))
        .unwrap();

    let err = registry
        .resolve_options("bad.kind", &make_request("x"))
        .await
        .unwrap_err();

    assert!(matches!(err, ProviderError::KindMismatch { .. }));
}

#[tokio::test]
async fn resolve_options_unsupported_version_is_error() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider("bad.version", Arc::new(BadVersionOptionProvider))
        .unwrap();

    let err = registry
        .resolve_options("bad.version", &make_request("x"))
        .await
        .unwrap_err();

    assert!(matches!(err, ProviderError::UnsupportedVersion { .. }));
}

#[tokio::test]
async fn resolve_options_propagates_provider_error() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider("failing.provider", Arc::new(FailingOptionProvider))
        .unwrap();

    let err = registry
        .resolve_options("failing.provider", &make_request("x"))
        .await
        .unwrap_err();

    assert!(matches!(err, ProviderError::ResolveFailed { .. }));
}

// ── DynamicRecordProvider — happy path ────────────────────────────────────────

#[tokio::test]
async fn resolve_fields_returns_specs() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_dynamic_record_provider("sheet.columns", Arc::new(EchoFieldProvider))
        .unwrap();

    let envelope = registry
        .resolve_fields("sheet.columns", &make_request("col_picker"))
        .await
        .unwrap();

    assert_eq!(envelope.items.len(), 1);
    assert_eq!(envelope.kind, DynamicResponseKind::Fields);
    // EchoFieldProvider embeds field_id as spec id
    if let FieldSpec::Text { meta, .. } = &envelope.items[0] {
        assert_eq!(meta.id, "col_picker");
    } else {
        panic!("expected FieldSpec::Text");
    }
}

#[tokio::test]
async fn resolve_fields_not_found() {
    let registry = ProviderRegistry::new();
    let err = registry
        .resolve_fields("no.such.provider", &make_request("x"))
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::NotFound { .. }));
}

// ── Registry — registration helpers ──────────────────────────────────────────

#[tokio::test]
async fn upsert_option_provider_returns_old_value() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider("a.b", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap();

    let old = registry
        .upsert_option_provider("a.b", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap();

    assert!(old.is_some());
}

#[tokio::test]
async fn upsert_option_provider_first_insert_returns_none() {
    let mut registry = ProviderRegistry::new();
    let old = registry
        .upsert_option_provider("a.new", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap();
    assert!(old.is_none());
}

#[tokio::test]
async fn contains_key_reflects_both_provider_types() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider("opt.key", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap();
    registry
        .register_dynamic_record_provider("dyn.key", Arc::new(EchoFieldProvider))
        .unwrap();

    assert!(registry.contains_key("opt.key"));
    assert!(registry.contains_key("dyn.key"));
    assert!(!registry.contains_key("not.registered"));
}

#[tokio::test]
async fn provider_keys_are_sorted() {
    let mut registry = ProviderRegistry::new();
    for key in ["z.provider", "a.provider", "m.provider"] {
        registry
            .register_option_provider(key, Arc::new(StaticOptionProvider { options: vec![] }))
            .unwrap();
    }

    let keys = registry.option_provider_keys();
    assert_eq!(keys, vec!["a.provider", "m.provider", "z.provider"]);
}

#[tokio::test]
async fn provider_counts_are_tracked_independently() {
    let mut registry = ProviderRegistry::new();
    registry
        .register_option_provider("o1", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap();
    registry
        .register_option_provider("o2", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap();
    registry
        .register_dynamic_record_provider("d1", Arc::new(EchoFieldProvider))
        .unwrap();

    assert_eq!(registry.option_provider_count(), 2);
    assert_eq!(registry.dynamic_record_provider_count(), 1);
}

// ── Key validation ────────────────────────────────────────────────────────────

#[tokio::test]
async fn empty_key_is_invalid() {
    let mut registry = ProviderRegistry::new();
    let err = registry
        .register_option_provider("", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap_err();
    assert!(matches!(err, ProviderError::InvalidKey { .. }));
}

#[tokio::test]
async fn whitespace_only_key_is_invalid() {
    let mut registry = ProviderRegistry::new();
    let err = registry
        .register_option_provider("   ", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap_err();
    assert!(matches!(err, ProviderError::InvalidKey { .. }));
}

#[tokio::test]
async fn key_with_leading_dot_is_invalid() {
    let mut registry = ProviderRegistry::new();
    let err = registry
        .register_option_provider(".leading", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap_err();
    assert!(matches!(err, ProviderError::InvalidKey { .. }));
}

#[tokio::test]
async fn key_with_uppercase_is_invalid() {
    let mut registry = ProviderRegistry::new();
    let err = registry
        .register_option_provider("Cloud.Regions", Arc::new(StaticOptionProvider { options: vec![] }))
        .unwrap_err();
    assert!(matches!(err, ProviderError::InvalidKey { .. }));
}

#[tokio::test]
async fn valid_key_formats_are_accepted() {
    let mut registry = ProviderRegistry::new();
    for key in ["simple", "with.dot", "with-dash", "with_underscore", "a1.b2-c3_d4"] {
        registry
            .register_option_provider(key, Arc::new(StaticOptionProvider { options: vec![] }))
            .unwrap_or_else(|_| panic!("key `{key}` should be valid"));
    }
}
