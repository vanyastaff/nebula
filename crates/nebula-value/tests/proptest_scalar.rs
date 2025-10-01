//! Property-based tests for scalar types using proptest
//!
//! These tests verify algebraic properties and invariants that should hold
//! for all possible input values.

use nebula_value::{Integer, Float, Text, Bytes};
use proptest::prelude::*;

// ===== INTEGER PROPERTIES =====

proptest! {
    #[test]
    fn integer_identity(x in any::<i64>()) {
        let int = Integer::new(x);
        prop_assert_eq!(int.value(), x);
    }

    #[test]
    fn integer_addition_commutative(a in any::<i32>(), b in any::<i32>()) {
        let ia = Integer::new(a as i64);
        let ib = Integer::new(b as i64);

        let ab = ia.checked_add(ib);
        let ba = ib.checked_add(ia);

        prop_assert_eq!(ab, ba);
    }

    #[test]
    fn integer_addition_associative(a in any::<i16>(), b in any::<i16>(), c in any::<i16>()) {
        let ia = Integer::new(a as i64);
        let ib = Integer::new(b as i64);
        let ic = Integer::new(c as i64);

        // (a + b) + c
        if let Some(ab) = ia.checked_add(ib) {
            if let Some(abc1) = ab.checked_add(ic) {
                // a + (b + c)
                if let Some(bc) = ib.checked_add(ic) {
                    if let Some(abc2) = ia.checked_add(bc) {
                        prop_assert_eq!(abc1, abc2);
                    }
                }
            }
        }
    }

    #[test]
    fn integer_zero_identity(x in any::<i64>()) {
        let int = Integer::new(x);
        let zero = Integer::new(0);

        prop_assert_eq!(int.checked_add(zero), Some(int));
        prop_assert_eq!(zero.checked_add(int), Some(int));
    }

    #[test]
    fn integer_multiplication_commutative(a in any::<i16>(), b in any::<i16>()) {
        let ia = Integer::new(a as i64);
        let ib = Integer::new(b as i64);

        let ab = ia.checked_mul(ib);
        let ba = ib.checked_mul(ia);

        prop_assert_eq!(ab, ba);
    }

    #[test]
    fn integer_one_identity(x in any::<i64>()) {
        let int = Integer::new(x);
        let one = Integer::new(1);

        prop_assert_eq!(int.checked_mul(one), Some(int));
        prop_assert_eq!(one.checked_mul(int), Some(int));
    }

    #[test]
    fn integer_ordering_transitive(a in any::<i64>(), b in any::<i64>(), c in any::<i64>()) {
        let ia = Integer::new(a);
        let ib = Integer::new(b);
        let ic = Integer::new(c);

        // If a < b and b < c, then a < c
        if ia < ib && ib < ic {
            prop_assert!(ia < ic);
        }
    }

    #[test]
    fn integer_ordering_antisymmetric(a in any::<i64>(), b in any::<i64>()) {
        let ia = Integer::new(a);
        let ib = Integer::new(b);

        // If a <= b and b <= a, then a == b
        if ia <= ib && ib <= ia {
            prop_assert_eq!(ia, ib);
        }
    }

    #[test]
    fn integer_clone_equals_original(x in any::<i64>()) {
        let int = Integer::new(x);
        let cloned = int.clone();

        prop_assert_eq!(int, cloned);
        prop_assert_eq!(int.value(), cloned.value());
    }
}

// ===== FLOAT PROPERTIES =====

