//! `#[derive(Schema)]` on payload-carrying enums → tagged-union schemas.
//!
//! The C1 invariant (schema variant key == serde wire key) is checked against an
//! independent oracle: each enum derives `Serialize` too, and the test asserts the
//! key serde actually emits equals the key the schema declares — so a drift
//! between the derive's variant-key algorithm and serde_derive's fails the test.

use nebula_schema::{FieldValues, HasSchema, Schema, SchemaKind, SerdeTagging, schema_of};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ── helpers (read the union shape off the serialized schema wire) ─────────────

fn schema_wire<T: HasSchema>() -> Value {
    serde_json::to_value(schema_of::<T>()).expect("schema serializes")
}

fn variant_keys<T: HasSchema>() -> Vec<String> {
    schema_wire::<T>()["fields"][0]["variants"]
        .as_array()
        .expect("a union has one root mode field with variants")
        .iter()
        .map(|v| {
            v["key"]
                .as_str()
                .expect("variant key is a string")
                .to_owned()
        })
        .collect()
}

fn variant_payload_field_keys<T: HasSchema>(variant_key: &str) -> Vec<String> {
    let wire = schema_wire::<T>();
    let variants = wire["fields"][0]["variants"].as_array().expect("variants");
    let variant = variants
        .iter()
        .find(|v| v["key"] == json!(variant_key))
        .expect("variant present");
    variant["field"]["fields"]
        .as_array()
        .map(|fields| {
            fields
                .iter()
                .map(|f| f["key"].as_str().expect("field key").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

// ── External tagging (serde default) ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
struct OAuthCfg {
    client_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
enum ExternalAuth {
    OAuth(OAuthCfg),
    ApiKey { key: String },
    None,
}

#[test]
fn external_enum_is_a_union_with_verbatim_keys() {
    let schema = schema_of::<ExternalAuth>();
    assert_eq!(schema.kind(), SchemaKind::Union);
    assert_eq!(schema.serde_tagging(), Some(&SerdeTagging::External));
    // serde's default variant key is the ident VERBATIM (NOT snake_case — that was
    // the EnumSelect UI convention, wrong for serde fidelity).
    assert_eq!(variant_keys::<ExternalAuth>(), ["OAuth", "ApiKey", "None"]);
}

#[test]
fn external_wire_keys_match_schema_variants() {
    // Newtype variant → { "OAuth": { .. } }.
    let oauth = serde_json::to_value(ExternalAuth::OAuth(OAuthCfg {
        client_id: "x".to_owned(),
    }))
    .unwrap();
    assert_eq!(oauth.as_object().unwrap().keys().next().unwrap(), "OAuth");

    // Struct variant → { "ApiKey": { "key": .. } }.
    let api = serde_json::to_value(ExternalAuth::ApiKey {
        key: "k".to_owned(),
    })
    .unwrap();
    assert_eq!(api.as_object().unwrap().keys().next().unwrap(), "ApiKey");

    // Unit variant → the bare string "None" (NOT { "None": {} }).
    let none = serde_json::to_value(ExternalAuth::None).unwrap();
    assert_eq!(none, json!("None"));

    // Every serde wire key is a declared schema variant.
    let keys = variant_keys::<ExternalAuth>();
    for expected in ["OAuth", "ApiKey", "None"] {
        assert!(keys.iter().any(|k| k == expected), "missing {expected}");
    }
}

#[test]
fn newtype_payload_fields_come_from_the_inner_struct() {
    // External `{ "OAuth": { "client_id": .. } }` — the payload schema is OAuthCfg.
    assert_eq!(
        variant_payload_field_keys::<ExternalAuth>("OAuth"),
        ["client_id"]
    );
    let oauth = serde_json::to_value(ExternalAuth::OAuth(OAuthCfg {
        client_id: "x".to_owned(),
    }))
    .unwrap();
    assert!(oauth["OAuth"].get("client_id").is_some());
}

// ── Container rename_all uses serde's EXACT variant algorithm ─────────────────

#[derive(Serialize, Schema)]
#[serde(rename_all = "snake_case")]
enum Renamed {
    HTTPProxy(OAuthCfg),
    PlainText,
}

#[test]
fn rename_all_matches_serde_variant_algorithm_exactly() {
    // serde's snake_case for a VARIANT inserts `_` before every uppercase letter
    // (no acronym grouping): HTTPProxy → h_t_t_p_proxy, not http_proxy.
    assert_eq!(variant_keys::<Renamed>(), ["h_t_t_p_proxy", "plain_text"]);

    // Oracle: serde emits the identical key.
    let wire = serde_json::to_value(Renamed::HTTPProxy(OAuthCfg {
        client_id: "x".to_owned(),
    }))
    .unwrap();
    assert_eq!(
        wire.as_object().unwrap().keys().next().unwrap(),
        "h_t_t_p_proxy"
    );
    assert_eq!(
        serde_json::to_value(Renamed::PlainText).unwrap(),
        json!("plain_text")
    );
}

// ── Per-variant #[serde(rename)] ─────────────────────────────────────────────

#[derive(Serialize, Schema)]
enum WithRename {
    #[serde(rename = "v2")]
    Version2 { n: i64 },
}

#[test]
fn variant_rename_wins() {
    assert_eq!(variant_keys::<WithRename>(), ["v2"]);
    let wire = serde_json::to_value(WithRename::Version2 { n: 1 }).unwrap();
    assert_eq!(wire.as_object().unwrap().keys().next().unwrap(), "v2");
}

// ── Struct-variant field keys follow the VARIANT's rename_all ─────────────────

#[derive(Serialize, Schema)]
enum Mixed {
    #[serde(rename_all = "camelCase")]
    Create { user_name: String, is_admin: bool },
}

#[test]
fn struct_variant_field_keys_follow_variant_rename_all() {
    assert_eq!(
        variant_payload_field_keys::<Mixed>("Create"),
        ["userName", "isAdmin"]
    );
    // Oracle: serde renames the variant's fields the same way.
    let wire = serde_json::to_value(Mixed::Create {
        user_name: "a".to_owned(),
        is_admin: true,
    })
    .unwrap();
    let payload = &wire["Create"];
    assert!(payload.get("userName").is_some());
    assert!(payload.get("isAdmin").is_some());
}

// ── Adjacent tagging records the SerdeTagging ────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Schema)]
#[serde(tag = "type", content = "data")]
enum Event {
    Click { x: i64 },
    Noop,
}

#[test]
fn adjacent_enum_records_tagging() {
    let schema = schema_of::<Event>();
    assert_eq!(schema.kind(), SchemaKind::Union);
    assert_eq!(
        schema.serde_tagging(),
        Some(&SerdeTagging::Adjacent {
            tag: "type".to_owned(),
            content: "data".to_owned(),
        })
    );
    assert_eq!(variant_keys::<Event>(), ["Click", "Noop"]);

    // Oracle: serde adjacent wire is { "type": "Click", "data": { .. } }.
    let wire = serde_json::to_value(Event::Click { x: 1 }).unwrap();
    assert_eq!(wire["type"], json!("Click"));
    assert!(wire["data"].get("x").is_some());
    // Unit variant omits content: { "type": "Noop" }.
    let noop = serde_json::to_value(Event::Noop).unwrap();
    assert_eq!(noop["type"], json!("Noop"));
    assert!(noop.get("data").is_none());
}

// ── #[serde(skip)] drops a variant from the union ────────────────────────────

#[derive(Serialize, Schema)]
enum WithSkip {
    Kept {
        n: i64,
    },
    #[serde(skip)]
    #[allow(
        dead_code,
        reason = "exercises that #[serde(skip)] drops the union arm"
    )]
    Hidden,
}

#[test]
fn serde_skip_drops_the_variant() {
    assert_eq!(variant_keys::<WithSkip>(), ["Kept"]);
    // The kept variant still round-trips through serde under its wire key.
    let wire = serde_json::to_value(WithSkip::Kept { n: 1 }).unwrap();
    assert_eq!(wire.as_object().unwrap().keys().next().unwrap(), "Kept");
}

// ── The union schema is a DESERIALIZATION contract ───────────────────────────

#[derive(Serialize, Schema)]
enum WithSkipDeser {
    Kept {
        n: i64,
    },
    #[serde(skip_deserializing)]
    SerOnly {
        m: i64,
    },
}

#[test]
fn skip_deserializing_variant_is_dropped_but_serde_still_serializes_it() {
    // A `#[serde(skip_deserializing)]` variant is never produced by the
    // deserializer, so it is dropped from the union schema (the schema is a
    // deserialization contract — see `SchemaKind::Union`)...
    assert_eq!(variant_keys::<WithSkipDeser>(), ["Kept"]);
    let kept = serde_json::to_value(WithSkipDeser::Kept { n: 1 }).unwrap();
    assert_eq!(kept.as_object().unwrap().keys().next().unwrap(), "Kept");
    // ...even though serde STILL serializes the skipped variant. The schema is
    // deliberately not an output/producer contract; this asymmetry is the
    // documented scope.
    let ser_only = serde_json::to_value(WithSkipDeser::SerOnly { m: 1 }).unwrap();
    assert_eq!(
        ser_only.as_object().unwrap().keys().next().unwrap(),
        "SerOnly"
    );
}

// ── Newtype payload must be a Record (fail-loud on union / Any) ───────────────

#[derive(Serialize, Schema)]
#[expect(
    dead_code,
    reason = "constructed only via schema_of, which panics by design"
)]
enum InnerUnion {
    A { x: i64 },
    B,
}

#[derive(Serialize, Schema)]
#[expect(
    dead_code,
    reason = "constructed only via schema_of, which panics by design"
)]
enum OuterWrapsEnum {
    Wrap(InnerUnion),
}

