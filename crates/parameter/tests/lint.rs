//! Integration tests for the v3 schema lint pass.
//!
//! Covers all 23 diagnostic categories: structure, reference, rule consistency,
//! object/mode, transformer, notice, and filter checks.

use nebula_parameter::{
    collection::ParameterCollection,
    conditions::Condition,
    filter_field::{FilterField, FilterFieldType},
    lint::{LintDiagnostic, LintLevel, lint_collection},
    parameter::Parameter,
    path::ParameterPath,
    transformer::Transformer,
};
use nebula_validator::Rule;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn has_error(diags: &[LintDiagnostic], needle: &str) -> bool {
    diags
        .iter()
        .any(|d| d.level == LintLevel::Error && d.message.contains(needle))
}

fn has_warning(diags: &[LintDiagnostic], needle: &str) -> bool {
    diags
        .iter()
        .any(|d| d.level == LintLevel::Warning && d.message.contains(needle))
}

fn has_error_at(diags: &[LintDiagnostic], path_prefix: &str, needle: &str) -> bool {
    diags.iter().any(|d| {
        d.level == LintLevel::Error && d.path.starts_with(path_prefix) && d.message.contains(needle)
    })
}

fn has_warning_at(diags: &[LintDiagnostic], path_prefix: &str, needle: &str) -> bool {
    diags.iter().any(|d| {
        d.level == LintLevel::Warning
            && d.path.starts_with(path_prefix)
            && d.message.contains(needle)
    })
}

// ── Clean schemas ───────────────────────────────────────────────────────────

#[test]
fn clean_schema_produces_no_diagnostics() {
    let coll = ParameterCollection::new()
        .add(Parameter::string("name").label("Name").required())
        .add(Parameter::integer("age").label("Age"))
        .add(Parameter::boolean("active").label("Active"));

    assert!(lint_collection(&coll).is_empty());
}

#[test]
fn empty_schema_is_lint_clean() {
    assert!(lint_collection(&ParameterCollection::new()).is_empty());
}

// ── 1. Duplicate parameter IDs ──────────────────────────────────────────────

#[test]
fn duplicate_top_level_id_is_error() {
    let coll = ParameterCollection::new()
        .add(Parameter::string("name").label("Name"))
        .add(Parameter::string("name").label("Name Duplicate"));

    let diags = lint_collection(&coll);
    assert!(
        has_error(&diags, "duplicate"),
        "expected duplicate-id error, got: {diags:?}"
    );
}

#[test]
fn three_fields_two_duplicates_reports_two_errors() {
    let coll = ParameterCollection::new()
        .add(Parameter::string("x").label("X"))
        .add(Parameter::string("x").label("X2"))
        .add(Parameter::string("x").label("X3"));

    let diags = lint_collection(&coll);
    let dup_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.level == LintLevel::Error && d.message.contains("duplicate"))
        .collect();
    assert_eq!(
        dup_errors.len(),
        2,
        "second and third occurrence should both be errors, got: {dup_errors:?}"
    );
}

// ── 2. Empty parameter IDs ─────────────────────────────────────────────────

#[test]
fn empty_field_id_is_error() {
    let coll = ParameterCollection::new().add(Parameter::string(""));

    let diags = lint_collection(&coll);
    assert!(
        has_error(&diags, "empty"),
        "expected error for empty id, got: {diags:?}"
    );
}

// ── 3. Duplicate mode variant IDs ──────────────────────────────────────────

#[test]
fn duplicate_mode_variant_ids_is_error() {
    let coll = ParameterCollection::new().add(
        Parameter::mode("output")
            .variant(Parameter::string("json").label("JSON"))
            .variant(Parameter::string("json").label("JSON 2")),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_error(&diags, "duplicate"),
        "expected duplicate variant ID error, got: {diags:?}"
    );
}

// ── 4. Invalid default_variant ─────────────────────────────────────────────

#[test]
fn invalid_default_variant_is_error() {
    let coll = ParameterCollection::new().add(
        Parameter::mode("output")
            .variant(Parameter::string("json").label("JSON"))
            .variant(Parameter::string("xml").label("XML"))
            .default_variant("csv"),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_error(&diags, "default_variant"),
        "expected invalid default_variant error, got: {diags:?}"
    );
}

#[test]
fn valid_default_variant_is_clean() {
    let coll = ParameterCollection::new().add(
        Parameter::mode("output")
            .variant(Parameter::string("json").label("JSON"))
            .variant(Parameter::string("xml").label("XML"))
            .default_variant("json"),
    );

    let diags = lint_collection(&coll);
    assert!(
        !has_error(&diags, "default_variant"),
        "valid default_variant should not be flagged, got: {diags:?}"
    );
}

// ── 5. Condition references unknown fields ─────────────────────────────────

#[test]
fn visible_when_references_unknown_field_is_error() {
    let coll = ParameterCollection::new()
        .add(Parameter::string("name").label("Name"))
        .add(
            Parameter::string("nickname")
                .label("Nickname")
                .visible_when(Condition::eq("nonexistent", "yes")),
        );

    let diags = lint_collection(&coll);
    assert!(
        has_error(&diags, "unknown field"),
        "expected unknown field error, got: {diags:?}"
    );
}

