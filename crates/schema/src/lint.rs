//! Build-time structural lints.

use std::collections::{HashMap, HashSet};

use nebula_validator::{
    DeferredRule, Logic, Predicate, Rule, ValueRule, foundation::FieldPath as ValidatorFieldPath,
};

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
    lint_required_cycles_new(fields, prefix, report);
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
                ValidationError::builder("duplicate_key")
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
                        ValidationError::builder("missing_loader")
                            .at(path.clone())
                            .message("dynamic select has no loader key configured")
                            .warn()
                            .build(),
                    );
                }
                if !select.dynamic && select.loader.is_some() {
                    report.push(
                        ValidationError::builder("loader_without_dynamic")
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
                        ValidationError::builder("missing_loader")
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
                        ValidationError::builder("notice.misuse")
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
                        ValidationError::builder("notice_missing_description")
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
            ValidationError::builder("missing_item_schema")
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
            ValidationError::builder("invalid_default_variant")
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
                ValidationError::builder("duplicate_variant")
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
                    ValidationError::builder("missing_variant_label")
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
                ValidationError::builder("self_dependency")
                    .at(path.clone())
                    .message(format!("depends_on contains self reference `{dependency}`"))
                    .build(),
            );
            continue;
        }

        if dependency.is_root() {
            report.push(
                ValidationError::builder("dangling_reference")
                    .at(path.clone())
                    .message("depends_on references an empty path")
                    .build(),
            );
            continue;
        }

        let root_key = first_key.unwrap_or_default();
        if !local_keys.contains(root_key) && !root_keys.contains(root_key) {
            report.push(
                ValidationError::builder("dangling_reference")
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
    _local_keys: &HashSet<&str>,
    root_keys: &HashSet<&str>,
    report: &mut ValidationReport,
) {
    let Some(rule) = maybe_rule else { return };
    let mut refs = Vec::new();
    rule.field_references(&mut refs);
    for field_ref in refs {
        // Predicates now emit JSON-Pointer-shaped references ("/foo/bar").
        // Strip the leading `/` and resolve the first segment as the key to
        // check. Legacy `$root.` prefix is preserved for back-compat.
        if let Some(rp) = field_ref.strip_prefix("$root.") {
            let rk = rp.split('.').next().unwrap_or_default();
            if !root_keys.contains(rk) {
                report.push(
                    ValidationError::builder("dangling_reference")
                        .at(path.clone())
                        .message(format!("rule references unknown root key `{rk}`"))
                        .build(),
                );
            }
            continue;
        }
        if let Some(rest) = field_ref.strip_prefix('/') {
            let rk = rest.split('/').next().unwrap_or_default();
            if !root_keys.contains(rk) {
                report.push(
                    ValidationError::builder("dangling_reference")
                        .at(path.clone())
                        .message(format!("rule references unknown root key `{rk}`"))
                        .build(),
                );
            }
            continue;
        }
        // Breaking cleanup: only JSON Pointer references are supported.
        report.push(
            ValidationError::builder("dangling_reference")
                .at(path.clone())
                .message(format!(
                    "rule reference `{field_ref}` must be a JSON Pointer path (for example `/foo/bar`)"
                ))
                .build(),
        );
        continue;
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
        Rule::Value(v) => match v {
            ValueRule::Pattern(_)
            | ValueRule::MinLength(_)
            | ValueRule::MaxLength(_)
            | ValueRule::Email
            | ValueRule::Url => supports_string_rules(field),
            ValueRule::Min(_)
            | ValueRule::Max(_)
            | ValueRule::GreaterThan(_)
            | ValueRule::LessThan(_) => supports_number_rules(field),
            ValueRule::MinItems(_) | ValueRule::MaxItems(_) => supports_collection_rules(field),
            _ => true,
        },
        Rule::Logic(l) => match l.as_ref() {
            Logic::All(rules) | Logic::Any(rules) => {
                for nested in rules {
                    lint_single_compat_new(field, nested, path, report);
                }
                true
            },
            Logic::Not(inner) => {
                lint_single_compat_new(field, inner, path, report);
                true
            },
        },
        Rule::Described(inner, _) => {
            lint_single_compat_new(field, inner, path, report);
            true
        },
        _ => true,
    };

    if !compatible {
        report.push(
            ValidationError::builder("rule.incompatible")
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
            ValidationError::builder("rule.contradictory")
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
            ValidationError::builder("rule.contradictory")
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
        Rule::Value(v) => match v {
            ValueRule::Pattern(_) => "pattern",
            ValueRule::MinLength(_) => "min_length",
            ValueRule::MaxLength(_) => "max_length",
            ValueRule::Min(_) => "min",
            ValueRule::Max(_) => "max",
            ValueRule::GreaterThan(_) => "greater_than",
            ValueRule::LessThan(_) => "less_than",
            ValueRule::OneOf(_) => "one_of",
            ValueRule::MinItems(_) => "min_items",
            ValueRule::MaxItems(_) => "max_items",
            ValueRule::Email => "email",
            ValueRule::Url => "url",
            _ => "unknown_rule",
        },
        Rule::Deferred(d) => match d {
            DeferredRule::UniqueBy(_) => "unique_by",
            DeferredRule::Custom(_) => "custom",
            _ => "unknown_rule",
        },
        Rule::Predicate(p) => match p {
            Predicate::Eq(..) => "eq",
            Predicate::Ne(..) => "ne",
            Predicate::Gt(..) => "gt",
            Predicate::Gte(..) => "gte",
            Predicate::Lt(..) => "lt",
            Predicate::Lte(..) => "lte",
            Predicate::IsTrue(_) => "is_true",
            Predicate::IsFalse(_) => "is_false",
            Predicate::Set(_) => "set",
            Predicate::Empty(_) => "empty",
            Predicate::Contains(..) => "contains",
            Predicate::Matches(..) => "matches",
            Predicate::In(..) => "in",
            _ => "unknown_rule",
        },
        Rule::Logic(l) => match l.as_ref() {
            Logic::All(_) => "all",
            Logic::Any(_) => "any",
            Logic::Not(_) => "not",
        },
        Rule::Described(inner, _) => rule_name(inner),
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
            Rule::Value(ValueRule::MinLength(min)) => {
                *min_length = Some(min_length.map_or(*min, |current| current.max(*min)));
            },
            Rule::Value(ValueRule::MaxLength(max)) => {
                *max_length = Some(max_length.map_or(*max, |current| current.min(*max)));
            },
            Rule::Value(ValueRule::MinItems(min)) => {
                *min_items = Some(min_items.map_or(*min, |current| current.max(*min)));
            },
            Rule::Value(ValueRule::MaxItems(max)) => {
                *max_items = Some(max_items.map_or(*max, |current| current.min(*max)));
            },
            Rule::Logic(l) => match l.as_ref() {
                Logic::All(rules) | Logic::Any(rules) => {
                    collect_min_max(rules, min_length, max_length, min_items, max_items);
                },
                Logic::Not(inner) => {
                    collect_min_max(
                        std::slice::from_ref(inner),
                        min_length,
                        max_length,
                        min_items,
                        max_items,
                    );
                },
            },
            Rule::Described(inner, _) => {
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

fn emit_visibility_cycle_on_edge(from: &FieldPath, to: &FieldPath, report: &mut ValidationReport) {
    report.push(
        ValidationError::builder("visibility_cycle")
            .at(from.clone())
            .message(format!(
                "visibility rule graph contains a cycle (dependency `{from}` -> `{to}`)"
            ))
            .build(),
    );
}

fn emit_required_cycle_on_edge(from: &FieldPath, to: &FieldPath, report: &mut ValidationReport) {
    report.push(
        ValidationError::builder("required_cycle")
            .at(from.clone())
            .message(format!(
                "required rule graph contains a cycle (dependency `{from}` -> `{to}`)"
            ))
            .build(),
    );
}

/// Convert a validator JSON-pointer field path into a schema [`FieldPath`]
/// (dot/bracket wire form used throughout this crate).
fn validator_path_to_schema_path(vp: &ValidatorFieldPath) -> Option<FieldPath> {
    let mut out = FieldPath::root();
    let mut any = false;
    for seg in vp.segments() {
        let s = seg.as_ref();
        if s.is_empty() {
            continue;
        }
        any = true;
        let segment = if s.chars().all(|c| c.is_ascii_digit()) {
            let idx: usize = s.parse().ok()?;
            PathSegment::Index(idx)
        } else {
            PathSegment::Key(crate::key::FieldKey::new(s).ok()?)
        };
        out = out.join(segment);
    }
    if any { Some(out) } else { None }
}

/// Normalize dependency paths to the same shape as `collect_defined_field_paths`.
///
/// `collect_defined_field_paths` tracks list-item object descendants under the list
/// key (`items.name`), not indexed instances (`items[0].name`). To make
/// JSON-pointer refs such as `/items/0/name` comparable against that set, we
/// drop index segments here.
fn normalize_visibility_target_path(path: &FieldPath) -> FieldPath {
    let mut normalized = FieldPath::root();
    for segment in path.segments() {
        if matches!(segment, PathSegment::Index(_)) {
            continue;
        }
        normalized = normalized.join(segment.clone());
    }
    normalized
}

/// Resolve a [`Rule::field_references`] string to an absolute schema [`FieldPath`].
///
/// - `$root.` — anchor at schema root (same convention as [`lint_rule_refs_new`]).
/// - Leading `/` — JSON Pointer from schema root (validator [`ValidatorFieldPath`]).
/// - Any other form is ignored (schema lint accepts JSON-pointer forms only).
fn resolve_visibility_dependency(field_ref: &str, _scope_prefix: &FieldPath) -> Option<FieldPath> {
    if let Some(rest) = field_ref.strip_prefix("$root.") {
        let vp = ValidatorFieldPath::parse(rest)?;
        return validator_path_to_schema_path(&vp);
    }
    if field_ref.starts_with('/') {
        let vp = ValidatorFieldPath::parse(field_ref)?;
        return validator_path_to_schema_path(&vp);
    }
    None
}

fn collect_defined_field_paths(
    fields: &[Field],
    prefix: &FieldPath,
    defined: &mut HashSet<FieldPath>,
) {
    for field in fields {
        let path = prefix.clone().join(field.key().clone());
        defined.insert(path.clone());

        match field {
            Field::List(list) => {
                if let Some(Field::Object(obj)) = list.item.as_deref() {
                    collect_defined_field_paths(&obj.fields, &path, defined);
                }
            },
            Field::Object(obj) => {
                collect_defined_field_paths(&obj.fields, &path, defined);
            },
            Field::Mode(mode) => {
                for variant in &mode.variants {
                    if let Ok(vk) = crate::key::FieldKey::new(variant.key.as_str()) {
                        let vpath = path.clone().join(vk.clone());
                        defined.insert(vpath.clone());
                        if let Field::Object(obj) = variant.field.as_ref() {
                            collect_defined_field_paths(&obj.fields, &vpath, defined);
                        }
                    }
                }
            },
            _ => {},
        }
    }
}

fn append_visibility_edges(
    fields: &[Field],
    prefix: &FieldPath,
    defined: &HashSet<FieldPath>,
    edges: &mut Vec<(FieldPath, FieldPath)>,
) {
    for field in fields {
        let path = prefix.clone().join(field.key().clone());

        if let Some(rule) = field_visible_rule(field) {
            let mut refs = Vec::new();
            rule.field_references(&mut refs);
            for field_ref in refs {
                if let Some(target) = resolve_visibility_dependency(field_ref, prefix) {
                    let normalized_target = normalize_visibility_target_path(&target);
                    if defined.contains(&normalized_target) {
                        edges.push((path.clone(), normalized_target));
                    }
                }
            }
        }

        match field {
            Field::List(list) => {
                if let Some(Field::Object(obj)) = list.item.as_deref() {
                    append_visibility_edges(&obj.fields, &path, defined, edges);
                }
            },
            Field::Object(obj) => {
                append_visibility_edges(&obj.fields, &path, defined, edges);
            },
            Field::Mode(mode) => {
                for variant in &mode.variants {
                    if let Ok(vk) = crate::key::FieldKey::new(variant.key.as_str()) {
                        let vpath = path.clone().join(vk.clone());
                        if let Some(rule) = field_visible_rule(variant.field.as_ref()) {
                            let mut refs = Vec::new();
                            rule.field_references(&mut refs);
                            for field_ref in refs {
                                if let Some(target) =
                                    resolve_visibility_dependency(field_ref, &vpath)
                                {
                                    let normalized_target =
                                        normalize_visibility_target_path(&target);
                                    if defined.contains(&normalized_target) {
                                        edges.push((vpath.clone(), normalized_target));
                                    }
                                }
                            }
                        }
                        if let Field::Object(obj) = variant.field.as_ref() {
                            append_visibility_edges(&obj.fields, &vpath, defined, edges);
                        }
                    }
                }
            },
            _ => {},
        }
    }
}

fn append_required_edges(
    fields: &[Field],
    prefix: &FieldPath,
    defined: &HashSet<FieldPath>,
    edges: &mut Vec<(FieldPath, FieldPath)>,
) {
    for field in fields {
        let path = prefix.clone().join(field.key().clone());

        if let Some(rule) = field_required_rule(field) {
            let mut refs = Vec::new();
            rule.field_references(&mut refs);
            for field_ref in refs {
                if let Some(target) = resolve_visibility_dependency(field_ref, prefix) {
                    let normalized_target = normalize_visibility_target_path(&target);
                    if defined.contains(&normalized_target) {
                        edges.push((path.clone(), normalized_target));
                    }
                }
            }
        }

        match field {
            Field::List(list) => {
                if let Some(Field::Object(obj)) = list.item.as_deref() {
                    append_required_edges(&obj.fields, &path, defined, edges);
                }
            },
            Field::Object(obj) => {
                append_required_edges(&obj.fields, &path, defined, edges);
            },
            Field::Mode(mode) => {
                for variant in &mode.variants {
                    if let Ok(vk) = crate::key::FieldKey::new(variant.key.as_str()) {
                        let vpath = path.clone().join(vk.clone());
                        if let Some(rule) = field_required_rule(variant.field.as_ref()) {
                            let mut refs = Vec::new();
                            rule.field_references(&mut refs);
                            for field_ref in refs {
                                if let Some(target) =
                                    resolve_visibility_dependency(field_ref, &vpath)
                                {
                                    let normalized_target =
                                        normalize_visibility_target_path(&target);
                                    if defined.contains(&normalized_target) {
                                        edges.push((vpath.clone(), normalized_target));
                                    }
                                }
                            }
                        }
                        if let Field::Object(obj) = variant.field.as_ref() {
                            append_required_edges(&obj.fields, &vpath, defined, edges);
                        }
                    }
                }
            },
            _ => {},
        }
    }
}

fn visibility_adjacency(edges: &[(FieldPath, FieldPath)]) -> HashMap<FieldPath, Vec<FieldPath>> {
    let mut adj: HashMap<FieldPath, Vec<FieldPath>> = HashMap::new();
    for (from, to) in edges {
        adj.entry(from.clone()).or_default().push(to.clone());
    }
    adj
}

fn find_visibility_cycle_edge(
    adj: &HashMap<FieldPath, Vec<FieldPath>>,
) -> Option<(FieldPath, FieldPath)> {
    // 0 = white, 1 = gray, 2 = black
    let mut color: HashMap<FieldPath, u8> = HashMap::new();

    fn dfs(
        u: &FieldPath,
        adj: &HashMap<FieldPath, Vec<FieldPath>>,
        color: &mut HashMap<FieldPath, u8>,
    ) -> Option<(FieldPath, FieldPath)> {
        color.insert(u.clone(), 1);
        for v in adj.get(u).into_iter().flatten() {
            match color.get(v).copied().unwrap_or(0) {
                1 => {
                    return Some((u.clone(), v.clone()));
                },
                0 => {
                    if let Some(cycle) = dfs(v, adj, color) {
                        return Some(cycle);
                    }
                },
                _ => {},
            }
        }
        color.insert(u.clone(), 2);
        None
    }

    for start in adj.keys() {
        if color.get(start).copied().unwrap_or(0) == 0
            && let Some(cycle) = dfs(start, adj, &mut color)
        {
            return Some(cycle);
        }
    }
    None
}

fn lint_visibility_cycles_new(
    fields: &[Field],
    _prefix: &FieldPath,
    report: &mut ValidationReport,
) {
    let mut defined: HashSet<FieldPath> = HashSet::new();
    collect_defined_field_paths(fields, &FieldPath::root(), &mut defined);

    let mut edges: Vec<(FieldPath, FieldPath)> = Vec::new();
    append_visibility_edges(fields, &FieldPath::root(), &defined, &mut edges);

    let adj = visibility_adjacency(&edges);
    if let Some((from, to)) = find_visibility_cycle_edge(&adj) {
        emit_visibility_cycle_on_edge(&from, &to, report);
    }
}

fn lint_required_cycles_new(fields: &[Field], _prefix: &FieldPath, report: &mut ValidationReport) {
    let mut defined: HashSet<FieldPath> = HashSet::new();
    collect_defined_field_paths(fields, &FieldPath::root(), &mut defined);

    let mut edges: Vec<(FieldPath, FieldPath)> = Vec::new();
    append_required_edges(fields, &FieldPath::root(), &defined, &mut edges);

    let adj = visibility_adjacency(&edges);
    if let Some((from, to)) = find_visibility_cycle_edge(&adj) {
        emit_required_cycle_on_edge(&from, &to, report);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use nebula_validator::{Predicate, Rule};
    use serde_json::json;

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
    fn detects_visibility_cycle_between_top_level_fields() {
        let fields = vec![
            Field::string(FieldKey::new("a").unwrap())
                .visible_when(Rule::predicate(Predicate::eq("/b", json!("on")).unwrap()))
                .into_field(),
            Field::string(FieldKey::new("b").unwrap())
                .visible_when(Rule::predicate(Predicate::eq("/a", json!("on")).unwrap()))
                .into_field(),
        ];
        let report = run(fields);
        assert!(
            report.errors().any(|e| e.code == "visibility_cycle"),
            "expected visibility_cycle, got {:?}",
            report.errors().map(|e| &e.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn detects_visibility_cycle_inside_nested_object() {
        let outer = Field::object("outer")
            .add(
                Field::string("x")
                    .visible_when(Rule::predicate(
                        Predicate::eq("/outer/y", json!(true)).unwrap(),
                    ))
                    .into_field(),
            )
            .add(
                Field::string("y")
                    .visible_when(Rule::predicate(
                        Predicate::eq("/outer/x", json!(true)).unwrap(),
                    ))
                    .into_field(),
            );
        let report = run(vec![outer.into()]);
        assert!(
            report.errors().any(|e| e.code == "visibility_cycle"),
            "expected visibility_cycle, got {:?}",
            report.errors().map(|e| &e.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn acyclic_visibility_rules_do_not_error() {
        let fields = vec![
            Field::string("toggle").into_field(),
            Field::string("detail")
                .visible_when(Rule::predicate(
                    Predicate::eq("/toggle", json!(true)).unwrap(),
                ))
                .into_field(),
        ];
        let report = run(fields);
        assert!(!report.errors().any(|e| e.code == "visibility_cycle"));
    }

    #[test]
    fn detects_visibility_cycle_with_list_index_reference() {
        let fields = vec![
            Field::list("items")
                .item(
                    Field::object("row")
                        .add(
                            Field::string("x")
                                .visible_when(Rule::predicate(
                                    Predicate::eq("/items/0/y", json!(true)).unwrap(),
                                ))
                                .into_field(),
                        )
                        .add(
                            Field::string("y")
                                .visible_when(Rule::predicate(
                                    Predicate::eq("/items/0/x", json!(true)).unwrap(),
                                ))
                                .into_field(),
                        ),
                )
                .into_field(),
        ];

        let report = run(fields);
        assert!(
            report.errors().any(|e| e.code == "visibility_cycle"),
            "expected visibility_cycle, got {:?}",
            report.errors().map(|e| &e.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn pointer_refs_in_nested_scope_are_checked_against_root_keys() {
        let fields = vec![
            Field::object("outer")
                .add(
                    Field::string("x")
                        .visible_when(Rule::predicate(
                            Predicate::eq("/outer/y", json!(true)).unwrap(),
                        ))
                        .into_field(),
                )
                .into_field(),
            Field::string("top").into_field(),
        ];

        let report = run(fields);
        assert!(
            !report.errors().any(|e| e.code == "dangling_reference"),
            "did not expect dangling_reference, got {:?}",
            report
                .errors()
                .map(|e| (&e.code, e.path.to_string()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn detects_visibility_cycle_through_mode_variant_payload() {
        let fields = vec![
            Field::string("a")
                .visible_when(Rule::predicate(Predicate::eq("/m/v", json!(true)).unwrap()))
                .into_field(),
            Field::mode("m")
                .variant(
                    "v",
                    "Variant",
                    Field::string("payload")
                        .visible_when(Rule::predicate(Predicate::eq("/a", json!(true)).unwrap()))
                        .into_field(),
                )
                .into_field(),
        ];

        let report = run(fields);
        assert!(
            report.errors().any(|e| e.code == "visibility_cycle"),
            "expected visibility_cycle, got {:?}",
            report.errors().map(|e| &e.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn detects_required_cycle_between_top_level_fields() {
        let fields = vec![
            Field::string("a")
                .required_when(Rule::predicate(Predicate::eq("/b", json!(true)).unwrap()))
                .into_field(),
            Field::string("b")
                .required_when(Rule::predicate(Predicate::eq("/a", json!(true)).unwrap()))
                .into_field(),
        ];

        let report = run(fields);
        assert!(
            report.errors().any(|e| e.code == "required_cycle"),
            "expected required_cycle, got {:?}",
            report.errors().map(|e| &e.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn detects_required_cycle_inside_nested_object() {
        let outer = Field::object("outer")
            .add(
                Field::string("x")
                    .required_when(Rule::predicate(
                        Predicate::eq("/outer/y", json!(true)).unwrap(),
                    ))
                    .into_field(),
            )
            .add(
                Field::string("y")
                    .required_when(Rule::predicate(
                        Predicate::eq("/outer/x", json!(true)).unwrap(),
                    ))
                    .into_field(),
            );

        let report = run(vec![outer.into()]);
        assert!(
            report.errors().any(|e| e.code == "required_cycle"),
            "expected required_cycle, got {:?}",
            report.errors().map(|e| &e.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn detects_required_cycle_with_list_index_reference() {
        let fields = vec![
            Field::list("items")
                .item(
                    Field::object("row")
                        .add(
                            Field::string("x")
                                .required_when(Rule::predicate(
                                    Predicate::eq("/items/0/y", json!(true)).unwrap(),
                                ))
                                .into_field(),
                        )
                        .add(
                            Field::string("y")
                                .required_when(Rule::predicate(
                                    Predicate::eq("/items/0/x", json!(true)).unwrap(),
                                ))
                                .into_field(),
                        ),
                )
                .into_field(),
        ];

        let report = run(fields);
        assert!(
            report.errors().any(|e| e.code == "required_cycle"),
            "expected required_cycle, got {:?}",
            report.errors().map(|e| &e.code).collect::<Vec<_>>()
        );
    }

    #[test]
    fn detects_required_cycle_through_mode_variant_payload() {
        let fields = vec![
            Field::string("a")
                .required_when(Rule::predicate(Predicate::eq("/m/v", json!(true)).unwrap()))
                .into_field(),
            Field::mode("m")
                .variant(
                    "v",
                    "Variant",
                    Field::string("payload")
                        .required_when(Rule::predicate(Predicate::eq("/a", json!(true)).unwrap()))
                        .into_field(),
                )
                .into_field(),
        ];

        let report = run(fields);
        assert!(
            report.errors().any(|e| e.code == "required_cycle"),
            "expected required_cycle, got {:?}",
            report.errors().map(|e| &e.code).collect::<Vec<_>>()
        );
    }
}
