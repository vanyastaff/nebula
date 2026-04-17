use nebula_schema::{
    BooleanWidget, ExecutionMode, Field, FieldValues, NumberWidget, RequiredMode, Schema,
    SecretWidget, SelectWidget, StringWidget, Transformer, VisibilityMode,
};
use serde_json::json;

#[test]
fn builds_typed_fields_with_rules() {
    let field = Field::string("name")
        .label("Name")
        .required()
        .min_length(2)
        .max_length(32)
        .into_field();

    assert_eq!(field.key().as_str(), "name");
    assert_eq!(field.rules().len(), 2);
    assert!(matches!(field.required(), RequiredMode::Always));
}

#[test]
fn supports_select_and_number_builders() {
    let select = Field::select("mode")
        .widget(SelectWidget::Combobox)
        .option("a", "Option A")
        .multiple()
        .searchable()
        .into_field();

    let number = Field::number("retries")
        .integer()
        .widget(NumberWidget::Stepper)
        .min(0)
        .max(10)
        .into_field();

    assert_eq!(select.key().as_str(), "mode");
    assert_eq!(number.key().as_str(), "retries");
}

#[test]
fn schema_add_and_find_work() {
    let schema = Schema::new()
        .add(Field::string("name").widget(StringWidget::Plain))
        .add(Field::secret("api_key").widget(SecretWidget::Plain))
        .add(Field::boolean("enabled").widget(BooleanWidget::Toggle));

    assert_eq!(schema.len(), 3);
    assert!(!schema.is_empty());
    assert!(schema.find("api_key").is_some());
    assert!(schema.find("missing").is_none());
}

#[test]
fn schema_add_replaces_duplicate_key() {
    let schema = Schema::new()
        .add(Field::string("name").min_length(2))
        .add(Field::string("name").min_length(10));

    assert_eq!(schema.len(), 1);
    let name = schema.find("name").expect("name field exists");
    assert_eq!(name.rules().len(), 1);
}

#[test]
fn serde_roundtrip_field_and_schema() {
    let schema = Schema::new().add(
        Field::string("username")
            .visible_when(nebula_validator::Rule::Eq {
                field: "enabled".to_owned(),
                value: json!(true),
            })
            .required(),
    );

    let encoded = serde_json::to_value(&schema).expect("schema serializes");
    let decoded: Schema = serde_json::from_value(encoded).expect("schema deserializes");
    let field = decoded.find("username").expect("field exists");

    assert!(matches!(field.visible(), VisibilityMode::When(_)));
    assert!(matches!(field.required(), RequiredMode::Always));
}

#[test]
fn validate_reports_missing_required() {
    let schema = Schema::new().add(Field::string("username").required());
    let values = FieldValues::new();
    let report = schema.validate(&values, ExecutionMode::StaticOnly);

    assert!(report.has_errors());
    assert_eq!(report.errors().len(), 1);
    assert_eq!(report.errors()[0].key, "username");
    assert_eq!(report.errors()[0].code, "required");
}

#[test]
fn validate_applies_visibility_and_rules() {
    let schema = Schema::new().add(Field::boolean("enabled").required()).add(
        Field::string("api_key")
            .visible_when(nebula_validator::Rule::Eq {
                field: "enabled".to_owned(),
                value: json!(true),
            })
            .required()
            .min_length(5),
    );

    let mut values = FieldValues::new();
    values.set_raw("enabled", json!(false));
    let report_hidden = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(!report_hidden.has_errors());

    values.set_raw("enabled", json!(true));
    values.set_raw("api_key", json!("abc"));
    let report_short = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report_short.has_errors());
    assert_eq!(report_short.errors()[0].key, "api_key");
}

#[test]
fn normalize_backfills_defaults() {
    let schema = Schema::new()
        .add(Field::string("host").default(json!("localhost")))
        .add(Field::number("port").default(json!(5432)));
    let mut values = FieldValues::new();
    values.set_raw("host", json!("db.internal"));

    let normalized = schema.normalize(&values);

    assert_eq!(normalized.get_string_by_str("host"), Some("db.internal"));
    assert_eq!(normalized.get_raw_by_str("port"), Some(json!(5432)));
}