#[test]
fn visible_when_references_known_sibling_is_clean() {
    let coll = ParameterCollection::new()
        .add(Parameter::boolean("show_extra").label("Show Extra"))
        .add(
            Parameter::string("extra")
                .label("Extra")
                .visible_when(Condition::eq("show_extra", true)),
        );

    let diags = lint_collection(&coll);
    assert!(
        !has_error(&diags, "unknown field"),
        "expected no unknown field errors, got: {diags:?}"
    );
}

// ── 7. depends_on self-reference ────────────────────────────────────────────

#[test]
fn depends_on_self_reference_is_error() {
    let coll = ParameterCollection::new()
        .add(Parameter::string("source").label("Source"))
        .add(Parameter::select("target").depends_on(&["target"]));

    let diags = lint_collection(&coll);
    assert!(
        has_error(&diags, "self-reference"),
        "expected self-reference error, got: {diags:?}"
    );
}

// ── 6. depends_on references non-existent parameter ────────────────────────

#[test]
fn depends_on_nonexistent_parameter_is_error() {
    let coll =
        ParameterCollection::new().add(Parameter::select("items").depends_on(&["missing_param"]));

    let diags = lint_collection(&coll);
    assert!(
        has_error(&diags, "non-existent"),
        "expected non-existent parameter error, got: {diags:?}"
    );
}

// ── 8. $root.x references non-existent root parameter ─────────────────────

#[test]
fn root_reference_to_nonexistent_parameter_is_error() {
    let coll = ParameterCollection::new()
        .add(Parameter::string("name").label("Name"))
        .add(
            Parameter::string("other")
                .label("Other")
                .visible_when(Condition::eq(ParameterPath::root("missing"), "yes")),
        );

    let diags = lint_collection(&coll);
    assert!(
        has_error(&diags, "non-existent root"),
        "expected root reference error, got: {diags:?}"
    );
}

// ── 9. Contradictory min_length > max_length ────────────────────────────────

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
        has_error_at(&diags, "slug", "min_length"),
        "expected contradictory min/max length error, got: {diags:?}"
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
        !has_error_at(&diags, "pin", "min_length"),
        "equal min==max should not be flagged, got: {diags:?}"
    );
}

// ── 10. Contradictory min_items > max_items ─────────────────────────────────

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
        has_error_at(&diags, "tags", "min_items"),
        "expected contradictory min/max items error, got: {diags:?}"
    );
}

// ── 11. Sections Object sub-param missing group ─────────────────────────────

#[test]
fn sections_object_sub_param_missing_group_is_warning() {
    let coll = ParameterCollection::new().add(
        Parameter::object("config")
            .add(Parameter::string("host").label("Host"))
            .add(Parameter::string("port").label("Port"))
            .add(Parameter::string("path").label("Path"))
            .sections(),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "missing `group`"),
        "expected missing group warning, got: {diags:?}"
    );
}

#[test]
fn sections_object_sub_param_with_group_is_clean() {
    let coll = ParameterCollection::new().add(
        Parameter::object("config")
            .add(Parameter::string("host").label("Host").group("network"))
            .add(Parameter::string("port").label("Port").group("network"))
            .add(Parameter::string("path").label("Path").group("paths"))
            .sections(),
    );

    let diags = lint_collection(&coll);
    assert!(
        !has_warning(&diags, "missing `group`"),
        "all sub-params have group, should be clean, got: {diags:?}"
    );
}

// ── 12. group on parameter inside non-Sections Object ───────────────────────

#[test]
fn group_on_inline_object_sub_param_is_warning() {
    let coll = ParameterCollection::new().add(
        Parameter::object("config")
            .add(Parameter::string("host").label("Host").group("oops"))
            .add(Parameter::string("port").label("Port"))
            .add(Parameter::string("path").label("Path")),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "not Sections mode"),
        "expected group-on-non-sections warning, got: {diags:?}"
    );
}

// ── 13. required on sub-parameter of PickFields Object ──────────────────────

#[test]
fn required_on_pickfields_sub_param_is_warning() {
    let coll = ParameterCollection::new().add(
        Parameter::object("config")
            .add(Parameter::string("a").label("A").required())
            .add(Parameter::string("b").label("B"))
            .add(Parameter::string("c").label("C"))
            .pick_fields(),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "PickFields"),
        "expected PickFields required warning, got: {diags:?}"
    );
}

// ── 14. PickFields/Sections with <=2 sub-parameters ────────────────────────

#[test]
fn pickfields_with_two_params_is_warning() {
    let coll = ParameterCollection::new().add(
        Parameter::object("config")
            .add(Parameter::string("a").label("A"))
            .add(Parameter::string("b").label("B"))
            .pick_fields(),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_warning_at(&diags, "config", "only 2 sub-parameters"),
        "expected too-few-params warning, got: {diags:?}"
    );
}

