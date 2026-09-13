//! Recorded JSON shape and admission round-trip regressions.

use std::assert_matches;

use nebula_metadata::{
    CatalogReference, DeprecationNotice, MAX_METADATA_JSON_BYTES, MetadataBuildError,
    MetadataDecodeLimits, MetadataDraft, MetadataError, PluginDependency, PluginManifest,
    RecordedBaseMetadata, check_json_record, decode_json_reader, decode_json_slice,
};
use nebula_schema::{Field, Rule, Schema, ValidSchema, field_key};
use semver::{Version, VersionReq};
use serde::de::DeserializeOwned;
use serde_json::json;

fn reject_positional<T: DeserializeOwned>(wire: &str) {
    assert!(serde_json::from_str::<T>(wire).is_err());
    assert!(serde_json::from_reader::<_, T>(wire.as_bytes()).is_err());
    assert!(serde_json::from_value::<T>(serde_json::from_str(wire).unwrap()).is_err());
    assert!(decode_json_slice::<T>(wire.as_bytes(), MetadataDecodeLimits::default()).is_err());
    assert!(decode_json_reader::<T>(wire.as_bytes(), MetadataDecodeLimits::default()).is_err());
}

#[test]
fn recorded_base_rejects_positional_arrays() {
    reject_positional::<RecordedBaseMetadata<String>>(
        r#"[2,"example.action","Example","",{"fields":[]}]"#,
    );
}

