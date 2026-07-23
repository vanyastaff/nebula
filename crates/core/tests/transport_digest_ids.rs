use std::{fmt::Debug, str::FromStr};

use nebula_core::{
    ArtifactSetDigest, PluginSetId, TransportDigestParseError, WorkerFlavorRevisionId,
};
use serde::{Serialize, de::DeserializeOwned};

fn assert_wire_round_trip<T>(value: T, expected: &str)
where
    T: Copy + Debug + Eq + ToString + FromStr<Err = TransportDigestParseError> + Serialize,
    T: DeserializeOwned,
{
    assert_eq!(value.to_string(), expected);
    assert_eq!(expected.parse::<T>(), Ok(value));

    let encoded = serde_json::to_string(&value).expect("transport digest must serialize");
    assert_eq!(encoded, format!("\"{expected}\""));

    let decoded =
        serde_json::from_str::<T>(&encoded).expect("canonical transport digest must deserialize");
    assert_eq!(decoded, value);
}

#[test]
fn every_transport_digest_uses_the_same_canonical_wire_contract() {
    let bytes = [0xabu8; 32];
    let expected = "ab".repeat(32);

    let plugin_set = PluginSetId::from_bytes(bytes);
    let worker_flavor = WorkerFlavorRevisionId::from_bytes(bytes);
    let artifact_set = ArtifactSetDigest::from_bytes(bytes);

    assert_eq!(plugin_set.as_bytes(), &bytes);
    assert_eq!(worker_flavor.as_bytes(), &bytes);
    assert_eq!(artifact_set.as_bytes(), &bytes);

    assert_wire_round_trip(plugin_set, &expected);
    assert_wire_round_trip(worker_flavor, &expected);
    assert_wire_round_trip(artifact_set, &expected);

    assert_eq!(
        format!("{plugin_set:?}"),
        format!("PluginSetId({expected})")
    );
}

#[test]
fn parser_rejects_non_canonical_hex_with_precise_errors() {
    let uppercase = "AB".repeat(32);
    assert!(matches!(
        uppercase.parse::<PluginSetId>(),
        Err(TransportDigestParseError::InvalidHex { index: 0 })
    ));

    let too_short = "ab".repeat(31);
    assert!(matches!(
        too_short.parse::<PluginSetId>(),
        Err(TransportDigestParseError::InvalidLength { actual: 62 })
    ));

    let too_long = format!("{}00", "ab".repeat(32));
    assert!(matches!(
        too_long.parse::<PluginSetId>(),
        Err(TransportDigestParseError::InvalidLength { actual: 66 })
    ));

    let invalid = format!("{}g{}", "a".repeat(17), "a".repeat(46));
    assert!(matches!(
        invalid.parse::<PluginSetId>(),
        Err(TransportDigestParseError::InvalidHex { index: 17 })
    ));
}

#[test]
fn serde_rejects_uppercase_invalid_length_and_non_string_values() {
    for invalid in ["AB".repeat(32), "ab".repeat(31), "ab".repeat(33)] {
        let encoded = serde_json::to_string(&invalid).expect("test input must serialize");
        assert!(serde_json::from_str::<WorkerFlavorRevisionId>(&encoded).is_err());
    }

    for invalid in [
        serde_json::json!(42),
        serde_json::json!([0, 0]),
        serde_json::json!({"digest": "00".repeat(32)}),
    ] {
        assert!(serde_json::from_value::<ArtifactSetDigest>(invalid).is_err());
    }
}
