//! Integration tests for the schema lint pass.
//!
//! Covers: clean schemas, duplicate field ids, contradictory rules,
//! nested field lint, empty ids.

use nebula_parameter::lint::{LintLevel, lint_schema};
use nebula_parameter::{Field, FieldMetadata, Rule, Schema};

// ── Clean schemas ────────────────────────────────────────────────────────────

#[test]
fn clean_schema_produces_no_diagnostics() {
    let schema = Schema::new()
        .field(Field::text("name").with_label("Name").required())
        .field(Field::integer("age").with_label("Age"))
        .field(Field::boolean("active").with_label("Active"));

    assert!(lint_schema(&schema).is_empty());
}

#[test]
fn empty_schema_is_lint_clean() {
    assert!(lint_schema(&Schema::new()).is_empty());
}

// ── Duplicate ids ────────────────────────────────────────────────────────────

#[test]
fn duplicate_top_level_id_is_error() {
    let schema = Schema::new()
        .field(Field::text("name").with_label("Name"))
        .field(Field::text("name").with_label("Name Duplicate"));

    let diags = lint_schema(&schema);
    assert!(
        diags
            .iter()
            .any(|d| d.level == LintLevel::Error && d.path == "name"),
        "expected duplicate-id error for 'name', got: {diags:?}"
    );
}

#[test]
fn three_fields_two_duplicates_reports_two_errors() {
    let schema = Schema::new()
        .field(Field::text("x").with_label("X"))
        .field(Field::text("x").with_label("X2"))
        .field(Field::text("x").with_label("X3"));

    let diags = lint_schema(&schema);
    let dup_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.level == LintLevel::Error && d.path == "x")
        .collect();
    assert_eq!(
        dup_errors.len(),
        2,
        "second and third occurrence should both be errors"
    );
}

// ── Empty ids ────────────────────────────────────────────────────────────────

#[test]
fn empty_field_id_is_error() {
    let schema = Schema::new().field(Field::Text {
        meta: FieldMetadata {
            id: String::new(),
            label: "Unnamed".to_owned(),
            ..FieldMetadata::default()
        },
        multiline: false,
    });

    let diags = lint_schema(&schema);
    assert!(
        diags.iter().any(|d| d.level == LintLevel::Error),
        "expected error for empty id, got: {diags:?}"
    );
}

// ── Contradictory min/max length ─────────────────────────────────────────────

#[test]
fn contradictory_min_max_length_is_error() {
    let schema = Schema::new().field(
        Field::text("slug")
            .with_label("Slug")
            .with_rule(Rule::MinLength {
                min: 10,
                message: None,
            })
            .with_rule(Rule::MaxLength {
                max: 5,
                message: None,
            }),
    );

    let diags = lint_schema(&schema);
    assert!(
        diags
            .iter()
            .any(|d| d.level == LintLevel::Error && d.path.starts_with("slug")),
        "expected contradictory min/max length error, got: {diags:?}"
    );
}

#[test]
fn equal_min_max_length_is_not_contradictory() {
    let schema = Schema::new().field(
        Field::text("pin")
            .with_label("PIN")
            .with_rule(Rule::MinLength {
                min: 4,
                message: None,
            })
            .with_rule(Rule::MaxLength {
                max: 4,
                message: None,
            }),
    );

    let diags = lint_schema(&schema);
    assert!(
        !diags
            .iter()
            .any(|d| d.level == LintLevel::Error && d.path.starts_with("pin")),
        "equal min==max should not be flagged, got: {diags:?}"
    );
}

// ── Contradictory min/max items ──────────────────────────────────────────────

#[test]
fn contradictory_min_max_items_is_error() {
    let schema = Schema::new().field(
        Field::text("tags")
            .with_label("Tags")
            .with_rule(Rule::MinItems {
                min: 5,
                message: None,
            })
            .with_rule(Rule::MaxItems {
                max: 2,
                message: None,
            }),
    );

    let diags = lint_schema(&schema);
    assert!(
        diags
            .iter()
            .any(|d| d.level == LintLevel::Error && d.path.starts_with("tags")),
        "expected contradictory min/max items error, got: {diags:?}"
    );
}

// ── Mixed valid and invalid ──────────────────────────────────────────────────

#[test]
fn only_problematic_fields_are_flagged() {
    let schema = Schema::new()
        .field(Field::text("ok_field").with_label("OK"))
        .field(Field::text("dup").with_label("Dup A"))
        .field(Field::text("dup").with_label("Dup B")); // duplicate

    let diags = lint_schema(&schema);
    assert!(
        diags.iter().all(|d| d.path == "dup"),
        "only 'dup' should be flagged"
    );
    assert!(
        !diags.iter().any(|d| d.path == "ok_field"),
        "ok_field should be clean"
    );
}
