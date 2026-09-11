use std::assert_matches;

use nebula_expression::{
    BuiltinOutput, BuiltinOutputBound, BuiltinOutputBuilder, BuiltinOutputLimit, CompiledProgram,
    EvaluationContext, EvaluationPolicy, EvaluationStepLimit, ExpressionEngine, ExpressionError,
    ExpressionResult, ProgramSyntax, Template, eval::BuiltinView, parse_expression,
};
use serde_json::{Value, json};

fn step_limit(max_steps: usize) -> EvaluationStepLimit {
    EvaluationStepLimit::new(max_steps).unwrap()
}

fn oversized_result(
    _arguments: &[&Value],
    _view: BuiltinView<'_>,
    _context: &EvaluationContext,
    output: BuiltinOutputBuilder,
) -> ExpressionResult<BuiltinOutput> {
    output.repeat_string("x", 1_048_577)
}

#[test]
fn auto_evaluation_preserves_mixed_text() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::builder()
        .input(json!({"name": "Ada"}))
        .build();
    assert_eq!(
        engine
            .evaluate("Hello {{ $input.name }}!", &context)
            .unwrap(),
        json!("Hello Ada!")
    );
}

#[test]
fn parse_only_accepts_quoted_template_markers_as_raw_strings() {
    let source = "'{{ 1 + }}'";
    parse_expression(source).unwrap();
    assert_eq!(
        ExpressionEngine::new()
            .evaluate(source, &EvaluationContext::new())
            .unwrap(),
        json!("{{ 1 + }}")
    );
}

#[test]
fn template_delimiters_inside_strings_are_not_boundaries() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();
    let template = Template::new("before {{ '}}' }} after").unwrap();
    assert_eq!(
        template.render(&engine, &context).unwrap(),
        "before }} after"
    );
}

#[test]
fn nested_object_braces_are_not_template_boundaries() {
    assert_eq!(
        ExpressionEngine::new()
            .evaluate("{{ {outer:{value:3}} }}", &EvaluationContext::new())
            .unwrap(),
        json!({"outer": {"value": 3}})
    );
}

#[test]
fn template_expressions_share_one_step_budget() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(4)));
    let template = Template::new("{{ 1 + 2 }}{{ 3 + 4 }}").unwrap();
    let error = template
        .render(&engine, &EvaluationContext::new())
        .unwrap_err();
    assert_matches!(
        error,
        ExpressionError::StepLimitExceeded {
            limit: 4,
            actual: 5
        }
    );
}

#[test]
fn context_cannot_raise_engine_step_limit() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(3)));
    let context = EvaluationContext::builder()
        .policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(100)))
        .build();
    let error = engine.evaluate("1 + 2 + 3", &context).unwrap_err();
    assert_matches!(
        error,
        ExpressionError::StepLimitExceeded {
            limit: 3,
            actual: 4
        }
    );
}

#[test]
fn context_cannot_raise_engine_json_parse_limit() {
    let engine =
        ExpressionEngine::new().with_policy(EvaluationPolicy::new().with_max_json_parse_length(4));
    let context = EvaluationContext::builder()
        .policy(EvaluationPolicy::new().with_max_json_parse_length(100))
        .build();
    engine
        .evaluate("parse_json('[1,2,3]')", &context)
        .unwrap_err();
}

#[test]
fn builtin_output_configuration_cannot_raise_fixed_hard_limits() {
    let unbounded = BuiltinOutputBound::new(usize::MAX).unwrap();
    let limits = EvaluationPolicy::new()
        .with_max_builtin_output_bytes(unbounded)
        .with_max_builtin_output_string_bytes(unbounded)
        .with_max_builtin_output_collection_items(unbounded)
        .with_max_builtin_output_nodes(unbounded)
        .with_max_builtin_output_depth(unbounded)
        .builtin_output_limits();

    assert_eq!(
        limits.max_total_bytes(),
        nebula_expression::BuiltinOutputLimits::DEFAULT_MAX_TOTAL_BYTES
    );
    assert_eq!(
        limits.max_string_bytes(),
        nebula_expression::BuiltinOutputLimits::DEFAULT_MAX_STRING_BYTES
    );
    assert_eq!(
        limits.max_collection_items(),
        nebula_expression::BuiltinOutputLimits::DEFAULT_MAX_COLLECTION_ITEMS
    );
    assert_eq!(
        limits.max_value_nodes(),
        nebula_expression::BuiltinOutputLimits::DEFAULT_MAX_VALUE_NODES
    );
    assert_eq!(
        limits.max_value_depth(),
        nebula_expression::BuiltinOutputLimits::DEFAULT_MAX_VALUE_DEPTH
    );
}

