//! Integration tests for builtin functions added in expression-v1

use nebula_expression::{EvaluationContext, ExpressionEngine};
use serde_json::json;

fn eval(expr: &str) -> serde_json::Value {
    let engine = ExpressionEngine::default();
    let ctx = EvaluationContext::default();
    engine.evaluate(expr, &ctx).unwrap()
}

fn eval_err(expr: &str) -> String {
    let engine = ExpressionEngine::default();
    let ctx = EvaluationContext::default();
    engine.evaluate(expr, &ctx).unwrap_err().to_string()
}

// ──────────────────────────────────────────────
// Array: some
// ──────────────────────────────────────────────

#[test]
fn some_returns_true_when_match() {
    assert_eq!(eval("some([1,2,3], x => x > 2)"), json!(true));
}

#[test]
fn some_returns_false_when_no_match() {
    assert_eq!(eval("some([1,2,3], x => x > 10)"), json!(false));
}

#[test]
fn some_empty_array_returns_false() {
    assert_eq!(eval("some([], x => x > 0)"), json!(false));
}

// ──────────────────────────────────────────────
// Array: every
// ──────────────────────────────────────────────

#[test]
fn every_returns_true_when_all_match() {
    assert_eq!(eval("every([1,2,3], x => x > 0)"), json!(true));
}

#[test]
fn every_returns_false_when_one_fails() {
    assert_eq!(eval("every([1,2,3], x => x > 1)"), json!(false));
}

#[test]
fn every_empty_array_returns_true() {
    // Vacuous truth
    assert_eq!(eval("every([], x => x > 100)"), json!(true));
}

// ──────────────────────────────────────────────
// Array: find
// ──────────────────────────────────────────────

#[test]
fn find_returns_first_match() {
    assert_eq!(eval("find([1,2,3], x => x > 1)"), json!(2));
}

#[test]
fn find_returns_null_when_no_match() {
    assert_eq!(eval("find([1,2,3], x => x > 10)"), json!(null));
}

#[test]
fn find_empty_array_returns_null() {
    assert_eq!(eval("find([], x => x > 0)"), json!(null));
}

// ──────────────────────────────────────────────
// Array: find_index
// ──────────────────────────────────────────────

#[test]
fn find_index_returns_index_of_first_match() {
    assert_eq!(eval("find_index([1,2,3], x => x > 1)"), json!(1));
}

#[test]
fn find_index_returns_negative_one_when_no_match() {
    assert_eq!(eval("find_index([1,2,3], x => x > 10)"), json!(-1));
}

#[test]
fn find_index_empty_array() {
    assert_eq!(eval("find_index([], x => x > 0)"), json!(-1));
}

// ──────────────────────────────────────────────
// Array: unique
// ──────────────────────────────────────────────

#[test]
fn unique_removes_duplicates() {
    assert_eq!(eval("unique([1,2,2,3,1])"), json!([1, 2, 3]));
}

#[test]
fn unique_preserves_order() {
    assert_eq!(eval("unique([3,1,2,1,3])"), json!([3, 1, 2]));
}

#[test]
fn unique_empty_array() {
    assert_eq!(eval("unique([])"), json!([]));
}

