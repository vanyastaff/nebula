//! Build-time structural lints.

use std::collections::HashSet;

use nebula_validator::Rule;

use crate::{
    Field, FieldPath, ListField, ModeField, RequiredMode, VisibilityMode,
    error::{ValidationError, ValidationReport},
    path::PathSegment,
};

/// Build-time lint entry point used by `SchemaBuilder::build()`.
///
/// Walks the field tree rooted at `prefix` and appends `ValidationError`
/// issues to `report`. Errors block the build; warnings are advisory.
pub(crate) fn lint_tree(fields: &[Field], prefix: &FieldPath, report: &mut ValidationReport) {
    // Collect root-level key set for cross-reference checks.
    let root_keys: HashSet<&str> = fields.iter().map(|f| f.key().as_str()).collect();
    lint_fields_new(fields, prefix, &root_keys, report);
    lint_visibility_cycles_new(fields, prefix, report);
}

fn lint_fields_new(
    fields: &[Field],
    prefix: &FieldPath,
    root_keys: &HashSet<&str>,
    report: &mut ValidationReport,
) {
    // Pass 1: duplicate keys in this scope.
    let mut seen: HashSet<&str> = HashSet::new();
    for field in fields {
        let key = field.key().as_str();
        if !seen.insert(key) {
            let path = prefix.clone().join(field.key().clone());
            report.push(
                ValidationError::new("duplicate_key")
                    .at(path)
                    .message(format!("duplicate field key `{key}`"))
                    .build(),
            );
        }
    }

    // Pass 2: per-field checks.
    let local_keys: HashSet<&str> = fields.iter().map(|f| f.key().as_str()).collect();
    for field in fields {
        let path = prefix.clone().join(field.key().clone());

        // Rule type compatibility.
        lint_rule_compat_new(field, field.rules(), &path, report);

        // Visibility rule references.
        lint_rule_refs_new(
            field_visible_rule(field),
            &path,
            &local_keys,
            root_keys,
            report,
        );
        // Required rule references.
        lint_rule_refs_new(
            field_required_rule(field),
            &path,
            &local_keys,
            root_keys,
            report,
        );
        // Rules list references.
        for rule in field.rules() {
            lint_rule_refs_new(Some(rule), &path, &local_keys, root_keys, report);
        }

        // Contradictory rules (best-effort warning).
        lint_contradictory_rules_new(field.rules(), &path, report);

        // Field-type-specific checks.
        match field {
            Field::Select(select) => {
                lint_depends_on_new(
                    &select.depends_on,
                    field.key().as_str(),
                    &path,
                    &local_keys,
                    root_keys,
                    report,
                );
                if select.dynamic && select.loader.is_none() {
                    report.push(
                        ValidationError::new("missing_loader")
                            .at(path.clone())
                            .message("dynamic select has no loader key configured")
                            .warn()
                            .build(),
                    );
                }
                if !select.dynamic && select.loader.is_some() {
                    report.push(
                        ValidationError::new("loader_without_dynamic")
                            .at(path.clone())
                            .message("select has loader key but dynamic flag is disabled")
                            .warn()
                            .build(),
                    );
                }
            },
            Field::Dynamic(dynamic) => {
                lint_depends_on_new(
                    &dynamic.depends_on,
                    field.key().as_str(),
                    &path,
                    &local_keys,
                    root_keys,
                    report,
                );
                if dynamic.loader.is_none() {
                    report.push(
                        ValidationError::new("missing_loader")
                            .at(path.clone())
                            .message("dynamic field has no loader key configured")
                            .warn()
                            .build(),
                    );
                }
            },
            Field::List(list) => {
                lint_list_new(list, &path, root_keys, report);
            },
            Field::Object(obj) => {
                lint_fields_new(&obj.fields, &path, root_keys, report);
            },
            Field::Mode(mode) => {
                lint_mode_new(mode, &path, root_keys, report);
            },
            Field::Notice(notice) => {
                if !matches!(notice.required, RequiredMode::Never)
                    || notice.default.is_some()
                    || !notice.rules.is_empty()
                    || !notice.transformers.is_empty()
                {
                    report.push(
                        ValidationError::new("notice.misuse")
                            .at(path.clone())
                            .message(
                                "notice field should stay display-only \
                                 (no required/default/rules/transformers)",
                            )
                            .warn()
                            .build(),
                    );
                }
                if notice.description.is_none() {
                    report.push(
                        ValidationError::new("notice_missing_description")
                            .at(path.clone())
                            .message("notice field should include description text")
                            .warn()
                            .build(),
                    );
                }
            },
            _ => {},
        }
    }
}

