//! Property-based tests for Value operations and conversions

use nebula_value::{Array, Value};
use proptest::prelude::*;
use serde_json::json;
use std::convert::TryFrom;

// Strategy for generating Values
fn any_simple_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::null()),
        any::<bool>().prop_map(Value::boolean),
        any::<i64>().prop_map(Value::integer),
        prop::num::f64::NORMAL.prop_map(Value::float),
        ".*".prop_map(|s| Value::text(&s)),
        prop::collection::vec(any::<u8>(), 0..100).prop_map(Value::bytes),
    ]
}

// ===== VALUE TYPE CHECKING =====

proptest! {
    #[test]
    fn value_is_null_correct(is_null in any::<bool>()) {
        let val = if is_null {
            Value::null()
        } else {
            Value::integer(42)
        };

        prop_assert_eq!(val.is_null(), is_null);
    }

    #[test]
    fn value_is_numeric_correct(x in any::<i64>(), f in prop::num::f64::NORMAL) {
        let int_val = Value::integer(x);
        let float_val = Value::float(f);
        let text_val = Value::text("hello");

        prop_assert!(int_val.is_numeric());
        prop_assert!(float_val.is_numeric());
        prop_assert!(!text_val.is_numeric());
    }
}

// ===== ARITHMETIC OPERATIONS =====

proptest! {
    #[test]
    fn value_integer_addition_commutative(a in any::<i32>(), b in any::<i32>()) {
        let va = Value::integer(a as i64);
        let vb = Value::integer(b as i64);

        let ab = va.add(&vb);
        let ba = vb.add(&va);

        // Both should succeed or both should fail
        match (ab, ba) {
            (Ok(r1), Ok(r2)) => {
                if let (Value::Integer(i1), Value::Integer(i2)) = (r1, r2) {
                    prop_assert_eq!(i1, i2);
                }
            }
            (Err(_), Err(_)) => {}, // Both overflow, OK
            _ => prop_assert!(false, "One succeeded, one failed"),
        }
    }

    #[test]
    fn value_integer_zero_identity(x in any::<i64>()) {
        let val = Value::integer(x);
        let zero = Value::integer(0);

        let result1 = val.add(&zero).unwrap();
        let result2 = zero.add(&val).unwrap();

        if let (Value::Integer(i1), Value::Integer(i2)) = (result1, result2) {
            prop_assert_eq!(i1.value(), x);
            prop_assert_eq!(i2.value(), x);
        }
    }

    #[test]
    fn value_integer_one_identity(x in any::<i64>()) {
        let val = Value::integer(x);
        let one = Value::integer(1);

        let result = val.mul(&one).unwrap();

        if let Value::Integer(i) = result {
            prop_assert_eq!(i.value(), x);
        }
    }

    #[test]
    fn value_text_concat_associative(a in ".*", b in ".*", c in ".*") {
        let va = Value::text(&a);
        let vb = Value::text(&b);
        let vc = Value::text(&c);

        // (a + b) + c
        let ab = va.add(&vb).unwrap();
        let abc1 = ab.add(&vc).unwrap();

        // a + (b + c)
        let bc = vb.add(&vc).unwrap();
        let abc2 = va.add(&bc).unwrap();

        if let (Value::Text(t1), Value::Text(t2)) = (abc1, abc2) {
            prop_assert_eq!(t1.as_str(), t2.as_str());
        }
    }

    #[test]
    fn value_mixed_type_coercion(a in any::<i32>(), b in prop::num::f64::NORMAL) {
        let int_val = Value::integer(a as i64);
        let float_val = Value::float(b);

        // Integer + Float should coerce to Float
        let result = int_val.add(&float_val).unwrap();

        prop_assert!(result.is_float());
    }
}

// ===== COMPARISON OPERATIONS =====

proptest! {
    #[test]
    fn value_equality_reflexive(x in any::<i64>()) {
        let val = Value::integer(x);
        prop_assert!(val.eq(&val));
    }

    #[test]
    fn value_equality_symmetric(a in any::<i64>(), b in any::<i64>()) {
        let va = Value::integer(a);
        let vb = Value::integer(b);

        let ab = va.eq(&vb);
        let ba = vb.eq(&va);

        prop_assert_eq!(ab, ba);
    }

    #[test]
    fn value_not_equal_to_different_type(x in any::<i64>()) {
        let int_val = Value::integer(x);
        let text_val = Value::text("hello");

        let result = int_val.eq(&text_val);
        prop_assert!(!result);
    }
}

// ===== LOGICAL OPERATIONS =====

proptest! {
    #[test]
    fn value_and_commutative(a in any::<bool>(), b in any::<bool>()) {
        let va = Value::boolean(a);
        let vb = Value::boolean(b);

        let ab = va.and(&vb);
        let ba = vb.and(&va);

        prop_assert_eq!(ab, ba);
    }

    #[test]
    fn value_not_involution(a in any::<bool>()) {
        let val = Value::boolean(a);

        let not_val = val.not();
        let not_not_val = !not_val;

        prop_assert_eq!(not_not_val, a);
    }

    #[test]
    fn value_and_identity(a in any::<bool>()) {
        let val = Value::boolean(a);
        let true_val = Value::boolean(true);

        let result = val.and(&true_val);

        prop_assert_eq!(result, a);
    }

    #[test]
    fn value_or_identity(a in any::<bool>()) {
        let val = Value::boolean(a);
        let false_val = Value::boolean(false);

        let result = val.or(&false_val);

        prop_assert_eq!(result, a);
    }
}