#[test]
fn retained_program_compilation_modes_preserve_output_types() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();
    let cases = [
        ("1 + 2", json!(3)),
        (" \n{{- 1 + 2 -}}\t", json!(3)),
        ("'{{ 1 + }}'", json!("{{ 1 + }}")),
        ("{{ null }}", json!(null)),
        ("{{ [1,2] }}", json!([1, 2])),
        ("{{ 1 }} / {{ 2 }}", json!("1 / 2")),
        ("\u{00e9} {{ '\u{03bb}' }}", json!("\u{00e9} \u{03bb}")),
    ];
    for (source, expected) in cases {
        let program = CompiledProgram::compile(source).unwrap();
        assert_eq!(program.source(), source);
        assert_eq!(
            engine.evaluate_compiled(&program, &context).unwrap(),
            expected,
            "{source}"
        );
    }
    let template = CompiledProgram::compile_template("{{ 1 + 2 }}").unwrap();
    assert_eq!(
        engine.evaluate_compiled(&template, &context).unwrap(),
        json!("3")
    );
    let text = CompiledProgram::compile_template("hello").unwrap();
    assert_eq!(
        engine.evaluate_compiled(&text, &context).unwrap(),
        json!("hello")
    );
    CompiledProgram::compile_expression("{{ 1 + 2 }}").unwrap_err();
}

#[test]
fn requested_syntax_survives_auto_body_selection_and_cloning() {
    for source in ["7", "'{{ 7 }}'", " {{ 7 }} ", "before {{ 7 }} after"] {
        let program = CompiledProgram::compile(source).unwrap();
        assert_eq!(program.syntax(), ProgramSyntax::Auto);
        assert_eq!(program.clone().syntax(), ProgramSyntax::Auto);
        assert_eq!(program.source(), source);
    }
    assert_eq!(
        CompiledProgram::compile_expression("7").unwrap().syntax(),
        ProgramSyntax::Expression
    );
    assert_eq!(
        CompiledProgram::compile_template("7").unwrap().syntax(),
        ProgramSyntax::Template
    );
}

#[test]
fn explicit_syntax_controls_quoted_markers_and_envelopes() {
    let engine = ExpressionEngine::new();
    let context = EvaluationContext::new();
    for (syntax, source, expected) in [
        (ProgramSyntax::Auto, "{{ 7 }}", json!(7)),
        (ProgramSyntax::Template, "{{ 7 }}", json!("7")),
        (ProgramSyntax::Auto, "'{{ 7 }}'", json!("{{ 7 }}")),
        (ProgramSyntax::Expression, "'{{ 7 }}'", json!("{{ 7 }}")),
        (ProgramSyntax::Template, "'{{ 7 }}'", json!("'7'")),
        (ProgramSyntax::Template, "static", json!("static")),
        (ProgramSyntax::Template, "{{ null }}", json!("null")),
        (ProgramSyntax::Template, "{{ [1,2] }}", json!("[1,2]")),
    ] {
        let program = CompiledProgram::compile_with_syntax(source, syntax).unwrap();
        assert_eq!(program.syntax(), syntax);
        assert_eq!(program.source(), source);
        assert_eq!(
            engine.evaluate_compiled(&program, &context).unwrap(),
            expected
        );
    }
    CompiledProgram::compile_with_syntax("{{ 7 }}", ProgramSyntax::Expression).unwrap_err();
    CompiledProgram::compile_with_syntax("'{{ incomplete'", ProgramSyntax::Template).unwrap_err();
    for syntax in [ProgramSyntax::Auto, ProgramSyntax::Expression] {
        let program = CompiledProgram::compile_with_syntax("'{{ incomplete'", syntax).unwrap();
        assert_eq!(
            engine.evaluate_compiled(&program, &context).unwrap(),
            json!("{{ incomplete")
        );
    }
}