/// A newtype variant over another enum (a union payload) would splice the union's
/// synthetic root key — a key serde never emits. The runtime guard rejects it at
/// `schema()` init rather than producing a C1-broken schema.
#[test]
#[should_panic(expected = "not a record")]
fn newtype_over_enum_panics_at_schema_init() {
    let _ = schema_of::<OuterWrapsEnum>();
}

#[derive(Serialize, Schema)]
#[expect(
    dead_code,
    reason = "constructed only via schema_of, which panics by design"
)]
enum OuterWrapsAny {
    Raw(Value),
}

/// A newtype variant over `serde_json::Value` (the gradual `Any`, zero fields)
/// would become a closed empty object that rejects every payload — the inverse of
/// `Any`. The guard rejects it at `schema()` init.
#[test]
#[should_panic(expected = "not a record")]
fn newtype_over_any_panics_at_schema_init() {
    let _ = schema_of::<OuterWrapsAny>();
}

// ── Value-layer ingress/egress: serde wire ⇄ {mode,value} envelope ────────────
//
// The C1 value-layer bridge. `serde_json::to_value(value)` is the independent
// oracle for the wire shape; `ValidSchema::values_from_wire` must accept exactly
// what `#[derive(Serialize)]` emits, `validate()` must then pass, and
// `FieldValues::to_typed` must reconstruct the original value — proving the
// ingress/egress pair is faithful to serde for every variant shape.

