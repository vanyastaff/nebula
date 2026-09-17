use super::*;
use crate::{FieldKey, FieldPath, Property};

fn fk(s: &str) -> FieldKey {
    FieldKey::new(s).unwrap()
}

#[test]
fn build_empty_schema_ok() {
    let s = Schema::builder().build().unwrap();
    assert_eq!(s.properties().len(), 0);
}

#[test]
fn schema_properties_keep_legacy_fields_wire_key() {
    let schema = Schema {
        properties: vec![Property::string(fk("name")).required().into()],
    };
    let wire = serde_json::to_value(&schema).unwrap();
    assert_eq!(
        wire.get("fields").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );
    assert!(wire.get("properties").is_none());

    let decoded: Schema = serde_json::from_value(wire).unwrap();
    assert!(decoded.find_property("name").is_some());
    assert_eq!(decoded.properties().len(), 1);
}

#[test]
fn build_detects_duplicate_key() {
    let r = Schema::builder()
        .property(Property::string(fk("x")))
        .property(Property::number(fk("x")))
        .build();
    let err = r.unwrap_err();
    assert!(err.errors().any(|e| e.code() == "duplicate_key"));
}

#[test]
fn build_finds_field_by_key() {
    let s = Schema::builder()
        .property(Property::string(fk("a")))
        .build()
        .unwrap();
    let key = FieldKey::new("a").unwrap();
    assert!(s.find_property(&key).is_some());
}

#[test]
fn schema_flags_track_depth() {
    let s = Schema::builder()
        .property(Property::string(fk("a")))
        .property(Property::number(fk("b")))
        .build()
        .unwrap();
    assert_eq!(s.flags().max_depth, 1);
}

#[test]
fn loader_key_rejects_invalid_field_key_for_select() {
    let schema = Schema::builder()
        .property(Property::select(fk("select_field")))
        .build()
        .expect("valid schema");
    let err = resolve_select_loader_key(schema.properties(), "bad-key").unwrap_err();
    assert_eq!(err.code(), "invalid_key");
}

#[test]
fn loader_key_rejects_invalid_field_key_for_dynamic() {
    let schema = Schema::builder()
        .property(Property::dynamic(fk("dynamic_field")))
        .build()
        .expect("valid schema");
    let err = resolve_dynamic_loader_key(schema.properties(), "bad-key").unwrap_err();
    assert_eq!(err.code(), "invalid_key");
}

#[test]
fn build_rejects_schema_depth_beyond_index_limits() {
    fn nested_object(depth: usize) -> Property {
        let mut current: Property = Property::string(fk("leaf")).into();
        for i in (0..depth).rev() {
            let key = FieldKey::new(format!("n{i}")).expect("generated key");
            current = Property::object(key).property(current).into();
        }
        current
    }

    let result = Schema::builder().property(nested_object(260)).build();
    let report = result.expect_err("deep schema should be rejected");
    assert!(report.errors().any(|e| e.code() == "schema.depth_limit"));
}

#[test]
fn build_rejects_schema_just_beyond_max_depth() {
    fn nested_object(depth: usize) -> Property {
        let mut current: Property = Property::string(fk("leaf")).into();
        for i in (0..depth).rev() {
            current = Property::object(fk(&format!("n{i}")))
                .property(current)
                .into();
        }
        current
    }
    // One level past the explicit cap must be rejected (pins MAX_SCHEMA_DEPTH,
    // not just the old u8::MAX backstop).
    let result = Schema::builder()
        .property(nested_object(usize::from(MAX_SCHEMA_DEPTH) + 2))
        .build();
    let report = result.expect_err("over-deep schema should be rejected");
    assert!(report.errors().any(|e| e.code() == "schema.depth_limit"));
}

#[test]
fn deserialize_rejects_deeply_nested_schema_via_from_value() {
    // `ValidSchema::deserialize` from an already-materialized `Value` — the
    // `serde_json::from_value` path has no streaming-parser recursion cap
    // (unlike `from_str`/`from_slice`, which self-limit at 128) — must be
    // rejected by the build-time depth guard rather than recursing unbounded
    // through the lint passes. Schema-tree analogue of value.rs's
    // `field_value_deserialize_rejects_deeply_nested_input`.
    fn nested_object(depth: usize) -> Property {
        let mut current: Property = Property::string(fk("leaf")).into();
        for i in (0..depth).rev() {
            current = Property::object(fk(&format!("n{i}")))
                .property(current)
                .into();
        }
        current
    }
    // Serialize a too-deep field to obtain the exact wire shape, then feed it
    // back through `from_value` inside a `ValidSchema` envelope.
    let deep = nested_object(usize::from(MAX_SCHEMA_DEPTH) + 5);
    let wire = serde_json::json!({
        "fields": [serde_json::to_value(&deep).expect("serialize field")]
    });
    let err = serde_json::from_value::<ValidSchema>(wire).expect_err("must reject");
    assert!(
        err.to_string().contains("depth"),
        "expected depth-limit rejection, got: {err}"
    );
}