// ── 15. Mode variant missing label ─────────────────────────────────────────

#[test]
fn mode_variant_missing_label_is_warning() {
    let coll = ParameterCollection::new().add(
        Parameter::mode("format")
            .variant(Parameter::string("json")) // no label
            .variant(Parameter::string("xml").label("XML")),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "missing a label"),
        "expected variant missing label warning, got: {diags:?}"
    );
}

// ── 16. Transformer on non-string parameter ────────────────────────────────

#[test]
fn trim_on_number_is_warning() {
    let coll = ParameterCollection::new().add(
        Parameter::integer("count")
            .label("Count")
            .transformer(Transformer::Trim),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "non-string"),
        "expected non-string transformer warning, got: {diags:?}"
    );
}

// ── 17. Invalid regex pattern in Transformer::Regex ─────────────────────────

#[test]
fn invalid_regex_in_transformer_is_warning() {
    let coll = ParameterCollection::new().add(Parameter::string("data").label("Data").transformer(
        Transformer::Regex {
            pattern: "[invalid".into(),
            group: 1,
        },
    ));

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "invalid regex"),
        "expected invalid regex warning, got: {diags:?}"
    );
}

// ── 18. Regex capture group 0 ──────────────────────────────────────────────

#[test]
fn regex_capture_group_zero_is_warning() {
    let coll = ParameterCollection::new().add(Parameter::string("data").label("Data").transformer(
        Transformer::Regex {
            pattern: r"(\d+)".into(),
            group: 0,
        },
    ));

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "group 0"),
        "expected group 0 warning, got: {diags:?}"
    );
}

// ── 19. Chain/FirstMatch with single transformer ────────────────────────────

#[test]
fn chain_with_single_transformer_is_warning() {
    let coll = ParameterCollection::new().add(Parameter::string("data").label("Data").transformer(
        Transformer::Chain {
            transformers: vec![Transformer::Trim],
        },
    ));

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "single transformer"),
        "expected single-transformer Chain warning, got: {diags:?}"
    );
}

#[test]
fn first_match_with_single_transformer_is_warning() {
    let coll = ParameterCollection::new().add(Parameter::string("data").label("Data").transformer(
        Transformer::FirstMatch {
            transformers: vec![Transformer::Trim],
        },
    ));

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "single transformer"),
        "expected single-transformer FirstMatch warning, got: {diags:?}"
    );
}

// ── 20. Notice with required/secret/default/rules ──────────────────────────

#[test]
fn notice_with_required_is_warning() {
    let coll = ParameterCollection::new().add(
        Parameter::notice("info")
            .description("Read this")
            .required(),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "required"),
        "expected notice-with-required warning, got: {diags:?}"
    );
}

#[test]
fn notice_with_secret_is_warning() {
    let coll =
        ParameterCollection::new().add(Parameter::notice("info").description("Read this").secret());

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "secret"),
        "expected notice-with-secret warning, got: {diags:?}"
    );
}

// ── 21. Notice without description ─────────────────────────────────────────

#[test]
fn notice_without_description_is_warning() {
    let coll = ParameterCollection::new().add(Parameter::notice("info"));

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "description"),
        "expected notice-without-description warning, got: {diags:?}"
    );
}

#[test]
fn notice_with_description_and_no_extras_is_clean() {
    let coll =
        ParameterCollection::new().add(Parameter::notice("info").description("Important info"));

    let diags = lint_collection(&coll);
    assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
}

// ── 22. Filter with no static fields and no fields_loader ──────────────────

#[test]
fn filter_with_no_fields_is_warning() {
    let coll = ParameterCollection::new().add(Parameter::filter("query"));

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "no static fields"),
        "expected no-fields warning, got: {diags:?}"
    );
}

// ── 23. Filter with duplicate field IDs ────────────────────────────────────

#[test]
fn filter_with_duplicate_field_ids_is_warning() {
    let coll = ParameterCollection::new().add(
        Parameter::filter("query")
            .filter_field(FilterField {
                id: "status".into(),
                label: "Status".into(),
                field_type: FilterFieldType::String,
            })
            .filter_field(FilterField {
                id: "status".into(),
                label: "Status 2".into(),
                field_type: FilterFieldType::String,
            }),
    );

    let diags = lint_collection(&coll);
    assert!(
        has_warning(&diags, "duplicate filter field"),
        "expected duplicate filter field warning, got: {diags:?}"
    );
}

// ── Mixed: only problematic fields are flagged ─────────────────────────────

#[test]
fn only_problematic_fields_are_flagged() {
    let coll = ParameterCollection::new()
        .add(Parameter::string("ok_field").label("OK"))
        .add(Parameter::string("dup").label("Dup A"))
        .add(Parameter::string("dup").label("Dup B"));

    let diags = lint_collection(&coll);
    assert!(
        diags.iter().all(|d| d.path == "dup"),
        "only 'dup' should be flagged, got: {diags:?}"
    );
    assert!(
        !diags.iter().any(|d| d.path == "ok_field"),
        "ok_field should be clean"
    );
}
