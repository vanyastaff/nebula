//! Raw transport caps and structural decoding must both reject hostile records.

use std::{
    cell::Cell,
    error::Error,
    io::{self, Read},
};

use nebula_metadata::{
    DeprecationNotice, MAX_METADATA_JSON_BYTES, MetadataDecodeError, MetadataDecodeLimits,
    MetadataDraft, PluginManifest, RecordedBaseMetadata, decode_json_reader, decode_json_slice,
};
use nebula_schema::ValidSchema;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const CANARY: &str = "private_parser_canary";

fn wire() -> Value {
    json!(
        MetadataDraft::try_new("example".to_owned(), "Example", "")
            .unwrap()
            .bind_schema(ValidSchema::empty())
            .unwrap()
    )
}

fn assert_sanitized<T: DeserializeOwned + std::fmt::Debug>(value: Value) {
    let error = serde_json::from_value::<T>(value.clone()).unwrap_err();
    assert!(!format!("{error}: {error:?}").contains(CANARY));
    let bytes = serde_json::to_vec(&value).unwrap();
    for error in [
        decode_json_slice::<T>(&bytes, MetadataDecodeLimits::default()).unwrap_err(),
        decode_json_reader::<T>(bytes.as_slice(), MetadataDecodeLimits::default()).unwrap_err(),
    ] {
        assert_eq!(error, MetadataDecodeError::InvalidRecord);
        assert!(error.source().is_none());
        assert!(!format!("{error}: {error:?}").contains(CANARY));
    }
}

#[test]
fn missing_old_and_future_wire_versions_are_not_supported() {
    for version in [
        None,
        Some(json!(0)),
        Some(json!(1)),
        Some(json!(3)),
        Some(json!(CANARY)),
    ] {
        let mut record = wire();
        let mut manifest = json!(
            PluginManifest::builder("example", "Example")
                .build()
                .unwrap()
        );
        for value in [&mut record, &mut manifest] {
            value
                .as_object_mut()
                .unwrap()
                .remove("metadata_wire_version");
            if let Some(version) = &version {
                value["metadata_wire_version"] = version.clone();
            }
        }
        assert_sanitized::<RecordedBaseMetadata<String>>(record);
        assert_sanitized::<PluginManifest>(manifest);
    }
}

#[test]
fn legacy_fields_are_rejected_even_with_a_current_version() {
    let mut record = wire();
    record["documentation_url"] = json!("https://example.test/legacy");
    assert_sanitized::<RecordedBaseMetadata<String>>(record);
    assert_sanitized::<DeprecationNotice>(json!({"since":"1.0.0", "sunset":"2028-01-01"}));
    assert_sanitized::<DeprecationNotice>(json!({"since":"1.0.0", "replacement":"old.action"}));
}

#[test]
fn unknown_fields_variants_and_wrong_scalars_never_echo_payloads() {
    for field in [
        CANARY,
        "key",
        "name",
        "version",
        "schema",
        "maturity",
        "icon",
        "categories",
        "tags",
        "links",
    ] {
        let mut value = wire();
        value[field] = if field == "key" || field == "name" {
            json!({CANARY: CANARY})
        } else if field == "icon" {
            json!({"url": {CANARY: CANARY}})
        } else {
            json!(CANARY)
        };
        assert_sanitized::<RecordedBaseMetadata<String>>(value);
    }
    for notice in [
        json!({"since":"1.0.0", CANARY: CANARY}),
        json!({"since": {CANARY: CANARY}}),
        json!({"since":"1.0.0", "removal":{"kind":CANARY,"value":CANARY}}),
        json!({"since":"1.0.0", "replacement":{"kind":CANARY,"key":CANARY}}),
        json!({"since":"1.0.0", "reason":{CANARY:CANARY}}),
    ] {
        assert_sanitized::<DeprecationNotice>(notice);
    }
    for field in [
        CANARY,
        "key",
        "name",
        "version",
        "maturity",
        "dependencies",
        "icon",
    ] {
        let mut value = json!(
            PluginManifest::builder("example", "Example")
                .build()
                .unwrap()
        );
        value[field] = if field == "key" || field == "name" {
            json!({CANARY:CANARY})
        } else if field == "icon" {
            json!({"url": {CANARY: CANARY}})
        } else {
            json!(CANARY)
        };
        assert_sanitized::<PluginManifest>(value);
    }
}

