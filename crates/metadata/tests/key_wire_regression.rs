//! Shared keys must retain the recorded string-wire contract.

use std::{assert_matches, cell::Cell, fmt::Debug, str::FromStr};

use nebula_metadata::{
    MAX_SHARED_METADATA_BYTES, MetadataBuildError, MetadataDecodeLimits, MetadataDraft,
    MetadataError, RecordedBaseMetadata, decode_json_reader, decode_json_slice,
};
use nebula_schema::ValidSchema;
use serde::{Serialize, Serializer, ser::SerializeSeq as _};

#[test]
fn admission_rejects_numeric_key_serialization() {
    let admitted = MetadataDraft::try_new(42_u64, "Example", "")
        .unwrap()
        .bind_schema(ValidSchema::empty());
    assert_matches!(
        admitted,
        Err(MetadataBuildError::Metadata(MetadataError::InvalidKey))
    );
}

#[test]
fn recorded_ingress_rejects_key_that_reserializes_as_number() {
    let wire = r#"{"metadata_wire_version":2,"key":"42","name":"Example","description":"","schema":{"fields":[]}}"#;
    assert!(serde_json::from_str::<RecordedBaseMetadata<u64>>(wire).is_err());
    assert!(serde_json::from_reader::<_, RecordedBaseMetadata<u64>>(wire.as_bytes()).is_err());
    assert!(
        serde_json::from_value::<RecordedBaseMetadata<u64>>(serde_json::from_str(wire).unwrap())
            .is_err()
    );
    assert!(
        decode_json_slice::<RecordedBaseMetadata<u64>>(
            wire.as_bytes(),
            MetadataDecodeLimits::default()
        )
        .is_err()
    );
    assert!(
        decode_json_reader::<RecordedBaseMetadata<u64>>(
            wire.as_bytes(),
            MetadataDecodeLimits::default()
        )
        .is_err()
    );
}

fn assert_key_roundtrip<K>(key: K)
where
    K: Serialize + FromStr + Clone + PartialEq + Debug,
{
    let admitted = MetadataDraft::try_new(key, "Example", "")
        .unwrap()
        .bind_schema(ValidSchema::empty())
        .unwrap();
    let wire = serde_json::to_value(&admitted).unwrap();
    assert!(wire["key"].is_string());
    let bytes = serde_json::to_vec(&admitted).unwrap();
    let slice: RecordedBaseMetadata<K> =
        decode_json_slice(&bytes, MetadataDecodeLimits::default()).unwrap();
    assert_eq!(slice.readmit_against(&admitted).unwrap(), admitted);
    let reader: RecordedBaseMetadata<K> =
        decode_json_reader(bytes.as_slice(), MetadataDecodeLimits::default()).unwrap();
    assert_eq!(reader.readmit_against(&admitted).unwrap(), admitted);
    let owned: RecordedBaseMetadata<K> = serde_json::from_value(wire).unwrap();
    assert_eq!(owned.readmit_against(&admitted).unwrap(), admitted);
}

#[test]
fn string_and_all_core_entity_keys_retain_exact_roundtrips() {
    assert_key_roundtrip("escaped\n\"key".to_owned());
    assert_key_roundtrip("example.action".parse::<nebula_core::ActionKey>().unwrap());
    assert_key_roundtrip(
        "example.credential"
            .parse::<nebula_core::CredentialKey>()
            .unwrap(),
    );
    assert_key_roundtrip(
        "example.resource"
            .parse::<nebula_core::ResourceKey>()
            .unwrap(),
    );
    assert_key_roundtrip("example.plugin".parse::<nebula_core::PluginKey>().unwrap());
}

#[derive(Debug)]
struct VaryingKey<'a> {
    calls: Cell<usize>,
    entries: &'a Cell<usize>,
}

impl Serialize for VaryingKey<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == 0 {
            return serializer.serialize_str("initial");
        }
        let mut sequence = serializer.serialize_seq(None)?;
        for _ in 0..=MAX_SHARED_METADATA_BYTES {
            self.entries.set(self.entries.get() + 1);
            sequence.serialize_element(&0_u8)?;
        }
        sequence.end()
    }
}

#[test]
fn key_capture_enforces_its_own_limit_when_serializer_output_changes() {
    let entries = Cell::new(0);
    let key = VaryingKey {
        calls: Cell::new(0),
        entries: &entries,
    };
    let admitted = MetadataDraft::try_new(key, "Example", "")
        .unwrap()
        .bind_schema(ValidSchema::empty());
    assert_matches!(
        admitted,
        Err(MetadataBuildError::Metadata(MetadataError::InvalidKey))
    );
    assert!(entries.get() > 0);
    assert!(
        entries.get() <= MAX_SHARED_METADATA_BYTES / 2 + 1,
        "the capture must stop at its own ceiling before exhausting the serializer"
    );
}
