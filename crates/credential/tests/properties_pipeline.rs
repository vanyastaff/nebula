//! Credential properties complete validation without executing authored programs.
//! Serde consumes the normalized, explicitly exposed resolved data, not the
//! original request or its redacted diagnostic projection.

use nebula_credential::{Credential, CredentialRegistry, credentials::ApiKeyCredential};
use nebula_schema::{
    AuthoredValue, HasSchema, ResolvedValues, SecretValue, ValidationReport, field_key, schema_of,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn prepare<P: HasSchema>(wire: Value) -> Result<ResolvedValues, ValidationReport> {
    let schema = schema_of::<P>()?;
    schema
        .validate(schema.values_from_wire(wire)?)?
        .resolve_data()
}

fn assert_issue(report: &ValidationReport, code: &str, path: &str) {
    assert!(
        report
            .errors()
            .any(|error| error.code() == code && error.path().as_str() == path),
        "missing {code} at {path}: {report:?}",
    );
}

#[test]
fn metadata_schema_is_schema_of_properties() {
    let mut registry = CredentialRegistry::new();
    registry
        .register(ApiKeyCredential, "properties-pipeline-test")
        .unwrap();
    let metadata = registry.metadata(ApiKeyCredential::KEY).unwrap();
    let schema = schema_of::<<ApiKeyCredential as Credential>::Properties>().unwrap();
    assert_eq!(metadata.schema(), &schema);
    assert_eq!(
        schema,
        <<ApiKeyCredential as Credential>::Properties as HasSchema>::schema().unwrap()
    );
}

#[test]
fn properties_pipeline_accepts_well_formed_json() {
    let resolved = prepare::<<ApiKeyCredential as Credential>::Properties>(json!({
        "server": "https://api.example.com", "api_key": "sk-test-12345",
    }))
    .unwrap();
    assert_eq!(
        resolved.clone().into_json(),
        json!({
            "server": "https://api.example.com", "api_key": "<redacted>"
        })
    );
    let SecretValue::String(secret) = resolved.get_secret(&field_key!("api_key")).unwrap() else {
        panic!("API key must already be protected before provider resolution");
    };
    assert_eq!(secret.expose(), "sk-test-12345");
    let typed: <ApiKeyCredential as Credential>::Properties =
        resolved.into_typed_exposing_secrets().unwrap();
    assert_eq!(typed.server.as_deref(), Some("https://api.example.com"));
    assert_eq!(typed.api_key.expose_secret(), "sk-test-12345");
}

#[test]
fn properties_pipeline_rejects_missing_required_api_key() {
    let report = prepare::<<ApiKeyCredential as Credential>::Properties>(json!({
        "server": "https://api.example.com"
    }))
    .unwrap_err();
    assert_issue(&report, "required", "/api_key");
}

#[test]
fn expressions_in_properties_fail_schema_and_serde() {
    let raw = json!({
        "server": "https://api.example.com",
        "api_key": {"$expr": "{{ $execution.id }}"},
    });
    let report = prepare::<<ApiKeyCredential as Credential>::Properties>(raw.clone()).unwrap_err();
    assert_issue(&report, "type_mismatch", "/api_key");
    assert!(serde_json::from_value::<<ApiKeyCredential as Credential>::Properties>(raw).is_err());
    assert!(!format!("{report:?} {report}").contains("$execution.id"));
}

#[test]
fn expressions_in_optional_property_field_also_fail_schema_and_serde() {
    let raw = json!({
        "server": {"$expr": "{{ $workflow.base_url }}"}, "api_key": "sk-real-secret",
    });
    let report = prepare::<<ApiKeyCredential as Credential>::Properties>(raw.clone()).unwrap_err();
    assert_issue(&report, "type_mismatch", "/server");
    assert!(serde_json::from_value::<<ApiKeyCredential as Credential>::Properties>(raw).is_err());
    assert!(!format!("{report:?} {report}").contains("sk-real-secret"));
}

#[test]
fn template_looking_property_strings_remain_literal_secret_data() {
    let resolved = prepare::<<ApiKeyCredential as Credential>::Properties>(json!({
        "api_key": "{{ $execution.id }}",
    }))
    .unwrap();
    let typed: <ApiKeyCredential as Credential>::Properties =
        resolved.into_typed_exposing_secrets().unwrap();
    assert_eq!(typed.api_key.expose_secret(), "{{ $execution.id }}");
}

#[test]
fn explicit_authored_programs_cannot_complete_the_data_only_pipeline() {
    let schema = schema_of::<<ApiKeyCredential as Credential>::Properties>().unwrap();
    let input = AuthoredValue::from_template_json(json!({
        "server": {"$expr": "{{ $input.url }}"}, "api_key": "private-token",
    }))
    .unwrap();
    let prepared = schema.validate(input).unwrap();
    assert_eq!(prepared.pending().len(), 1);
    let error = prepared.resolve_data().unwrap_err();
    assert_issue(&error, "expression.forbidden", "/server");
    assert!(!format!("{error:?}").contains("private-token"));
}

#[derive(Debug, Deserialize, Serialize, PartialEq, nebula_schema::Schema)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "braced and unit structs have distinct serde wire shapes"
)]
struct EmptyProperties {}