fn lint_list_new(
    list: &ListField,
    path: &FieldPath,
    root_keys: &HashSet<&str>,
    report: &mut ValidationReport,
) {
    if list.item.is_none() {
        report.push(
            ValidationError::new("missing_item_schema")
                .at(path.clone())
                .message("list field must define item schema")
                .build(),
        );
        return;
    }
    if let Some(Field::Object(obj)) = list.item.as_deref() {
        lint_fields_new(&obj.fields, path, root_keys, report);
    }
}

fn lint_mode_new(
    mode: &ModeField,
    path: &FieldPath,
    root_keys: &HashSet<&str>,
    report: &mut ValidationReport,
) {
    if let Some(default_variant) = mode.default_variant.as_deref()
        && !mode.variants.iter().any(|v| v.key == default_variant)
    {
        report.push(
            ValidationError::new("invalid_default_variant")
                .at(path.clone())
                .message(format!(
                    "default variant `{default_variant}` does not exist in mode variants"
                ))
                .build(),
        );
    }

    let mut seen: HashSet<&str> = HashSet::new();
    for variant in &mode.variants {
        if !seen.insert(variant.key.as_str()) {
            report.push(
                ValidationError::new("duplicate_variant")
                    .at(path.clone())
                    .message(format!("duplicate mode variant key `{}`", variant.key))
                    .build(),
            );
        }
        if variant.label.trim().is_empty() {
            // Build variant path for precise location.
            if let Ok(vk) = crate::key::FieldKey::new(variant.key.as_str()) {
                let vpath = path.clone().join(vk);
                report.push(
                    ValidationError::new("missing_variant_label")
                        .at(vpath)
                        .message("mode variant label is empty")
                        .warn()
                        .build(),
                );
            }
        }
        // Recurse into variant payload.
        if let Field::Object(obj) = variant.field.as_ref()
            && let Ok(vk) = crate::key::FieldKey::new(variant.key.as_str())
        {
            let vpath = path.clone().join(vk);
            lint_fields_new(&obj.fields, &vpath, root_keys, report);
        }
    }
}

fn lint_depends_on_new(
    depends_on: &[FieldPath],
    field_key: &str,
    path: &FieldPath,
    local_keys: &HashSet<&str>,
    root_keys: &HashSet<&str>,
    report: &mut ValidationReport,
) {
    for dependency in depends_on {
        let first_key = dependency.segments().iter().find_map(|s| {
            if let PathSegment::Key(k) = s {
                Some(k.as_str())
            } else {
                None
            }
        });

        if first_key == Some(field_key) {
            report.push(
                ValidationError::new("self_dependency")
                    .at(path.clone())
                    .message(format!("depends_on contains self reference `{dependency}`"))
                    .build(),
            );
            continue;
        }

        if dependency.is_root() {
            report.push(
                ValidationError::new("dangling_reference")
                    .at(path.clone())
                    .message("depends_on references an empty path")
                    .build(),
            );
            continue;
        }

        let root_key = first_key.unwrap_or_default();
        if !local_keys.contains(root_key) && !root_keys.contains(root_key) {
            report.push(
                ValidationError::new("dangling_reference")
                    .at(path.clone())
                    .message(format!("depends_on references unknown key `{root_key}`"))
                    .build(),
            );
        }
    }
}

fn lint_rule_refs_new(
    maybe_rule: Option<&Rule>,
    path: &FieldPath,
    local_keys: &HashSet<&str>,
    root_keys: &HashSet<&str>,
    report: &mut ValidationReport,
) {
    let Some(rule) = maybe_rule else { return };
    let mut refs = Vec::new();
    rule.field_references(&mut refs);
    for field_ref in refs {
        if let Some(rp) = field_ref.strip_prefix("$root.") {
            let rk = rp.split('.').next().unwrap_or_default();
            if !root_keys.contains(rk) {
                report.push(
                    ValidationError::new("dangling_reference")
                        .at(path.clone())
                        .message(format!("rule references unknown root key `{rk}`"))
                        .build(),
                );
            }
            continue;
        }
        let lk = field_ref.split('.').next().unwrap_or_default();
        if !local_keys.contains(lk) {
            report.push(
                ValidationError::new("dangling_reference")
                    .at(path.clone())
                    .message(format!("rule references unknown local key `{lk}`"))
                    .build(),
            );
        }
    }
}

fn lint_rule_compat_new(
    field: &Field,
    rules: &[Rule],
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    for rule in rules {
        lint_single_compat_new(field, rule, path, report);
    }
}

