//! Schema-time lint diagnostics.
//!
//! Provides a static lint pass over a [`ParameterCollection`] that detects
//! structural problems independent of any runtime values: duplicate parameter
//! ids, contradictory rules, dangling references, and integrity violations.

use std::collections::HashSet;

use crate::{
    collection::ParameterCollection, conditions::Condition, display_mode::DisplayMode,
    parameter::Parameter, parameter_type::ParameterType, rules::Rule, transformer::Transformer,
};

/// A single lint finding emitted by [`lint_collection`].
#[derive(Debug, Clone, PartialEq)]
pub struct LintDiagnostic {
    /// Dot-separated path to the offending parameter or rule.
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

/// Runs the static lint pass over `collection` and returns all diagnostics found.
///
/// This is a schema-time check -- it does not require runtime values.
///
/// # Examples
///
/// ```
/// use nebula_parameter::{
///     collection::ParameterCollection, lint::lint_collection, parameter::Parameter,
/// };
///
/// let coll = ParameterCollection::new()
///     .add(Parameter::string("name").label("Name"))
///     .add(Parameter::string("email").label("Email"));
///
/// assert!(lint_collection(&coll).is_empty());
/// ```
#[must_use]
pub fn lint_collection(collection: &ParameterCollection) -> Vec<LintDiagnostic> {
    let mut diags = Vec::new();

    // Collect all root-level parameter IDs for reference checking.
    let root_ids: HashSet<&str> = collection
        .parameters
        .iter()
        .map(|p| p.id.as_str())
        .collect();

    lint_parameters(
        &collection.parameters,
        "",
        &root_ids,
        &mut diags,
        ObjectContext::Root,
    );

    diags
}

/// Context about the parent Object (if any) for sub-parameter checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectContext {
    Root,
    ObjectInline,
    ObjectCollapsed,
    ObjectPickFields,
    ObjectSections,
}

impl ObjectContext {
    fn from_display_mode(mode: DisplayMode) -> Self {
        match mode {
            DisplayMode::Inline => Self::ObjectInline,
            DisplayMode::Collapsed => Self::ObjectCollapsed,
            DisplayMode::PickFields => Self::ObjectPickFields,
            DisplayMode::Sections => Self::ObjectSections,
        }
    }

    fn is_pick_fields(self) -> bool {
        matches!(self, Self::ObjectPickFields)
    }
}

/// Lint a list of parameters at the same scope level.
fn lint_parameters(
    params: &[Parameter],
    prefix: &str,
    root_ids: &HashSet<&str>,
    diags: &mut Vec<LintDiagnostic>,
    ctx: ObjectContext,
) {
    // Build local scope IDs for reference checking within this scope.
    let local_ids: HashSet<&str> = params.iter().map(|p| p.id.as_str()).collect();

    // 1. Duplicate IDs
    check_duplicate_ids(params, prefix, diags);

    for param in params {
        let path = make_path(prefix, &param.id);

        // 2. Empty IDs
        if param.id.is_empty() {
            diags.push(LintDiagnostic {
                path: if prefix.is_empty() {
                    "<empty>".to_owned()
                } else {
                    format!("{prefix}.<empty>")
                },
                level: LintLevel::Error,
                message: "parameter ID must not be empty".to_owned(),
            });
        }

        // 5. Condition references to unknown fields
        check_condition_refs(
            param.visible_when.as_ref(),
            &path,
            "visible_when",
            &local_ids,
            root_ids,
            diags,
        );
        check_condition_refs(
            param.required_when.as_ref(),
            &path,
            "required_when",
            &local_ids,
            root_ids,
            diags,
        );
        check_condition_refs(
            param.disabled_when.as_ref(),
            &path,
            "disabled_when",
            &local_ids,
            root_ids,
            diags,
        );

        // 9, 10. Contradictory rules
        check_contradictory_rules(&param.rules, &path, diags);

        // 5 (rules). Rule field_references to unknown fields
        check_rule_refs(&param.rules, &path, &local_ids, root_ids, diags);

        // 16-19. Transformer checks
        check_transformers(&param.transformers, &param.param_type, &path, diags);

        // 11-12. group checks for Object sub-parameters
        match ctx {
            // 11. Sections Object sub-param missing group
            ObjectContext::ObjectSections
                if param.group.is_none()
                    && !matches!(param.param_type, ParameterType::Notice { .. }) =>
            {
                diags.push(LintDiagnostic {
                    path: path.clone(),
                    level: LintLevel::Warning,
                    message: "sub-parameter inside Sections Object is missing `group`".to_owned(),
                });
            }
            // 12. group on parameter inside non-Sections Object
            ObjectContext::ObjectInline | ObjectContext::ObjectCollapsed
                if param.group.is_some() =>
            {
                diags.push(LintDiagnostic {
                    path: path.clone(),
                    level: LintLevel::Warning,
                    message: "`group` is set but parent Object is not Sections mode".to_owned(),
                });
            }
            _ => {}
        }

        // 13. required on sub-parameter of PickFields Object
        if ctx.is_pick_fields() && param.required {
            diags.push(LintDiagnostic {
                path: path.clone(),
                level: LintLevel::Warning,
                message: "`required` on sub-parameter of PickFields Object is unusual".to_owned(),
            });
        }

        // 20, 21. Notice checks
        check_notice(param, &path, diags);

        // Type-specific recursive checks
        lint_parameter_type(&param.param_type, &param.id, &path, root_ids, diags);
    }
}