/// Full round-trip oracle: value → serde wire → ingress → validate → to_typed.
fn assert_union_wire_roundtrips<T>(value: T)
where
    T: HasSchema + Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug + Clone,
{
    let schema = schema_of::<T>();
    assert_eq!(schema.kind(), SchemaKind::Union, "fixture must be a union");

    let wire = serde_json::to_value(&value).expect("value serializes");
    let values = schema
        .values_from_wire(wire)
        .expect("serde wire ingests into the union envelope");
    let valid = schema
        .validate(&values)
        .expect("ingested union envelope validates");
    let back: T = valid
        .raw()
        .to_typed::<T>()
        .expect("validated union deserializes back to the enum");
    assert_eq!(
        back, value,
        "wire → ingress → validate → to_typed must round-trip the value"
    );
}

#[test]
fn external_data_variant_roundtrips() {
    assert_union_wire_roundtrips(ExternalAuth::OAuth(OAuthCfg {
        client_id: "abc".to_owned(),
    }));
}

#[test]
fn external_struct_variant_roundtrips() {
    assert_union_wire_roundtrips(ExternalAuth::ApiKey {
        key: "k".to_owned(),
    });
}

#[test]
fn external_unit_variant_roundtrips() {
    // serde emits the bare string "None"; ingress must accept it and to_typed
    // must reconstruct it (not `{"None": {}}`).
    assert_union_wire_roundtrips(ExternalAuth::None);
}