#[test]
fn retained_program_resolves_runtime_context_on_each_call() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<CompiledProgram>();
    let program = CompiledProgram::compile("$input.value").unwrap();
    let engine = ExpressionEngine::new();
    for value in [1, 2] {
        let context = EvaluationContext::builder()
            .input(json!({"value": value}))
            .build();
        assert_eq!(
            engine
                .evaluate_compiled(&program.clone(), &context)
                .unwrap(),
            json!(value)
        );
    }
    let missing = EvaluationContext::builder().input(json!({})).build();
    engine.evaluate_compiled(&program, &missing).unwrap_err();
    let null = EvaluationContext::builder()
        .input(json!({"value": null}))
        .build();
    assert_eq!(
        engine.evaluate_compiled(&program, &null).unwrap(),
        json!(null)
    );
}

#[test]
fn retained_program_keeps_policy_checks_at_evaluation() {
    let program = CompiledProgram::compile("uppercase('a')").unwrap();
    let engine = ExpressionEngine::new().restrict_to_functions(["length"]);
    engine
        .evaluate_compiled(&program, &EvaluationContext::new())
        .unwrap_err();
}

#[test]
fn retained_template_higher_order_calls_share_budget() {
    let program = CompiledProgram::compile_template(
        "{{ map($input, x => x + 1) }}{{ map($input, x => x + 1) }}",
    )
    .unwrap();
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(10)));
    let context = EvaluationContext::builder().input(json!([1, 2])).build();
    let error = engine.evaluate_compiled(&program, &context).unwrap_err();
    assert_matches!(error, ExpressionError::StepLimitExceeded { limit: 10, .. });
}

#[test]
fn compilation_rejects_excessive_left_associative_depth() {
    let source = std::iter::repeat_n("1", 600)
        .collect::<Vec<_>>()
        .join(" + ");
    CompiledProgram::compile_expression(&source).unwrap_err();
}

#[test]
fn compilation_rejects_excessive_source_before_tokenizing() {
    let source = format!("'{}'", "x".repeat(1_048_577));
    CompiledProgram::compile_expression(&source).unwrap_err();
    CompiledProgram::compile_template(&source).unwrap_err();
}

#[test]
fn ordinary_builtins_cannot_bypass_small_work_budget() {
    let program = CompiledProgram::compile("sort($input)").unwrap();
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(10)));
    let context = EvaluationContext::builder()
        .input(json!(vec![0; 100]))
        .build();
    let error = engine.evaluate_compiled(&program, &context).unwrap_err();
    assert_matches!(error, ExpressionError::StepLimitExceeded { limit: 10, .. });
}

#[cfg(feature = "cache")]
#[test]
fn retained_program_evaluation_does_not_use_parse_cache() {
    let program = CompiledProgram::compile("{{ $input + 1 }}").unwrap();
    let engine = ExpressionEngine::with_cache_size(8);
    let context = EvaluationContext::builder().input(json!(5)).build();
    for _ in 0..3 {
        assert_eq!(
            engine.evaluate_compiled(&program, &context).unwrap(),
            json!(6)
        );
    }
    assert_eq!(engine.cache_overview().expr_misses, 0);
    assert_eq!(engine.cache_overview().expr_hits, 0);
}

#[cfg(feature = "cache")]
#[test]
fn maybe_expression_retains_its_compiled_program() {
    let expression = nebula_expression::MaybeExpression::<Value>::expression("$input + 1");
    let engine = ExpressionEngine::with_cache_size(8);
    let context = EvaluationContext::builder().input(json!(5)).build();
    assert_eq!(
        expression.resolve_as_value(&engine, &context).unwrap(),
        json!(6)
    );
    let cloned = expression.clone();
    let engine_ref = &engine;
    let context_ref = &context;
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                assert_eq!(
                    cloned.resolve_as_value(engine_ref, context_ref).unwrap(),
                    json!(6)
                );
            })
            .join()
            .unwrap();
    });
    assert_eq!(
        expression.resolve_as_value(&engine, &context).unwrap(),
        json!(6)
    );
    assert_eq!(engine.cache_overview().expr_misses, 0);
}

