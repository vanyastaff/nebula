use nebula_schema::{
    ExecutionMode, Field, FieldPath, FieldValues, LintLevel, LoaderContext, LoaderRegistry,
    LoaderResult, Schema,
};
use serde_json::json;

fn has_lint(
    report: &nebula_schema::LintReport,
    level: LintLevel,
    code: &str,
    path_prefix: &str,
) -> bool {
    report
        .diagnostics()
        .iter()
        .any(|diag| diag.level == level && diag.code == code && diag.path.starts_with(path_prefix))
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
    assert!(has_lint(
        &report,
        LintLevel::Error,
        "dangling_reference",
        "name.visible"
    ));
    assert!(has_lint(
        &report,
        LintLevel::Error,
        "rule.contradictory",
        "name.rules"
    ));
    assert!(has_lint(
        &report,
        LintLevel::Error,
        "dangling_dependency",
        "region.depends_on"
    ));
    assert!(has_lint(
        &report,
        LintLevel::Error,
        "invalid_default_variant",
        "auth"
    ));
    assert!(has_lint(
        &report,
        LintLevel::Warning,
        "missing_variant_label",
        "auth.variants.token"
    ));
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
    assert!(has_lint(
        &report,
        LintLevel::Error,
        "visibility_cycle",
        "a.visible"
    ));
}

#[test]
fn runtime_validation_still_works_with_linted_schema() {
    let schema = Schema::new().add(Field::boolean("enabled").required()).add(
        Field::string("name")
            .required_when(nebula_validator::Rule::Eq {
                field: "enabled".to_owned(),
                value: json!(true),
            })
            .min_length(3),
    );
    let mut values = FieldValues::new();
    values.set_raw("enabled", json!(true));
    values.set_raw("name", json!("ab"));

    let report = schema.validate(&values, ExecutionMode::StaticOnly);
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
    assert!(has_lint(
        &report,
        LintLevel::Warning,
        "rule.incompatible",
        "retries.rules"
    ));
    assert!(has_lint(
        &report,
        LintLevel::Warning,
        "rule.incompatible",
        "name.rules"
    ));
    assert!(has_lint(
        &report,
        LintLevel::Warning,
        "rule.incompatible",
        "flag.rules"
    ));
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
        !has_lint(
            &report,
            LintLevel::Warning,
            "rule.incompatible",
            "title.rules"
        ),
        "compatible string rules should not be flagged: {:?}",
        report.diagnostics()
    );
    assert!(
        !has_lint(
            &report,
            LintLevel::Warning,
            "rule.incompatible",
            "timeout.rules"
        ),
        "compatible numeric rules should not be flagged: {:?}",
        report.diagnostics()
    );
    assert!(
        !has_lint(
            &report,
            LintLevel::Warning,
            "rule.incompatible",
            "tags.rules"
        ),
        "compatible list rules should not be flagged: {:?}",
        report.diagnostics()
    );
}
