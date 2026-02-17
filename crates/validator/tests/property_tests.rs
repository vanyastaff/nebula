//! Property-based tests for nebula-validator.

use nebula_validator::prelude::*;
use proptest::prelude::*;

// ============================================================================
// IDEMPOTENCY: validate(x) == validate(x)
// ============================================================================

proptest! {
    #[test]
    fn min_length_idempotent(s in ".*") {
        let v = min_length(3);
        let r1 = v.validate(&*s);
        let r2 = v.validate(&*s);
        prop_assert_eq!(r1.is_ok(), r2.is_ok());
    }

    #[test]
    fn max_length_idempotent(s in ".*") {
        let v = max_length(10);
        let r1 = v.validate(&*s);
        let r2 = v.validate(&*s);
        prop_assert_eq!(r1.is_ok(), r2.is_ok());
    }

    #[test]
    fn in_range_idempotent(n in any::<i64>()) {
        let v = in_range(0i64, 100i64);
        let r1 = v.validate(&n);
        let r2 = v.validate(&n);
        prop_assert_eq!(r1.is_ok(), r2.is_ok());
    }

    #[test]
    fn email_idempotent(s in ".*") {
        let v = email();
        let r1 = v.validate(&*s);
        let r2 = v.validate(&*s);
        prop_assert_eq!(r1.is_ok(), r2.is_ok());
    }
}

// ============================================================================
// COMBINATOR LAWS: a.and(b) fails iff a fails or b fails
// ============================================================================

proptest! {
    #[test]
    fn and_fails_iff_either_fails(s in ".{0,30}") {
        let a = min_length(3);
        let b = max_length(10);
        let combined = a.and(b);

        let a_ok = a.validate(&*s).is_ok();
        let b_ok = b.validate(&*s).is_ok();
        let combined_ok = combined.validate(&*s).is_ok();

        prop_assert_eq!(combined_ok, a_ok && b_ok);
    }
}

// ============================================================================
// OR SYMMETRY: a.or(b) passes iff a passes or b passes
// ============================================================================

proptest! {
    #[test]
    fn or_passes_iff_either_passes(s in ".{0,20}") {
        let a = min_length(5);
        let b = max_length(3);
        let combined = a.or(b);

        let a_ok = a.validate(&*s).is_ok();
        let b_ok = b.validate(&*s).is_ok();
        let combined_ok = combined.validate(&*s).is_ok();

        prop_assert_eq!(combined_ok, a_ok || b_ok);
    }
}

// ============================================================================
// DOUBLE NEGATION: not(not(v)) agrees with v
// ============================================================================

proptest! {
    #[test]
    fn double_negation(s in ".{0,20}") {
        let v = min_length(5);
        let double_neg = not(not(v));

        let v_ok = v.validate(&*s).is_ok();
        let double_neg_ok = double_neg.validate(&*s).is_ok();

        prop_assert_eq!(v_ok, double_neg_ok);
    }
}

// ============================================================================
// TYPE CONVERSION ROUNDTRIP: numeric widenings preserve value
// ============================================================================

proptest! {
    #[test]
    fn i32_to_i64_preserves_value(n in any::<i32>()) {
        let result: i64 = AsValidatable::<i64>::as_validatable(&n).unwrap();
        prop_assert_eq!(result, i64::from(n));
    }

    #[test]
    fn f32_to_f64_preserves_value(n in any::<f32>().prop_filter("finite", |f| f.is_finite())) {
        let result: f64 = AsValidatable::<f64>::as_validatable(&n).unwrap();
        prop_assert_eq!(result, f64::from(n));
    }

    #[test]
    fn i64_to_f64_safe_range(n in -(1i64 << 53)..=(1i64 << 53)) {
        let result: f64 = AsValidatable::<f64>::as_validatable(&n).unwrap();
        prop_assert_eq!(result as i64, n);
    }

    #[test]
    fn i64_to_f64_unsafe_rejects(n in prop::num::i64::ANY.prop_filter("large", |&n| {
        let converted = n as f64;
        converted >= 9_223_372_036_854_775_808.0_f64 || converted as i64 != n
    })) {
        let result = AsValidatable::<f64>::as_validatable(&n);
        prop_assert!(result.is_err());
    }

    #[test]
    fn u8_to_i64_preserves_value(n in any::<u8>()) {
        let result: i64 = AsValidatable::<i64>::as_validatable(&n).unwrap();
        prop_assert_eq!(result, i64::from(n));
    }

    #[test]
    fn i16_to_i64_preserves_value(n in any::<i16>()) {
        let result: i64 = AsValidatable::<i64>::as_validatable(&n).unwrap();
        prop_assert_eq!(result, i64::from(n));
    }
}
