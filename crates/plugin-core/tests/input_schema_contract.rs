//! Typed discovery and the admitted subset of the core actions' serde wire format.

use std::sync::OnceLock;

use nebula_action::{
    Action, ActionContext, ActionError, ActionFactory, ActionMetadataDraft, ActionResult,
    InstanceFactory, StatelessAction,
};
use nebula_core::{ActionKey, Dependencies, action_key};
use nebula_plugin::ResolvedPlugin;
use nebula_plugin_core::{CorePlugin, actions::*};
use nebula_schema::{
    AuthoredValue, HasSchema, ResolvedValues, SchemaKind, ValidSchema, ValidationReport,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Deserialize, nebula_schema::Schema)]
struct LegacyInput {
    _legacy: Option<String>,
}

struct LegacySchemaAction;

impl Action for LegacySchemaAction {
    type Input = LegacyInput;
    type Output = Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("legacy.schema"),
            nebula_action::metadata_name!("Legacy schema"),
            "Legacy schema compatibility fixture",
        )
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for LegacySchemaAction {
    async fn execute(
        &self,
        _input: LegacyInput,
        _context: &(impl ActionContext + ?Sized),
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(Value::Null))
    }
}

fn check_data(schema: &ValidSchema, wire: Value) -> Result<Value, ValidationReport> {
    schema
        .validate(AuthoredValue::from_data(wire).unwrap())?
        .resolve_data()
        .map(ResolvedValues::into_json)
}