#[test]
fn adding_a_function_allowlist_preserves_engine_limits() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(2)))
        .restrict_to_functions(["length"]);
    let error = engine
        .evaluate("1 + 2", &EvaluationContext::new())
        .unwrap_err();
    assert_matches!(
        error,
        ExpressionError::StepLimitExceeded {
            limit: 2,
            actual: 3
        }
    );
}

#[test]
fn variable_materialization_consumes_the_shared_budget() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(10)));
    let context = EvaluationContext::builder()
        .input(json!(vec![0; 100]))
        .build();
    let error = engine.evaluate("$input", &context).unwrap_err();
    assert_matches!(error, ExpressionError::StepLimitExceeded { limit: 10, .. });
}

#[test]
fn a_context_cannot_raise_the_default_json_input_limit() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(10_000_000)));
    let payload = format!("\"{}\"", "x".repeat(1_048_576));
    let context = EvaluationContext::builder()
        .input(json!(payload))
        .policy(EvaluationPolicy::new().with_max_json_parse_length(2_097_152))
        .build();
    engine.evaluate("parse_json($input)", &context).unwrap_err();
}

#[test]
fn static_template_output_consumes_the_shared_budget() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(2)));
    let program = CompiledProgram::compile_template("static text").unwrap();
    assert_matches!(
        engine.evaluate_compiled(&program, &EvaluationContext::new()),
        Err(ExpressionError::StepLimitExceeded { limit: 2, .. })
    );
}

#[test]
fn replacement_expansion_is_bounded_before_allocation() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(10_000_000)));
    let context = EvaluationContext::builder()
        .input(json!("x".repeat(1025)))
        .build();
    let result = engine.evaluate("replace($input, 'x', repeat('y', 1024))", &context);
    assert_matches!(
        result.map(|_| ()),
        Err(ExpressionError::ResourceLimitExceeded { .. })
    );
}

#[test]
fn custom_builtins_cannot_return_values_above_the_hard_result_budget() {
    let mut engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(10_000_000)));
    engine.register_function("oversized_result", oversized_result);
    let result = engine.evaluate("oversized_result()", &EvaluationContext::new());
    assert_matches!(
        result,
        Err(ExpressionError::BuiltinOutputLimitExceeded {
            dimension: BuiltinOutputLimit::TotalBytes,
            limit: 1_048_576,
            actual: 1_048_577,
        })
    );
}

#[test]
fn variable_materialization_rejects_excessive_value_depth() {
    let mut value = json!(0);
    for _ in 0..300 {
        value = Value::Array(vec![value]);
    }
    let context = EvaluationContext::builder().input(value).build();
    let result = ExpressionEngine::new().evaluate("$input", &context);
    assert_matches!(
        result.map(|_| ()),
        Err(ExpressionError::ResourceLimitExceeded { .. })
    );
}

#[test]
fn raw_string_materialization_consumes_the_shared_budget() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(5)));
    assert_matches!(
        engine.evaluate("'123456'", &EvaluationContext::new()),
        Err(ExpressionError::StepLimitExceeded { limit: 5, .. })
    );
}

#[test]
fn identifier_materialization_consumes_the_shared_budget() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(5)));
    assert_matches!(
        engine.evaluate("abcdef", &EvaluationContext::new()),
        Err(ExpressionError::StepLimitExceeded { limit: 5, .. })
    );
}

#[test]
fn object_keys_consume_the_shared_budget() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(5)));
    assert_matches!(
        engine.evaluate("{'abcdef': 1}", &EvaluationContext::new()),
        Err(ExpressionError::StepLimitExceeded { limit: 5, .. })
    );
}

#[test]
fn concatenation_output_consumes_the_shared_budget() {
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(200)));
    let context = EvaluationContext::builder()
        .input(json!("x".repeat(60)))
        .build();
    assert_matches!(
        engine.evaluate("$input + $input + $input", &context),
        Err(ExpressionError::StepLimitExceeded { limit: 200, .. })
    );
}