#[test]
fn build_rejects_deep_list_of_lists() {
    // `List<List<List<…>>>` with no object anywhere: the depth guard must
    // descend non-object list items, not just `List<Object>`. A list-only
    // deep schema would otherwise bypass MAX_SCHEMA_DEPTH and still drive the
    // unbounded lint/validate/promote recursion.
    fn nested_list(depth: usize) -> Property {
        let mut current: Property = Property::string(fk("leaf")).into();
        for i in (0..depth).rev() {
            current = Property::list(fk(&format!("l{i}"))).item(current).into();
        }
        current
    }
    let result = Schema::builder()
        .property(nested_list(usize::from(MAX_SCHEMA_DEPTH) + 5))
        .build();
    let report = result.expect_err("deep list-of-lists must be rejected");
    assert!(
        report.errors().any(|e| e.code() == "schema.depth_limit"),
        "expected schema.depth_limit, got: {report:?}"
    );
}

#[test]
fn lint_rejects_deeply_nested_schema_without_builder() {
    // `Schema::lint()` runs `lint_tree` directly; it must apply the same
    // depth guard so a `Schema` built/deserialized outside the builder cannot
    // enter the unbounded lint recursion.
    fn nested_object(depth: usize) -> Property {
        let mut current: Property = Property::string(fk("leaf")).into();
        for i in (0..depth).rev() {
            current = Property::object(fk(&format!("n{i}")))
                .property(current)
                .into();
        }
        current
    }
    let schema = Schema {
        properties: vec![nested_object(usize::from(MAX_SCHEMA_DEPTH) + 5)],
    };
    let report = schema.lint();
    assert!(
        report.errors().any(|e| e.code() == "schema.depth_limit"),
        "Schema::lint must reject an over-deep tree, got: {report:?}"
    );
}

#[test]
fn build_rejects_mode_variant_list_item_index_overflow() {
    let too_many_fields = (0..(usize::from(u16::MAX) + 2))
        .map(|i| Property::string(fk(&format!("f{i}"))))
        .collect::<Vec<_>>();

    let result = Schema::builder()
        .property(
            Property::mode(fk("payload")).variant(
                "bulk",
                "Bulk",
                Property::list(fk("items"))
                    .item(Property::object(fk("item")).properties(too_many_fields)),
            ),
        )
        .build();

    let report = result.expect_err("mode variant list item overflow should be rejected");
    assert!(
        report.errors().any(|e| e.code() == "schema.index_overflow"),
        "expected schema.index_overflow, got: {report:?}"
    );
}

#[test]
fn build_deduplicates_select_depends_on_for_runtime_schema() {
    let dep = FieldPath::parse("team_id").unwrap();
    let schema = Schema::builder()
        .property(Property::string(fk("team_id")))
        .property(
            Property::select(fk("workspace"))
                .dynamic()
                .loader("workspace_loader")
                .depends_on(dep.clone())
                .depends_on(dep),
        )
        .build()
        .expect("schema should build");

    let field = schema
        .find_property(&fk("workspace"))
        .expect("field must exist");
    let Property::Select(select) = field else {
        panic!("expected select field");
    };
    assert_eq!(select.depends_on.len(), 1);
}

#[test]
fn build_deduplicates_nested_dynamic_depends_on_for_runtime_schema() {
    let dep = FieldPath::parse("team_id").unwrap();
    let schema = Schema::builder()
        .property(Property::string(fk("team_id")))
        .property(
            Property::object(fk("container")).property(
                Property::dynamic(fk("resource"))
                    .loader("resource_loader")
                    .depends_on(dep.clone())
                    .depends_on(dep),
            ),
        )
        .build()
        .expect("schema should build");

    let path = FieldPath::parse("container.resource").unwrap();
    let field = schema
        .find_property_by_path(&path)
        .expect("nested field should be indexed");
    let Property::Dynamic(dynamic) = field else {
        panic!("expected dynamic field");
    };
    assert_eq!(dynamic.depends_on.len(), 1);
}

#[test]
fn build_deduplicates_rules_and_transformers_for_runtime_schema() {
    let schema = Schema::builder()
        .property(
            Property::string(fk("name"))
                .min_length(3)
                .min_length(3)
                .with_transformer(crate::Transformer::Trim)
                .with_transformer(crate::Transformer::Trim),
        )
        .build()
        .expect("schema should build");

    let field = schema.find_property(&fk("name")).expect("field must exist");
    let Property::String(string) = field else {
        panic!("expected string field");
    };
    assert_eq!(string.rules.len(), 1);
    assert_eq!(string.transformers.len(), 1);
}

#[test]
fn build_deduplicates_nested_rules_and_transformers_for_runtime_schema() {
    let schema = Schema::builder()
        .property(
            Property::object(fk("container")).property(
                Property::number(fk("count"))
                    .min(1)
                    .min(1)
                    .with_transformer(crate::Transformer::Trim)
                    .with_transformer(crate::Transformer::Trim),
            ),
        )
        .build()
        .expect("schema should build");

    let path = FieldPath::parse("container.count").unwrap();
    let field = schema
        .find_property_by_path(&path)
        .expect("nested field should be indexed");
    let Property::Number(number) = field else {
        panic!("expected number field");
    };
    assert_eq!(number.rules.len(), 1);
    assert_eq!(number.transformers.len(), 1);
}