#[test]
fn normalize_recurses_for_object_list_and_mode_defaults() {
    let schema = Schema::new()
        .add(
            Field::object("config")
                .add(Field::string("host").default(json!("localhost")))
                .add(Field::number("port").default(json!(8080))),
        )
        .add(
            Field::list("items").item(
                Field::object("item")
                    .add(Field::string("name").default(json!("unnamed")))
                    .add(Field::number("qty").default(json!(1))),
            ),
        )
        .add(
            Field::mode("auth")
                .variant(
                    "bearer",
                    "Bearer",
                    Field::object("payload").add(Field::secret("token").default(json!("secret"))),
                )
                .default_variant("bearer"),
        );

    let mut values = FieldValues::new();
    values.set_raw("config", json!({ "host": "db.internal" }));
    values.set_raw("items", json!([{ "name": "apple" }, {}]));

    let normalized = schema.normalize(&values);
    assert_eq!(
        normalized.get_raw_by_str("config"),
        Some(json!({ "host": "db.internal", "port": 8080 }))
    );
    assert_eq!(
        normalized.get_raw_by_str("items"),
        Some(json!([{ "name": "apple", "qty": 1 }, { "name": "unnamed", "qty": 1 }]))
    );
    assert_eq!(
        normalized.get_raw_by_str("auth"),
        Some(json!({ "mode": "bearer", "value": { "token": "secret" } }))
    );
}

#[test]
fn validate_enforces_scalar_type_mismatches() {
    let schema = Schema::new()
        .add(Field::string("name").required())
        .add(Field::number("retries").required())
        .add(Field::boolean("enabled").required());
    let mut values = FieldValues::new();
    values.set_raw("name", json!(123));
    values.set_raw("retries", json!("bad"));
    values.set_raw("enabled", json!("true"));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report.has_errors());
    assert!(report.errors().iter().any(|issue| issue.key == "name"));
    assert!(report.errors().iter().any(|issue| issue.key == "retries"));
    assert!(report.errors().iter().any(|issue| issue.key == "enabled"));
}

#[test]
fn validate_applies_transformers_before_rules() {
    let schema = Schema::new().add(
        Field::string("api_key")
            .with_transformer(Transformer::Trim)
            .with_rule(nebula_validator::Rule::MaxLength {
                max: 6,
                message: None,
            }),
    );
    let mut values = FieldValues::new();
    values.set_raw("api_key", json!("  SECRET  "));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(!report.has_errors());
}

#[test]
fn validate_enforces_file_value_shape() {
    let schema = Schema::new()
        .add(Field::file("single").required())
        .add(Field::file("many").multiple().required());
    let mut values = FieldValues::new();
    values.set_raw("single", json!(true));
    values.set_raw("many", json!(["a.txt", 42]));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(report.has_errors());
    assert!(report.errors().iter().any(|issue| issue.key == "single"));
    assert!(report.errors().iter().any(|issue| issue.key == "many"));
}

#[test]
fn serde_roundtrip_supports_all_field_variants() {
    use nebula_schema::InputHint;

    let schema = Schema::new()
        .add(Field::string("s"))
        .add(Field::secret("sec"))
        .add(Field::number("n"))
        .add(Field::boolean("b"))
        .add(Field::select("sel").option("a", "A"))
        .add(Field::object("obj").add(Field::string("child")))
        .add(Field::list("list").item(Field::string("item")))
        .add(Field::mode("mode").variant("simple", "Simple", Field::string("payload")))
        .add(Field::code("code"))
        // Date/DateTime/Time/Color → StringField with hint (replaces removed variants)
        .add(Field::string("date").hint(InputHint::Date))
        .add(Field::string("datetime").hint(InputHint::DateTime))
        .add(Field::string("time").hint(InputHint::Time))
        .add(Field::string("color_field").hint(InputHint::Color))
        .add(Field::file("file"))
        // Hidden → visible(Never) on any field
        .add(Field::string("hidden_field").visible(nebula_schema::VisibilityMode::Never))
        .add(Field::computed("computed"))
        .add(Field::dynamic("dynamic"))
        .add(Field::notice("notice"));

    let encoded = serde_json::to_value(&schema).expect("serialize full variant schema");
    let decoded: Schema = serde_json::from_value(encoded).expect("deserialize full variant schema");

    // 13 unique keys (the 5 removed variants are now represented as string fields with hints)
    assert_eq!(decoded.len(), 18);
    assert!(decoded.find("computed").is_some());
    assert!(decoded.find("notice").is_some());
}
