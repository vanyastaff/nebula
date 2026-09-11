//! Integration tests for `#[derive(ResourceConfig)]`.
//!
//! These tests exercise the emitted `ResourceConfig::fingerprint` implementation
//! via compilation + runtime assertions. They also cover the `validate` hook and
//! `skip_fingerprint` field attribute.

use nebula_resource::ResourceConfig;

// ── Unit struct ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ResourceConfig)]
struct UnitCfg;

#[test]
fn unit_struct_fingerprint_is_zero() {
    assert_eq!(UnitCfg.fingerprint(), 0);
}

#[test]
fn unit_struct_identical_instances_equal_fingerprint() {
    assert_eq!(UnitCfg.fingerprint(), UnitCfg.fingerprint());
}

#[test]
fn unit_struct_schema_matches_serde_null_wire() {
    let wire = serde_json::to_value(UnitCfg).unwrap();
    assert_eq!(wire, serde_json::Value::Null);
    let schema = nebula_schema::schema_of::<UnitCfg>().unwrap();
    let resolved = schema
        .validate(nebula_schema::AuthoredValue::from_data(wire).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(resolved.into_typed::<UnitCfg>().unwrap(), UnitCfg);

    let report = schema
        .validate(nebula_schema::AuthoredValue::from_data(serde_json::json!({})).unwrap())
        .unwrap_err();
    assert!(
        report
            .errors()
            .any(|error| { error.code() == "type_mismatch" && error.path().as_str().is_empty() })
    );
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ResourceConfig)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "serde object-wire fixture must remain distinct from unit null"
)]
struct EmptyRecordCfg {}

#[test]
fn empty_braced_struct_schema_matches_serde_object_wire() {
    let wire = serde_json::to_value(EmptyRecordCfg {}).unwrap();
    assert_eq!(wire, serde_json::json!({}));
    let schema = nebula_schema::schema_of::<EmptyRecordCfg>().unwrap();
    let resolved = schema
        .validate(nebula_schema::AuthoredValue::from_data(wire).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(
        resolved.into_typed::<EmptyRecordCfg>().unwrap(),
        EmptyRecordCfg {}
    );

    let report = schema
        .validate(nebula_schema::AuthoredValue::from_data(serde_json::Value::Null).unwrap())
        .unwrap_err();
    assert!(
        report
            .errors()
            .any(|error| { error.code() == "type_mismatch" && error.path().as_str().is_empty() })
    );
}

// ── Named-field struct ────────────────────────────────────────────────────────

#[derive(Clone, ResourceConfig, serde::Serialize, serde::Deserialize, nebula_schema::Schema)]
#[config(schema = external)]
struct NamedCfg {
    host: String,
    port: u16,
}

#[test]
fn named_config_schema_rejects_wrong_field_types_before_decoding() {
    let schema = nebula_schema::schema_of::<NamedCfg>().unwrap();
    let report = schema
        .validate(
            nebula_schema::AuthoredValue::from_data(serde_json::json!({
                "host": "localhost", "port": "not-a-port"
            }))
            .unwrap(),
        )
        .unwrap_err();
    assert!(
        report
            .errors()
            .any(|error| { error.code() == "type_mismatch" && error.path().as_str() == "/port" })
    );
}

#[test]
fn named_struct_identical_instances_equal_fingerprint() {
    let a = NamedCfg {
        host: "localhost".into(),
        port: 5432,
    };
    let b = NamedCfg {
        host: "localhost".into(),
        port: 5432,
    };
    assert_eq!(
        a.fingerprint(),
        b.fingerprint(),
        "identical configs must produce equal fingerprints"
    );
}

#[test]
fn named_struct_differing_field_produces_different_fingerprint() {
    let a = NamedCfg {
        host: "host-a".into(),
        port: 5432,
    };
    let b = NamedCfg {
        host: "host-b".into(),
        port: 5432,
    };
    assert_ne!(
        a.fingerprint(),
        b.fingerprint(),
        "configs differing in `host` must have different fingerprints"
    );
}

#[test]
fn named_struct_differing_port_produces_different_fingerprint() {
    let a = NamedCfg {
        host: "localhost".into(),
        port: 5432,
    };
    let b = NamedCfg {
        host: "localhost".into(),
        port: 5433,
    };
    assert_ne!(
        a.fingerprint(),
        b.fingerprint(),
        "configs differing in `port` must have different fingerprints"
    );
}

#[test]
fn named_struct_fingerprint_nonzero_for_nonempty_fields() {
    let cfg = NamedCfg {
        host: "localhost".into(),
        port: 5432,
    };
    // With non-empty fields the fingerprint is very unlikely to hash to 0;
    // this assertion guards against a regression where the body returns a
    // literal 0 for structs with fields.
    assert_ne!(
        cfg.fingerprint(),
        0,
        "non-empty named config must not return 0"
    );
}

// ── skip_fingerprint field ────────────────────────────────────────────────────

#[derive(Clone, ResourceConfig, nebula_schema::Schema)]
#[config(schema = external)]
struct SkipFieldCfg {
    /// Included in fingerprint.
    endpoint: String,
    /// Excluded from fingerprint — changing this alone must not trigger hot-reload.
    // guard-justified: the field is under test precisely for being skipped by
    // the fingerprint fold; it is set but never read, which is the point.
    #[expect(
        dead_code,
        reason = "exercised via skip_fingerprint, intentionally never read"
    )]
    #[config(skip_fingerprint)]
    debug_label: String,
}

