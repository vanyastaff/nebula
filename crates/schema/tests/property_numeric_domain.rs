//! Primitive domains survive field and list-item derivation.

use nebula_schema::{AuthoredValue, HasSchema, Schema};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Schema, Deserialize)]
struct Byte {
    value: u8,
}

#[derive(Schema, Deserialize)]
struct SignedByte {
    value: i8,
}

#[derive(Schema, Deserialize)]
struct WideUnsigned {
    value: u64,
}

#[derive(Schema, Deserialize)]
struct ByteList {
    values: Option<Vec<u8>>,
}

#[derive(Schema, Deserialize)]
struct SignedList {
    values: Option<Vec<i8>>,
}

#[derive(Schema, Deserialize)]
struct WideList {
    values: Option<Vec<u64>>,
}

#[derive(Schema, Deserialize)]
struct BoundedByte {
    #[property(validate(range(1..=300)))]
    value: u8,
}

#[derive(Schema, Deserialize)]
struct Float {
    value: f32,
    values: Option<Vec<f32>>,
}

fn resolve<T: HasSchema>(
    value: Value,
) -> Result<nebula_schema::ResolvedValues, nebula_schema::ValidationReport> {
    T::schema()?
        .validate(AuthoredValue::from_data(value).expect("literal input"))?
        .resolve_data()
}

fn rejects<T: HasSchema>(value: Value, path: &str) {
    let errors = resolve::<T>(value).expect_err("outside the Rust numeric domain");
    assert!(errors.errors().any(|error| error.path().to_string() == path && ["min", "max"].contains(&error.code())), "{errors:?}");
}

#[test]
fn byte_field_rejects_negative_and_overflow_before_decode() {
    rejects::<Byte>(json!({"value": -1}), "/value");
    rejects::<Byte>(json!({"value": 256}), "/value");
    for value in [u8::MIN, u8::MAX] {
        let decoded = resolve::<Byte>(json!({"value": value}))
            .expect("boundary")
            .into_typed::<Byte>()
            .expect("decode");
        assert_eq!(decoded.value, value);
    }
}

#[test]
fn signed_byte_field_preserves_both_extrema() {
    rejects::<SignedByte>(json!({"value": -129}), "/value");
    rejects::<SignedByte>(json!({"value": 128}), "/value");
    for value in [i8::MIN, i8::MAX] {
        let decoded = resolve::<SignedByte>(json!({"value": value}))
            .expect("boundary")
            .into_typed::<SignedByte>()
            .expect("decode");
        assert_eq!(decoded.value, value);
    }
}

#[test]
fn unsigned_64_bit_maximum_is_not_narrowed() {
    rejects::<WideUnsigned>(json!({"value": -1}), "/value");
    let decoded = resolve::<WideUnsigned>(json!({"value": u64::MAX}))
        .expect("exact maximum")
        .into_typed::<WideUnsigned>()
        .expect("decode");
    assert_eq!(decoded.value, u64::MAX);
}

#[test]
fn list_items_retain_domains_and_optional_empty_lists_remain_valid() {
    rejects::<ByteList>(json!({"values": [-1]}), "/values/0");
    rejects::<ByteList>(json!({"values": [256]}), "/values/0");
    rejects::<SignedList>(json!({"values": [-129]}), "/values/0");
    rejects::<SignedList>(json!({"values": [128]}), "/values/0");
    rejects::<WideList>(json!({"values": [-1]}), "/values/0");
    let empty = resolve::<ByteList>(json!({"values": []}))
        .expect("empty list")
        .into_typed::<ByteList>()
        .expect("decode");
    assert_eq!(empty.values, Some(vec![]));
    let signed = resolve::<SignedList>(json!({"values": [i8::MIN, i8::MAX]}))
        .expect("signed boundaries")
        .into_typed::<SignedList>()
        .expect("decode");
    assert_eq!(signed.values, Some(vec![i8::MIN, i8::MAX]));
    let wide = resolve::<WideList>(json!({"values": [0, u64::MAX]}))
        .expect("unsigned boundaries")
        .into_typed::<WideList>()
        .expect("decode");
    assert_eq!(wide.values, Some(vec![0, u64::MAX]));
}

#[test]
fn authored_range_and_primitive_bounds_are_conjunctive() {
    rejects::<BoundedByte>(json!({"value": 0}), "/value");
    rejects::<BoundedByte>(json!({"value": 256}), "/value");
    for value in [1, u8::MAX] {
        let decoded = resolve::<BoundedByte>(json!({"value": value}))
            .expect("intersection")
            .into_typed::<BoundedByte>()
            .expect("decode");
        assert_eq!(decoded.value, value);
    }
}

#[test]
fn float32_fields_and_items_reject_finite_json_that_would_decode_as_infinity() {
    rejects::<Float>(json!({"value": 1e100}), "/value");
    rejects::<Float>(json!({"value": -1e100}), "/value");
    rejects::<Float>(json!({"value": 0, "values": [1e100]}), "/values/0");
    let decoded = resolve::<Float>(json!({"value": f32::MAX, "values": [f32::MIN]}))
        .expect("finite boundaries")
        .into_typed::<Float>()
        .expect("decode");
    assert_eq!(decoded.value, f32::MAX);
    assert_eq!(decoded.values, Some(vec![f32::MIN]));
}