fn lint_single_compat_new(
    field: &Field,
    rule: &Rule,
    path: &FieldPath,
    report: &mut ValidationReport,
) {
    let compatible = match rule {
        Rule::Pattern { .. }
        | Rule::MinLength { .. }
        | Rule::MaxLength { .. }
        | Rule::Email { .. }
        | Rule::Url { .. } => supports_string_rules(field),
        Rule::Min { .. } | Rule::Max { .. } => supports_number_rules(field),
        Rule::MinItems { .. } | Rule::MaxItems { .. } => supports_collection_rules(field),
        Rule::All { rules } | Rule::Any { rules } => {
            for nested in rules {
                lint_single_compat_new(field, nested, path, report);
            }
            true
        },
        Rule::Not { inner } => {
            lint_single_compat_new(field, inner, path, report);
            true
        },
        _ => true,
    };

    if !compatible {
        report.push(
            ValidationError::new("rule.incompatible")
                .at(path.clone())
                .message(format!(
                    "rule `{}` is not compatible with `{}` field",
                    rule_name(rule),
                    field_type_name(field)
                ))
                .warn()
                .build(),
        );
    }
}

fn lint_contradictory_rules_new(rules: &[Rule], path: &FieldPath, report: &mut ValidationReport) {
    let mut min_length: Option<usize> = None;
    let mut max_length: Option<usize> = None;
    let mut min_items: Option<usize> = None;
    let mut max_items: Option<usize> = None;
    collect_min_max(
        rules,
        &mut min_length,
        &mut max_length,
        &mut min_items,
        &mut max_items,
    );

    if let (Some(min), Some(max)) = (min_length, max_length)
        && min > max
    {
        report.push(
            ValidationError::new("rule.contradictory")
                .at(path.clone())
                .message(format!(
                    "min_length ({min}) is greater than max_length ({max})"
                ))
                .build(),
        );
    }
    if let (Some(min), Some(max)) = (min_items, max_items)
        && min > max
    {
        report.push(
            ValidationError::new("rule.contradictory")
                .at(path.clone())
                .message(format!(
                    "min_items ({min}) is greater than max_items ({max})"
                ))
                .build(),
        );
    }
}

fn field_visible_rule(field: &Field) -> Option<&Rule> {
    match field.visible() {
        VisibilityMode::Always | VisibilityMode::Never => None,
        VisibilityMode::When(rule) => Some(rule),
    }
}

fn field_required_rule(field: &Field) -> Option<&Rule> {
    match field.required() {
        RequiredMode::Never | RequiredMode::Always => None,
        RequiredMode::When(rule) => Some(rule),
    }
}

fn supports_string_rules(field: &Field) -> bool {
    matches!(
        field,
        Field::String(_) | Field::Secret(_) | Field::Code(_) | Field::File(_)
    )
}

fn supports_number_rules(field: &Field) -> bool {
    matches!(field, Field::Number(_))
}

fn supports_collection_rules(field: &Field) -> bool {
    match field {
        Field::List(_) => true,
        Field::Select(select) => select.multiple,
        Field::File(file) => file.multiple,
        _ => false,
    }
}

fn field_type_name(field: &Field) -> &'static str {
    field.type_name()
}

fn rule_name(rule: &Rule) -> &'static str {
    match rule {
        Rule::Pattern { .. } => "pattern",
        Rule::MinLength { .. } => "min_length",
        Rule::MaxLength { .. } => "max_length",
        Rule::Min { .. } => "min",
        Rule::Max { .. } => "max",
        Rule::OneOf { .. } => "one_of",
        Rule::MinItems { .. } => "min_items",
        Rule::MaxItems { .. } => "max_items",
        Rule::Email { .. } => "email",
        Rule::Url { .. } => "url",
        Rule::UniqueBy { .. } => "unique_by",
        Rule::Custom { .. } => "custom",
        Rule::Eq { .. } => "eq",
        Rule::Ne { .. } => "ne",
        Rule::Gt { .. } => "gt",
        Rule::Gte { .. } => "gte",
        Rule::Lt { .. } => "lt",
        Rule::Lte { .. } => "lte",
        Rule::IsTrue { .. } => "is_true",
        Rule::IsFalse { .. } => "is_false",
        Rule::Set { .. } => "set",
        Rule::Empty { .. } => "empty",
        Rule::Contains { .. } => "contains",
        Rule::Matches { .. } => "matches",
        Rule::In { .. } => "in",
        Rule::All { .. } => "all",
        Rule::Any { .. } => "any",
        Rule::Not { .. } => "not",
        _ => "unknown_rule",
    }
}

