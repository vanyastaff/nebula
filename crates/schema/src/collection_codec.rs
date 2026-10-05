//! Reviewed directional codecs for serde's string-key maps and sets.
//!
//! Set codecs describe the array accepted/emitted by serde. Rust element
//! equality does not imply JSON identity, and input deserialization deduplicates
//! repeated elements, so neither direction asserts JSON-level uniqueness.

use crate::{
    InputCodec, OutputCodec, PropertyType, SchemaTypeBuilder, SchemaTypeUse, ValidationReport,
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    hash::{BuildHasher, Hash},
};

impl<T: PropertyType, S: BuildHasher + 'static> PropertyType for HashMap<String, T, S> {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder.define::<Self>(|builder| {
            let value = T::define_schema_type(builder)?;
            Ok(json!({"kind": "record", "properties": [], "additional_properties": {"typed": value.to_json()}}))
        })
    }
}
impl<T: InputCodec, S: BuildHasher + Default + 'static> InputCodec for HashMap<String, T, S> {}
impl<T: OutputCodec, S: BuildHasher + 'static> OutputCodec for HashMap<String, T, S> {}

impl<T: PropertyType> PropertyType for BTreeMap<String, T> {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder.define::<Self>(|builder| {
            let value = T::define_schema_type(builder)?;
            Ok(json!({"kind": "record", "properties": [], "additional_properties": {"typed": value.to_json()}}))
        })
    }
}
impl<T: InputCodec> InputCodec for BTreeMap<String, T> {}
impl<T: OutputCodec> OutputCodec for BTreeMap<String, T> {}

impl<T: PropertyType + Eq + Hash, S: BuildHasher + 'static> PropertyType for HashSet<T, S> {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder.define::<Self>(|builder| {
            let element = T::define_schema_type(builder)?;
            Ok(json!({"kind": "array", "element": element.to_json(), "unique": false}))
        })
    }
}
impl<T: InputCodec + Eq + Hash, S: BuildHasher + Default + 'static> InputCodec for HashSet<T, S> {}
impl<T: OutputCodec + Eq + Hash, S: BuildHasher + 'static> OutputCodec for HashSet<T, S> {}

impl<T: PropertyType + Ord> PropertyType for BTreeSet<T> {
    fn define_schema_type(
        builder: &mut SchemaTypeBuilder,
    ) -> Result<SchemaTypeUse, ValidationReport> {
        builder.define::<Self>(|builder| {
            let element = T::define_schema_type(builder)?;
            Ok(json!({"kind": "array", "element": element.to_json(), "unique": false}))
        })
    }
}
impl<T: InputCodec + Ord> InputCodec for BTreeSet<T> {}
impl<T: OutputCodec + Ord> OutputCodec for BTreeSet<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_map_values_reject_wrong_shapes_and_numeric_overflow() {
        let input = crate::InputContract::for_type::<HashMap<String, Option<u8>>>().unwrap();
        let decoded: HashMap<String, Option<u8>> = input
            .validate_data(json!({"a": 7, "b": null}))
            .unwrap()
            .into_typed(&input)
            .unwrap();
        assert_eq!(decoded.get("a"), Some(&Some(7)));
        assert_eq!(decoded.get("b"), Some(&None));
        for data in [json!({"a": 256}), json!({"a": "wrong"}), json!([])] {
            assert!(input.validate_data(data).is_err());
        }
        let output = crate::OutputContract::for_type::<BTreeMap<String, Option<u8>>>().unwrap();
        output
            .validate_data(&serde_json::to_value(decoded).unwrap())
            .unwrap();
        assert!(output.validate_data(&json!({"a": 256})).is_err());
    }

    #[test]
    fn serde_set_input_accepts_duplicates_and_deduplicates_them() {
        let input = crate::InputContract::for_type::<HashSet<u8>>().unwrap();
        let decoded: HashSet<u8> = input
            .validate_data(json!([7, 7, 8]))
            .unwrap()
            .into_typed(&input)
            .unwrap();
        assert_eq!(decoded, HashSet::from([7, 8]));
        assert!(input.validate_data(json!([256])).is_err());
        let ordered = crate::InputContract::for_type::<BTreeSet<u8>>().unwrap();
        let decoded: BTreeSet<u8> = ordered
            .validate_data(json!([8, 7, 7]))
            .unwrap()
            .into_typed(&ordered)
            .unwrap();
        assert_eq!(decoded, BTreeSet::from([7, 8]));
    }

    #[crate::schema_type(both)]
    #[derive(PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct ProjectedElement {
        value: u8,
        #[serde(skip)]
        local_identity: u8,
    }

    #[test]
    fn serde_set_output_may_project_distinct_rust_elements_identically() {
        let values = HashSet::from([
            ProjectedElement {
                value: 7,
                local_identity: 1,
            },
            ProjectedElement {
                value: 7,
                local_identity: 2,
            },
        ]);
        assert_eq!(values.len(), 2);
        let data = serde_json::to_value(values).unwrap();
        assert_eq!(data, json!([{"value": 7}, {"value": 7}]));
        crate::OutputContract::for_type::<HashSet<ProjectedElement>>()
            .unwrap()
            .validate_data(&data)
            .unwrap();
        crate::OutputContract::for_type::<BTreeSet<ProjectedElement>>()
            .unwrap()
            .validate_data(&data)
            .unwrap();
    }
}
