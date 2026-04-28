use nebula_schema::{
    Field, FieldPath, FieldValues, LoaderContext, LoaderRegistry, LoaderResult, Schema,
    ValidationReport, field_key,
};
use serde_json::json;

fn raw_schema(fields: impl IntoIterator<Item = Field>) -> Schema {
    let fields: Vec<Field> = fields.into_iter().collect();
    serde_json::from_value(json!({ "fields": fields })).expect("raw schema from field list")
}

fn has_error(report: &ValidationReport, code: &str, path_prefix: &str) -> bool {
    report
        .errors()
        .any(|e| e.code == code && e.path.to_string().starts_with(path_prefix))
}

fn has_warning(report: &ValidationReport, code: &str, path_prefix: &str) -> bool {
    report
        .warnings()
        .any(|e| e.code == code && e.path.to_string().starts_with(path_prefix))
}

#[test]
fn lint_schema_reports_dangling_refs_and_structural_issues() {
    let schema = raw_schema(vec![
        Field::string(field_key!("toggle")).into(),
        Field::string(field_key!("name"))
            .visible_when(nebula_validator::Rule::predicate(
                nebula_validator::Predicate::eq("missing", json!(true)).unwrap(),
            ))
            .with_rule(nebula_validator::Rule::min_length(5))
            .with_rule(nebula_validator::Rule::max_length(2))
            .into(),
        Field::select(field_key!("region"))
            .dynamic()
            .loader("regions_loader")
            .depends_on(FieldPath::parse("unknown_ref").unwrap())
            .into(),
        Field::mode(field_key!("auth"))
            .variant("token", "", Field::secret(field_key!("token")))
            .default_variant("missing_variant")
            .into(),
    ]);

    let report = schema.lint();
    assert!(report.has_errors());
    assert!(
        has_error(&report, "dangling_reference", "name"),
        "expected dangling_reference at name, got: {:?}",
        report
            .errors()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
    assert!(
        has_error(&report, "rule.contradictory", "name"),
        "expected rule.contradictory at name"
    );
    assert!(
        has_error(&report, "dangling_reference", "region"),
        "expected dangling_reference at region"
    );
    assert!(
        has_error(&report, "invalid_default_variant", "auth"),
        "expected invalid_default_variant at auth"
    );
    assert!(
        has_warning(&report, "missing_variant_label", "auth"),
        "expected missing_variant_label warning at auth"
    );
}

#[tokio::test]
async fn loader_registry_resolves_select_and_dynamic_loaders() {
    let schema = raw_schema(vec![
        Field::select(field_key!("workspace"))
            .dynamic()
            .loader("workspace_loader")
            .depends_on(FieldPath::parse("team_id").unwrap())
            .into(),
        Field::dynamic(field_key!("resource"))
            .loader("resource_loader")
            .depends_on(FieldPath::parse("workspace").unwrap())
            .into(),
    ]);

    let registry = LoaderRegistry::new()
        .register_option("workspace_loader", |_ctx| async {
            Ok(LoaderResult::done(vec![nebula_schema::SelectOption::new(
                json!("ws_1"),
                "Workspace 1",
            )]))
        })
        .register_record("resource_loader", |ctx| async move {
            let workspace = ctx
                .values
                .get_string_by_str("workspace")
                .unwrap_or("none")
                .to_owned();
            Ok(LoaderResult::done(vec![json!({
                "id": "res_1",
                "workspace": workspace
            })]))
        });

    let mut values = FieldValues::new();
    values
        .try_set_raw("workspace", json!("ws_1"))
        .expect("test-only known-good key");
    values
        .try_set_raw("team_id", json!("team_1"))
        .expect("test-only known-good key");
    let context = LoaderContext::new("workspace", values.clone()).with_filter("prod");

    let options = schema
        .load_select_options("workspace", &registry, context.clone())
        .await
        .expect("workspace options should load");
    assert_eq!(options.items.len(), 1);

    let records = schema
        .load_dynamic_records("resource", &registry, context)
        .await
        .expect("resource records should load");
    assert_eq!(records.items.len(), 1);
    assert_eq!(records.items[0]["workspace"], json!("ws_1"));
}

#[tokio::test]
async fn valid_schema_loader_apis_resolve_loaders() {
    let schema = Schema::builder()
        .add(Field::string(field_key!("team_id")))
        .add(
            Field::select(field_key!("workspace"))
                .dynamic()
                .loader("workspace_loader")
                .depends_on(FieldPath::parse("team_id").unwrap()),
        )
        .add(
            Field::dynamic(field_key!("resource"))
                .loader("resource_loader")
                .depends_on(FieldPath::parse("workspace").unwrap()),
        )
        .build()
        .expect("schema should build");

    let registry = LoaderRegistry::new()
        .register_option("workspace_loader", |_ctx| async {
            Ok(LoaderResult::done(vec![nebula_schema::SelectOption::new(
                json!("ws_1"),
                "Workspace 1",
            )]))
        })
        .register_record("resource_loader", |_ctx| async {
            Ok(LoaderResult::done(vec![json!({"id": "res_1"})]))
        });

    let context = LoaderContext::new("workspace", FieldValues::new());
    let options = schema
        .load_select_options("workspace", &registry, context.clone())
        .await
        .expect("workspace options should load");
    assert_eq!(options.items.len(), 1);

    let records = schema
        .load_dynamic_records("resource", &registry, context)
        .await
        .expect("resource records should load");
    assert_eq!(records.items.len(), 1);
}

#[tokio::test]
async fn nested_schema_loader_apis_resolve_object_paths() {
    let schema = raw_schema(vec![
        Field::object(field_key!("config"))
            .add(
                Field::select(field_key!("workspace"))
                    .dynamic()
                    .loader("workspace_loader"),
            )
            .add(Field::dynamic(field_key!("resource")).loader("resource_loader"))
            .into(),
    ]);

    let registry = LoaderRegistry::new()
        .register_option("workspace_loader", |ctx| async move {
            assert_eq!(ctx.field_key, "config.workspace");
            Ok(LoaderResult::done(vec![nebula_schema::SelectOption::new(
                json!("ws_nested"),
                "Nested Workspace",
            )]))
        })
        .register_record("resource_loader", |ctx| async move {
            assert_eq!(ctx.field_key, "config.resource");
            Ok(LoaderResult::done(vec![json!({"id": "res_nested"})]))
        });

    let workspace_path = FieldPath::parse("config.workspace").unwrap();
    let resource_path = FieldPath::parse("config.resource").unwrap();

    let options = schema
        .load_select_options_at(
            &workspace_path,
            &registry,
            LoaderContext::new("config.workspace", FieldValues::new()),
        )
        .await
        .expect("nested workspace options should load");
    assert_eq!(options.items.len(), 1);
    assert_eq!(options.items[0].label, "Nested Workspace");

    let records = schema
        .load_dynamic_records_at(
            &resource_path,
            &registry,
            LoaderContext::new("config.resource", FieldValues::new()),
        )
        .await
        .expect("nested resource records should load");
    assert_eq!(records.items.len(), 1);
    assert_eq!(records.items[0]["id"], json!("res_nested"));
}

#[tokio::test]
async fn nested_schema_loader_apis_resolve_list_item_paths() {
    let schema = raw_schema(vec![
        Field::list(field_key!("rows"))
            .item(
                Field::object(field_key!("row")).add(
                    Field::select(field_key!("workspace"))
                        .dynamic()
                        .loader("workspace_loader"),
                ),
            )
            .into(),
    ]);

    let registry = LoaderRegistry::new().register_option("workspace_loader", |ctx| async move {
        Ok(LoaderResult::done(vec![nebula_schema::SelectOption::new(
            json!(ctx.field_key),
            "Workspace from list",
        )]))
    });

    let indexed_path = FieldPath::parse("rows[0].workspace").unwrap();
    let indexed = schema
        .load_select_options_at(
            &indexed_path,
            &registry,
            LoaderContext::new("rows[0].workspace", FieldValues::new()),
        )
        .await
        .expect("indexed list path should resolve");
    assert_eq!(indexed.items.len(), 1);
    assert_eq!(indexed.items[0].value, json!("rows[0].workspace"));

    let schema_path = FieldPath::parse("rows.workspace").unwrap();
    let schema_level = schema
        .load_select_options_at(
            &schema_path,
            &registry,
            LoaderContext::new("rows.workspace", FieldValues::new()),
        )
        .await
        .expect("schema-level list path should resolve");
    assert_eq!(schema_level.items.len(), 1);
    assert_eq!(schema_level.items[0].value, json!("rows.workspace"));
}

#[tokio::test]
async fn nested_valid_schema_loader_api_resolves_mode_variant_paths() {
    let schema = Schema::builder()
        .add(
            Field::mode(field_key!("auth")).variant(
                "oauth",
                "OAuth",
                Field::object(field_key!("creds"))
                    .add(Field::dynamic(field_key!("resource")).loader("resource_loader")),
            ),
        )
        .build()
        .expect("schema should build");

    let registry = LoaderRegistry::new().register_record("resource_loader", |ctx| async move {
        assert_eq!(ctx.field_key, "auth.oauth.resource");
        Ok(LoaderResult::done(vec![json!({"id": "oauth_resource"})]))
    });

    let path = FieldPath::parse("auth.oauth.resource").unwrap();
    let records = schema
        .load_dynamic_records_at(
            &path,
            &registry,
            LoaderContext::new("auth.oauth.resource", FieldValues::new()),
        )
        .await
        .expect("mode-variant resource records should load");

    assert_eq!(records.items.len(), 1);
    assert_eq!(records.items[0]["id"], json!("oauth_resource"));
}

#[tokio::test]
async fn nested_loader_errors_anchor_to_nested_path() {
    let schema = raw_schema(vec![
        Field::object(field_key!("config"))
            .add(
                Field::select(field_key!("workspace"))
                    .dynamic()
                    .loader("missing_workspace_loader"),
            )
            .into(),
    ]);
    let registry = LoaderRegistry::new();
    let path = FieldPath::parse("config.workspace").unwrap();

    let error = schema
        .load_select_options_at(
            &path,
            &registry,
            LoaderContext::new("config.workspace", FieldValues::new()),
        )
        .await
        .expect_err("missing nested loader must fail");

    assert_eq!(error.code, "loader.not_registered");
    assert_eq!(error.path.to_string(), "config.workspace");
}

#[tokio::test]
async fn top_level_loader_string_api_rejects_nested_paths() {
    let schema = raw_schema(vec![
        Field::object(field_key!("config"))
            .add(
                Field::select(field_key!("workspace"))
                    .dynamic()
                    .loader("workspace_loader"),
            )
            .into(),
    ]);
    let registry = LoaderRegistry::new();
    let error = schema
        .load_select_options(
            "config.workspace",
            &registry,
            LoaderContext::new("config.workspace", FieldValues::new()),
        )
        .await
        .expect_err("top-level string API should reject nested paths");

    assert_eq!(error.code, "invalid_key");
    assert_eq!(error.path.to_string(), "");
}

#[tokio::test]
async fn loader_registry_reports_missing_loader_registration() {
    let schema = raw_schema(vec![
        Field::select(field_key!("region"))
            .dynamic()
            .loader("missing_loader")
            .into(),
    ]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("region", FieldValues::new());
    let error = schema
        .load_select_options("region", &registry, context)
        .await
        .expect_err("missing loader must fail");
    assert!(error.to_string().contains("missing_loader"));
}

#[tokio::test]
async fn load_select_options_unknown_key_emits_field_not_found() {
    let schema = raw_schema(vec![
        Field::select(field_key!("region"))
            .dynamic()
            .loader("x")
            .into(),
    ]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("ghost", FieldValues::new());
    let error = schema
        .load_select_options("ghost", &registry, context)
        .await
        .expect_err("unknown key must fail");
    assert_eq!(error.code, "field.not_found");
    assert_eq!(error.path.to_string(), "ghost");
}

#[tokio::test]
async fn load_select_options_wrong_field_type_emits_type_mismatch() {
    let schema = raw_schema(vec![Field::string(field_key!("email")).into()]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("email", FieldValues::new());
    let error = schema
        .load_select_options("email", &registry, context)
        .await
        .expect_err("wrong field type must fail");
    assert_eq!(error.code, "field.type_mismatch");
    assert_eq!(error.path.to_string(), "email");
    assert!(
        error
            .params
            .iter()
            .any(|(k, v)| k == "expected" && v == "select"),
        "expected param missing: {:?}",
        error.params
    );
    assert!(
        error
            .params
            .iter()
            .any(|(k, v)| k == "actual" && v == "string"),
        "actual param missing: {:?}",
        error.params
    );
}

#[tokio::test]
async fn load_select_options_without_loader_emits_missing_config() {
    let schema = raw_schema(vec![
        Field::select(field_key!("region"))
            .option("us", "US")
            .into(),
    ]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("region", FieldValues::new());
    let error = schema
        .load_select_options("region", &registry, context)
        .await
        .expect_err("missing loader config must fail");
    assert_eq!(error.code, "loader.missing_config");
    assert_eq!(error.path.to_string(), "region");
}

#[tokio::test]
async fn load_dynamic_records_wrong_field_type_emits_type_mismatch() {
    let schema = raw_schema(vec![Field::number(field_key!("count")).into()]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("count", FieldValues::new());
    let error = schema
        .load_dynamic_records("count", &registry, context)
        .await
        .expect_err("wrong field type must fail");
    assert_eq!(error.code, "field.type_mismatch");
    assert!(
        error
            .params
            .iter()
            .any(|(k, v)| k == "expected" && v == "dynamic"),
        "expected param missing: {:?}",
        error.params
    );
}

#[tokio::test]
async fn load_dynamic_records_unknown_key_emits_field_not_found() {
    let schema = raw_schema(vec![
        Field::dynamic(field_key!("resource"))
            .loader("loader_x")
            .into(),
    ]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("ghost", FieldValues::new());
    let error = schema
        .load_dynamic_records("ghost", &registry, context)
        .await
        .expect_err("unknown key must fail");
    assert_eq!(error.code, "field.not_found");
    assert_eq!(error.path.to_string(), "ghost");
}

#[tokio::test]
async fn load_dynamic_records_without_loader_emits_missing_config() {
    let schema = raw_schema(vec![Field::dynamic(field_key!("resource")).into()]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("resource", FieldValues::new());
    let error = schema
        .load_dynamic_records("resource", &registry, context)
        .await
        .expect_err("missing loader config must fail");
    assert_eq!(error.code, "loader.missing_config");
    assert_eq!(error.path.to_string(), "resource");
}

#[test]
fn lint_schema_detects_visibility_cycles() {
    let schema = raw_schema(vec![
        Field::string(field_key!("a"))
            .visible_when(nebula_validator::Rule::predicate(
                nebula_validator::Predicate::eq("b", json!(true)).unwrap(),
            ))
            .into(),
        Field::string(field_key!("b"))
            .visible_when(nebula_validator::Rule::predicate(
                nebula_validator::Predicate::eq("a", json!(true)).unwrap(),
            ))
            .into(),
    ]);

    let report = schema.lint();
    let cycle_paths: Vec<String> = report
        .errors()
        .filter(|e| e.code == "visibility_cycle")
        .map(|e| e.path.to_string())
        .collect();
    assert!(
        cycle_paths.iter().any(|p| p == "a" || p == "b"),
        "expected visibility_cycle anchored at field `a` or `b`, got {cycle_paths:?}"
    );
}

#[test]
fn runtime_validation_still_works_with_linted_schema() {
    let schema = Schema::builder()
        .add(Field::boolean(field_key!("enabled")).required())
        .add(
            Field::string(field_key!("name"))
                .required_when(nebula_validator::Rule::predicate(
                    nebula_validator::Predicate::eq("enabled", json!(true)).unwrap(),
                ))
                .min_length(3),
        )
        .build()
        .expect("valid schema");

    let mut values = FieldValues::new();
    values
        .try_set_raw("enabled", json!(true))
        .expect("test-only known-good key");
    values
        .try_set_raw("name", json!("ab"))
        .expect("test-only known-good key");

    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
}

#[test]
fn lint_schema_reports_rule_incompatible_warnings() {
    let schema = raw_schema(vec![
        Field::number(field_key!("retries"))
            .with_rule(nebula_validator::Rule::pattern("^\\d+$"))
            .with_rule(nebula_validator::Rule::email())
            .into(),
        Field::string(field_key!("name"))
            .with_rule(nebula_validator::Rule::Value(
                nebula_validator::ValueRule::Min(serde_json::Number::from(1)),
            ))
            .into(),
        Field::boolean(field_key!("flag"))
            .with_rule(nebula_validator::Rule::all([
                nebula_validator::Rule::max_length(10),
                nebula_validator::Rule::not(nebula_validator::Rule::min_items(1)),
            ]))
            .into(),
    ]);

    let report = schema.lint();
    assert!(
        has_warning(&report, "rule.incompatible", "retries"),
        "expected rule.incompatible warning at retries"
    );
    assert!(
        has_warning(&report, "rule.incompatible", "name"),
        "expected rule.incompatible warning at name"
    );
    assert!(
        has_warning(&report, "rule.incompatible", "flag"),
        "expected rule.incompatible warning at flag"
    );
}

#[test]
fn lint_schema_accepts_compatible_rule_types() {
    let schema = raw_schema(vec![
        Field::string(field_key!("title"))
            .min_length(3)
            .with_rule(nebula_validator::Rule::url())
            .into(),
        Field::number(field_key!("timeout"))
            .with_rule(nebula_validator::Rule::Value(
                nebula_validator::ValueRule::Min(serde_json::Number::from(1)),
            ))
            .into(),
        Field::list(field_key!("tags"))
            .item(Field::string(field_key!("tag")))
            .with_rule(nebula_validator::Rule::min_items(1))
            .into(),
        Field::select(field_key!("regions"))
            .multiple()
            .with_rule(nebula_validator::Rule::max_items(3))
            .into(),
    ]);

    let report = schema.lint();
    assert!(
        !has_warning(&report, "rule.incompatible", "title"),
        "compatible string rules should not be flagged: {:?}",
        report
            .warnings()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
    assert!(
        !has_warning(&report, "rule.incompatible", "timeout"),
        "compatible numeric rules should not be flagged"
    );
    assert!(
        !has_warning(&report, "rule.incompatible", "tags"),
        "compatible list rules should not be flagged"
    );
}

#[test]
fn lint_treats_blank_loader_key_as_missing_loader() {
    let schema = raw_schema(vec![
        Field::select(field_key!("region"))
            .dynamic()
            .loader("   ")
            .into(),
        Field::dynamic(field_key!("resource")).loader("").into(),
    ]);

    let report = schema.lint();
    assert!(
        has_warning(&report, "missing_loader", "region"),
        "blank select loader should be treated as missing"
    );
    assert!(
        has_warning(&report, "missing_loader", "resource"),
        "blank dynamic loader should be treated as missing"
    );
}

#[test]
fn lint_reports_duplicate_depends_on_entries() {
    let dependency = FieldPath::parse("team_id").unwrap();
    let schema = raw_schema(vec![
        Field::select(field_key!("workspace"))
            .dynamic()
            .loader("workspace_loader")
            .depends_on(dependency.clone())
            .depends_on(dependency)
            .into(),
    ]);

    let report = schema.lint();
    assert!(
        has_warning(&report, "duplicate_dependency", "workspace"),
        "expected duplicate_dependency warning, got: {:?}",
        report
            .warnings()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn load_select_options_blank_loader_emits_missing_config() {
    let schema = raw_schema(vec![
        Field::select(field_key!("region"))
            .dynamic()
            .loader(" ")
            .into(),
    ]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("region", FieldValues::new());
    let error = schema
        .load_select_options("region", &registry, context)
        .await
        .expect_err("blank loader config must fail");
    assert_eq!(error.code, "loader.missing_config");
}

#[tokio::test]
async fn load_dynamic_records_blank_loader_emits_missing_config() {
    let schema = raw_schema(vec![
        Field::dynamic(field_key!("resource")).loader(" ").into(),
    ]);
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("resource", FieldValues::new());
    let error = schema
        .load_dynamic_records("resource", &registry, context)
        .await
        .expect_err("blank loader config must fail");
    assert_eq!(error.code, "loader.missing_config");
}

#[test]
fn loader_dependency_cycle_detected() {
    // region depends_on cloud_provider, cloud_provider depends_on region -> cycle
    let schema = Schema::builder()
        .add(
            Field::select(field_key!("region"))
                .dynamic()
                .loader("region_loader")
                .depends_on(FieldPath::parse("cloud_provider").unwrap()),
        )
        .add(
            Field::select(field_key!("cloud_provider"))
                .dynamic()
                .loader("cloud_loader")
                .depends_on(FieldPath::parse("region").unwrap()),
        )
        .build()
        .expect_err("circular dependency must fail build");

    assert!(
        schema.errors().any(|e| e.code == "loader_dependency_cycle"),
        "expected loader_dependency_cycle error, got: {:?}",
        schema
            .errors()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn loader_dependency_no_cycle() {
    // cloud_provider has no depends_on, region depends_on cloud_provider -> no cycle
    // The build itself succeeds without a loader_dependency_cycle error.
    let result = Schema::builder()
        .add(
            Field::select(field_key!("cloud_provider"))
                .dynamic()
                .loader("cloud_loader"),
        )
        .add(
            Field::select(field_key!("region"))
                .dynamic()
                .loader("region_loader")
                .depends_on(FieldPath::parse("cloud_provider").unwrap()),
        )
        .build();

    assert!(
        result.is_ok(),
        "acyclic loader graph should build successfully, got: {:?}",
        result.as_ref().err().map(|r| r
            .errors()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>())
    );
}

#[test]
fn loader_dependency_transitive_cycle() {
    // A depends_on B, B depends_on C, C depends_on A -> transitive cycle
    let schema = Schema::builder()
        .add(
            Field::select(field_key!("a"))
                .dynamic()
                .loader("loader_a")
                .depends_on(FieldPath::parse("b").unwrap()),
        )
        .add(
            Field::select(field_key!("b"))
                .dynamic()
                .loader("loader_b")
                .depends_on(FieldPath::parse("c").unwrap()),
        )
        .add(
            Field::select(field_key!("c"))
                .dynamic()
                .loader("loader_c")
                .depends_on(FieldPath::parse("a").unwrap()),
        )
        .build()
        .expect_err("transitive circular dependency must fail build");

    assert!(
        schema.errors().any(|e| e.code == "loader_dependency_cycle"),
        "expected loader_dependency_cycle error for transitive cycle, got: {:?}",
        schema
            .errors()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn select_options_consistent_types_ok() {
    // All string options — no warning expected.
    let schema = raw_schema(vec![
        Field::select(field_key!("color"))
            .option(json!("red"), "Red")
            .option(json!("green"), "Green")
            .option(json!("blue"), "Blue")
            .into(),
    ]);

    let report = schema.lint();
    assert!(
        !has_warning(&report, "option.type_inconsistent", "color"),
        "consistent string options should not produce a warning, got: {:?}",
        report
            .warnings()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn select_options_mixed_types_warns() {
    // Mix of string and number option values — should warn.
    let schema = raw_schema(vec![
        Field::select(field_key!("mixed"))
            .option(json!("alpha"), "Alpha")
            .option(json!(1), "One")
            .into(),
    ]);

    let report = schema.lint();
    assert!(
        has_warning(&report, "option.type_inconsistent", "mixed"),
        "mixed-type options should produce option.type_inconsistent warning, got: {:?}",
        report
            .warnings()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn select_options_complex_value_without_multiple_warns() {
    // Non-multiple select with an array option value — should warn.
    let schema = raw_schema(vec![
        Field::select(field_key!("tags"))
            .option(json!(["a", "b"]), "Tags A+B")
            .option(json!(["c"]), "Tag C")
            .into(),
    ]);

    let report = schema.lint();
    assert!(
        has_warning(&report, "option.type_inconsistent", "tags"),
        "non-multiple select with array option value should warn, got: {:?}",
        report
            .warnings()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn select_options_multiple_with_array_values_ok() {
    // Multiple select with array option values — consistent type, no warning expected.
    let schema = raw_schema(vec![
        Field::select(field_key!("tags"))
            .option(json!(["a", "b"]), "Tags A+B")
            .option(json!(["c", "d"]), "Tags C+D")
            .multiple()
            .into(),
    ]);

    let report = schema.lint();
    assert!(
        !has_warning(&report, "option.type_inconsistent", "tags"),
        "multiple select with array option values should not produce option.type_inconsistent warning, got: {:?}",
        report
            .warnings()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn select_single_option_complex_type_warns() {
    // Non-multiple select with a single option whose value is an array — should warn.
    let schema = raw_schema(vec![
        Field::select(field_key!("data"))
            .option(json!(["x", "y"]), "X and Y")
            .into(),
    ]);

    let report = schema.lint();
    assert!(
        has_warning(&report, "option.type_inconsistent", "data"),
        "non-multiple select with single complex option value should warn, got: {:?}",
        report
            .warnings()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}