/// Recursively lint a parameter's type-specific contents.
fn lint_parameter_type(
    param_type: &ParameterType,
    param_id: &str,
    path: &str,
    root_ids: &HashSet<&str>,
    diags: &mut Vec<LintDiagnostic>,
) {
    match param_type {
        ParameterType::Object {
            parameters,
            display_mode,
        } => {
            let ctx = ObjectContext::from_display_mode(*display_mode);

            // 14. PickFields/Sections with <=2 sub-parameters
            if display_mode.is_pick_mode() && parameters.len() <= 2 {
                diags.push(LintDiagnostic {
                    path: path.to_owned(),
                    level: LintLevel::Warning,
                    message: format!(
                        "{display_mode:?} Object has only {} sub-parameters (consider Inline)",
                        parameters.len()
                    ),
                });
            }

            lint_parameters(parameters, path, root_ids, diags, ctx);
        }

        ParameterType::List { item, .. } => {
            let item_path = format!("{path}[]");
            lint_parameters(
                std::slice::from_ref(item.as_ref()),
                &item_path,
                root_ids,
                diags,
                ObjectContext::Root,
            );
        }

        ParameterType::Mode {
            variants,
            default_variant,
        } => {
            // 3. Duplicate variant IDs
            check_duplicate_ids(variants, &format!("{path}.variants"), diags);

            // 4. Invalid default_variant
            if let Some(dv) = default_variant
                && !variants.iter().any(|v| v.id == *dv)
            {
                diags.push(LintDiagnostic {
                    path: path.to_owned(),
                    level: LintLevel::Error,
                    message: format!("default_variant `{dv}` references non-existent variant"),
                });
            }

            // 15. Variant missing label
            for variant in variants {
                if variant.label.is_none() {
                    diags.push(LintDiagnostic {
                        path: format!("{path}.variants.{}", variant.id),
                        level: LintLevel::Warning,
                        message: "mode variant is missing a label".to_owned(),
                    });
                }
            }

            // Recurse into each variant
            for variant in variants {
                let variant_path = format!("{path}.variants.{}", variant.id);
                lint_parameter_type(
                    &variant.param_type,
                    &variant.id,
                    &variant_path,
                    root_ids,
                    diags,
                );

                // Check variant-level conditions/rules/transformers
                check_condition_refs(
                    variant.visible_when.as_ref(),
                    &variant_path,
                    "visible_when",
                    &HashSet::new(),
                    root_ids,
                    diags,
                );
                check_condition_refs(
                    variant.required_when.as_ref(),
                    &variant_path,
                    "required_when",
                    &HashSet::new(),
                    root_ids,
                    diags,
                );
                check_condition_refs(
                    variant.disabled_when.as_ref(),
                    &variant_path,
                    "disabled_when",
                    &HashSet::new(),
                    root_ids,
                    diags,
                );
                check_contradictory_rules(&variant.rules, &variant_path, diags);
                check_transformers(
                    &variant.transformers,
                    &variant.param_type,
                    &variant_path,
                    diags,
                );
            }
        }

        ParameterType::Select { depends_on, .. } => {
            check_depends_on(depends_on, param_id, path, root_ids, diags);
        }

        ParameterType::Filter {
            depends_on,
            fields,
            fields_loader,
            ..
        } => {
            check_depends_on(depends_on, param_id, path, root_ids, diags);

            // 22. Filter with no static fields and no fields_loader
            if fields.is_empty() && fields_loader.is_none() {
                diags.push(LintDiagnostic {
                    path: path.to_owned(),
                    level: LintLevel::Warning,
                    message: "Filter has no static fields and no fields_loader".to_owned(),
                });
            }

            // 23. Filter with duplicate field IDs
            let mut seen = HashSet::new();
            for field in fields {
                if !seen.insert(field.id.as_str()) {
                    diags.push(LintDiagnostic {
                        path: format!("{path}.fields.{}", field.id),
                        level: LintLevel::Warning,
                        message: format!("duplicate filter field ID `{}`", field.id),
                    });
                }
            }
        }

        ParameterType::Dynamic { depends_on, .. } => {
            check_depends_on(depends_on, param_id, path, root_ids, diags);
        }

        _ => {}
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

/// Build a dot-separated path.
fn make_path(prefix: &str, id: &str) -> String {
    if prefix.is_empty() {
        id.to_owned()
    } else {
        format!("{prefix}.{id}")
    }
}

/// Check for duplicate IDs in a parameter list (diagnostic 1 & 3).
fn check_duplicate_ids(params: &[Parameter], prefix: &str, diags: &mut Vec<LintDiagnostic>) {
    let mut seen = HashSet::new();
    for param in params {
        if !param.id.is_empty() && !seen.insert(param.id.as_str()) {
            diags.push(LintDiagnostic {
                path: make_path(prefix, &param.id),
                level: LintLevel::Error,
                message: format!("duplicate parameter ID `{}`", param.id),
            });
        }
    }
}

/// Check condition field references against known IDs (diagnostic 5 & 8).
fn check_condition_refs(
    condition: Option<&Condition>,
    param_path: &str,
    condition_name: &str,
    local_ids: &HashSet<&str>,
    root_ids: &HashSet<&str>,
    diags: &mut Vec<LintDiagnostic>,
) {
    let Some(cond) = condition else { return };

    let mut refs = Vec::new();
    cond.field_references(&mut refs);

    for field_ref in refs {
        if let Some(root_field) = field_ref.strip_prefix("$root.") {
            // 8. $root.x reference check — first segment for root-level check
            let root_id = root_field.split('.').next().unwrap_or(root_field);
            if !root_ids.contains(root_id) {
                diags.push(LintDiagnostic {
                    path: format!("{param_path}.{condition_name}"),
                    level: LintLevel::Error,
                    message: format!(
                        "`{field_ref}` references non-existent root-level parameter `{root_id}`"
                    ),
                });
            }
        } else {
            // 5. Sibling reference check — first segment must be a known local ID
            let first_segment = field_ref.split('.').next().unwrap_or(field_ref);
            if !local_ids.contains(first_segment) {
                diags.push(LintDiagnostic {
                    path: format!("{param_path}.{condition_name}"),
                    level: LintLevel::Error,
                    message: format!("`{field_ref}` references unknown field `{first_segment}`"),
                });
            }
        }
    }
}

/// Check rule field references against known IDs (diagnostic 5 for rules).
fn check_rule_refs(
    rules: &[Rule],
    param_path: &str,
    local_ids: &HashSet<&str>,
    root_ids: &HashSet<&str>,
    diags: &mut Vec<LintDiagnostic>,
) {
    for rule in rules {
        let mut refs = Vec::new();
        rule.field_references(&mut refs);

        for field_ref in refs {
            if let Some(root_field) = field_ref.strip_prefix("$root.") {
                let root_id = root_field.split('.').next().unwrap_or(root_field);
                if !root_ids.contains(root_id) {
                    diags.push(LintDiagnostic {
                        path: format!("{param_path}.rules"),
                        level: LintLevel::Error,
                        message: format!(
                            "`{field_ref}` references non-existent root-level parameter `{root_id}`"
                        ),
                    });
                }
            } else {
                let first_segment = field_ref.split('.').next().unwrap_or(field_ref);
                if !local_ids.contains(first_segment) {
                    diags.push(LintDiagnostic {
                        path: format!("{param_path}.rules"),
                        level: LintLevel::Error,
                        message: format!(
                            "`{field_ref}` references unknown field `{first_segment}`"
                        ),
                    });
                }
            }
        }
    }
}

/// Check contradictory min/max rules (diagnostics 9 & 10).
fn check_contradictory_rules(rules: &[Rule], path: &str, diags: &mut Vec<LintDiagnostic>) {
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

    // 9. Contradictory min_length > max_length
    if let (Some(min), Some(max)) = (min_length, max_length)
        && min > max
    {
        diags.push(LintDiagnostic {
            path: format!("{path}.rules"),
            level: LintLevel::Error,
            message: format!("contradictory rules: min_length ({min}) > max_length ({max})"),
        });
    }

    // 10. Contradictory min_items > max_items
    if let (Some(min), Some(max)) = (min_items, max_items)
        && min > max
    {
        diags.push(LintDiagnostic {
            path: format!("{path}.rules"),
            level: LintLevel::Error,
            message: format!("contradictory rules: min_items ({min}) > max_items ({max})"),
        });
    }
}

/// Recursively collect min/max values from rules (including inside All/Any/Not).
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
                *min_length = Some(min_length.map_or(*min, |prev| prev.max(*min)));
            }
            Rule::MaxLength { max, .. } => {
                *max_length = Some(max_length.map_or(*max, |prev| prev.min(*max)));
            }
            Rule::MinItems { min, .. } => {
                *min_items = Some(min_items.map_or(*min, |prev| prev.max(*min)));
            }
            Rule::MaxItems { max, .. } => {
                *max_items = Some(max_items.map_or(*max, |prev| prev.min(*max)));
            }
            Rule::All { rules: sub } | Rule::Any { rules: sub } => {
                collect_min_max(sub, min_length, max_length, min_items, max_items);
            }
            Rule::Not { inner: sub } => {
                collect_min_max(
                    std::slice::from_ref(sub.as_ref()),
                    min_length,
                    max_length,
                    min_items,
                    max_items,
                );
            }
            _ => {}
        }
    }
}

