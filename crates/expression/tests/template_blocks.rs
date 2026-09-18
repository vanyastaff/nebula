//! Template control-flow blocks: `{% if %}`, `{% for %}`, `{# #}`.
//!
//! Pins the block surface: branch selection, nesting, loop variables,
//! `{% else %}` on both tags, whitespace control, escape rules, and the
//! compile-time errors for malformed structure.

use nebula_expression::{
    CompiledProgram, EvaluationContext, EvaluationPolicy, EvaluationStepLimit, ExpressionEngine,
    ExpressionError, ProgramSyntax, Template,
};
use serde_json::json;

fn engine() -> ExpressionEngine {
    ExpressionEngine::new()
}

fn context() -> EvaluationContext {
    EvaluationContext::builder()
        .input(json!({
            "on": true,
            "off": false,
            "count": 2,
            "items": ["a", "b", "c"],
            "empty": [],
        }))
        .build()
}

fn render(source: &str) -> String {
    Template::new(source)
        .unwrap_or_else(|error| panic!("{source}: {error}"))
        .render(&engine(), &context())
        .unwrap_or_else(|error| panic!("{source}: {error}"))
}

/// Compile-or-render failure for a malformed template.
fn render_error(source: &str) -> ExpressionError {
    match Template::new(source) {
        Ok(template) => template
            .render(&engine(), &context())
            .expect_err("malformed template must fail"),
        Err(error) => error,
    }
}

// ──────────────────────────────────────────────
// if
// ──────────────────────────────────────────────

#[test]
fn if_selects_the_first_truthy_branch() {
    assert_eq!(
        render("{% if $input.on %}yes{% else %}no{% endif %}"),
        "yes"
    );
    assert_eq!(
        render("{% if $input.off %}yes{% else %}no{% endif %}"),
        "no"
    );
    assert_eq!(
        render("{% if $input.off %}a{% elif $input.on %}b{% else %}c{% endif %}"),
        "b"
    );
    assert_eq!(
        render("{% if $input.off %}a{% elif $input.off %}b{% else %}c{% endif %}"),
        "c"
    );
}

#[test]
fn if_without_else_renders_nothing_when_false() {
    assert_eq!(render("[{% if $input.off %}hidden{% endif %}]"), "[]");
}

#[test]
fn if_branches_nest() {
    assert_eq!(
        render("{% if $input.on %}{% if $input.count > 1 %}nested{% endif %}{% endif %}"),
        "nested"
    );
}

// ──────────────────────────────────────────────
// for
// ──────────────────────────────────────────────

#[test]
fn for_iterates_the_bound_name() {
    assert_eq!(
        render("{% for x in $input.items %}{{ x }}-{% endfor %}"),
        "a-b-c-"
    );
}

#[test]
fn for_exposes_loop_variables() {
    assert_eq!(
        render("{% for x in $input.items %}{{ loop.index }}{{ loop.index0 }}{% endfor %}"),
        "102132"
    );
    assert_eq!(
        render(
            "{% for x in $input.items %}{% if loop.first %}F{% endif %}{% if loop.last %}L{% endif %}{% endfor %}"
        ),
        "FL"
    );
    assert_eq!(
        render("{% for x in $input.items %}{{ loop.length }}{% endfor %}"),
        "333"
    );
}

#[test]
fn for_else_renders_only_for_an_empty_iterable() {
    assert_eq!(
        render("{% for x in $input.empty %}item{% else %}empty{% endfor %}"),
        "empty"
    );
    assert_eq!(
        render("{% for x in $input.items %}item{% else %}empty{% endfor %}"),
        "itemitemitem"
    );
}

#[test]
fn for_bodies_nest_conditionals_and_other_loops() {
    assert_eq!(
        render(
            "{% for x in $input.items %}{% if x == 'b' %}B{% else %}{{ x }}{% endif %}{% endfor %}"
        ),
        "aBc"
    );
    assert_eq!(
        render("{% for x in [1, 2] %}{% for y in [10, 20] %}{{ x * y }} {% endfor %}{% endfor %}"),
        "10 20 20 40 "
    );
}

#[test]
fn nested_loops_shadow_loop_object() {
    // The inner loop's `loop` must not leak its last index to the outer
    // one after the body finishes.
    assert_eq!(
        render(
            "{% for x in [1, 2] %}{% for y in [1, 2, 3] %}{% endfor %}{{ loop.index }}{% endfor %}"
        ),
        "12"
    );
}

// ──────────────────────────────────────────────
// Comments and escapes
// ──────────────────────────────────────────────

#[test]
fn comments_produce_no_output() {
    assert_eq!(render("a{# hidden {{ $input.on }} #}b"), "ab");
    assert_eq!(render("{# only a comment #}"), "");
}

