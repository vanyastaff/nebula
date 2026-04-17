//! Build-time structural lints.
//!
//! Two APIs coexist here:
//! - Legacy: `lint_schema` → `LintReport` (used by `Schema::lint()`).
//! - New: `lint_tree` → `ValidationReport` (used by `SchemaBuilder::build()`).
//!
//! Task 26 will unify them once the legacy API is removed.

use std::{borrow::Cow, collections::HashSet};

use nebula_validator::Rule;

use crate::{
    Field, FieldPath, ListField, ModeField, ObjectField, RequiredMode, Schema, VisibilityMode,
};

// ── Legacy types ──────────────────────────────────────────────────────────────

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

// ── Legacy entry point ────────────────────────────────────────────────────────

/// Run static lint checks for a schema (legacy API).
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

// ── New ValidationError-based API ─────────────────────────────────────────────

use crate::{
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

    #[test]
    fn lint_schema_legacy_still_works() {
        // Legacy API: lint a schema with a list field missing item schema.
        let schema = Schema::new().add(Field::list(FieldKey::new("items").unwrap()));
        let report = lint_schema(&schema);
        assert!(report.has_errors());
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|d| d.code == "missing_item_schema")
        );
    }
}
