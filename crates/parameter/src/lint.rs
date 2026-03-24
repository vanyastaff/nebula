//! Schema-time lint diagnostics.
//!
//! Provides a static lint pass over a [`Schema`] that detects structural
//! problems independent of any runtime values: duplicate field ids,
//! contradictory rules, dangling references, and integrity violations.

use std::collections::HashSet;

use crate::field::Field;
use crate::option::OptionSource;
use crate::rules::Rule;
use crate::schema::Schema;

/// A single lint finding emitted by [`lint_schema`].
#[derive(Debug, Clone, PartialEq)]
pub struct LintDiagnostic {
    /// Dot-separated path to the offending field or rule.
    pub path: String,
    /// Lint severity.
    pub level: LintLevel,
    /// Human-readable description of the problem.
    pub message: String,
}

/// Severity of a [`LintDiagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    /// Non-blocking advisory notice.
    Warning,
    /// Structural error that should be fixed before deployment.
    Error,
}

/// Runs the static lint pass over `schema` and returns all diagnostics found.
///
/// This is a schema-time check — it does not require runtime values.
#[must_use]
pub fn lint_schema(schema: &Schema) -> Vec<LintDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen_ids: Vec<String> = Vec::new();

    // Collect all field ids for reference checking.
    let all_ids = collect_field_ids(&schema.fields);

    for field in &schema.fields {
        lint_field(field, &mut seen_ids, &all_ids, &mut diagnostics);
    }

    lint_groups(schema, &all_ids, &mut diagnostics);

    diagnostics
}

/// Collects all top-level field IDs in the schema.
fn collect_field_ids(fields: &[Field]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for field in fields {
        ids.insert(field.meta().id.clone());
    }
    ids
}

fn lint_field(
    field: &Field,
    seen_ids: &mut Vec<String>,
    all_ids: &HashSet<String>,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let id = &field.meta().id;

    // Duplicate id check.
    if seen_ids.iter().any(|s| s == id) {
        diagnostics.push(LintDiagnostic {
            path: id.clone(),
            level: LintLevel::Error,
            message: format!("duplicate field id `{id}`"),
        });
    } else {
        seen_ids.push(id.clone());
    }

    // Empty id check.
    if id.is_empty() {
        diagnostics.push(LintDiagnostic {
            path: id.clone(),
            level: LintLevel::Error,
            message: "field id must not be empty".to_owned(),
        });
    }

    // Rule consistency checks.
    lint_rules(id, &field.meta().rules, diagnostics);

    // Dangling references in condition rules.
    lint_condition_refs(
        id,
        "visible_when",
        &field.meta().visible_when,
        all_ids,
        diagnostics,
    );
    lint_condition_refs(
        id,
        "required_when",
        &field.meta().required_when,
        all_ids,
        diagnostics,
    );
    lint_condition_refs(
        id,
        "disabled_when",
        &field.meta().disabled_when,
        all_ids,
        diagnostics,
    );

    // Self-reference in depends_on.
    lint_depends_on(field, all_ids, diagnostics);

    // Recurse into nested fields.
    match field {
        Field::Object { fields: nested, .. } => {
            let mut nested_seen: Vec<String> = Vec::new();
            for child in nested {
                lint_field(child, &mut nested_seen, all_ids, diagnostics);
            }
        }
        Field::List { item, .. } => {
            let mut item_seen: Vec<String> = Vec::new();
            lint_field(item, &mut item_seen, all_ids, diagnostics);
        }
        Field::Mode { variants, .. } => {
            lint_mode_variant_keys(field, variants, diagnostics);
            for variant in variants {
                let mut variant_seen: Vec<String> = Vec::new();
                lint_field(&variant.content, &mut variant_seen, all_ids, diagnostics);
            }
        }
        _ => {}
    }
}

/// Check that condition rule references point to existing field ids.
fn lint_condition_refs(
    field_id: &str,
    condition_name: &str,
    condition: &Option<Rule>,
    all_ids: &HashSet<String>,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let Some(rule) = condition else { return };
    let mut refs = Vec::new();
    rule.field_references(&mut refs);
    for referenced in refs {
        if !all_ids.contains(referenced) {
            diagnostics.push(LintDiagnostic {
                path: field_id.to_owned(),
                level: LintLevel::Error,
                message: format!("`{condition_name}` references unknown field `{referenced}`"),
            });
        }
    }
}