#[test]
fn escaped_block_delimiters_stay_literal() {
    assert_eq!(
        render(r"\{% if true %}\{% endif %}"),
        "{% if true %}{% endif %}"
    );
    assert_eq!(render("{#{# literal"), "{# literal");
}

// ──────────────────────────────────────────────
// Whitespace control
// ──────────────────────────────────────────────

#[test]
fn block_strip_markers_trim_the_edges() {
    assert_eq!(
        render("before {%- if $input.on -%}  yes  {%- endif -%} after"),
        "beforeyesafter"
    );
}

#[test]
fn for_strip_markers_apply_per_iteration() {
    assert_eq!(
        render("{% for x in $input.items -%}\n  {{ x }}\n{%- endfor %}"),
        "abc"
    );
}

// ──────────────────────────────────────────────
// Compile-time structure errors
// ──────────────────────────────────────────────

#[test]
fn unclosed_blocks_fail_at_compile_time() {
    for source in ["{% if true %}x", "{% for x in [] %}x", "{# unclosed"] {
        let error = Template::new(source).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("Unclosed") || message.contains("expected closing"),
            "{source}: {message}"
        );
    }
}

#[test]
fn mismatched_and_unknown_tags_fail_at_compile_time() {
    for (source, expected) in [
        ("{% endif %}", "Unexpected"),
        ("{% endfor %}", "Unexpected"),
        ("{% else %}", "Unexpected"),
        ("{% nope %}", "Unknown template tag"),
        ("{% if $input.on %}{% endfor %}", "Unexpected"),
        ("{% for x in $input.items %}{% endif %}", "Unexpected"),
        ("{% if %}", "requires a condition"),
        ("{% for x %}x{% endfor %}", "requires"),
    ] {
        let error = render_error(source);
        let message = error.to_string();
        assert!(
            message.contains(expected),
            "{source}: expected `{expected}` in {message}"
        );
    }
}

// ──────────────────────────────────────────────
// Program-level behavior
// ──────────────────────────────────────────────

#[test]
fn template_blocks_compile_with_explicit_template_syntax() {
    let program =
        CompiledProgram::compile_with_syntax("{% if true %}x{% endif %}", ProgramSyntax::Template)
            .unwrap();
    assert_eq!(program.syntax(), ProgramSyntax::Template);
    assert_eq!(
        engine().evaluate_compiled(&program, &context()).unwrap(),
        json!("x")
    );
}

#[test]
fn auto_compilation_keeps_a_blocked_template_a_string() {
    // AUTO keeps the JSON type of a lone `{{ … }}` envelope. A template with
    // a control-flow tag is not an envelope, so it must stay a string even
    // when its body could evaluate to another type.
    let engine = engine();
    let program = CompiledProgram::compile("{% if true %}{{ 1 }}{% endif %}").unwrap();
    assert_eq!(program.syntax(), ProgramSyntax::Auto);
    assert_eq!(
        engine.evaluate_compiled(&program, &context()).unwrap(),
        json!("1")
    );
}

#[test]
fn empty_iterable_under_the_result_type_check_is_an_empty_string() {
    assert_eq!(render("{% for x in $input.empty %}{{ x }}{% endfor %}"), "");
}

#[test]
fn block_evaluation_shares_the_step_budget() {
    // A loop must charge every iteration against one budget: the same
    // template that completes under the default policy must abort under a
    // budget that a fraction of the iterations would exhaust.
    let source = format!(
        "{{% for x in [{}] %}}{{{{ x }}}}{{% endfor %}}",
        (0..50)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let template = Template::new(source.as_str()).unwrap();
    assert_eq!(
        template.render(&engine(), &context()).unwrap(),
        "012345678910111213141516171819202122232425262728293031323334353637383940414243444546474849",
        "the default budget must complete all 50 iterations"
    );

    let bounded = ExpressionEngine::new().with_policy(
        EvaluationPolicy::new().with_max_eval_steps(EvaluationStepLimit::new(20).unwrap()),
    );
    let error = template.render(&bounded, &context()).unwrap_err();
    assert!(
        matches!(error, ExpressionError::StepLimitExceeded { .. }),
        "got: {error}"
    );
}

#[test]
fn loop_variables_resolve_as_typed_values() {
    // `loop.index` must be a number, not a string that happens to print right.
    let engine = engine();
    let program =
        CompiledProgram::compile_template("{% for x in [1] %}{{ loop.index + 1 }}{% endfor %}")
            .unwrap();
    assert_eq!(
        engine.evaluate_compiled(&program, &context()).unwrap(),
        json!("2")
    );
}
