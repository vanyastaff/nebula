//! Scenario: character-count vs byte-count semantics for length validators.
//!
//! `min_length` / `max_length` count **characters** (Unicode scalar
//! values), while the `*_bytes` variants count **UTF-8 bytes**. Both
//! matter — the char-counting surface is what users expect for "name
//! must be at most 20 letters", whereas the byte surface is required
//! for on-the-wire limits (database columns, protocol fields).

use nebula_validator::{
    Validator,
    foundation::Validate,
    validators::{exact_length, exact_length_bytes, max_length, max_length_bytes, min_length},
};

use super::common::expect_errors;

// Cyrillic letter "а" (U+0430) → 2 UTF-8 bytes.
// Chinese "中" (U+4E2D) → 3 UTF-8 bytes.
// Emoji "🚀" (U+1F680) → 4 UTF-8 bytes.
const CYR: &str = "Алиса"; //   5 chars, 10 bytes
const CHINESE: &str = "你好世界"; // 4 chars, 12 bytes
const EMOJI: &str = "🚀🚀🚀"; //   3 chars, 12 bytes

#[test]
fn min_length_counts_characters_not_bytes() {
    // 5 cyrillic chars satisfies min_length(5) even though it's 10 bytes.
    assert!(min_length(5).validate(CYR).is_ok());
    // Fails min_length(6) — there are only 5 chars.
    assert!(min_length(6).validate(CYR).is_err());
}

#[test]
fn max_length_counts_characters_not_bytes() {
    // 3 emoji chars pass max_length(3) despite being 12 bytes.
    assert!(max_length(3).validate(EMOJI).is_ok());
    assert!(max_length(2).validate(EMOJI).is_err());
}

#[test]
fn exact_length_counts_characters_not_bytes() {
    assert!(exact_length(4).validate(CHINESE).is_ok());
    assert!(exact_length(12).validate(CHINESE).is_err());
}

#[test]
fn bytes_variants_count_bytes_not_characters() {
    // 5 cyrillic chars = 10 bytes.
    assert!(max_length_bytes(10).validate(CYR).is_ok());
    assert!(max_length_bytes(9).validate(CYR).is_err());

    assert!(exact_length_bytes(12).validate(CHINESE).is_ok());
    assert!(exact_length_bytes(4).validate(CHINESE).is_err());
}

#[derive(Validator)]
struct DisplayName {
    // NOTE: the derive macro currently emits `value.len() < #bound`, which
    // counts **UTF-8 bytes**, not characters. The standalone `min_length()`
    // validator counts chars. Tests below pin the current behaviour so a
    // future unification of the two surfaces is visible in the diff.
    #[validate(min_length = 3, max_length = 20)]
    name: String,
}

#[test]
fn derive_length_currently_counts_bytes() {
    // 4 Chinese chars = 12 bytes — passes max_length(20) comfortably.
    let ok = DisplayName {
        name: CHINESE.into(),
    };
    assert!(ok.validate_fields().is_ok());

    // 2 Chinese chars = 6 bytes. Byte-based min_length(3) accepts 6 ≥ 3,
    // even though a char-based check would reject 2 < 3. If this starts
    // failing, the derive was switched to char counting — update the
    // comment in `DisplayName` above to match.
    let two_chinese = DisplayName {
        name: "你好".into(),
    };
    assert!(
        two_chinese.validate_fields().is_ok(),
        "derive-level min_length is byte-based; see DisplayName comment"
    );

    // One ASCII char (1 byte) genuinely falls below the 3-byte floor.
    let single = DisplayName { name: "x".into() };
    assert!(!expect_errors(single.validate_fields()).is_empty());
}

#[test]
fn combining_characters_are_distinct_scalar_values() {
    // Explicit escapes avoid editor-level normalisation surprises.
    let precomposed = "\u{00E9}"; // one scalar value: é
    let decomposed = "e\u{0301}"; // two scalar values: e + combining acute
    assert_eq!(precomposed.chars().count(), 1);
    assert_eq!(decomposed.chars().count(), 2);

    // Char-based validators see them as different lengths.
    assert!(min_length(1).validate(precomposed).is_ok());
    assert!(min_length(2).validate(precomposed).is_err());
    assert!(min_length(2).validate(decomposed).is_ok());
}
