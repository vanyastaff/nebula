use std::{borrow::Cow, collections::HashSet};

use nebula_validator::Rule;

use crate::{
    Field, FieldPath, ListField, ModeField, ObjectField, RequiredMode, Schema, VisibilityMode,
};

/// Severity level for lint diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    /// Advisory message; schema is still usable.
    Warning,
    /// Structural issue that should be fixed before usage.
    Error,
}

/// Single static lint diagnostic emitted for a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintDiagnostic {
    /// Dot path to the problematic field or rule.
    pub path: String,
    /// Stable machine-readable code for tooling.
    pub code: Cow<'static, str>,
    /// Severity level.
    pub level: LintLevel,
    /// Human-readable detail.
    pub message: String,
}

impl LintDiagnostic {
    fn new(
        path: impl Into<String>,
        code: impl Into<Cow<'static, str>>,
        level: LintLevel,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            code: code.into(),
            level,
            message: message.into(),
        }
    }
}

/// Collection of lint diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LintReport {
    diagnostics: Vec<LintDiagnostic>,
}

impl LintReport {
    /// Create an empty lint report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow all diagnostics.
    pub fn diagnostics(&self) -> &[LintDiagnostic] {
        self.diagnostics.as_slice()
    }

    /// Returns true when at least one error is present.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diag| diag.level == LintLevel::Error)
    }

    /// Returns true when at least one warning is present.
    pub fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diag| diag.level == LintLevel::Warning)
    }

    fn push(&mut self, diagnostic: LintDiagnostic) {
        self.diagnostics.push(diagnostic);
    }

    fn push_error(
        &mut self,
        path: impl Into<String>,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<String>,
    ) {
        self.push(LintDiagnostic::new(path, code, LintLevel::Error, message));
    }

    fn push_warning(
        &mut self,
        path: impl Into<String>,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<String>,
    ) {
        self.push(LintDiagnostic::new(path, code, LintLevel::Warning, message));
    }
}

/// Run static lint checks for a schema.
pub fn lint_schema(schema: &Schema) -> LintReport {
    let mut report = LintReport::new();
    let root_ids: HashSet<&str> = schema
        .fields()
        .iter()
        .map(|field| field.key().as_str())
        .collect();
    lint_fields(schema.fields(), "", &root_ids, &mut report);
    lint_visibility_cycles(schema.fields(), &mut report);
    report
}

fn lint_fields(fields: &[Field], prefix: &str, root_ids: &HashSet<&str>, report: &mut LintReport) {
    let local_ids: HashSet<&str> = fields.iter().map(|field| field.key().as_str()).collect();
    check_duplicate_keys(fields, prefix, report);

    for field in fields {
        let key = field.key().as_str();
        let path = make_path(prefix, key);
        lint_rule_type_compatibility(field, field.rules(), &format!("{path}.rules"), report);

        lint_rule_refs(
            field_visible_rule(field),
            &format!("{path}.visible"),
            &local_ids,
            root_ids,
            report,
        );
        lint_rule_refs(
            field_required_rule(field),
            &format!("{path}.required"),
            &local_ids,
            root_ids,
            report,
        );
        lint_rules_refs(
            field.rules(),
            &format!("{path}.rules"),
            &local_ids,
            root_ids,
            report,
        );
        lint_contradictory_rules(field.rules(), &format!("{path}.rules"), report);

        match field {
            Field::Select(select) => {
                lint_select_field(select, key, &path, &local_ids, root_ids, report)
            },
            Field::Dynamic(dynamic) => {
                lint_depends_on(
                    &dynamic.depends_on,
                    key,
                    &path,
                    &local_ids,
                    root_ids,
                    report,
                );
                if dynamic.loader.is_none() {
                    report.push_warning(
                        format!("{path}.loader"),
                        "missing_loader",
                        "dynamic field has no loader key configured",
                    );
                }
            },
            Field::List(list) => lint_list_field(list, &path, root_ids, report),
            Field::Object(object) => lint_object_field(object, &path, root_ids, report),
            Field::Mode(mode) => lint_mode_field(mode, &path, root_ids, report),
            Field::Notice(notice) => {
                if !matches!(notice.required, RequiredMode::Never)
                    || notice.default.is_some()
                    || !notice.rules.is_empty()
                    || !notice.transformers.is_empty()
                {
                    report.push_warning(
                        path.clone(),
                        "notice_misuse",
                        "notice field should stay display-only (no required/default/rules/transformers)",
                    );
                }
                if notice.description.is_none() {
                    report.push_warning(
                        path,
                        "notice_missing_description",
                        "notice field should include description text",
                    );
                }
            },
            _ => {},
        }
    }
}

