use nebula_expression::{CompiledProgram, EvaluationContext, ExpressionEngine, value_utils};
use serde_json::{Value, json};

fn evaluate(source: &str) -> nebula_expression::ExpressionResult<Value> {
    ExpressionEngine::new().evaluate(source, &EvaluationContext::new())
}

#[test]
fn numeric_order_is_exact_across_integer_and_float_boundaries() {
    let ordered = [
        json!(-1.0e40),
        json!(i64::MIN),
        json!(-9_007_199_254_740_993_i64),
        json!(-9_007_199_254_740_992.0_f64),
        json!(-1),
        json!(-0.5),
        json!(0),
        json!(0.5),
        json!(1),
        json!(9_007_199_254_740_992.0_f64),
        json!(9_007_199_254_740_993_u64),
        json!(i64::MAX),
        json!(9_223_372_036_854_775_808.0_f64),
        json!(u64::MAX),
        json!(18_446_744_073_709_551_616.0_f64),
        json!(1.0e40),
    ];
    let programs = ["<", ">", "<=", ">="].map(|operator| {
        CompiledProgram::compile_expression(&format!("$input[0] {operator} $input[1]")).unwrap()
    });
    let engine = ExpressionEngine::new();
    for (left_index, left) in ordered.iter().enumerate() {
        for (right_index, right) in ordered.iter().enumerate() {
            let context = EvaluationContext::builder()
                .input(json!([left, right]))
                .build();
            let expected = [
                left_index < right_index,
                left_index > right_index,
                left_index <= right_index,
                left_index >= right_index,
            ];
            for (program, expected) in programs.iter().zip(expected) {
                assert_eq!(
                    engine.evaluate_compiled(program, &context).unwrap(),
                    json!(expected),
                    "{}: {left} versus {right}",
                    program.source()
                );
            }
        }
    }
}

#[test]
fn numeric_equality_agrees_with_exact_ordering() {
    assert_eq!(evaluate("1 == 1.0").unwrap(), json!(true));
    assert_eq!(
        evaluate("9007199254740993 == 9007199254740992.0").unwrap(),
        json!(false)
    );
    assert_eq!(
        evaluate("9007199254740993 != 9007199254740992.0").unwrap(),
        json!(true)
    );
}

#[test]
fn sort_and_extrema_share_exact_mixed_numeric_order() {
    assert_eq!(
        evaluate("sort([9007199254740993,9007199254740992.0])").unwrap(),
        json!([9_007_199_254_740_992.0_f64, 9_007_199_254_740_993_u64])
    );
    assert_eq!(
        evaluate("min(9007199254740993,9007199254740992.0)").unwrap(),
        json!(9_007_199_254_740_992.0_f64)
    );
    assert_eq!(
        evaluate("max(9007199254740992.0,9007199254740993)").unwrap(),
        json!(9_007_199_254_740_993_u64)
    );
}

#[test]
fn literals_cover_signed_and_unsigned_json_integer_range() {
    assert_eq!(evaluate("-9223372036854775808").unwrap(), json!(i64::MIN));
    assert_eq!(evaluate("18446744073709551615").unwrap(), json!(u64::MAX));
    evaluate("18446744073709551616").unwrap_err();
    evaluate("-9223372036854775809").unwrap_err();
}

#[test]
fn exponent_literals_remain_finite() {
    assert_eq!(evaluate("1.25e2 + 5E-1").unwrap(), json!(125.5));
    evaluate("1e309").unwrap_err();
}

#[test]
fn integer_arithmetic_preserves_representable_results() {
    assert_eq!(
        evaluate("9223372036854775807 + 2").unwrap(),
        json!(9_223_372_036_854_775_809_u64)
    );
    assert_eq!(
        evaluate("18446744073709551615 - 1").unwrap(),
        json!(u64::MAX - 1)
    );
    assert_eq!(
        evaluate("9007199254740993 * 2").unwrap(),
        json!(18_014_398_509_481_986_u64)
    );
}

#[test]
fn overflowing_integer_arithmetic_returns_error() {
    evaluate("18446744073709551615 + 1").unwrap_err();
    evaluate("-9223372036854775808 - 1").unwrap_err();
    evaluate("9223372036854775807 * 3").unwrap_err();
}

#[test]
fn signed_minimum_remainder_is_zero_without_panicking() {
    assert_eq!(
        evaluate("(-9223372036854775807 - 1) % -1").unwrap(),
        json!(0)
    );
}

#[test]
fn nonfinite_conversion_and_arithmetic_never_return_null() {
    for source in [
        "to_number('NaN')",
        "to_number('inf')",
        "to_number('1e309')",
        "to_number('1e308') * 2",
        "to_number('1e308') + to_number('1e308')",
    ] {
        evaluate(source).unwrap_err();
    }
}

#[test]
fn number_conversion_preserves_exact_native_and_string_integers() {
    assert_eq!(
        evaluate("to_number(9007199254740993)").unwrap(),
        json!(9_007_199_254_740_993_u64)
    );
    assert_eq!(
        evaluate("to_number('18446744073709551615')").unwrap(),
        json!(u64::MAX)
    );
}

#[test]
fn integer_conversion_rejects_fractional_and_out_of_range_numbers() {
    for value in [
        json!(1.5),
        json!(u64::MAX),
        json!(9_223_372_036_854_775_808.0_f64),
    ] {
        value_utils::to_integer(&value).unwrap_err();
    }
    assert_eq!(value_utils::to_integer(&json!(12.0)).unwrap(), 12);
    assert_eq!(value_utils::to_integer(&json!(i64::MIN)).unwrap(), i64::MIN);
}

#[test]
fn integral_math_preserves_the_full_json_integer_range() {
    for function in ["abs", "round", "floor", "ceil"] {
        assert_eq!(
            evaluate(&format!("{function}(9007199254740993)")).unwrap(),
            json!(9_007_199_254_740_993_u64)
        );
        assert_eq!(
            evaluate(&format!("{function}(18446744073709551615)")).unwrap(),
            json!(u64::MAX)
        );
    }
    assert_eq!(
        evaluate("abs(-9223372036854775808)").unwrap(),
        json!(9_223_372_036_854_775_808_u64)
    );
    assert_eq!(
        evaluate("round(9007199254740993, 2)").unwrap(),
        json!(9_007_199_254_740_993_u64)
    );
    assert_eq!(
        evaluate("--9223372036854775808").unwrap(),
        json!(9_223_372_036_854_775_808_u64)
    );
}

#[test]
fn integer_text_conversion_rejects_out_of_range_values() {
    evaluate("to_number('18446744073709551616')").unwrap_err();
    evaluate("to_number('-9223372036854775809')").unwrap_err();
}

#[test]
fn coerced_extrema_compare_numeric_values() {
    assert_eq!(evaluate("min('10', 2)").unwrap(), json!(2));
    assert_eq!(evaluate("max('2', 10)").unwrap(), json!(10));
}

#[test]
fn round_rejects_decimal_counts_that_would_truncate() {
    evaluate("round(1.5, 4294967296)").unwrap_err();
}