#[test]
fn skip_fingerprint_field_ignored_in_hash() {
    let a = SkipFieldCfg {
        endpoint: "https://api.example.com".into(),
        debug_label: "label-a".into(),
    };
    let b = SkipFieldCfg {
        endpoint: "https://api.example.com".into(),
        debug_label: "label-b".into(),
    };
    assert_eq!(
        a.fingerprint(),
        b.fingerprint(),
        "changing only a skip_fingerprint field must not affect the fingerprint"
    );
}

#[test]
fn skip_fingerprint_included_field_still_differs() {
    let a = SkipFieldCfg {
        endpoint: "https://api-a.example.com".into(),
        debug_label: "same".into(),
    };
    let b = SkipFieldCfg {
        endpoint: "https://api-b.example.com".into(),
        debug_label: "same".into(),
    };
    assert_ne!(
        a.fingerprint(),
        b.fingerprint(),
        "changing the non-skipped field must still change the fingerprint"
    );
}

// ── validate hook ─────────────────────────────────────────────────────────────

fn validate_url_cfg(cfg: &UrlCfg) -> Result<(), nebula_resource::Error> {
    if cfg.url.is_empty() {
        Err(nebula_resource::Error::permanent("url must not be empty"))
    } else {
        Ok(())
    }
}

#[derive(Clone, ResourceConfig, nebula_schema::Schema)]
#[config(validate = validate_url_cfg, schema = external)]
struct UrlCfg {
    url: String,
}

#[test]
fn validate_hook_returns_ok_for_valid_config() {
    let cfg = UrlCfg {
        url: "https://example.com".into(),
    };
    assert!(cfg.validate().is_ok());
}

#[test]
fn validate_hook_returns_err_for_invalid_config() {
    let cfg = UrlCfg { url: String::new() };
    let err = cfg.validate().unwrap_err();
    assert!(
        matches!(err.kind(), nebula_resource::error::ErrorKind::Permanent),
        "empty url should produce a Permanent error"
    );
}

// ── Tuple struct ──────────────────────────────────────────────────────────────

#[derive(Clone, ResourceConfig)]
#[config(schema = external)]
struct TupleCfg(String, u32);

// This fixture exercises tuple fingerprinting only. Positional array roots are
// not part of RootShape, so schema admission must fail rather than claim a record.
impl nebula_schema::HasSchema for TupleCfg {
    fn schema() -> Result<nebula_schema::ValidSchema, nebula_schema::ValidationReport> {
        Err(
            nebula_schema::ValidationError::builder("schema.unsupported_tuple_root")
                .message("positional tuple configuration has no supported root schema")
                .build()
                .into(),
        )
    }
}

#[test]
fn tuple_config_does_not_publish_an_incorrect_record_schema() {
    let report = nebula_schema::schema_of::<TupleCfg>().unwrap_err();
    assert!(
        report
            .errors()
            .any(|error| error.code() == "schema.unsupported_tuple_root")
    );
}

#[test]
fn tuple_struct_identical_instances_equal_fingerprint() {
    let a = TupleCfg("a".into(), 1);
    let b = TupleCfg("a".into(), 1);
    assert_eq!(a.fingerprint(), b.fingerprint());
}

#[test]
fn tuple_struct_different_first_field_differs() {
    let a = TupleCfg("a".into(), 1);
    let b = TupleCfg("b".into(), 1);
    assert_ne!(a.fingerprint(), b.fingerprint());
}