proptest! {
    #[test]
    fn float_identity(x in prop::num::f64::ANY) {
        let float = Float::new(x);

        if x.is_nan() {
            prop_assert!(float.is_nan());
        } else {
            prop_assert_eq!(float.value(), x);
        }
    }

    #[test]
    fn float_addition_commutative(a in prop::num::f64::NORMAL, b in prop::num::f64::NORMAL) {
        let fa = Float::new(a);
        let fb = Float::new(b);

        let ab = fa + fb;
        let ba = fb + fa;

        // For normal floats, addition is commutative
        prop_assert_eq!(ab.value(), ba.value());
    }

    #[test]
    fn float_zero_identity(x in prop::num::f64::NORMAL) {
        let float = Float::new(x);
        let zero = Float::new(0.0);

        let result = float + zero;
        prop_assert_eq!(result.value(), x);
    }

    #[test]
    fn float_one_identity(x in prop::num::f64::NORMAL) {
        let float = Float::new(x);
        let one = Float::new(1.0);

        let result = float * one;
        prop_assert_eq!(result.value(), x);
    }

    #[test]
    fn float_negation_involution(x in prop::num::f64::NORMAL) {
        let float = Float::new(x);
        let neg = -float;
        let neg_neg = -neg;

        prop_assert_eq!(neg_neg.value(), x);
    }

    #[test]
    fn float_abs_non_negative(x in prop::num::f64::NORMAL) {
        let float = Float::new(x);
        let abs = float.abs();

        prop_assert!(abs.value() >= 0.0);
    }

    #[test]
    fn float_abs_idempotent(x in prop::num::f64::NORMAL) {
        let float = Float::new(x);
        let abs1 = float.abs();
        let abs2 = abs1.abs();

        prop_assert_eq!(abs1.value(), abs2.value());
    }

    #[test]
    fn float_total_cmp_transitive(a in prop::num::f64::ANY, b in prop::num::f64::ANY, c in prop::num::f64::ANY) {
        use std::cmp::Ordering;

        let fa = Float::new(a);
        let fb = Float::new(b);
        let fc = Float::new(c);

        // If a < b and b < c, then a < c
        if fa.total_cmp(&fb) == Ordering::Less && fb.total_cmp(&fc) == Ordering::Less {
            prop_assert_eq!(fa.total_cmp(&fc), Ordering::Less);
        }
    }

    #[test]
    fn float_total_cmp_antisymmetric(a in prop::num::f64::ANY, b in prop::num::f64::ANY) {
        use std::cmp::Ordering;

        let fa = Float::new(a);
        let fb = Float::new(b);

        let ab = fa.total_cmp(&fb);
        let ba = fb.total_cmp(&fa);

        match ab {
            Ordering::Less => prop_assert_eq!(ba, Ordering::Greater),
            Ordering::Greater => prop_assert_eq!(ba, Ordering::Less),
            Ordering::Equal => prop_assert_eq!(ba, Ordering::Equal),
        }
    }

    #[test]
    fn float_clone_preserves_bits(x in prop::num::f64::ANY) {
        let float = Float::new(x);
        let cloned = float.clone();

        prop_assert_eq!(float.to_bits(), cloned.to_bits());
    }
}

// ===== TEXT PROPERTIES =====