#[test]
fn direct_serde_enforces_raw_counts_field_bounds_and_aggregate_budget() {
    let mut excessive_tags = wire();
    excessive_tags["tags"] = json!(vec!["same"; 65]);
    assert_sanitized::<RecordedBaseMetadata<String>>(excessive_tags);
    let mut excessive_description = wire();
    excessive_description["description"] = json!("d".repeat(8193));
    assert_sanitized::<RecordedBaseMetadata<String>>(excessive_description);
    let mut aggregate = wire();
    aggregate["name"] = json!("n".repeat(20000));
    aggregate["icon"] = json!("i".repeat(20000));
    assert_sanitized::<RecordedBaseMetadata<String>>(aggregate);
    let mut categories = wire();
    categories["categories"] = json!((0..17).map(|i| format!("category{i}")).collect::<Vec<_>>());
    assert_sanitized::<RecordedBaseMetadata<String>>(categories);
    let mut links = wire();
    links["links"] = json!(
        (0..17)
            .map(|i| json!({"relation":"setup", "target":format!("/doc{i}")}))
            .collect::<Vec<_>>()
    );
    assert_sanitized::<RecordedBaseMetadata<String>>(links);
}

#[test]
fn raw_envelope_ceiling_counts_whitespace_and_accepts_the_exact_boundary() {
    let canonical = serde_json::to_vec(&wire()).unwrap();
    let limits = MetadataDecodeLimits::new(canonical.len()).unwrap();
    let record: RecordedBaseMetadata<String> = decode_json_slice(&canonical, limits).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), wire());
    assert_eq!(
        decode_json_slice::<RecordedBaseMetadata<String>>(
            &canonical,
            MetadataDecodeLimits::new(canonical.len() - 1).unwrap()
        )
        .unwrap_err(),
        MetadataDecodeError::EnvelopeTooLarge
    );
    let mut exact = canonical;
    exact.resize(MAX_METADATA_JSON_BYTES, b' ');
    let record: RecordedBaseMetadata<String> =
        decode_json_slice(&exact, MetadataDecodeLimits::default()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), wire());
    exact.push(b' ');
    assert_eq!(
        decode_json_slice::<RecordedBaseMetadata<String>>(&exact, MetadataDecodeLimits::default())
            .unwrap_err(),
        MetadataDecodeError::EnvelopeTooLarge
    );
    assert_eq!(
        MetadataDecodeLimits::new(MAX_METADATA_JSON_BYTES + 1).unwrap_err(),
        MetadataDecodeError::InvalidLimits
    );
    assert_eq!(
        MetadataDecodeLimits::new(0).unwrap_err(),
        MetadataDecodeError::InvalidLimits
    );
}

#[test]
fn reader_consumes_only_max_plus_one_and_sanitizes_io_errors() {
    struct Endless<'a>(&'a Cell<usize>);
    impl Read for Endless<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            buffer.fill(b' ');
            self.0.set(self.0.get() + buffer.len());
            Ok(buffer.len())
        }
    }
    let consumed = Cell::new(0);
    let error = decode_json_reader::<RecordedBaseMetadata<String>>(
        Endless(&consumed),
        MetadataDecodeLimits::new(128).unwrap(),
    )
    .unwrap_err();
    assert_eq!(error, MetadataDecodeError::EnvelopeTooLarge);
    assert_eq!(consumed.get(), 129);
    struct Failed;
    impl Read for Failed {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other(CANARY))
        }
    }
    let error =
        decode_json_reader::<RecordedBaseMetadata<String>>(Failed, MetadataDecodeLimits::default())
            .unwrap_err();
    assert_eq!(error, MetadataDecodeError::ReadFailed);
    assert!(error.source().is_none());
    assert!(!format!("{error}: {error:?}").contains(CANARY));
}

#[test]
fn nested_leaf_decoder_caps_the_whole_outer_envelope() {
    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Leaf {
        base: RecordedBaseMetadata<String>,
        extra: String,
    }
    let bytes = serde_json::to_vec(&json!({"base":wire(), "extra":"details"})).unwrap();
    let leaf: Leaf = decode_json_reader(
        bytes.as_slice(),
        MetadataDecodeLimits::new(bytes.len()).unwrap(),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(leaf.base).unwrap(), wire());
    assert_eq!(leaf.extra, "details");
    assert_eq!(
        decode_json_slice::<Leaf>(&bytes, MetadataDecodeLimits::new(bytes.len() - 1).unwrap())
            .unwrap_err(),
        MetadataDecodeError::EnvelopeTooLarge
    );
    assert_sanitized::<Leaf>(wire());
}
