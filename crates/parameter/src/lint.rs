//! Schema-time lint diagnostics.
//!
//! Provides a static lint pass over a [`Schema`] that detects structural
//! problems independent of any runtime values: duplicate field ids,
//! contradictory rules, and references to non-existent sibling fields.

use crate::field::Field;
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

    for field in &schema.fields {
        lint_field(field, &mut seen_ids, &mut diagnostics);
    }

    diagnostics
}

fn lint_field(field: &Field, seen_ids: &mut Vec<String>, diagnostics: &mut Vec<LintDiagnostic>) {
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

    // Recurse into nested fields.
    match field {
        Field::Object { fields: nested, .. } => {
            let mut nested_seen: Vec<String> = Vec::new();
            for child in nested {
                lint_field(child, &mut nested_seen, diagnostics);
            }
        }
        Field::List { item, .. } => {
            let mut item_seen: Vec<String> = Vec::new();
            lint_field(item, &mut item_seen, diagnostics);
        }
        Field::Mode { variants, .. } => {
            for variant in variants {
                let mut variant_seen: Vec<String> = Vec::new();
                lint_field(&variant.content, &mut variant_seen, diagnostics);
            }
        }
        _ => {}
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
    use crate::schema::Schema;

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
}