fn lint_select_field(
    select: &crate::field::SelectField,
    key: &str,
    path: &str,
    local_ids: &HashSet<&str>,
    root_ids: &HashSet<&str>,
    report: &mut LintReport,
) {
    lint_depends_on(&select.depends_on, key, path, local_ids, root_ids, report);

    if select.dynamic && select.loader.is_none() {
        report.push_warning(
            format!("{path}.loader"),
            "missing_loader",
            "dynamic select has no loader key configured",
        );
    }

    if !select.dynamic && select.loader.is_some() {
        report.push_warning(
            format!("{path}.dynamic"),
            "loader_without_dynamic",
            "select has loader key but dynamic flag is disabled",
        );
    }
}

fn lint_list_field(
    list: &ListField,
    path: &str,
    root_ids: &HashSet<&str>,
    report: &mut LintReport,
) {
    if list.item.is_none() {
        report.push_error(
            format!("{path}.item"),
            "missing_item_schema",
            "list field must define item schema",
        );
        return;
    }
    if let Some(item) = list.item.as_deref() {
        lint_fields(
            std::slice::from_ref(item),
            &format!("{path}[]"),
            root_ids,
            report,
        );
    }
}

fn lint_object_field(
    object: &ObjectField,
    path: &str,
    root_ids: &HashSet<&str>,
    report: &mut LintReport,
) {
    lint_fields(&object.fields, path, root_ids, report);
}

fn lint_mode_field(
    mode: &ModeField,
    path: &str,
    root_ids: &HashSet<&str>,
    report: &mut LintReport,
) {
    if let Some(default_variant) = mode.default_variant.as_deref()
        && !mode
            .variants
            .iter()
            .any(|variant| variant.key == default_variant)
    {
        report.push_error(
            path.to_owned(),
            "invalid_default_variant",
            format!("default variant `{default_variant}` does not exist in mode variants"),
        );
    }

    let mut seen_keys = HashSet::new();
    for variant in &mode.variants {
        let variant_path = format!("{path}.variants.{}", variant.key);
        if !seen_keys.insert(variant.key.as_str()) {
            report.push_error(
                variant_path.clone(),
                "duplicate_variant",
                format!("duplicate mode variant key `{}`", variant.key),
            );
        }
        if variant.label.trim().is_empty() {
            report.push_warning(
                variant_path.clone(),
                "missing_variant_label",
                "mode variant label is empty",
            );
        }
        lint_fields(
            std::slice::from_ref(variant.field.as_ref()),
            &variant_path,
            root_ids,
            report,
        );
    }
}

fn lint_depends_on(
    depends_on: &[FieldPath],
    field_key: &str,
    path: &str,
    local_ids: &HashSet<&str>,
    root_ids: &HashSet<&str>,
    report: &mut LintReport,
) {
    use crate::path::PathSegment;

    for dependency in depends_on {
        let dep_display = dependency.to_string();
        let dep_path = format!("{path}.depends_on");

        // Detect self-reference by comparing the first key segment.
        let first_key = dependency.segments().iter().find_map(|s| {
            if let PathSegment::Key(k) = s {
                Some(k.as_str())
            } else {
                None
            }
        });

        if first_key == Some(field_key) {
            report.push_error(
                dep_path.clone(),
                "self_dependency",
                format!("depends_on contains self reference `{dep_display}`"),
            );
            continue;
        }

        if dependency.is_root() {
            // Root path means no segments — treat as dangling.
            report.push_error(
                dep_path,
                "dangling_dependency",
                "depends_on references an empty path",
            );
            continue;
        }

        // Check the first key segment against local IDs; fall back to root IDs.
        let root_key = first_key.unwrap_or_default();
        if local_ids.contains(root_key) {
            continue;
        }
        if root_ids.contains(root_key) {
            continue;
        }
        report.push_error(
            dep_path,
            "dangling_dependency",
            format!("depends_on references unknown key `{root_key}`"),
        );
    }
}

fn lint_rule_refs(
    maybe_rule: Option<&Rule>,
    path: &str,
    local_ids: &HashSet<&str>,
    root_ids: &HashSet<&str>,
    report: &mut LintReport,
) {
    let Some(rule) = maybe_rule else { return };
    let mut refs = Vec::new();
    rule.field_references(&mut refs);
    for field_ref in refs {
        check_ref(field_ref, path, local_ids, root_ids, report);
    }
}

fn lint_rules_refs(
    rules: &[Rule],
    path: &str,
    local_ids: &HashSet<&str>,
    root_ids: &HashSet<&str>,
    report: &mut LintReport,
) {
    for rule in rules {
        let mut refs = Vec::new();
        rule.field_references(&mut refs);
        for field_ref in refs {
            check_ref(field_ref, path, local_ids, root_ids, report);
        }
    }
}