#[derive(Debug, Deserialize, Serialize, PartialEq, nebula_schema::Schema)]
struct UnitProperties;

fn assert_properties_roundtrip<P>(properties: P)
where
    P: HasSchema + Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let wire = serde_json::to_value(&properties).unwrap();
    let resolved = prepare::<P>(wire.clone()).unwrap();
    assert_eq!(resolved.to_wire_json(), wire);
    assert_eq!(resolved.into_typed::<P>().unwrap(), properties);
}

#[test]
fn unit_and_empty_record_properties_have_distinct_serde_contracts() {
    assert_properties_roundtrip(());
    assert_properties_roundtrip(UnitProperties);
    assert_properties_roundtrip(EmptyProperties {});
    for error in [
        prepare::<()>(json!({})).unwrap_err(),
        prepare::<UnitProperties>(json!({})).unwrap_err(),
        prepare::<EmptyProperties>(json!(null)).unwrap_err(),
    ] {
        assert_issue(&error, "type_mismatch", "");
    }
}

#[test]
fn scalar_properties_preserve_their_known_types() {
    assert_properties_roundtrip(true);
    assert_properties_roundtrip("{{ literal-secret-looking-data }}".to_owned());
    assert_properties_roundtrip(i8::MIN);
    assert_properties_roundtrip(i8::MAX);
    assert_properties_roundtrip(u64::MAX);
    assert_properties_roundtrip(f64::MIN_POSITIVE);
    for error in [
        prepare::<bool>(json!(1)).unwrap_err(),
        prepare::<String>(json!(true)).unwrap_err(),
        prepare::<i8>(json!("127")).unwrap_err(),
    ] {
        assert_issue(&error, "type_mismatch", "");
    }
}

#[test]
fn scalar_property_bounds_are_enforced_before_rust_decoding() {
    for wire in [json!(-129), json!(128), json!(u64::MAX)] {
        assert!(serde_json::from_value::<i8>(wire.clone()).is_err());
        let report = prepare::<i8>(wire).unwrap_err();
        assert_eq!(report.errors().count(), 1);
        assert_eq!(report.errors().next().unwrap().path().as_str(), "");
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, nebula_schema::Schema)]
struct OAuthProps {
    client_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, nebula_schema::Schema)]
enum AuthMethod {
    OAuth(OAuthProps),
    ApiKey { token: String },
    Anonymous,
}

fn assert_union_properties_roundtrip(value: AuthMethod) {
    let schema = schema_of::<AuthMethod>().unwrap();
    assert_eq!(schema.kind(), nebula_schema::SchemaKind::Union);
    let wire = serde_json::to_value(&value).unwrap();
    let resolved = prepare::<AuthMethod>(wire.clone()).unwrap();
    assert_eq!(resolved.to_wire_json(), wire);
    let back: AuthMethod = resolved.into_typed_exposing_secrets().unwrap();
    assert_eq!(back, value, "credential union pipeline must round-trip");
}

#[test]
fn union_properties_pipeline_external_data_variant() {
    assert_union_properties_roundtrip(AuthMethod::OAuth(OAuthProps {
        client_id: "id".to_owned(),
    }));
}

#[test]
fn union_properties_pipeline_external_struct_variant() {
    assert_union_properties_roundtrip(AuthMethod::ApiKey {
        token: "t".to_owned(),
    });
}

#[test]
fn union_properties_pipeline_unit_variant() {
    assert_union_properties_roundtrip(AuthMethod::Anonymous);
}

#[test]
fn union_properties_pipeline_rejects_unknown_variant() {
    let schema = schema_of::<AuthMethod>().unwrap();
    let error = schema.values_from_wire(json!({"Nope": {}})).unwrap_err();
    assert_eq!(error.code(), "union.unknown_variant");
}

#[test]
fn union_properties_pipeline_refuses_expression_payload() {
    let wire = json!({"ApiKey": {"token": {"$expr": "{{ $secret }}"}}});
    let error = prepare::<AuthMethod>(wire.clone()).unwrap_err();
    assert_eq!(
        error
            .errors()
            .map(nebula_schema::ValidationError::code)
            .collect::<Vec<_>>(),
        ["type_mismatch"]
    );
    assert!(serde_json::from_value::<AuthMethod>(wire).is_err());
    assert!(!format!("{error:?} {error}").contains("$secret"));
}