/// Check depends_on references (diagnostics 6, 7, 8).
fn check_depends_on(
    depends_on: &[crate::path::ParameterPath],
    param_id: &str,
    path: &str,
    root_ids: &HashSet<&str>,
    diags: &mut Vec<LintDiagnostic>,
) {
    for dep in depends_on {
        let dep_str = dep.as_str();

        // 7. Self-reference
        if dep_str == param_id || dep_str.strip_prefix("$root.") == Some(param_id) {
            diags.push(LintDiagnostic {
                path: path.to_owned(),
                level: LintLevel::Error,
                message: format!("depends_on self-reference `{dep_str}`"),
            });
            continue;
        }

        if dep.is_absolute() {
            // 8. $root.x reference check
            let segments = dep.segments();
            let root_id = segments.first().copied().unwrap_or("");
            if !root_ids.contains(root_id) {
                diags.push(LintDiagnostic {
                    path: path.to_owned(),
                    level: LintLevel::Error,
                    message: format!(
                        "depends_on `{dep_str}` references non-existent root parameter `{root_id}`"
                    ),
                });
            }
        } else {
            // 6. Sibling reference — check first segment against root (for top-level params)
            let segments = dep.segments();
            let first = segments.first().copied().unwrap_or("");
            if !root_ids.contains(first) {
                diags.push(LintDiagnostic {
                    path: path.to_owned(),
                    level: LintLevel::Error,
                    message: format!(
                        "depends_on `{dep_str}` references non-existent parameter `{first}`"
                    ),
                });
            }
        }
    }
}