#[test]
fn unique_with_strings() {
    assert_eq!(eval(r#"unique(["a","b","a","c"])"#), json!(["a", "b", "c"]));
}

// ──────────────────────────────────────────────
// Array: group_by
// ──────────────────────────────────────────────

#[test]
fn group_by_groups_elements_by_key() {
    let result = eval(
        r#"group_by([{"name":"a","age":1},{"name":"b","age":2},{"name":"c","age":1}], x => x.age)"#,
    );
    let obj = result.as_object().unwrap();
    assert_eq!(obj.len(), 2);
    assert_eq!(obj["1"].as_array().unwrap().len(), 2);
    assert_eq!(obj["2"].as_array().unwrap().len(), 1);
}

#[test]
fn group_by_empty_array() {
    assert_eq!(eval("group_by([], x => x)"), json!({}));
}

// ──────────────────────────────────────────────
// Array: flat_map
// ──────────────────────────────────────────────

#[test]
fn flat_map_flattens_one_level() {
    assert_eq!(eval("flat_map([[1,2],[3,4]], x => x)"), json!([1, 2, 3, 4]));
}

#[test]
fn flat_map_with_transform() {
    // Each element mapped to an array, then flattened
    assert_eq!(
        eval("flat_map([1,2,3], x => [x, x])"),
        json!([1, 1, 2, 2, 3, 3])
    );
}

#[test]
fn flat_map_non_array_results_kept() {
    // If lambda returns a non-array, it's kept as-is
    assert_eq!(eval("flat_map([1,2,3], x => x)"), json!([1, 2, 3]));
}

#[test]
fn flat_map_empty_array() {
    assert_eq!(eval("flat_map([], x => x)"), json!([]));
}

// ──────────────────────────────────────────────
// Object: merge
// ──────────────────────────────────────────────

#[test]
fn merge_two_objects() {
    let result = eval(r#"merge({"a":1}, {"b":2})"#);
    assert_eq!(result, json!({"a": 1, "b": 2}));
}

#[test]
fn merge_right_wins_on_conflict() {
    let result = eval(r#"merge({"a":1}, {"a":3, "b":2})"#);
    assert_eq!(result["a"], json!(3));
    assert_eq!(result["b"], json!(2));
}

#[test]
fn merge_three_objects() {
    let result = eval(r#"merge({"a":1}, {"b":2}, {"a":3})"#);
    assert_eq!(result, json!({"a": 3, "b": 2}));
}

// ──────────────────────────────────────────────
// Object: pick
// ──────────────────────────────────────────────

#[test]
fn pick_selected_keys() {
    let result = eval(r#"pick({"a":1, "b":2, "c":3}, "a", "c")"#);
    assert_eq!(result, json!({"a": 1, "c": 3}));
}

#[test]
fn pick_missing_key_ignored() {
    let result = eval(r#"pick({"a":1, "b":2}, "a", "z")"#);
    assert_eq!(result, json!({"a": 1}));
}

#[test]
fn pick_no_keys() {
    let result = eval(r#"pick({"a":1, "b":2})"#);
    assert_eq!(result, json!({}));
}

#[test]
fn pick_rejects_non_string_selector() {
    let err = eval_err(r#"pick({"a":1, "b":2}, 42)"#);
    assert!(
        err.contains("string"),
        "Error should mention 'string': {err}"
    );
}

// ──────────────────────────────────────────────
// Object: omit
// ──────────────────────────────────────────────

#[test]
fn omit_removes_specified_keys() {
    let result = eval(r#"omit({"a":1, "b":2, "c":3}, "b")"#);
    assert_eq!(result, json!({"a": 1, "c": 3}));
}

#[test]
fn omit_missing_key_no_error() {
    let result = eval(r#"omit({"a":1}, "z")"#);
    assert_eq!(result, json!({"a": 1}));
}

#[test]
fn omit_multiple_keys() {
    let result = eval(r#"omit({"a":1, "b":2, "c":3}, "a", "c")"#);
    assert_eq!(result, json!({"b": 2}));
}

#[test]
fn omit_rejects_non_string_selector() {
    let err = eval_err(r#"omit({"a":1}, true)"#);
    assert!(
        err.contains("string"),
        "Error should mention 'string': {err}"
    );
}

// ──────────────────────────────────────────────
// Object: entries
// ──────────────────────────────────────────────

#[test]
fn entries_converts_to_pairs() {
    let result = eval(r#"entries({"a":1})"#);
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["key"], json!("a"));
    assert_eq!(arr[0]["value"], json!(1));
}

#[test]
fn entries_empty_object() {
    assert_eq!(eval(r"entries({})"), json!([]));
}

// ──────────────────────────────────────────────
// Object: from_entries
// ──────────────────────────────────────────────

#[test]
fn from_entries_converts_pairs_to_object() {
    let result = eval(r#"from_entries([{"key":"a", "value":1}, {"key":"b", "value":2}])"#);
    assert_eq!(result, json!({"a": 1, "b": 2}));
}

#[test]
fn from_entries_empty_array() {
    assert_eq!(eval("from_entries([])"), json!({}));
}

#[test]
fn from_entries_missing_value_defaults_to_null() {
    let result = eval(r#"from_entries([{"key":"x"}])"#);
    assert_eq!(result, json!({"x": null}));
}

#[test]
fn from_entries_missing_key_errors() {
    let err = eval_err(r#"from_entries([{"value":1}])"#);
    assert!(err.contains("key"), "Error should mention 'key': {err}");
}

// ──────────────────────────────────────────────
// Object: entries + from_entries roundtrip
// ──────────────────────────────────────────────

#[test]
fn entries_from_entries_roundtrip() {
    let engine = ExpressionEngine::default();
    let mut ctx = EvaluationContext::default();
    let obj = json!({"x": 10, "y": 20});
    ctx.set_execution_var("obj", obj);
    // Two-step roundtrip via a context variable to avoid nested call parsing
    let entries = engine.evaluate("entries($obj)", &ctx).unwrap();
    ctx.set_execution_var("pairs", entries);
    let result = engine.evaluate("from_entries($pairs)", &ctx).unwrap();
    assert_eq!(result, json!({"x": 10, "y": 20}));
}

// ──────────────────────────────────────────────
// String: pad_start
// ──────────────────────────────────────────────

#[test]
fn pad_start_with_zeros() {
    assert_eq!(eval(r#"pad_start("5", 3, "0")"#), json!("005"));
}

#[test]
fn pad_start_default_space() {
    assert_eq!(eval(r#"pad_start("5", 3)"#), json!("  5"));
}

#[test]
fn pad_start_already_long_enough() {
    assert_eq!(eval(r#"pad_start("hello", 3, "0")"#), json!("hello"));
}

#[test]
fn pad_start_rejects_excessive_length() {
    let err = eval_err(r#"pad_start("x", 99999999)"#);
    assert!(
        err.contains("exceeds maximum"),
        "Error should mention exceeds maximum: {err}"
    );
}

#[test]
fn pad_start_rejects_non_integer_length() {
    let err = eval_err(r#"pad_start("x", "abc")"#);
    assert!(
        err.contains("integer"),
        "Error should mention integer: {err}"
    );
}

// ──────────────────────────────────────────────
// String: pad_end
// ──────────────────────────────────────────────

#[test]
fn pad_end_with_zeros() {
    assert_eq!(eval(r#"pad_end("5", 3, "0")"#), json!("500"));
}

#[test]
fn pad_end_default_space() {
    assert_eq!(eval(r#"pad_end("5", 3)"#), json!("5  "));
}

#[test]
fn pad_end_already_long_enough() {
    assert_eq!(eval(r#"pad_end("hello", 3, "0")"#), json!("hello"));
}

#[test]
fn pad_end_rejects_excessive_length() {
    let err = eval_err(r#"pad_end("x", 99999999)"#);
    assert!(
        err.contains("exceeds maximum"),
        "Error should mention exceeds maximum: {err}"
    );
}

#[test]
fn pad_end_rejects_non_integer_length() {
    let err = eval_err(r#"pad_end("x", "abc")"#);
    assert!(
        err.contains("integer"),
        "Error should mention integer: {err}"
    );
}

// ──────────────────────────────────────────────
// String: repeat
// ──────────────────────────────────────────────

#[test]
fn repeat_string() {
    assert_eq!(eval(r#"repeat("ab", 3)"#), json!("ababab"));
}

#[test]
fn repeat_zero_times() {
    assert_eq!(eval(r#"repeat("ab", 0)"#), json!(""));
}

#[test]
fn repeat_negative_count_errors() {
    let err = eval_err(r#"repeat("ab", -1)"#);
    assert!(
        err.contains("non-negative"),
        "Error should mention non-negative: {err}"
    );
}

// ──────────────────────────────────────────────
// Utility: coalesce
// ──────────────────────────────────────────────

#[test]
fn coalesce_returns_first_non_null() {
    assert_eq!(eval("coalesce(null, null, 42)"), json!(42));
}

#[test]
fn coalesce_returns_first_arg_if_not_null() {
    assert_eq!(eval(r#"coalesce("hello", 42)"#), json!("hello"));
}

#[test]
fn coalesce_all_null_returns_null() {
    assert_eq!(eval("coalesce(null, null)"), json!(null));
}

#[test]
fn coalesce_single_value() {
    assert_eq!(eval("coalesce(99)"), json!(99));
}

// ──────────────────────────────────────────────
// Utility: type_of
// ──────────────────────────────────────────────

#[test]
fn type_of_number() {
    assert_eq!(eval("type_of(42)"), json!("number"));
}

#[test]
fn type_of_string() {
    assert_eq!(eval(r#"type_of("hi")"#), json!("string"));
}

#[test]
fn type_of_array() {
    assert_eq!(eval("type_of([])"), json!("array"));
}

#[test]
fn type_of_object() {
    assert_eq!(eval("type_of({})"), json!("object"));
}

#[test]
fn type_of_null() {
    assert_eq!(eval("type_of(null)"), json!("null"));
}

#[test]
fn type_of_boolean() {
    assert_eq!(eval("type_of(true)"), json!("boolean"));
}

// ──────────────────────────────────────────────
// Lambda scope isolation
// ──────────────────────────────────────────────

#[test]
fn lambda_param_does_not_shadow_execution_var() {
    // An execution var named "x" must NOT affect the bare identifier "x" used as
    // a lambda parameter in a higher-order function.
    let engine = ExpressionEngine::default();
    let mut ctx = EvaluationContext::default();
    ctx.set_execution_var("x", json!(999)); // should be invisible inside the lambda body
    let result = engine
        .evaluate("filter([1,2,3], x => x > 1)", &ctx)
        .unwrap();
    assert_eq!(result, json!([2, 3]));
}

// ──────────────────────────────────────────────
// Object: pick / omit – non-string key errors
// ──────────────────────────────────────────────

#[test]
fn pick_rejects_non_string_key() {
    let err = eval_err("pick({\"a\":1}, 42)");
    assert!(
        err.contains("string"),
        "Error should mention string type: {err}"
    );
}

#[test]
fn omit_rejects_non_string_key() {
    let err = eval_err("omit({\"a\":1}, true)");
    assert!(
        err.contains("string"),
        "Error should mention string type: {err}"
    );
}

// ──────────────────────────────────────────────
// String: integer coercion for pad_start/pad_end/repeat
// ──────────────────────────────────────────────

#[test]
fn pad_start_accepts_float_length() {
    // Non-strict mode: float 3.0 should be coerced to integer 3
    assert_eq!(eval(r#"pad_start("5", 3.0, "0")"#), json!("005"));
}

#[test]
fn pad_end_accepts_float_length() {
    assert_eq!(eval(r#"pad_end("5", 3.0, "0")"#), json!("500"));
}

#[test]
fn repeat_accepts_float_count() {
    assert_eq!(eval(r#"repeat("ab", 2.0)"#), json!("abab"));
}

// ──────────────────────────────────────────────
// String: Unicode-aware length / substring (n8n-compatible)
// ──────────────────────────────────────────────

#[test]
fn length_counts_unicode_chars_not_bytes() {
    // "🙂" is 1 char (4 UTF-8 bytes); "über" is 4 chars (5 UTF-8 bytes);
    // "hello" is 5 chars (5 UTF-8 bytes); empty is 0.
    assert_eq!(eval(r#"length("🙂")"#), json!(1));
    assert_eq!(eval(r#"length("über")"#), json!(4));
    assert_eq!(eval(r#"length("hello")"#), json!(5));
    assert_eq!(eval(r#"length("")"#), json!(0));
}

#[test]
fn length_array_still_returns_element_count() {
    // Polymorphic length: array semantics unchanged.
    assert_eq!(eval("length([1, 2, 3])"), json!(3));
    assert_eq!(eval("length([])"), json!(0));
}

#[test]
fn length_object_returns_key_count() {
    // Polymorphic length: object reports number of top-level keys.
    assert_eq!(eval(r#"length({"a": 1, "b": 2, "c": 3})"#), json!(3));
    assert_eq!(eval("length({})"), json!(0));
}

#[test]
fn length_rejects_non_collection_types() {
    // Numbers / booleans / null have no meaningful length.
    let err = eval_err("length(42)");
    assert!(err.to_lowercase().contains("type"), "got: {err}");
    let err = eval_err("length(true)");
    assert!(err.to_lowercase().contains("type"), "got: {err}");
    let err = eval_err("length(null)");
    assert!(err.to_lowercase().contains("type"), "got: {err}");
}

#[test]
fn substring_handles_emoji_at_boundary() {
    // Char index 0..1 of "🙂hello" must be the emoji, NOT a slice of its
    // UTF-8 bytes (which would corrupt the codepoint).
    assert_eq!(eval(r#"substring("🙂hello", 0, 1)"#), json!("🙂"));
    assert_eq!(eval(r#"substring("🙂hello", 1, 6)"#), json!("hello"));
}

#[test]
fn substring_default_end_uses_char_count() {
    // Without `end`, the slice goes to the last character — not byte length.
    // Pre-fix: `s.len()` (byte) on a multibyte string overshot `chars.len()`,
    // and the silent `.min(chars.len())` masked it.
    assert_eq!(eval(r#"substring("über", 1)"#), json!("ber"));
    assert_eq!(eval(r#"substring("🙂🙂🙂", 1)"#), json!("🙂🙂"));
}

#[test]
fn substring_clamps_end_to_char_length() {
    // end past char_count clamps; pre-fix behaviour also clamped, but only
    // because chars.get() returned None — verify the new path keeps that.
    assert_eq!(eval(r#"substring("hi", 0, 100)"#), json!("hi"));
}

#[test]
fn substring_start_past_end_returns_empty() {
    assert_eq!(eval(r#"substring("hello", 4, 2)"#), json!(""));
}

// ──────────────────────────────────────────────
// Datetime: timezone support (T13)
// ──────────────────────────────────────────────

#[test]
fn format_date_default_is_utc_rfc3339() {
    // Unix timestamp 0 → 1970-01-01 00:00:00 UTC.
    assert_eq!(eval("format_date(0)"), json!("1970-01-01T00:00:00+00:00"));
}

#[test]
fn format_date_with_named_tz_shifts_displayed_clock() {
    // Unix timestamp 0 in Europe/Moscow (MSK = +03:00) is 03:00 wall time.
    assert_eq!(
        eval(r#"format_date(0, "YYYY-MM-DD HH:mm:ss", "Europe/Moscow")"#),
        json!("1970-01-01 03:00:00")
    );
}

#[test]
fn format_date_with_format_no_tz_stays_in_utc() {
    assert_eq!(
        eval(r#"format_date(0, "YYYY-MM-DD HH:mm:ss")"#),
        json!("1970-01-01 00:00:00")
    );
}

#[test]
fn format_date_with_only_tz_emits_rfc3339_in_that_zone() {
    // 2-arg form: arg[1] is probed as a timezone first. `Europe/Moscow`
    // parses successfully, so the call renders RFC 3339 in Moscow time
    // (UTC+3) — `1970-01-01T03:00:00+03:00`. Pre-fix, this 2-arg shape
    // was unreachable: arg[1] was always treated as a format string
    // and the user had to pass `""` for format to get tz-only output.
    assert_eq!(
        eval(r#"format_date(0, "Europe/Moscow")"#),
        json!("1970-01-01T03:00:00+03:00")
    );
}

#[test]
fn format_date_2arg_falls_back_to_format_when_not_a_tz() {
    // If arg[1] doesn't parse as an IANA name, treat it as a format.
    // Backward-compat with the original 2-arg shape.
    assert_eq!(eval(r#"format_date(0, "YYYY-MM-DD")"#), json!("1970-01-01"));
}

#[test]
fn format_date_unknown_tz_returns_error() {
    let err = eval_err(r#"format_date(0, "YYYY-MM-DD", "Mars/Olympus_Mons")"#);
    assert!(
        err.to_lowercase().contains("unknown timezone"),
        "got: {err}"
    );
}

#[test]
fn parse_date_with_tz_interprets_naive_as_local_wall_time() {
    // "2024-01-01 00:00:00" interpreted as Moscow wall time = 2023-12-31
    // 21:00 UTC → timestamp 1704056400.
    assert_eq!(
        eval(r#"parse_date("2024-01-01 00:00:00", "Europe/Moscow")"#),
        json!(1_704_056_400)
    );
}

#[test]
fn parse_date_with_tz_ignores_explicit_offset() {
    // RFC 3339 strings already nail down the instant — `tz` is redundant.
    assert_eq!(
        eval(r#"parse_date("2024-01-01T00:00:00+03:00", "Europe/Moscow")"#),
        eval(r#"parse_date("2024-01-01T00:00:00+03:00")"#)
    );
}