fn assert_wire<T: HasSchema + Serialize + DeserializeOwned>(wire: Value) {
    let direct: T = serde_json::from_value(wire.clone()).unwrap();
    let expected = serde_json::to_value(direct).unwrap();
    let schema = T::schema().unwrap();
    let resolved = schema
        .validate(AuthoredValue::from_data(wire.clone()).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(
        resolved.values().to_json(),
        wire,
        "schema changed literal wire data"
    );
    let decoded: T = resolved.into_typed().unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
}

macro_rules! input_contract {
    ($module:ident, $input:ty, $action:literal,
     fields: [$($field:literal),+ $(,)?], wire: $wire:expr,
     missing: [$($missing:literal),* $(,)?], invalid: [$(($path:literal, $invalid:expr)),+ $(,)?]) => {
        mod $module {
            use super::*;

            #[test]
            fn discovery_keeps_known_fields_and_caches_the_checked_schema() {
                let schema = <$input>::schema().unwrap();
                assert_eq!(schema.kind(), SchemaKind::Record);
                assert_eq!(
                    schema.fields().iter().map(|field| field.key().as_str()).collect::<Vec<_>>(),
                    vec![$($field),+],
                );
                assert!(schema.ptr_eq(&<$input>::schema().unwrap()));
                let plugin = ResolvedPlugin::from(CorePlugin::try_new().unwrap()).unwrap();
                let metadata = plugin.action(&ActionKey::new($action).unwrap()).unwrap()
                    .metadata();
                assert_eq!(metadata.base().schema(), &schema);
                assert_eq!(metadata.output_schema().kind(), SchemaKind::Any);
            }

            #[test]
            fn changed_schema_advances_its_catalog_interface_major() {
                let plugin = ResolvedPlugin::from(CorePlugin::try_new().unwrap()).unwrap();
                let current = plugin.action(&ActionKey::new($action).unwrap()).unwrap()
                    .metadata();
                assert_eq!(current.base().version().major, 2);
                assert_eq!(current.base().version().minor, 0);
                let legacy_factory = InstanceFactory::new(
                    ActionMetadataDraft::new(
                        ActionKey::new($action).unwrap(),
                        nebula_action::metadata_name!("Legacy action"),
                        "Legacy schema compatibility fixture",
                    )
                    .with_version(nebula_action::MetadataVersion::new(1, 0, 0))
                    .with_effect_contract(
                        nebula_action::effect::ActionEffectContract::NoExternalEffects,
                    ),
                    LegacySchemaAction,
                )
                .unwrap();
                let legacy = legacy_factory.metadata();
                assert_ne!(current.base().schema(), legacy.base().schema());
                assert_eq!(nebula_metadata::validate_base_compat(current.base(), legacy.base()), Ok(()));
            }

            #[test]
            fn required_outer_fields_reject_absence_without_inventing_defaults() {
                let schema = <$input>::schema().unwrap();
                let required: &[&str] = &[$($missing),*];
                if required.is_empty() {
                    assert_wire::<$input>(json!({}));
                }
                for field in required {
                    let mut wire = $wire;
                    wire.as_object_mut().unwrap().remove(*field).unwrap();
                    let error = check_data(&schema, wire).unwrap_err();
                    assert!(error.has_errors(), "missing {field} did not produce a report");
                }
            }

            #[test]
            fn declared_types_reject_wrong_wire_values() {
                let schema = <$input>::schema().unwrap();
                $(
                    let mut wire = $wire;
                    *wire.pointer_mut($path).unwrap() = $invalid;
                    let error = check_data(&schema, wire).unwrap_err();
                    assert!(error.errors().any(|error| error.path().to_string() == $path),
                        "missing diagnostic at {}: {error:?}", $path);
                )+
            }

            #[test]
            fn serde_wire_survives_validation_and_owned_typed_decode() {
                assert_wire::<$input>($wire);
            }
        }
    };
}

#[test]
fn core_bundle_announces_the_changed_action_contracts() {
    let plugin = ResolvedPlugin::from(CorePlugin::try_new().unwrap()).unwrap();
    assert_eq!(plugin.version().major, 2);
    assert_eq!(plugin.version().minor, 0);
}

input_contract!(aggregate_input, aggregate::AggregateInput, "core.aggregate",
    fields: ["data", "group_by", "aggregations", "on_error"],
    wire: json!({"data": [{"n": 2}], "group_by": [],
        "aggregations": [{"fn": "sum", "field": "n", "out": "total"}], "on_error": "fail"}),
    missing: ["data", "aggregations"],
    invalid: [("/data", json!({})), ("/data/0", json!(2)),
        ("/group_by", json!(null)), ("/aggregations", json!(false)),
        ("/aggregations/0/fn", json!("unknown")), ("/aggregations/0/out", json!(12)),
        ("/on_error", json!("unknown"))]
);

input_contract!(array_input, array::ArrayInput, "core.array",
    fields: ["data", "operations"],
    wire: json!({"data": [1, null, {"arbitrary/key": [true]}],
        "operations": [{"op": "chunk", "size": 2}]}),
    missing: ["data"],
    invalid: [("/data", json!({})), ("/operations", json!(null)),
        ("/operations/0", json!("chunk")), ("/operations/0/op", json!("unknown")),
        ("/operations/0/size", json!(-1))]
);

input_contract!(datetime_input, datetime::DateTimeInput, "core.datetime",
    fields: ["data", "op", "input", "format", "tz_offset_seconds", "amount", "unit", "from", "to"],
    wire: json!({"data": null, "op": "add", "input": "2026-06-19T00:00:00Z",
        "amount": 1, "unit": "milliseconds"}),
    missing: ["op", "input", "amount", "unit"],
    invalid: [("/op", json!("unknown")), ("/input", json!(3)),
        ("/amount", json!(1.5)), ("/unit", json!("months"))]
);

input_contract!(dedupe_input, dedupe::DedupeInput, "core.dedupe",
    fields: ["data", "keys"],
    wire: json!({"data": [{"id": 1}], "keys": ["id"]}),
    missing: ["data", "keys"],
    invalid: [("/data", json!(null)), ("/data/0", json!([])),
        ("/keys", json!("id")), ("/keys/0", json!(12))]
);

input_contract!(filter_input, filter::FilterInput, "core.filter",
    fields: ["data", "condition"],
    wire: json!({"data": [{"x": 1}], "condition": {"field": "x", "op": "exists"}}),
    missing: ["data", "condition"],
    invalid: [("/data", json!("wrong")), ("/data/0", json!(true)),
        ("/condition", json!([]))]
);

input_contract!(if_input, if_action::IfInput, "core.if",
    fields: ["data", "condition"],
    wire: json!({"data": null, "condition": {"all": []}}),
    missing: ["condition"],
    invalid: [("/condition", json!(null))]
);

input_contract!(json_transform_input, json_transform::JsonTransformInput, "core.json_transform",
    fields: ["data", "operations"],
    wire: json!({"data": {"a": 1}, "operations": [{"op": "pick", "fields": ["a"]}]}),
    missing: [],
    invalid: [("/operations", json!({})), ("/operations/0/op", json!("unknown")),
        ("/operations/0/fields", json!(null)), ("/operations/0/fields/0", json!(false))]
);

input_contract!(map_input, map::MapInput, "core.map",
    fields: ["data", "operations"],
    wire: json!({"data": [{"a": 1}], "operations": [{"op": "rename", "from": "a", "to": "b"}]}),
    missing: ["data", "operations"],
    invalid: [("/data", json!(true)), ("/data/0", json!(null)),
        ("/operations", json!(null)), ("/operations/0/from", json!([]))]
);

input_contract!(set_fields_input, set_fields::SetFieldsInput, "core.set_fields",
    fields: ["data", "assignments"],
    wire: json!({"data": null, "assignments": [{"name": "", "value": null},
        {"name": "literal", "value": {"$expr": "{{ untrusted }}", "": [false, 1]}}]}),
    missing: [],
    invalid: [("/assignments", json!(null)), ("/assignments/0", json!(5)),
        ("/assignments/0/name", json!(true))]
);

input_contract!(sort_input, sort::SortInput, "core.sort",
    fields: ["data", "keys"],
    wire: json!({"data": [{"": 1}], "keys": [{"field": "", "order": "desc",
        "nulls": "first", "case_insensitive": false}]}),
    missing: ["data", "keys"],
    invalid: [("/data", json!({})), ("/data/0", json!(2)), ("/keys/0/field", json!(3)),
        ("/keys/0/order", json!("unknown")), ("/keys/0/nulls", json!(null)),
        ("/keys/0/case_insensitive", json!("false"))]
);

input_contract!(switch_input, switch_action::SwitchInput, "core.switch",
    fields: ["data", "cases"],
    wire: json!({"data": null, "cases": [{"condition": {"any": []}, "port": "branch"}]}),
    missing: [],
    invalid: [("/cases", json!(null)), ("/cases/0", json!(true)),
        ("/cases/0/condition", json!("wrong")), ("/cases/0/port", json!(3))]
);

#[test]
fn required_data_arrays_keep_empty_array_semantics() {
    assert_wire::<aggregate::AggregateInput>(
        json!({"data": [], "aggregations": [{"fn": "count", "out": ""}]}),
    );
    assert_wire::<array::ArrayInput>(json!({"data": [], "operations": []}));
    assert_wire::<dedupe::DedupeInput>(json!({"data": [], "keys": [""]}));
    assert_wire::<filter::FilterInput>(json!({"data": [], "condition": {"all": []}}));
    assert_wire::<map::MapInput>(json!({"data": [], "operations": []}));
    assert_wire::<sort::SortInput>(json!({"data": [], "keys": [{"field": ""}]}));
}

#[test]
fn tagged_operation_variants_and_defaults_keep_their_serde_wire_shape() {
    for operation in [
        json!({"op": "chunk", "size": 1}),
        json!({"op": "flatten"}),
        json!({"op": "flatten", "depth": 0}),
        json!({"op": "take", "count": 0}),
        json!({"op": "skip", "count": usize::MAX}),
    ] {
        assert_wire::<array::ArrayInput>(json!({"data": [], "operations": [operation]}));
    }
    for operation in [
        json!({"op": "pick", "fields": []}),
        json!({"op": "omit", "fields": [""]}),
        json!({"op": "rename", "from": "", "to": ""}),
        json!({"op": "flatten"}),
        json!({"op": "flatten", "separator": ""}),
    ] {
        assert_wire::<json_transform::JsonTransformInput>(
            json!({"data": null, "operations": [operation.clone()]}),
        );
        assert_wire::<map::MapInput>(json!({"data": [], "operations": [operation]}));
    }
    for function in [
        "count",
        "count_distinct",
        "sum",
        "avg",
        "min",
        "max",
        "collect",
        "join",
    ] {
        assert_wire::<aggregate::AggregateInput>(json!({"data": [], "group_by": [],
            "aggregations": [{"fn": function, "field": "", "out": ""}]}));
    }
}

#[test]
fn datetime_optional_nulls_and_flattened_tags_survive_typed_decode() {
    let time = "2026-06-19T00:00:00.250Z";
    for wire in [
        json!({"op": "format", "input": time, "format": "", "tz_offset_seconds": null}),
        json!({"op": "format", "input": time, "format": "%S", "tz_offset_seconds": 19800}),
        json!({"op": "parse", "input": time}),
        json!({"op": "parse", "input": time, "format": null}),
        json!({"op": "add", "input": time, "amount": 0, "unit": "weeks"}),
        json!({"op": "subtract", "input": time, "amount": i64::MAX, "unit": "milliseconds"}),
        json!({"op": "diff", "from": time, "to": time, "unit": "days"}),
    ] {
        assert_wire::<datetime::DateTimeInput>(wire);
    }
}

#[test]
fn recursive_conditions_remain_data_and_decode_through_the_real_condition_visitor() {
    let condition = json!({"all": [
        {"any": [{"field": "", "op": "eq", "value": {"$expr": "{{ literal }}"}}]},
        {"not": {"field": "/~", "op": "exists"}}
    ]});
    assert_wire::<if_action::IfInput>(json!({"condition": condition}));
    assert_wire::<filter::FilterInput>(json!({"data": [], "condition": condition}));
    assert_wire::<switch_action::SwitchInput>(
        json!({"cases": [{"condition": condition, "port": "next"}]}),
    );
}

#[test]
fn opaque_condition_contents_are_not_advertised_as_a_full_serde_proof() {
    let wire = json!({"condition": {"all": [12]}});
    let resolved = if_action::IfInput::schema()
        .unwrap()
        .validate(AuthoredValue::from_data(wire).unwrap())
        .unwrap()
        .resolve_data()
        .unwrap();
    assert_eq!(
        resolved.values().to_json(),
        json!({"condition": {"all": [12]}})
    );
    assert_eq!(
        resolved
            .into_typed::<if_action::IfInput>()
            .unwrap_err()
            .code(),
        "type_mismatch"
    );
}

#[test]
fn nested_missing_fields_remain_the_serde_decoders_responsibility() {
    for wire in [
        json!({"assignments": [{"value": null}]}),
        json!({"assignments": [{"name": ""}]}),
    ] {
        let resolved = set_fields::SetFieldsInput::schema()
            .unwrap()
            .validate(AuthoredValue::from_data(wire).unwrap())
            .unwrap()
            .resolve_data()
            .unwrap();
        assert_eq!(
            resolved
                .into_typed::<set_fields::SetFieldsInput>()
                .unwrap_err()
                .code(),
            "type_mismatch"
        );
    }
}

#[test]
fn nullable_object_data_is_deferred_to_the_actions_existing_shape_check() {
    // Value accepts every JSON kind; the schema cannot express object-or-null.
    for wire in [
        json!({}),
        json!({"data": null}),
        json!({"data": {"": []}}),
        json!({"data": 3}),
    ] {
        assert_wire::<set_fields::SetFieldsInput>(wire);
    }
}

#[test]
fn integer_wire_representations_still_require_the_concrete_serde_decoder() {
    for wire in [
        json!({"data": [], "operations": [{"op": "take", "count": 1.0}]}),
        json!({"data": [], "operations": [{"op": "take"}]}),
    ] {
        let resolved = array::ArrayInput::schema()
            .unwrap()
            .validate(AuthoredValue::from_data(wire).unwrap())
            .unwrap()
            .resolve_data()
            .unwrap();
        assert_eq!(
            resolved
                .into_typed::<array::ArrayInput>()
                .unwrap_err()
                .code(),
            "type_mismatch"
        );
    }
}

#[test]
fn declared_members_are_checked_even_when_serde_would_ignore_them_for_the_variant() {
    let wire = json!({"data": [], "operations": [{"op": "take", "count": 0, "size": "ignored"}]});
    let typed: array::ArrayInput = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(typed).unwrap()["operations"],
        json!([{"op": "take", "count": 0}])
    );
    let error = check_data(&array::ArrayInput::schema().unwrap(), wire).unwrap_err();
    assert!(
        error
            .errors()
            .any(|error| error.path().to_string() == "/operations/0/size")
    );
}

#[test]
fn arbitrary_json_assignment_values_preserve_data_identity() {
    let mut values = vec![
        Value::Null,
        json!(false),
        json!(u64::MAX),
        json!(1.5),
        json!("{{ literal }}"),
    ];
    let mut nested = json!({"$expr": "{{ literal }}", "~1/~0": [], "": {}});
    // Leave depth headroom for the enclosing assignment record and array.
    for _ in 0..24 {
        values.push(nested.clone());
        nested = json!({"": [nested]});
    }
    for value in values {
        assert_wire::<set_fields::SetFieldsInput>(
            json!({"assignments": [{"name": "", "value": value}]}),
        );
    }
}