/// Check transformer diagnostics (16-19).
fn check_transformers(
    transformers: &[Transformer],
    param_type: &ParameterType,
    path: &str,
    diags: &mut Vec<LintDiagnostic>,
) {
    let is_string_type = matches!(param_type, ParameterType::String { .. });

    for (i, transformer) in transformers.iter().enumerate() {
        check_single_transformer(transformer, is_string_type, path, i, diags);
    }
}

/// Check a single transformer (possibly recursing into Chain/FirstMatch).
fn check_single_transformer(
    transformer: &Transformer,
    is_string_type: bool,
    path: &str,
    index: usize,
    diags: &mut Vec<LintDiagnostic>,
) {
    match transformer {
        // 16. String-only transformers on non-string parameters
        Transformer::Trim | Transformer::Lowercase | Transformer::Uppercase if !is_string_type => {
            diags.push(LintDiagnostic {
                path: format!("{path}.transformers[{index}]"),
                level: LintLevel::Warning,
                message: format!(
                    "{:?} transformer on non-string parameter",
                    transformer_name(transformer),
                ),
            });
        }

        // 17, 18. Regex checks
        Transformer::Regex { pattern, group } => {
            if regex::Regex::new(pattern).is_err() {
                diags.push(LintDiagnostic {
                    path: format!("{path}.transformers[{index}]"),
                    level: LintLevel::Warning,
                    message: format!("invalid regex pattern: `{pattern}`"),
                });
            } else if *group == 0 {
                // 18. Capture group 0
                diags.push(LintDiagnostic {
                    path: format!("{path}.transformers[{index}]"),
                    level: LintLevel::Warning,
                    message: "regex capture group 0 is the whole match (likely unintended; use group >= 1)".to_owned(),
                });
            }
        }

        // 19. Chain/FirstMatch with single transformer
        Transformer::Chain { transformers } => {
            if transformers.len() == 1 {
                diags.push(LintDiagnostic {
                    path: format!("{path}.transformers[{index}]"),
                    level: LintLevel::Warning,
                    message: "Chain with a single transformer is unnecessary".to_owned(),
                });
            }
            // Recurse
            for (j, inner) in transformers.iter().enumerate() {
                check_single_transformer(
                    inner,
                    is_string_type,
                    &format!("{path}.transformers[{index}].chain"),
                    j,
                    diags,
                );
            }
        }

        Transformer::FirstMatch { transformers } => {
            if transformers.len() == 1 {
                diags.push(LintDiagnostic {
                    path: format!("{path}.transformers[{index}]"),
                    level: LintLevel::Warning,
                    message: "FirstMatch with a single transformer is unnecessary".to_owned(),
                });
            }
            // Recurse
            for (j, inner) in transformers.iter().enumerate() {
                check_single_transformer(
                    inner,
                    is_string_type,
                    &format!("{path}.transformers[{index}].first_match"),
                    j,
                    diags,
                );
            }
        }

        _ => {}
    }
}

