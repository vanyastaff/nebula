use nebula_schema::{
    Field, FieldPath, FieldValues, LoaderContext, LoaderRegistry, LoaderResult, Schema,
    ValidationReport,
};
use serde_json::json;

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
    let schema = Schema::new()
        .add(Field::string("toggle"))
        .add(
            Field::string("name")
                .visible_when(nebula_validator::Rule::Eq {
                    field: "missing".to_owned(),
                    value: json!(true),
                })
                .with_rule(nebula_validator::Rule::MinLength {
                    min: 5,
                    message: None,
                })
                .with_rule(nebula_validator::Rule::MaxLength {
                    max: 2,
                    message: None,
                }),
        )
        .add(
            Field::select("region")
                .dynamic()
                .loader("regions_loader")
                .depends_on(FieldPath::parse("unknown_ref").unwrap()),
        )
        .add(
            Field::mode("auth")
                .variant("token", "", Field::secret("token"))
                .default_variant("missing_variant"),
        );

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
    let schema = Schema::new()
        .add(
            Field::select("workspace")
                .dynamic()
                .loader("workspace_loader")
                .depends_on(FieldPath::parse("team_id").unwrap()),
        )
        .add(
            Field::dynamic("resource")
                .loader("resource_loader")
                .depends_on(FieldPath::parse("workspace").unwrap()),
        );

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
    values.set_raw("workspace", json!("ws_1"));
    values.set_raw("team_id", json!("team_1"));
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
async fn loader_registry_reports_missing_loader_registration() {
    let schema = Schema::new().add(Field::select("region").dynamic().loader("missing_loader"));
    let registry = LoaderRegistry::new();
    let context = LoaderContext::new("region", FieldValues::new());
    let error = schema
        .load_select_options("region", &registry, context)
        .await
        .expect_err("missing loader must fail");
    assert!(error.to_string().contains("missing_loader"));
}

#[test]
fn lint_schema_detects_visibility_cycles() {
    let schema = Schema::new()
        .add(Field::string("a").visible_when(nebula_validator::Rule::Eq {
            field: "b".to_owned(),
            value: json!(true),
        }))
        .add(Field::string("b").visible_when(nebula_validator::Rule::Eq {
            field: "a".to_owned(),
            value: json!(true),
        }));

    let report = schema.lint();
    assert!(
        has_error(&report, "visibility_cycle", "a"),
        "expected visibility_cycle at a, got: {:?}",
        report
            .errors()
            .map(|e| (&e.code, e.path.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn runtime_validation_still_works_with_linted_schema() {
    let schema = Schema::builder()
        .add(Field::boolean("enabled").required())
        .add(
            Field::string("name")
                .required_when(nebula_validator::Rule::Eq {
                    field: "enabled".to_owned(),
                    value: json!(true),
                })
                .min_length(3),
        )
        .build()
        .expect("valid schema");

    let mut values = FieldValues::new();
    values.set_raw("enabled", json!(true));
    values.set_raw("name", json!("ab"));

    let report = schema.validate(&values).unwrap_err();
    assert!(report.has_errors());
}

#[test]
fn lint_schema_reports_rule_incompatible_warnings() {
    let schema = Schema::new()
        .add(
            Field::number("retries")
                .with_rule(nebula_validator::Rule::Pattern {
                    pattern: "^\\d+$".to_owned(),
                    message: None,
                })
                .with_rule(nebula_validator::Rule::Email { message: None }),
        )
        .add(
            Field::string("name").with_rule(nebula_validator::Rule::Min {
                min: serde_json::Number::from(1),
                message: None,
            }),
        )
        .add(
            Field::boolean("flag").with_rule(nebula_validator::Rule::All {
                rules: vec![
                    nebula_validator::Rule::MaxLength {
                        max: 10,
                        message: None,
                    },
                    nebula_validator::Rule::Not {
                        inner: Box::new(nebula_validator::Rule::MinItems {
                            min: 1,
                            message: None,
                        }),
                    },
                ],
            }),
        );

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
    let schema = Schema::new()
        .add(
            Field::string("title")
                .min_length(3)
                .with_rule(nebula_validator::Rule::Url { message: None }),
        )
        .add(
            Field::number("timeout").with_rule(nebula_validator::Rule::Min {
                min: serde_json::Number::from(1),
                message: None,
            }),
        )
        .add(Field::list("tags").item(Field::string("tag")).with_rule(
            nebula_validator::Rule::MinItems {
                min: 1,
                message: None,
            },
        ))
        .add(
            Field::select("regions")
                .multiple()
                .with_rule(nebula_validator::Rule::MaxItems {
                    max: 3,
                    message: None,
                }),
        );

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