#[test]
fn result_byte_accounting_uses_json_boolean_lengths() {
    const MAX_RESULT_BYTES: usize = nebula_expression::BuiltinOutputLimits::DEFAULT_MAX_TOTAL_BYTES;
    let engine = ExpressionEngine::new()
        .with_policy(EvaluationPolicy::new().with_max_eval_steps(step_limit(MAX_RESULT_BYTES * 2)));
    let boundary_text = "x".repeat(MAX_RESULT_BYTES - 7);

    let true_context = EvaluationContext::builder()
        .input(json!([true, boundary_text]))
        .build();
    assert_eq!(
        engine.evaluate("$input", &true_context).unwrap()[0],
        json!(true),
        "`true` is four bytes and the boundary-sized tree must be accepted"
    );

    let false_context = EvaluationContext::builder()
        .input(json!([false, "x".repeat(MAX_RESULT_BYTES - 7)]))
        .build();
    assert_matches!(
        engine.evaluate("$input", &false_context),
        Err(ExpressionError::ResourceLimitExceeded {
            resource: "result content bytes",
            ..
        }),
        "`false` is five bytes and must put the tree one byte over the limit"
    );
}

#[test]
fn escaped_template_openers_remain_literal() {
    let engine = ExpressionEngine::new();
    for (source, expected) in [
        (r"\{{ incomplete", "{{ incomplete"),
        ("{{{{ literal }}}}", "{{ literal }}"),
        (r"\\{{ 2 }}", r"\\2"),
        (r"\{{ literal }} {{ 2 }}", "{{ literal }} 2"),
    ] {
        let template = Template::new(source).unwrap();
        assert_eq!(
            template.render(&engine, &EvaluationContext::new()).unwrap(),
            expected
        );
    }
}

#[test]
fn marker_classification_does_not_silently_accept_invalid_authored_syntax() {
    for source in [
        "{{",
        "before {{ incomplete",
        "{{ 1 + }}",
        "{{ 1 }}",
        "{{ lowercase('X') }}",
    ] {
        assert!(nebula_expression::has_expression_marker(source), "{source}");
    }
    for source in [
        "plain text",
        "{ single brace",
        "{{{{ literal }}}}",
        r"\{{ incomplete",
    ] {
        assert!(
            !nebula_expression::has_expression_marker(source),
            "{source}"
        );
    }
    for count in 0..9 {
        let source = format!("\u{00e9} {}{{{{ incomplete", "\\".repeat(count));
        assert_eq!(
            nebula_expression::has_expression_marker(&source),
            count % 2 == 0,
            "{source}"
        );
    }
    assert!(nebula_expression::has_expression_marker(
        r"\{{ literal }} {{ 2 }}"
    ));
}

#[test]
fn compiled_left_associative_depth_boundary_evaluates_without_stack_overflow() {
    let source = format!("{}1", "1 + ".repeat(255));
    let program = CompiledProgram::compile_expression(&source).unwrap();
    assert_eq!(
        ExpressionEngine::new()
            .evaluate_compiled(&program, &EvaluationContext::new())
            .unwrap(),
        json!(256)
    );
}

#[test]
fn maybe_template_uses_the_shared_marker_classifier() {
    let malformed = nebula_expression::MaybeTemplate::from_string("{{ incomplete");
    assert!(malformed.is_template());
    malformed
        .resolve(&ExpressionEngine::new(), &EvaluationContext::new())
        .unwrap_err();
    assert!(!nebula_expression::MaybeTemplate::from_string(r"\{{ escaped }}").is_template());
    assert!(!nebula_expression::MaybeTemplate::from_string("{{{{ escaped }}}}").is_template());
}

#[cfg(feature = "cache")]
#[test]
fn maybe_template_uses_the_engine_program_cache() {
    let template = nebula_expression::MaybeTemplate::from_string("{{ 2 }}");
    let engine = ExpressionEngine::with_cache_size(8);
    for _ in 0..2 {
        assert_eq!(
            template
                .resolve(&engine, &EvaluationContext::new())
                .unwrap(),
            "2"
        );
    }
    assert_eq!(engine.cache_overview().template_misses, 1);
    assert_eq!(engine.cache_overview().template_hits, 1);
}