// ===== MERGE OPERATIONS =====

proptest! {
    #[test]
    fn value_merge_right_wins_scalar(a in any::<i64>(), b in any::<i64>()) {
        let va = Value::integer(a);
        let vb = Value::integer(b);

        let result = va.merge(&vb).unwrap();

        // For scalars, right value wins
        if let Value::Integer(i) = result {
            prop_assert_eq!(i.value(), b);
        }
    }

    #[test]
    fn value_array_merge_concat(a in prop::collection::vec(any::<i32>(), 0..10), b in prop::collection::vec(any::<i32>(), 0..10)) {
        let json_a: Vec<_> = a.iter().map(|&i| json!(i)).collect();
        let json_b: Vec<_> = b.iter().map(|&i| json!(i)).collect();

        let va = Value::Array(Array::from_vec(json_a));
        let vb = Value::Array(Array::from_vec(json_b));

        let result = va.merge(&vb).unwrap();

        if let Value::Array(arr) = result {
            prop_assert_eq!(arr.len(), a.len() + b.len());
        }
    }
}

// ===== CLONE PROPERTIES =====

proptest! {
    #[test]
    fn value_clone_equals_original(val in any_simple_value()) {
        let cloned = val.clone();

        match (&val, &cloned) {
            (Value::Null, Value::Null) => {},
            (Value::Boolean(a), Value::Boolean(b)) => prop_assert_eq!(a, b),
            (Value::Integer(a), Value::Integer(b)) => prop_assert_eq!(a, b),
            (Value::Float(a), Value::Float(b)) => {
                if a.is_nan() {
                    prop_assert!(b.is_nan());
                } else {
                    prop_assert_eq!(a.value(), b.value());
                }
            }
            (Value::Text(a), Value::Text(b)) => prop_assert_eq!(a, b),
            (Value::Bytes(a), Value::Bytes(b)) => prop_assert_eq!(a, b),
            _ => {}
        }
    }
}

// ===== CONVERSION PROPERTIES =====

proptest! {
    #[test]
    fn value_to_i64_roundtrip(x in any::<i64>()) {
        let val = Value::integer(x);
        let back = i64::try_from(val).unwrap();

        prop_assert_eq!(back, x);
    }

    #[test]
    fn value_to_bool_roundtrip(b in any::<bool>()) {
        let val = Value::boolean(b);
        let back = bool::try_from(val).unwrap();

        prop_assert_eq!(back, b);
    }

    #[test]
    fn value_to_string_roundtrip(s in ".*") {
        let val = Value::text(&s);
        let back = String::try_from(val).unwrap();

        prop_assert_eq!(back, s);
    }

    #[test]
    fn value_to_vec_u8_roundtrip(data in prop::collection::vec(any::<u8>(), 0..100)) {
        let val = Value::bytes(data.clone());
        let back = Vec::<u8>::try_from(val).unwrap();

        prop_assert_eq!(back, data);
    }
}

// ===== SERDE ROUNDTRIP =====

#[cfg(feature = "serde")]
proptest! {
    #[test]
    fn value_json_roundtrip_integer(x in any::<i64>()) {
        let val = Value::integer(x);
        let json = serde_json::to_string(&val).unwrap();
        let back: Value = serde_json::from_str(&json).unwrap();

        if let Value::Integer(i) = back {
            prop_assert_eq!(i.value(), x);
        } else {
            prop_assert!(false, "Deserialized to wrong type");
        }
    }

    #[test]
    fn value_json_roundtrip_boolean(b in any::<bool>()) {
        let val = Value::boolean(b);
        let json = serde_json::to_string(&val).unwrap();
        let back: Value = serde_json::from_str(&json).unwrap();

        if let Value::Boolean(result) = back {
            prop_assert_eq!(result, b);
        } else {
            prop_assert!(false, "Deserialized to wrong type");
        }
    }

    #[test]
    fn value_json_roundtrip_text(s in ".*") {
        let val = Value::text(&s);
        let json = serde_json::to_string(&val).unwrap();
        let back: Value = serde_json::from_str(&json).unwrap();

        if let Value::Text(t) = back {
            prop_assert_eq!(t.as_str(), s);
        } else {
            prop_assert!(false, "Deserialized to wrong type");
        }
    }
}

// ===== ERROR HANDLING =====

proptest! {
    #[test]
    fn value_wrong_type_conversion_fails(x in any::<i64>()) {
        let val = Value::integer(x);

        // Try to convert integer to bool - should fail
        let result = bool::try_from(val);
        prop_assert!(result.is_err());
    }

    #[test]
    fn value_divide_by_zero_fails(x in any::<i64>()) {
        let val = Value::integer(x);
        let zero = Value::integer(0);

        let result = val.div(&zero);
        prop_assert!(result.is_err());
    }
}