/// Check depends_on references for DynamicFields and Dynamic OptionSource.
fn lint_depends_on(
    field: &Field,
    all_ids: &HashSet<String>,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let id = &field.meta().id;

    let depends_on: Option<&[String]> = match field {
        Field::DynamicFields { depends_on, .. } => Some(depends_on),
        Field::Select {
            source: OptionSource::Dynamic { depends_on, .. },
            ..
        } => Some(depends_on),
        _ => None,
    };

    let Some(deps) = depends_on else { return };
    for dep in deps {
        if dep == id {
            diagnostics.push(LintDiagnostic {
                path: id.clone(),
                level: LintLevel::Error,
                message: format!("`depends_on` contains self-reference `{dep}`"),
            });
        } else if !all_ids.contains(dep) {
            diagnostics.push(LintDiagnostic {
                path: id.clone(),
                level: LintLevel::Error,
                message: format!("`depends_on` references unknown field `{dep}`"),
            });
        }
    }
}

/// Check Mode variant key uniqueness.
fn lint_mode_variant_keys(
    field: &Field,
    variants: &[crate::spec::ModeVariant],
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let id = &field.meta().id;
    let mut seen_keys = HashSet::new();
    for variant in variants {
        if !seen_keys.insert(&variant.key) {
            diagnostics.push(LintDiagnostic {
                path: id.clone(),
                level: LintLevel::Error,
                message: format!("duplicate mode variant key `{}`", variant.key),
            });
        }
    }
}

/// Check Group.fields integrity — every referenced field must exist.
fn lint_groups(schema: &Schema, all_ids: &HashSet<String>, diagnostics: &mut Vec<LintDiagnostic>) {
    for group in &schema.groups {
        let mut seen_in_group = HashSet::new();
        for field_id in &group.fields {
            if !all_ids.contains(field_id) {
                diagnostics.push(LintDiagnostic {
                    path: format!("group:{}", group.label),
                    level: LintLevel::Error,
                    message: format!("group references unknown field `{field_id}`"),
                });
            }
            if !seen_in_group.insert(field_id) {
                diagnostics.push(LintDiagnostic {
                    path: format!("group:{}", group.label),
                    level: LintLevel::Warning,
                    message: format!("group contains duplicate field `{field_id}`"),
                });
            }
        }
    }
}