#[test]
fn adjacent_data_variant_roundtrips() {
    assert_union_wire_roundtrips(Event::Click { x: 7 });
}

#[test]
fn adjacent_unit_variant_roundtrips() {
    // serde adjacent unit omits the content key entirely.
    assert_union_wire_roundtrips(Event::Noop);
}

#[test]
fn unknown_discriminant_is_rejected() {
    let schema = schema_of::<ExternalAuth>();
    let err = schema
        .values_from_wire(json!({ "Nope": { "client_id": "x" } }))
        .expect_err("a non-variant discriminant must be rejected at ingress");
    assert_eq!(err.code, "union.unknown_variant");
}

#[test]
fn wrong_typed_payload_fails_validation() {
    // The external shape is well-formed (data variant `OAuth`), so ingress
    // succeeds; the payload type error (`client_id` is not a string) surfaces in
    // `validate`, exactly where a normal record's type errors do.
    let schema = schema_of::<ExternalAuth>();
    let values = schema
        .values_from_wire(json!({ "OAuth": { "client_id": 123 } }))
        .expect("a well-shaped data variant ingests");
    let report = schema
        .validate(&values)
        .expect_err("a wrong-typed payload must fail validation");
    assert!(
        report.errors().any(|e| e.code == "type_mismatch"),
        "expected a type_mismatch, got: {:?}",
        report.errors().map(|e| e.code.as_ref()).collect::<Vec<_>>()
    );
}

#[test]
fn malformed_external_wire_is_rejected() {
    // An external union value must be a string (unit) or single-key object (data);
    // a bare scalar is neither.
    let schema = schema_of::<ExternalAuth>();
    let err = schema
        .values_from_wire(json!(42))
        .expect_err("a bare scalar is not a valid external union wire");
    assert_eq!(err.code, "union.malformed_wire");
}

#[test]
fn external_unit_variant_with_payload_is_rejected() {
    // `None` is a unit variant; an object data-shape for it is malformed.
    let schema = schema_of::<ExternalAuth>();
    let err = schema
        .values_from_wire(json!({ "None": {} }))
        .expect_err("a data shape for a unit variant is malformed");
    assert_eq!(err.code, "union.malformed_wire");
}

#[test]
fn adjacent_wire_missing_tag_is_rejected() {
    let schema = schema_of::<Event>();
    let err = schema
        .values_from_wire(json!({ "data": { "x": 1 } }))
        .expect_err("adjacent wire without the tag key is malformed");
    assert_eq!(err.code, "union.malformed_wire");
}

#[test]
fn record_ingress_matches_from_json_and_to_typed_round_trips() {
    // `values_from_wire` is behavior-identical to `FieldValues::from_json` for a
    // record schema, so a caller can swap one for the other unconditionally; and
    // `to_typed` round-trips a record too (the egress no-ops to `to_json`).
    let schema = schema_of::<OAuthCfg>();
    assert_eq!(schema.kind(), SchemaKind::Record);

    let wire = json!({ "client_id": "abc" });
    let via_wire = schema
        .values_from_wire(wire.clone())
        .expect("record ingests");
    let via_from_json = FieldValues::from_json(wire).expect("from_json");
    assert_eq!(
        via_wire, via_from_json,
        "record ingress must equal FieldValues::from_json"
    );

    let valid = schema.validate(&via_wire).expect("record validates");
    let back: OAuthCfg = valid.raw().to_typed().expect("record round-trips");
    assert_eq!(
        back,
        OAuthCfg {
            client_id: "abc".to_owned()
        }
    );
}