#[test]
fn manifest_rejects_positional_arrays() {
    reject_positional::<PluginManifest>(r#"[2,"example","Example"]"#);
}

#[test]
fn notice_rejects_positional_arrays() {
    reject_positional::<DeprecationNotice>(r#"["1.0.0"]"#);
}

#[test]
fn dependency_rejects_positional_arrays() {
    reject_positional::<PluginDependency>(r#"["example","^1"]"#);
}

#[test]
fn manifest_with_33_dependency_comparators_cannot_admit_undecodable_json() {
    let requirement = VersionReq {
        comparators: vec![
            ">=1.0.0"
                .parse::<VersionReq>()
                .unwrap()
                .comparators
                .remove(0);
            33
        ],
    };
    assert!(requirement.to_string().parse::<VersionReq>().is_err());
    let manifest = PluginManifest::builder("example", "Example")
        .dependency(PluginDependency::new(
            "dependency".parse().unwrap(),
            requirement,
        ))
        .build();
    assert!(
        manifest.is_err(),
        "manifest must reject unparseable dependency requirements"
    );
}

#[test]
fn schema_with_64_nested_objects_cannot_admit_undecodable_json() {
    let mut field: Field = Field::object(field_key!("nested")).into();
    for _ in 1..64 {
        field = Field::object(field_key!("nested")).add(field).into();
    }
    let schema = Schema::builder().add(field).build().unwrap();
    let wire = serde_json::to_vec(&json!({"schema": schema})).unwrap();
    assert!(serde_json::from_slice::<serde_json::Value>(&wire).is_err());
    let admitted = MetadataDraft::try_new("example".to_owned(), "Example", "")
        .unwrap()
        .bind_schema(schema);
    assert!(
        admitted.is_err(),
        "admission must reject JSON exceeding decoder depth"
    );
}

#[test]
fn replacement_with_33_comparators_cannot_admit_undecodable_json() {
    let requirement = VersionReq {
        comparators: vec![
            ">=1.0.0"
                .parse::<VersionReq>()
                .unwrap()
                .comparators
                .remove(0);
            33
        ],
    };
    assert!(requirement.to_string().parse::<VersionReq>().is_err());
    let replacement = CatalogReference::Action {
        key: "replacement".parse().unwrap(),
        version_requirement: Some(requirement),
    };
    let admitted = MetadataDraft::try_new("example".to_owned(), "Example", "")
        .unwrap()
        .with_deprecation(
            DeprecationNotice::new(Version::new(1, 0, 0)).with_replacement(replacement),
        )
        .bind_schema(ValidSchema::empty());
    assert!(
        admitted.is_err(),
        "admission must reject unparseable replacement requirements"
    );
}

fn nested_value(depth: usize) -> serde_json::Value {
    let mut value = json!("leaf");
    for _ in 0..depth {
        value = json!({"nested": value});
    }
    value
}

fn verify_schema_roundtrip(schema: ValidSchema) -> bool {
    let mut wire = serde_json::to_value(
        MetadataDraft::try_new("example".to_owned(), "Example", "")
            .unwrap()
            .bind_schema(ValidSchema::empty())
            .unwrap(),
    )
    .unwrap();
    wire["schema"] = serde_json::to_value(&schema).unwrap();
    let bytes = serde_json::to_vec(&wire).unwrap();
    let parser_accepts = serde_json::from_slice::<serde_json::Value>(&bytes).is_ok();
    let admitted = MetadataDraft::try_new("example".to_owned(), "Example", "")
        .unwrap()
        .bind_schema(schema);
    if parser_accepts {
        let admitted = admitted.unwrap();
        let recorded: RecordedBaseMetadata<String> =
            decode_json_slice(&bytes, MetadataDecodeLimits::default()).unwrap();
        assert_eq!(recorded.readmit_against(&admitted).unwrap(), admitted);
        let reader: RecordedBaseMetadata<String> =
            decode_json_reader(bytes.as_slice(), MetadataDecodeLimits::default()).unwrap();
        assert_eq!(reader.readmit_against(&admitted).unwrap(), admitted);
        let owned: RecordedBaseMetadata<String> = serde_json::from_value(wire).unwrap();
        assert_eq!(owned.readmit_against(&admitted).unwrap(), admitted);
    } else {
        assert_matches!(
            admitted,
            Err(MetadataBuildError::Metadata(
                MetadataError::RecordNotDecodable
            ))
        );
        assert!(serde_json::from_value::<RecordedBaseMetadata<String>>(wire).is_err());
        assert!(
            decode_json_reader::<RecordedBaseMetadata<String>>(
                bytes.as_slice(),
                MetadataDecodeLimits::default()
            )
            .is_err()
        );
    }
    parser_accepts
}

#[test]
fn nested_defaults_obey_exact_parser_depth_boundary() {
    let mut accepted = 0;
    let mut rejected = 0;
    for depth in 115..=130 {
        let schema = Schema::builder()
            .add(Field::object(field_key!("value")).default(nested_value(depth)))
            .build()
            .unwrap();
        if verify_schema_roundtrip(schema) {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    assert!(
        accepted > 0 && rejected > 0,
        "must exercise both sides of the parser boundary"
    );
}

#[test]
fn schema_nesting_and_rule_operand_depth_share_one_record_limit() {
    let rule = Rule::one_of(vec![nested_value(60)]).unwrap();
    let mut accepted = 0;
    let mut rejected = 0;
    for depth in 20..=40 {
        let mut field: Field = Field::object(field_key!("value"))
            .with_rule(rule.clone())
            .into();
        for _ in 0..depth {
            field = Field::object(field_key!("nested")).add(field).into();
        }
        let schema = Schema::builder().add(field).build().unwrap();
        if verify_schema_roundtrip(schema) {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    assert!(
        accepted > 0 && rejected > 0,
        "combined nesting must cross the parser boundary"
    );
}

#[test]
fn whole_leaf_record_checks_extra_nesting() {
    let mut boundary = None;
    for depth in 120..=130 {
        let value = nested_value(depth);
        let bytes = serde_json::to_vec(&value).unwrap();
        let expected = serde_json::from_slice::<serde_json::Value>(&bytes).is_ok();
        assert_eq!(check_json_record(&value).is_ok(), expected);
        if expected {
            boundary = Some(value);
        }
    }
    let base = boundary.unwrap();
    assert_eq!(check_json_record(&base), Ok(()));
    assert_eq!(
        check_json_record(&json!({"base": base})),
        Err(MetadataError::RecordNotDecodable)
    );
}

#[test]
fn whole_record_byte_boundary_counts_json_escaping() {
    let mut record = json!({"base": {}, "leaf": ""});
    let overhead = serde_json::to_vec(&record).unwrap().len();
    let budget = MAX_METADATA_JSON_BYTES - overhead;
    record["leaf"] = json!(format!(
        "{}{}",
        "\u{0001}".repeat(budget / 6),
        "x".repeat(budget % 6)
    ));
    assert_eq!(
        serde_json::to_vec(&record).unwrap().len(),
        MAX_METADATA_JSON_BYTES
    );
    assert_eq!(check_json_record(&record), Ok(()));
    record["leaf"] = json!(format!("{}x", record["leaf"].as_str().unwrap()));
    assert_eq!(
        serde_json::to_vec(&record).unwrap().len(),
        MAX_METADATA_JSON_BYTES + 1
    );
    assert_eq!(
        check_json_record(&record),
        Err(MetadataError::RecordTooLarge)
    );
}

#[test]
fn dependency_limit_retains_valid_intent_and_reports_invalid_field() {
    use nebula_metadata::{ManifestError, MetadataField};

    let comparator = ">=1.0.0"
        .parse::<VersionReq>()
        .unwrap()
        .comparators
        .remove(0);
    let valid = VersionReq {
        comparators: vec![comparator.clone(); 32],
    };
    let manifest = PluginManifest::builder("example", "Example")
        .dependency(PluginDependency::new(
            "dependency".parse().unwrap(),
            valid.clone(),
        ))
        .build()
        .unwrap();
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let decoded: PluginManifest =
        decode_json_reader(bytes.as_slice(), MetadataDecodeLimits::default()).unwrap();
    assert_eq!(decoded, manifest);
    assert_eq!(decoded.dependencies()[0].req(), &valid);

    let invalid = VersionReq {
        comparators: vec![comparator; 33],
    };
    let error = PluginManifest::builder("example", "Example")
        .dependency(PluginDependency::new(
            "dependency".parse().unwrap(),
            invalid,
        ))
        .build()
        .unwrap_err();
    assert_eq!(
        error,
        ManifestError::Metadata(MetadataError::InvalidVersion(
            MetadataField::ManifestDependencies
        ))
    );
}

#[test]
fn positional_errors_never_echo_supplied_values() {
    const CANARY: &str = "private_positional_value";
    let wire = format!(r#"["{CANARY}"]"#);
    for error in [
        serde_json::from_str::<RecordedBaseMetadata<String>>(&wire).unwrap_err(),
        serde_json::from_str::<PluginManifest>(&wire).unwrap_err(),
        serde_json::from_str::<DeprecationNotice>(&wire).unwrap_err(),
        serde_json::from_str::<PluginDependency>(&wire).unwrap_err(),
    ] {
        assert!(!format!("{error:?}: {error}").contains(CANARY));
    }
}
