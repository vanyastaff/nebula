use nebula_schema::{
    BooleanWidget, ExecutionMode, Field, FieldValues, NumberWidget, RequiredMode, Schema,
    SecretWidget, SelectWidget, StringWidget, VisibilityMode,
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
    values.set("enabled", json!(false));
    let report_hidden = schema.validate(&values, ExecutionMode::StaticOnly);
    assert!(!report_hidden.has_errors());

    values.set("enabled", json!(true));
    values.set("api_key", json!("abc"));
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
    values.set("host", json!("db.internal"));

    let normalized = schema.normalize(&values);

    assert_eq!(normalized.get_string("host"), Some("db.internal"));
    assert_eq!(normalized.get("port"), Some(&json!(5432)));
}

#[test]
fn serde_roundtrip_supports_all_field_variants() {
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
        .add(Field::date("date"))
        .add(Field::datetime("datetime"))
        .add(Field::time("time"))
        .add(Field::color("color"))
        .add(Field::file("file"))
        .add(Field::hidden("hidden"))
        .add(Field::computed("computed"))
        .add(Field::dynamic("dynamic"))
        .add(Field::notice("notice"));

    let encoded = serde_json::to_value(&schema).expect("serialize full variant schema");
    let decoded: Schema = serde_json::from_value(encoded).expect("deserialize full variant schema");

    assert_eq!(decoded.len(), 18);
    assert!(decoded.find("computed").is_some());
    assert!(decoded.find("notice").is_some());
}