fn collect_min_max(
    rules: &[Rule],
    min_length: &mut Option<usize>,
    max_length: &mut Option<usize>,
    min_items: &mut Option<usize>,
    max_items: &mut Option<usize>,
) {
    for rule in rules {
        match rule {
            Rule::MinLength { min, .. } => {
                *min_length = Some(min_length.map_or(*min, |current| current.max(*min)));
            },
            Rule::MaxLength { max, .. } => {
                *max_length = Some(max_length.map_or(*max, |current| current.min(*max)));
            },
            Rule::MinItems { min, .. } => {
                *min_items = Some(min_items.map_or(*min, |current| current.max(*min)));
            },
            Rule::MaxItems { max, .. } => {
                *max_items = Some(max_items.map_or(*max, |current| current.min(*max)));
            },
            Rule::All { rules } | Rule::Any { rules } => {
                collect_min_max(rules, min_length, max_length, min_items, max_items);
            },
            Rule::Not { inner } => {
                collect_min_max(
                    std::slice::from_ref(inner.as_ref()),
                    min_length,
                    max_length,
                    min_items,
                    max_items,
                );
            },
            _ => {},
        }
    }
}

fn emit_visibility_cycle(start: &str, prefix: &FieldPath, report: &mut ValidationReport) {
    let cycle_path = crate::key::FieldKey::new(start)
        .map(|k| prefix.clone().join(k))
        .unwrap_or_else(|_| prefix.clone());
    report.push(
        ValidationError::new("visibility_cycle")
            .at(cycle_path)
            .message(format!(
                "visibility rule graph contains cycle involving `{start}`"
            ))
            .build(),
    );
}

fn lint_visibility_cycles_new(fields: &[Field], prefix: &FieldPath, report: &mut ValidationReport) {
    // TODO(task-26): add full visibility_cycle detection using FieldPath-aware graph
    // For now, delegate to key-based cycle detection.
    let mut edges: Vec<(&str, &str)> = Vec::new();
    for field in fields {
        let source = field.key().as_str();
        if let Some(rule) = field_visible_rule(field) {
            let mut refs: Vec<&str> = Vec::new();
            rule.field_references(&mut refs);
            for target in refs {
                if target.starts_with("$root.") {
                    continue;
                }
                let target = target.split('.').next().unwrap_or_default();
                edges.push((source, target));
            }
        }
    }

    for (start, _) in &edges {
        let mut stack = vec![*start];
        let mut visited = HashSet::new();
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            for (edge_from, edge_to) in &edges {
                if *edge_from != current {
                    continue;
                }
                if *edge_to == *start {
                    emit_visibility_cycle(start, prefix, report);
                    return;
                }
                stack.push(edge_to);
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FieldKey, error::ValidationReport, field::Field, path::FieldPath};

    fn run(fields: Vec<Field>) -> ValidationReport {
        let mut report = ValidationReport::new();
        lint_tree(&fields, &FieldPath::root(), &mut report);
        report
    }

    #[test]
    fn detects_duplicate_key() {
        let fields = vec![
            Field::string(FieldKey::new("x").unwrap()).into_field(),
            Field::number(FieldKey::new("x").unwrap()).into_field(),
        ];
        let report = run(fields);
        assert!(report.errors().any(|e| e.code == "duplicate_key"));
    }

    #[test]
    fn passes_clean_fields() {
        let fields = vec![
            Field::string(FieldKey::new("a").unwrap()).into_field(),
            Field::number(FieldKey::new("b").unwrap()).into_field(),
        ];
        let report = run(fields);
        assert!(!report.has_errors());
    }

    #[test]
    fn detects_missing_item_schema() {
        let fields = vec![Field::list(FieldKey::new("items").unwrap()).into_field()];
        let report = run(fields);
        assert!(report.errors().any(|e| e.code == "missing_item_schema"));
    }

    #[test]
    fn detects_invalid_default_variant() {
        let fields = vec![
            Field::mode(FieldKey::new("m").unwrap())
                .default_variant("nonexistent")
                .into_field(),
        ];
        let report = run(fields);
        assert!(report.errors().any(|e| e.code == "invalid_default_variant"));
    }

    #[test]
    fn detects_duplicate_variant() {
        let fields = vec![
            Field::mode(FieldKey::new("m").unwrap())
                .variant("v1", "V1", Field::string(FieldKey::new("x").unwrap()))
                .variant("v1", "V1 again", Field::string(FieldKey::new("y").unwrap()))
                .into_field(),
        ];
        let report = run(fields);
        assert!(report.errors().any(|e| e.code == "duplicate_variant"));
    }
}
