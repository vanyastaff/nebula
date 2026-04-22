use nebula_schema::{
    BooleanWidget, Field, FieldValues, NumberWidget, RequiredMode, Schema, SecretWidget,
    SelectWidget, StringWidget, Transformer, VisibilityMode,
};
use serde_json::json;

fn raw_schema(fields: impl IntoIterator<Item = Field>) -> Schema {
    let fields: Vec<Field> = fields.into_iter().collect();
    serde_json::from_value(json!({ "fields": fields })).expect("raw schema from field list")
}

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
fn try_field_constructors_reject_invalid_keys() {
    let err = Field::try_string("bad-key").expect_err("invalid key should fail");
    assert_eq!(err.code, "invalid_key");
    assert!(Field::try_dynamic(" also bad ").is_err());
}

#[test]
fn try_field_constructors_accept_valid_keys() {
    let string = Field::try_string("name").expect("valid key");
    let select = Field::try_select("mode").expect("valid key");
    assert_eq!(string.key().as_str(), "name");
    assert_eq!(select.key().as_str(), "mode");
}

#[test]
fn schema_add_and_find_work() {
    let schema = raw_schema(vec![
        Field::string("name").widget(StringWidget::Plain).into(),
        Field::secret("api_key").widget(SecretWidget::Plain).into(),
        Field::boolean("enabled")
            .widget(BooleanWidget::Toggle)
            .into(),
    ]);

    assert_eq!(schema.len(), 3);
    assert!(!schema.is_empty());
    assert!(schema.find("api_key").is_some());
    assert!(schema.find("missing").is_none());
}

#[test]
#[allow(deprecated)]
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
    let schema = raw_schema(vec![
        Field::string("username")
            .visible_when(nebula_validator::Rule::predicate(
                nebula_validator::Predicate::eq("enabled", json!(true)).unwrap(),
            ))
            .required()
            .into(),
    ]);

    let encoded = serde_json::to_value(&schema).expect("schema serializes");
    let decoded: Schema = serde_json::from_value(encoded).expect("schema deserializes");
    let field = decoded.find("username").expect("field exists");

    assert!(matches!(field.visible(), VisibilityMode::When(_)));
    assert!(matches!(field.required(), RequiredMode::Always));
}

#[test]
fn validate_reports_missing_required() {
    let schema = Schema::builder()
        .add(Field::string("username").required())
        .build()
        .expect("valid schema");
    let values = FieldValues::new();
    let report = schema.validate(&values).unwrap_err();

    assert!(report.has_errors());
    assert_eq!(report.errors().count(), 1);
    assert!(report.errors().any(|e| e.path.to_string() == "username"));
    assert!(report.errors().any(|e| e.code == "required"));
}

#[test]
fn validate_applies_visibility_and_rules() {
    let schema = Schema::builder()
        .add(Field::boolean("enabled").required())
        .add(
            Field::string("api_key")
                .visible_when(nebula_validator::Rule::predicate(
                    nebula_validator::Predicate::eq("enabled", json!(true)).unwrap(),
                ))
                .required()
                .min_length(5),
        )
        .build()
        .expect("valid schema");

    let mut values = FieldValues::new();
    values.set_raw("enabled", json!(false));
    assert!(schema.validate(&values).is_ok());

    values.set_raw("enabled", json!(true));
    values.set_raw("api_key", json!("abc"));
    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
    assert!(report.errors().any(|e| e.path.to_string() == "api_key"));
}

#[test]
fn validate_enforces_scalar_type_mismatches() {
    let schema = Schema::builder()
        .add(Field::string("name").required())
        .add(Field::number("retries").required())
        .add(Field::boolean("enabled").required())
        .build()
        .expect("valid schema");
    let mut values = FieldValues::new();
    values.set_raw("name", json!(123));
    values.set_raw("retries", json!("bad"));
    values.set_raw("enabled", json!("true"));

    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
    assert!(
        report
            .errors()
            .any(|e| e.path.to_string() == "name" && e.code == "type_mismatch")
    );
    assert!(
        report
            .errors()
            .any(|e| e.path.to_string() == "retries" && e.code == "type_mismatch")
    );
    assert!(
        report
            .errors()
            .any(|e| e.path.to_string() == "enabled" && e.code == "type_mismatch")
    );
}

#[test]
fn validate_applies_transformers_before_rules() {
    let schema = Schema::builder()
        .add(
            Field::string("api_key")
                .with_transformer(Transformer::Trim)
                .with_rule(nebula_validator::Rule::max_length(6)),
        )
        .build()
        .expect("valid schema");
    let mut values = FieldValues::new();
    values.set_raw("api_key", json!("  SECRET  "));

    assert!(schema.validate(&values).is_ok());
}

#[test]
fn validate_enforces_file_value_shape() {
    let schema = Schema::builder()
        .add(Field::file("single").required())
        .add(Field::file("many").multiple().required())
        .build()
        .expect("valid schema");
    let mut values = FieldValues::new();
    values.set_raw("single", json!(true));
    values.set_raw("many", json!(["a.txt", 42]));

    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
    assert!(
        report
            .errors()
            .any(|e| e.path.to_string() == "single" && e.code == "type_mismatch")
    );
    assert!(
        report
            .errors()
            .any(|e| e.path.to_string() == "many" && e.code == "type_mismatch")
    );
}

#[test]
fn serde_roundtrip_supports_all_field_variants() {
    use nebula_schema::InputHint;

    let schema = raw_schema(vec![
        Field::string("s").into(),
        Field::secret("sec").into(),
        Field::number("n").into(),
        Field::boolean("b").into(),
        Field::select("sel").option("a", "A").into(),
        Field::object("obj").add(Field::string("child")).into(),
        Field::list("list").item(Field::string("item")).into(),
        Field::mode("mode")
            .variant("simple", "Simple", Field::string("payload"))
            .into(),
        Field::code("code").into(),
        // Date/DateTime/Time/Color → StringField with hint (replaces removed variants)
        Field::string("date").hint(InputHint::Date).into(),
        Field::string("datetime").hint(InputHint::DateTime).into(),
        Field::string("time").hint(InputHint::Time).into(),
        Field::string("color_field").hint(InputHint::Color).into(),
        Field::file("file").into(),
        // Hidden → visible(Never) on any field
        Field::string("hidden_field")
            .visible(VisibilityMode::Never)
            .into(),
        Field::computed("computed").into(),
        Field::dynamic("dynamic").into(),
        Field::notice("notice").into(),
    ]);

    let encoded = serde_json::to_value(&schema).expect("serialize full variant schema");
    let decoded: Schema = serde_json::from_value(encoded).expect("deserialize full variant schema");

    // 13 unique keys (the 5 removed variants are now represented as string fields with hints)
    assert_eq!(decoded.len(), 18);
    assert!(decoded.find("computed").is_some());
    assert!(decoded.find("notice").is_some());
}