/// Human-readable name for a transformer variant.
fn transformer_name(t: &Transformer) -> &'static str {
    match t {
        Transformer::Trim => "Trim",
        Transformer::Lowercase => "Lowercase",
        Transformer::Uppercase => "Uppercase",
        Transformer::Replace { .. } => "Replace",
        Transformer::StripPrefix { .. } => "StripPrefix",
        Transformer::StripSuffix { .. } => "StripSuffix",
        Transformer::Regex { .. } => "Regex",
        Transformer::JsonPath { .. } => "JsonPath",
        Transformer::Chain { .. } => "Chain",
        Transformer::FirstMatch { .. } => "FirstMatch",
    }
}

/// Check Notice-specific diagnostics (20 & 21).
fn check_notice(param: &Parameter, path: &str, diags: &mut Vec<LintDiagnostic>) {
    if !matches!(param.param_type, ParameterType::Notice { .. }) {
        return;
    }

    // 20. Notice with required, secret, default, or rules set
    let mut issues = Vec::new();
    if param.required {
        issues.push("required");
    }
    if param.secret {
        issues.push("secret");
    }
    if param.default.is_some() {
        issues.push("default");
    }
    if !param.rules.is_empty() {
        issues.push("rules");
    }
    if !issues.is_empty() {
        diags.push(LintDiagnostic {
            path: path.to_owned(),
            level: LintLevel::Warning,
            message: format!(
                "Notice parameter has {} set (notices are display-only)",
                issues.join(", ")
            ),
        });
    }

    // 21. Notice without description
    if param.description.is_none() {
        diags.push(LintDiagnostic {
            path: path.to_owned(),
            level: LintLevel::Warning,
            message: "Notice parameter has no description".to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameter::Parameter;

    // ── Clean schemas ───────────────────────────────────────────────────

    #[test]
    fn clean_collection_produces_no_diagnostics() {
        let coll = ParameterCollection::new()
            .add(Parameter::string("name").label("Name"))
            .add(Parameter::integer("age").label("Age"))
            .add(Parameter::boolean("active").label("Active"));

        assert!(lint_collection(&coll).is_empty());
    }

    #[test]
    fn empty_collection_is_clean() {
        assert!(lint_collection(&ParameterCollection::new()).is_empty());
    }

    // ── Duplicate IDs (1) ───────────────────────────────────────────────

    #[test]
    fn duplicate_top_level_id_is_error() {
        let coll = ParameterCollection::new()
            .add(Parameter::string("name").label("Name"))
            .add(Parameter::string("name").label("Name 2"));

        let diags = lint_collection(&coll);
        assert!(
            diags
                .iter()
                .any(|d| d.level == LintLevel::Error && d.message.contains("duplicate")),
            "expected duplicate-id error, got: {diags:?}"
        );
    }

    // ── Empty IDs (2) ───────────────────────────────────────────────────

    #[test]
    fn empty_id_is_error() {
        let coll = ParameterCollection::new().add(Parameter::string(""));

        let diags = lint_collection(&coll);
        assert!(
            diags
                .iter()
                .any(|d| d.level == LintLevel::Error && d.message.contains("empty")),
            "expected empty-id error, got: {diags:?}"
        );
    }

    // ── Contradictory min/max length (9) ────────────────────────────────

    #[test]
    fn contradictory_min_max_length_is_error() {
        let coll = ParameterCollection::new().add(
            Parameter::string("slug")
                .label("Slug")
                .with_rule(Rule::MinLength {
                    min: 10,
                    message: None,
                })
                .with_rule(Rule::MaxLength {
                    max: 5,
                    message: None,
                }),
        );

        let diags = lint_collection(&coll);
        assert!(
            diags
                .iter()
                .any(|d| d.level == LintLevel::Error && d.message.contains("min_length")),
            "expected contradictory min/max error, got: {diags:?}"
        );
    }

    #[test]
    fn equal_min_max_length_is_not_contradictory() {
        let coll = ParameterCollection::new().add(
            Parameter::string("pin")
                .label("PIN")
                .with_rule(Rule::MinLength {
                    min: 4,
                    message: None,
                })
                .with_rule(Rule::MaxLength {
                    max: 4,
                    message: None,
                }),
        );

        let diags = lint_collection(&coll);
        assert!(
            !diags
                .iter()
                .any(|d| d.level == LintLevel::Error && d.path.starts_with("pin")),
            "equal min==max should not be flagged, got: {diags:?}"
        );
    }

    // ── Contradictory min/max items (10) ────────────────────────────────

    #[test]
    fn contradictory_min_max_items_is_error() {
        let coll = ParameterCollection::new().add(
            Parameter::string("tags")
                .label("Tags")
                .with_rule(Rule::MinItems {
                    min: 5,
                    message: None,
                })
                .with_rule(Rule::MaxItems {
                    max: 2,
                    message: None,
                }),
        );

        let diags = lint_collection(&coll);
        assert!(
            diags
                .iter()
                .any(|d| d.level == LintLevel::Error && d.message.contains("min_items")),
            "expected contradictory min/max items error, got: {diags:?}"
        );
    }

    // ── Notice diagnostics (20, 21) ─────────────────────────────────────

    #[test]
    fn notice_with_required_is_warning() {
        let coll = ParameterCollection::new()
            .add(Parameter::notice("info").description("desc").required());

        let diags = lint_collection(&coll);
        assert!(
            diags
                .iter()
                .any(|d| d.level == LintLevel::Warning && d.message.contains("required")),
            "expected warning for required notice, got: {diags:?}"
        );
    }

    #[test]
    fn notice_without_description_is_warning() {
        let coll = ParameterCollection::new().add(Parameter::notice("info"));

        let diags = lint_collection(&coll);
        assert!(
            diags
                .iter()
                .any(|d| d.level == LintLevel::Warning && d.message.contains("description")),
            "expected warning for missing description, got: {diags:?}"
        );
    }

    #[test]
    fn notice_with_description_and_no_extras_is_clean() {
        let coll =
            ParameterCollection::new().add(Parameter::notice("info").description("hello world"));

        let diags = lint_collection(&coll);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }
}
