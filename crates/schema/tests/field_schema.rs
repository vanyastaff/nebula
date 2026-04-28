use nebula_schema::{
    BooleanWidget, Field, FieldValues, NumberWidget, RequiredMode, Schema, SecretWidget,
    SelectWidget, StringWidget, Transformer, VisibilityMode, field_key,
};
use serde_json::json;

fn raw_schema(fields: impl IntoIterator<Item = Field>) -> Schema {
    let fields: Vec<Field> = fields.into_iter().collect();
    serde_json::from_value(json!({ "fields": fields })).expect("raw schema from field list")
}

#[test]
fn builds_typed_fields_with_rules() {
    let field = Field::string(field_key!("name"))
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
    let select = Field::select(field_key!("mode"))
        .widget(SelectWidget::Combobox)
        .option("a", "Option A")
        .multiple()
        .searchable()
        .into_field();

    let number = Field::number(field_key!("retries"))
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
        Field::string(field_key!("name"))
            .widget(StringWidget::Plain)
            .into(),
        Field::secret(field_key!("api_key"))
            .widget(SecretWidget::Plain)
            .into(),
        Field::boolean(field_key!("enabled"))
            .widget(BooleanWidget::Toggle)
            .into(),
    ]);

    assert_eq!(schema.len(), 3);
    assert!(!schema.is_empty());
    assert!(schema.find("api_key").is_some());
    assert!(schema.find("missing").is_none());
}

#[test]
fn schema_builder_rejects_duplicate_key() {
    let result = Schema::builder()
        .add(Field::string(field_key!("name")).min_length(2))
        .add(Field::string(field_key!("name")).min_length(10))
        .build();

    let err = result.expect_err("duplicate key should cause build to fail");
    assert!(err.errors().any(|e| e.code == "duplicate_key"));
}

#[test]
fn serde_roundtrip_field_and_schema() {
    let schema = raw_schema(vec![
        Field::string(field_key!("username"))
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
        .add(Field::string(field_key!("username")).required())
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
        .add(Field::boolean(field_key!("enabled")).required())
        .add(
            Field::string(field_key!("api_key"))
                .visible_when(nebula_validator::Rule::predicate(
                    nebula_validator::Predicate::eq("enabled", json!(true)).unwrap(),
                ))
                .required()
                .min_length(5),
        )
        .build()
        .expect("valid schema");

    let mut values = FieldValues::new();
    values
        .try_set_raw("enabled", json!(false))
        .expect("test-only known-good key");
    assert!(schema.validate(&values).is_ok());

    values
        .try_set_raw("enabled", json!(true))
        .expect("test-only known-good key");
    values
        .try_set_raw("api_key", json!("abc"))
        .expect("test-only known-good key");
    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
    assert!(report.errors().any(|e| e.path.to_string() == "api_key"));
}

#[test]
fn validate_enforces_scalar_type_mismatches() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("name")).required())
        .add(Field::number(field_key!("retries")).required())
        .add(Field::boolean(field_key!("enabled")).required())
        .build()
        .expect("valid schema");
    let mut values = FieldValues::new();
    values
        .try_set_raw("name", json!(123))
        .expect("test-only known-good key");
    values
        .try_set_raw("retries", json!("bad"))
        .expect("test-only known-good key");
    values
        .try_set_raw("enabled", json!("true"))
        .expect("test-only known-good key");

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
            Field::string(field_key!("api_key"))
                .with_transformer(Transformer::Trim)
                .with_rule(nebula_validator::Rule::max_length(6)),
        )
        .build()
        .expect("valid schema");
    let mut values = FieldValues::new();
    values
        .try_set_raw("api_key", json!("  SECRET  "))
        .expect("test-only known-good key");

    assert!(schema.validate(&values).is_ok());
}

#[test]
fn validate_enforces_file_value_shape() {
    let schema = Schema::builder()
        .add(Field::file(field_key!("single")).required())
        .add(Field::file(field_key!("many")).multiple().required())
        .build()
        .expect("valid schema");
    let mut values = FieldValues::new();
    values
        .try_set_raw("single", json!(true))
        .expect("test-only known-good key");
    values
        .try_set_raw("many", json!(["a.txt", 42]))
        .expect("test-only known-good key");

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
        Field::string(field_key!("s")).into(),
        Field::secret(field_key!("sec")).into(),
        Field::number(field_key!("n")).into(),
        Field::boolean(field_key!("b")).into(),
        Field::select(field_key!("sel")).option("a", "A").into(),
        Field::object(field_key!("obj"))
            .add(Field::string(field_key!("child")))
            .into(),
        Field::list(field_key!("list"))
            .item(Field::string(field_key!("item")))
            .into(),
        Field::mode(field_key!("mode"))
            .variant("simple", "Simple", Field::string(field_key!("payload")))
            .into(),
        Field::code(field_key!("code")).into(),
        // Date/DateTime/Time/Color → StringField with hint (replaces removed variants)
        Field::string(field_key!("date"))
            .hint(InputHint::Date)
            .into(),
        Field::string(field_key!("datetime"))
            .hint(InputHint::DateTime)
            .into(),
        Field::string(field_key!("time"))
            .hint(InputHint::Time)
            .into(),
        Field::string(field_key!("color_field"))
            .hint(InputHint::Color)
            .into(),
        Field::file(field_key!("file")).into(),
        // Hidden → visible(Never) on any field
        Field::string(field_key!("hidden_field"))
            .visible(VisibilityMode::Never)
            .into(),
        Field::computed(field_key!("computed")).into(),
        Field::dynamic(field_key!("dynamic")).into(),
        Field::notice(field_key!("notice")).into(),
    ]);

    let encoded = serde_json::to_value(&schema).expect("serialize full variant schema");
    let decoded: Schema = serde_json::from_value(encoded).expect("deserialize full variant schema");

    // 13 unique keys (the 5 removed variants are now represented as string fields with hints)
    assert_eq!(decoded.len(), 18);
    assert!(decoded.find("computed").is_some());
    assert!(decoded.find("notice").is_some());
}