fn lint_rules(path: &str, rules: &[Rule], diagnostics: &mut Vec<LintDiagnostic>) {
    // Check for contradictory MinLength / MaxLength pair.
    let min_len = rules.iter().find_map(|r| {
        if let Rule::MinLength { min, .. } = r {
            Some(*min)
        } else {
            None
        }
    });
    let max_len = rules.iter().find_map(|r| {
        if let Rule::MaxLength { max, .. } = r {
            Some(*max)
        } else {
            None
        }
    });
    if let (Some(min), Some(max)) = (min_len, max_len)
        && min > max
    {
        diagnostics.push(LintDiagnostic {
            path: path.to_owned(),
            level: LintLevel::Error,
            message: format!("contradictory rules: min_length ({min}) > max_length ({max})"),
        });
    }

    // Check for contradictory MinItems / MaxItems pair.
    let min_items = rules.iter().find_map(|r| {
        if let Rule::MinItems { min, .. } = r {
            Some(*min)
        } else {
            None
        }
    });
    let max_items = rules.iter().find_map(|r| {
        if let Rule::MaxItems { max, .. } = r {
            Some(*max)
        } else {
            None
        }
    });
    if let (Some(min), Some(max)) = (min_items, max_items)
        && min > max
    {
        diagnostics.push(LintDiagnostic {
            path: path.to_owned(),
            level: LintLevel::Error,
            message: format!("contradictory rules: min_items ({min}) > max_items ({max})"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::Field;
    use crate::metadata::FieldMetadata;
    use crate::option::OptionSource;
    use crate::schema::Schema;
    use crate::spec::ModeVariant;

    #[test]
    fn lint_detects_duplicate_ids() {
        let schema = Schema::new()
            .field(Field::text("name").with_label("Name"))
            .field(Field::text("name").with_label("Name Dup"));

        let diagnostics = lint_schema(&schema);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.level == LintLevel::Error && d.message.contains("duplicate"))
        );
    }

    #[test]
    fn lint_detects_contradictory_length_rules() {
        use crate::rules::Rule;

        let field = Field::text("title")
            .with_label("Title")
            .with_rule(Rule::MinLength {
                min: 10,
                message: None,
            })
            .with_rule(Rule::MaxLength {
                max: 5,
                message: None,
            });

        let schema = Schema::new().field(field);
        let diagnostics = lint_schema(&schema);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.level == LintLevel::Error && d.message.contains("min_length"))
        );
    }

    #[test]
    fn lint_clean_schema_returns_empty() {
        let schema = Schema::new()
            .field(Field::text("email").with_label("Email").required())
            .field(Field::integer("age").with_label("Age"));

        let diagnostics = lint_schema(&schema);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn lint_detects_dangling_visible_when_reference() {
        let schema = Schema::new()
            .field(Field::text("name").with_label("Name"))
            .field(
                Field::text("token")
                    .with_label("Token")
                    .visible_when(Rule::Eq {
                        field: "nonexistent".to_owned(),
                        value: serde_json::json!("yes"),
                    }),
            );

        let diagnostics = lint_schema(&schema);
        assert!(diagnostics.iter().any(|d| d.level == LintLevel::Error
            && d.message.contains("nonexistent")
            && d.message.contains("visible_when")));
    }

    #[test]
    fn lint_detects_dangling_required_when_reference() {
        let schema = Schema::new().field(Field::text("token").with_label("Token").required_when(
            Rule::Set {
                field: "missing_field".to_owned(),
            },
        ));

        let diagnostics = lint_schema(&schema);
        assert!(diagnostics.iter().any(|d| d.level == LintLevel::Error
            && d.message.contains("missing_field")
            && d.message.contains("required_when")));
    }

    #[test]
    fn lint_detects_depends_on_unknown_field() {
        let schema = Schema::new().field(Field::DynamicFields {
            meta: FieldMetadata::new("dynamic"),
            provider: "test.provider".to_owned(),
            depends_on: vec!["ghost_field".to_owned()],
            mode: Default::default(),
            unknown_field_policy: Default::default(),
            loader: None,
        });

        let diagnostics = lint_schema(&schema);
        assert!(diagnostics.iter().any(|d| d.level == LintLevel::Error
            && d.message.contains("ghost_field")
            && d.message.contains("depends_on")));
    }

    #[test]
    fn lint_detects_depends_on_self_reference() {
        let schema = Schema::new()
            .field(Field::text("sheet_id").with_label("Sheet"))
            .field(Field::DynamicFields {
                meta: FieldMetadata::new("cols"),
                provider: "sheets.columns".to_owned(),
                depends_on: vec!["cols".to_owned()],
                mode: Default::default(),
                unknown_field_policy: Default::default(),
                loader: None,
            });

        let diagnostics = lint_schema(&schema);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.level == LintLevel::Error && d.message.contains("self-reference"))
        );
    }

    #[test]
    fn lint_detects_group_unknown_field() {
        let schema = Schema::new()
            .field(Field::text("name").with_label("Name"))
            .group(crate::schema::Group {
                label: "Main".to_owned(),
                fields: vec!["name".to_owned(), "nonexistent".to_owned()],
                collapsed: false,
            });

        let diagnostics = lint_schema(&schema);
        assert!(diagnostics.iter().any(|d| d.level == LintLevel::Error
            && d.message.contains("nonexistent")
            && d.message.contains("group")));
    }

    #[test]
    fn lint_detects_duplicate_mode_variant_keys() {
        let schema = Schema::new().field(Field::Mode {
            meta: FieldMetadata::new("auth"),
            variants: vec![
                ModeVariant {
                    key: "bearer".to_owned(),
                    label: "Bearer".to_owned(),
                    description: None,
                    content: Box::new(Field::text("token").with_label("Token")),
                },
                ModeVariant {
                    key: "bearer".to_owned(),
                    label: "Bearer Dup".to_owned(),
                    description: None,
                    content: Box::new(Field::text("token2").with_label("Token2")),
                },
            ],
            default_variant: None,
        });

        let diagnostics = lint_schema(&schema);
        assert!(
            diagnostics.iter().any(
                |d| d.level == LintLevel::Error && d.message.contains("duplicate mode variant")
            )
        );
    }

    #[test]
    fn lint_valid_condition_references_pass() {
        let schema = Schema::new()
            .field(Field::text("auth_mode").with_label("Auth"))
            .field(
                Field::text("token")
                    .with_label("Token")
                    .visible_when(Rule::Eq {
                        field: "auth_mode".to_owned(),
                        value: serde_json::json!("bearer"),
                    }),
            );

        let diagnostics = lint_schema(&schema);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn lint_detects_dynamic_select_depends_on_unknown() {
        let schema = Schema::new().field(Field::Select {
            meta: FieldMetadata::new("region"),
            source: OptionSource::Dynamic {
                provider: "load_regions".to_owned(),
                depends_on: vec!["missing_dep".to_owned()],
            },
            multiple: false,
            allow_custom: false,
            searchable: false,
            loader: None,
        });

        let diagnostics = lint_schema(&schema);
        assert!(diagnostics.iter().any(|d| d.level == LintLevel::Error
            && d.message.contains("missing_dep")
            && d.message.contains("depends_on")));
    }
}