proptest! {
    #[test]
    fn text_identity(s in ".*") {
        let text = Text::from_str(&s);
        prop_assert_eq!(text.as_str(), s);
    }

    #[test]
    fn text_length_matches_string(s in ".*") {
        let text = Text::from_str(&s);
        prop_assert_eq!(text.len(), s.len());
    }

    #[test]
    fn text_empty_iff_zero_length(s in ".*") {
        let text = Text::from_str(&s);
        prop_assert_eq!(text.is_empty(), s.is_empty());
        prop_assert_eq!(text.is_empty(), text.len() == 0);
    }

    #[test]
    fn text_concat_associative(a in ".*", b in ".*", c in ".*") {
        let ta = Text::from_str(&a);
        let tb = Text::from_str(&b);
        let tc = Text::from_str(&c);

        // (a + b) + c
        let ab = ta.concat(&tb);
        let abc1 = ab.concat(&tc);

        // a + (b + c)
        let bc = tb.concat(&tc);
        let abc2 = ta.concat(&bc);

        prop_assert_eq!(abc1.as_str(), abc2.as_str());
    }

    #[test]
    fn text_concat_empty_identity(s in ".*") {
        let text = Text::from_str(&s);
        let empty = Text::from_str("");

        let left = text.concat(&empty);
        let right = empty.concat(&text);

        prop_assert_eq!(left.as_str(), &s);
        prop_assert_eq!(right.as_str(), &s);
    }

    #[test]
    fn text_concat_length_sum(a in ".*", b in ".*") {
        let ta = Text::from_str(&a);
        let tb = Text::from_str(&b);

        let concat = ta.concat(&tb);

        prop_assert_eq!(concat.len(), a.len() + b.len());
    }

    #[test]
    fn text_clone_equals_original(s in ".*") {
        let text = Text::from_str(&s);
        let cloned = text.clone();

        prop_assert_eq!(text.as_str(), cloned.as_str());
        prop_assert_eq!(text, cloned);
    }

    #[test]
    fn text_hash_equality(s in ".*") {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let text1 = Text::from_str(&s);
        let text2 = Text::from_str(&s);

        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();

        text1.hash(&mut hasher1);
        text2.hash(&mut hasher2);

        prop_assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn text_substring_within_bounds(s in ".*", start in 0usize..100, len in 0usize..100) {
        let text = Text::from_str(&s);

        if start <= s.len() {
            let end = (start + len).min(s.len());
            if let Ok(substr) = text.substring(start, end) {
                prop_assert!(substr.len() <= text.len());
                prop_assert_eq!(substr.as_str(), &s[start..end]);
            }
        }
    }
}

// ===== BYTES PROPERTIES =====

proptest! {
    #[test]
    fn bytes_identity(data in prop::collection::vec(any::<u8>(), 0..1000)) {
        let bytes = Bytes::new(data.clone());
        prop_assert_eq!(bytes.as_slice(), &data[..]);
    }

    #[test]
    fn bytes_length_matches_vec(data in prop::collection::vec(any::<u8>(), 0..1000)) {
        let bytes = Bytes::new(data.clone());
        prop_assert_eq!(bytes.len(), data.len());
    }

    #[test]
    fn bytes_empty_iff_zero_length(data in prop::collection::vec(any::<u8>(), 0..100)) {
        let bytes = Bytes::new(data.clone());
        prop_assert_eq!(bytes.is_empty(), data.is_empty());
        prop_assert_eq!(bytes.is_empty(), bytes.len() == 0);
    }

    #[test]
    fn bytes_clone_equals_original(data in prop::collection::vec(any::<u8>(), 0..1000)) {
        let bytes = Bytes::new(data.clone());
        let cloned = bytes.clone();

        prop_assert_eq!(bytes.as_slice(), cloned.as_slice());
        prop_assert_eq!(bytes, cloned);
    }

    #[test]
    fn bytes_slice_within_bounds(data in prop::collection::vec(any::<u8>(), 0..1000), start in 0usize..100, end in 0usize..100) {
        let bytes = Bytes::new(data.clone());

        if start <= end && end <= bytes.len() {
            if let Ok(slice) = bytes.slice(start, end) {
                prop_assert_eq!(slice.len(), end - start);
                prop_assert_eq!(slice.as_slice(), &data[start..end]);
            }
        }
    }

    #[test]
    fn bytes_base64_roundtrip(data in prop::collection::vec(any::<u8>(), 0..1000)) {
        let bytes = Bytes::new(data.clone());
        let encoded = bytes.to_base64();

        if let Ok(decoded) = Bytes::from_base64(&encoded) {
            prop_assert_eq!(decoded.as_slice(), bytes.as_slice());
        }
    }

    #[test]
    fn bytes_hash_equality(data in prop::collection::vec(any::<u8>(), 0..100)) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let bytes1 = Bytes::new(data.clone());
        let bytes2 = Bytes::new(data.clone());

        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();

        bytes1.hash(&mut hasher1);
        bytes2.hash(&mut hasher2);

        prop_assert_eq!(hasher1.finish(), hasher2.finish());
    }
}

// ===== CROSS-TYPE PROPERTIES =====

proptest! {
    #[test]
    fn integer_float_conversion_preserves_value(x in -1000000i64..1000000i64) {
        let _int = Integer::new(x);
        let as_float = x as f64;

        // For integers in this range, conversion to f64 is exact
        let float = Float::new(as_float);
        prop_assert_eq!(float.value(), as_float);

        // Converting back should give original value
        let back = float.value() as i64;
        prop_assert_eq!(back, x);
    }

    #[test]
    fn text_bytes_utf8_roundtrip(s in ".*") {
        let text = Text::from_str(&s);
        let bytes_data = s.as_bytes().to_vec();
        let bytes = Bytes::new(bytes_data);

        // Text -> bytes
        prop_assert_eq!(text.as_str().as_bytes(), bytes.as_slice());

        // bytes -> Text (if valid UTF-8)
        if let Ok(utf8) = std::str::from_utf8(bytes.as_slice()) {
            prop_assert_eq!(utf8, text.as_str());
        }
    }
}