fn check_ref(
    field_ref: &str,
    path: &str,
    local_ids: &HashSet<&str>,
    root_ids: &HashSet<&str>,
    report: &mut LintReport,
) {
    if let Some(root_path) = field_ref.strip_prefix("$root.") {
        let root_key = root_path.split('.').next().unwrap_or_default();
        if !root_ids.contains(root_key) {
            report.push_error(
                path.to_owned(),
                "dangling_reference",
                format!("rule references unknown root key `{root_key}`"),
            );
        }
        return;
    }

    let local_key = field_ref.split('.').next().unwrap_or_default();
    if !local_ids.contains(local_key) {
        report.push_error(
            path.to_owned(),
            "dangling_reference",
            format!("rule references unknown local key `{local_key}`"),
        );
    }
}

fn lint_contradictory_rules(rules: &[Rule], path: &str, report: &mut LintReport) {
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
        report.push_error(
            path.to_owned(),
            "contradictory_rules",
            format!("min_length ({min}) is greater than max_length ({max})"),
        );
    }
    if let (Some(min), Some(max)) = (min_items, max_items)
        && min > max
    {
        report.push_error(
            path.to_owned(),
            "contradictory_rules",
            format!("min_items ({min}) is greater than max_items ({max})"),
        );
    }
}

fn lint_rule_type_compatibility(
    field: &Field,
    rules: &[Rule],
    path: &str,
    report: &mut LintReport,
) {
    for (index, rule) in rules.iter().enumerate() {
        lint_single_rule_compatibility(field, rule, &format!("{path}[{index}]"), report);
    }
}

fn lint_single_rule_compatibility(field: &Field, rule: &Rule, path: &str, report: &mut LintReport) {
    let compatible = match rule {
        Rule::Pattern { .. }
        | Rule::MinLength { .. }
        | Rule::MaxLength { .. }
        | Rule::Email { .. }
        | Rule::Url { .. } => supports_string_rules(field),
        Rule::Min { .. } | Rule::Max { .. } => supports_number_rules(field),
        Rule::MinItems { .. } | Rule::MaxItems { .. } => supports_collection_rules(field),
        Rule::All { rules } | Rule::Any { rules } => {
            for (index, nested) in rules.iter().enumerate() {
                lint_single_rule_compatibility(
                    field,
                    nested,
                    &format!("{path}.nested[{index}]"),
                    report,
                );
            }
            true
        },
        Rule::Not { inner } => {
            lint_single_rule_compatibility(field, inner, &format!("{path}.not"), report);
            true
        },
        _ => true,
    };

    if !compatible {
        report.push_warning(
            path.to_owned(),
            "rule_type_mismatch",
            format!(
                "rule `{}` is not compatible with `{}` field",
                rule_name(rule),
                field_type_name(field)
            ),
        );
    }
}

fn supports_string_rules(field: &Field) -> bool {
    matches!(
        field,
        Field::String(_)
            | Field::Secret(_)
            | Field::Code(_)
            | Field::Date(_)
            | Field::DateTime(_)
            | Field::Time(_)
            | Field::Color(_)
            | Field::Hidden(_)
            | Field::File(_)
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

fn check_duplicate_keys(fields: &[Field], prefix: &str, report: &mut LintReport) {
    let mut seen = HashSet::new();
    for field in fields {
        let key = field.key().as_str();
        if !seen.insert(key) {
            report.push_error(
                make_path(prefix, key),
                "duplicate_key",
                format!("duplicate field key `{key}`"),
            );
        }
    }
}

fn make_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
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

fn lint_visibility_cycles(fields: &[Field], report: &mut LintReport) {
    let mut edges: Vec<(&str, &str)> = Vec::new();
    for field in fields {
        let source = field.key().as_str();
        if let Some(rule) = field_visible_rule(field) {
            let mut refs = Vec::new();
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
                    report.push_error(
                        format!("{start}.visible"),
                        "visibility_cycle",
                        format!("visibility rule graph contains cycle involving `{start}`"),
                    );
                    return;
                }
                stack.push(edge_to);
            }
        }
    }
}

fn field_type_name(field: &Field) -> &'static str {
    match field {
        Field::String(_) => "string",
        Field::Secret(_) => "secret",
        Field::Number(_) => "number",
        Field::Boolean(_) => "boolean",
        Field::Select(_) => "select",
        Field::Object(_) => "object",
        Field::List(_) => "list",
        Field::Mode(_) => "mode",
        Field::Code(_) => "code",
        Field::Date(_) => "date",
        Field::DateTime(_) => "datetime",
        Field::Time(_) => "time",
        Field::Color(_) => "color",
        Field::File(_) => "file",
        Field::Hidden(_) => "hidden",
        Field::Computed(_) => "computed",
        Field::Dynamic(_) => "dynamic",
        Field::Notice(_) => "notice",
    }
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
