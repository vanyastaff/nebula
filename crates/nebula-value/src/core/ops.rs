//! Operations for Value: arithmetic, comparison, logical
//!
//! This module implements operations on Value with proper type coercion

use crate::core::NebulaError;
use crate::core::error::{ValueErrorExt, ValueResult};
use crate::core::value::Value;
use crate::scalar::Float;

impl Value {
    // ==================== Arithmetic Operations ====================

    /// Add two values (with type coercion)
    pub fn add(&self, other: &Value) -> ValueResult<Value> {
        match (self, other) {
            // Integer + Integer
            (Value::Integer(a), Value::Integer(b)) => a
                .checked_add(*b)
                .map(Value::Integer)
                .ok_or_else(|| NebulaError::validation("Integer overflow in addition")),

            // Float + Float
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a + *b)),

            // Integer + Float (promote to float)
            (Value::Integer(a), Value::Float(b)) => {
                Ok(Value::Float(Float::new(a.value() as f64) + *b))
            }

            // Float + Integer (promote to float)
            (Value::Float(a), Value::Integer(b)) => {
                Ok(Value::Float(*a + Float::new(b.value() as f64)))
            }

            // Decimal operations
            (Value::Decimal(a), Value::Decimal(b)) => Ok(Value::Decimal(*a + *b)),

            // Text concatenation
            (Value::Text(a), Value::Text(b)) => Ok(Value::Text(a.concat(b))),

            // Array concatenation
            (Value::Array(a), Value::Array(b)) => Ok(Value::Array(a.concat(b))),

            _ => Err(NebulaError::value_operation_not_supported(
                "add",
                format!("{} + {}", self.kind().name(), other.kind().name()),
            )),
        }
    }

    /// Subtract two values
    pub fn sub(&self, other: &Value) -> ValueResult<Value> {
        match (self, other) {
            // Integer - Integer
            (Value::Integer(a), Value::Integer(b)) => a
                .checked_sub(*b)
                .map(Value::Integer)
                .ok_or_else(|| NebulaError::validation("Integer overflow in subtraction")),

            // Float - Float
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a - *b)),

            // Integer - Float
            (Value::Integer(a), Value::Float(b)) => {
                Ok(Value::Float(Float::new(a.value() as f64) - *b))
            }

            // Float - Integer
            (Value::Float(a), Value::Integer(b)) => {
                Ok(Value::Float(*a - Float::new(b.value() as f64)))
            }

            // Decimal
            (Value::Decimal(a), Value::Decimal(b)) => Ok(Value::Decimal(*a - *b)),

            _ => Err(NebulaError::value_operation_not_supported(
                "subtract",
                format!("{} - {}", self.kind().name(), other.kind().name()),
            )),
        }
    }

    /// Multiply two values
    pub fn mul(&self, other: &Value) -> ValueResult<Value> {
        match (self, other) {
            // Integer * Integer
            (Value::Integer(a), Value::Integer(b)) => a
                .checked_mul(*b)
                .map(Value::Integer)
                .ok_or_else(|| NebulaError::validation("Integer overflow in multiplication")),

            // Float * Float
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a * *b)),

            // Integer * Float
            (Value::Integer(a), Value::Float(b)) => {
                Ok(Value::Float(Float::new(a.value() as f64) * *b))
            }

            // Float * Integer
            (Value::Float(a), Value::Integer(b)) => {
                Ok(Value::Float(*a * Float::new(b.value() as f64)))
            }

            // Decimal
            (Value::Decimal(a), Value::Decimal(b)) => Ok(Value::Decimal(*a * *b)),

            _ => Err(NebulaError::value_operation_not_supported(
                "multiply",
                format!("{} * {}", self.kind().name(), other.kind().name()),
            )),
        }
    }

    /// Divide two values
    pub fn div(&self, other: &Value) -> ValueResult<Value> {
        match (self, other) {
            // Check for division by zero
            (_, Value::Integer(b)) if b.value() == 0 => {
                Err(NebulaError::validation("Division by zero"))
            }
            (_, Value::Float(b)) if b.value() == 0.0 => {
                Err(NebulaError::validation("Division by zero"))
            }

            // Integer / Integer
            (Value::Integer(a), Value::Integer(b)) => a
                .checked_div(*b)
                .map(Value::Integer)
                .ok_or_else(|| NebulaError::validation("Integer overflow in division")),

            // Float / Float
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(*a / *b)),

            // Integer / Float
            (Value::Integer(a), Value::Float(b)) => {
                Ok(Value::Float(Float::new(a.value() as f64) / *b))
            }

            // Float / Integer
            (Value::Float(a), Value::Integer(b)) => {
                Ok(Value::Float(*a / Float::new(b.value() as f64)))
            }

            // Decimal
            (Value::Decimal(a), Value::Decimal(b)) => {
                if b.is_zero() {
                    return Err(NebulaError::validation("Division by zero"));
                }
                Ok(Value::Decimal(*a / *b))
            }

            _ => Err(NebulaError::value_operation_not_supported(
                "divide",
                format!("{} / {}", self.kind().name(), other.kind().name()),
            )),
        }
    }

    /// Modulo operation
    pub fn rem(&self, other: &Value) -> ValueResult<Value> {
        match (self, other) {
            (_, Value::Integer(b)) if b.value() == 0 => {
                Err(NebulaError::validation("Modulo by zero"))
            }

            (Value::Integer(a), Value::Integer(b)) => a
                .checked_rem(*b)
                .map(Value::Integer)
                .ok_or_else(|| NebulaError::validation("Integer overflow in modulo")),

            _ => Err(NebulaError::value_operation_not_supported(
                "modulo",
                format!("{} % {}", self.kind().name(), other.kind().name()),
            )),
        }
    }

    // ==================== Comparison Operations ====================

    /// Compare values for ordering
    pub fn compare(&self, other: &Value) -> ValueResult<std::cmp::Ordering> {
        use std::cmp::Ordering;

        match (self, other) {
            // Null comparisons
            (Value::Null, Value::Null) => Ok(Ordering::Equal),

            // Boolean
            (Value::Boolean(a), Value::Boolean(b)) => Ok(a.cmp(b)),

            // Integer
            (Value::Integer(a), Value::Integer(b)) => Ok(a.cmp(b)),

            // Float (using total_cmp for NaN handling)
            (Value::Float(a), Value::Float(b)) => Ok(a.total_cmp(b)),

            // Integer vs Float
            (Value::Integer(a), Value::Float(b)) => Ok(Float::new(a.value() as f64).total_cmp(b)),
            (Value::Float(a), Value::Integer(b)) => Ok(a.total_cmp(&Float::new(b.value() as f64))),

            // Text
            (Value::Text(a), Value::Text(b)) => Ok(a.cmp(b)),

            // Bytes
            (Value::Bytes(a), Value::Bytes(b)) => Ok(a.cmp(b)),

            _ => Err(NebulaError::value_operation_not_supported(
                "compare",
                format!("{} <=> {}", self.kind().name(), other.kind().name()),
            )),
        }
    }

    /// Less than
    pub fn lt(&self, other: &Value) -> ValueResult<bool> {
        self.compare(other).map(|ord| ord.is_lt())
    }

    /// Less than or equal
    pub fn le(&self, other: &Value) -> ValueResult<bool> {
        self.compare(other).map(|ord| ord.is_le())
    }

    /// Greater than
    pub fn gt(&self, other: &Value) -> ValueResult<bool> {
        self.compare(other).map(|ord| ord.is_gt())
    }

    /// Greater than or equal
    pub fn ge(&self, other: &Value) -> ValueResult<bool> {
        self.compare(other).map(|ord| ord.is_ge())
    }

    // ==================== Logical Operations ====================

    /// Logical AND
    pub fn and(&self, other: &Value) -> bool {
        self.to_boolean() && other.to_boolean()
    }

    /// Logical OR
    pub fn or(&self, other: &Value) -> bool {
        self.to_boolean() || other.to_boolean()
    }

    /// Logical NOT
    pub fn not(&self) -> bool {
        !self.to_boolean()
    }

    // ==================== Utility Operations ====================

    /// Check if value is truthy
    pub fn is_truthy(&self) -> bool {
        self.to_boolean()
    }

    /// Check if value is falsy
    pub fn is_falsy(&self) -> bool {
        !self.to_boolean()
    }

    // ==================== Merge Operations ====================

    /// Deep merge two objects
    ///
    /// For overlapping keys:
    /// - If both values are objects: recursively merge
    /// - Otherwise: right value overwrites left
    ///
    /// # Example
    ///
    /// ```ignore
    /// let a = object! { "x" => 1, "nested" => object! { "a" => 1 } };
    /// let b = object! { "y" => 2, "nested" => object! { "b" => 2 } };
    /// let merged = a.merge(&b)?;
    /// // Result: { "x": 1, "y": 2, "nested": { "a": 1, "b": 2 } }
    /// ```
    pub fn merge(&self, other: &Value) -> ValueResult<Value> {
        match (self, other) {
            // Deep merge for objects
            (Value::Object(a), Value::Object(b)) => Ok(Value::Object(a.merge(b))),

            // Arrays: concatenation (alternative: union/dedup available as separate method)
            (Value::Array(a), Value::Array(b)) => Ok(Value::Array(a.concat(b))),

            // For non-mergeable types, right overwrites left
            (_, right) => Ok(right.clone()),
        }
    }

    /// Shallow merge - only merge top level, no recursion
    pub fn merge_shallow(&self, other: &Value) -> ValueResult<Value> {
        match (self, other) {
            (Value::Object(a), Value::Object(b)) => {
                // Use Object::merge_shallow if available, otherwise fallback
                Ok(Value::Object(a.merge(b))) // TODO: add shallow variant to Object
            }
            _ => self.merge(other),
        }
    }

    /// Merge with a custom merge function for conflicts
    ///
    /// The merge function receives (left_value, right_value, key) and returns the merged value.
    #[cfg(feature = "serde")]
    pub fn merge_with<F>(&self, other: &Value, merge_fn: F) -> ValueResult<Value>
    where
        F: Fn(&Value, &Value, &str) -> ValueResult<Value> + Copy,
    {
        use crate::core::convert::{JsonValueExt, ValueRefExt};

        match (self, other) {
            (Value::Object(left), Value::Object(right)) => {
                // Start with left object
                let mut result = left.clone();

                // Merge each key from right
                for (key, right_val_json) in right.entries() {
                    if let Some(left_val_json) = left.get(key) {
                        // Key exists in both - apply merge function
                        // Convert both values from serde_json to nebula_value
                        let left_val = left_val_json.to_nebula_value_or_null();
                        let right_val = right_val_json.to_nebula_value_or_null();

                        // Apply custom merge function
                        let merged_val = merge_fn(&left_val, &right_val, key)?;

                        // Convert back to serde_json::Value for storage
                        result = result.insert(key.to_string(), merged_val.to_json());
                    } else {
                        // Key only in right - add it directly
                        result = result.insert(key.to_string(), right_val_json.clone());
                    }
                }

                Ok(Value::Object(result))
            }
            _ => Err(NebulaError::value_type_mismatch(
                "Object",
                self.kind().name(),
            )),
        }
    }

    /// Merge with a custom merge function (requires 'serde' feature)
    #[cfg(not(feature = "serde"))]
    pub fn merge_with<F>(&self, _other: &Value, _merge_fn: F) -> ValueResult<Value>
    where
        F: Fn(&Value, &Value, &str) -> ValueResult<Value> + Copy,
    {
        Err(NebulaError::validation(
            "merge_with requires 'serde' feature to be enabled",
        ))
    }

    /// Try to merge array elements (union without duplicates)
    ///
    /// Note: Currently performs simple concatenation.
    /// Array deduplication is complex because Array stores serde_json::Value internally.
    /// For full deduplication, convert to HashableValue first.
    pub fn merge_array_union(&self, other: &Value) -> ValueResult<Value> {
        match (self, other) {
            (Value::Array(a), Value::Array(b)) => {
                // Simple concatenation - deduplication requires HashableValue conversion
                // which may not be desired for all use cases
                Ok(Value::Array(a.concat(b)))
            }
            _ => Err(NebulaError::value_type_mismatch(
                "Array",
                self.kind().name(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_integers() {
        let a = Value::integer(10);
        let b = Value::integer(5);
        let result = a.add(&b).unwrap();

        assert!(result.is_integer());
        assert_eq!(result.as_integer(), Some(15));
    }

    #[test]
    fn test_add_floats() {
        let a = Value::float(10.5);
        let b = Value::float(5.3);
        let result = a.add(&b).unwrap();

        assert!(result.is_float());
        assert_eq!(result.as_float(), Some(15.8));
    }

    #[test]
    fn test_add_integer_float() {
        let a = Value::integer(10);
        let b = Value::float(5.5);
        let result = a.add(&b).unwrap();

        assert!(result.is_float());
        assert_eq!(result.as_float(), Some(15.5));
    }

    #[test]
    fn test_add_text() {
        let a = Value::text("Hello ");
        let b = Value::text("World");
        let result = a.add(&b).unwrap();

        assert!(result.is_text());
        assert_eq!(result.as_str(), Some("Hello World"));
    }

    #[test]
    fn test_subtract() {
        let a = Value::integer(10);
        let b = Value::integer(3);
        let result = a.sub(&b).unwrap();

        assert_eq!(result.as_integer(), Some(7));
    }

    #[test]
    fn test_multiply() {
        let a = Value::integer(5);
        let b = Value::integer(3);
        let result = a.mul(&b).unwrap();

        assert_eq!(result.as_integer(), Some(15));
    }

    #[test]
    fn test_divide() {
        let a = Value::integer(10);
        let b = Value::integer(2);
        let result = a.div(&b).unwrap();

        assert_eq!(result.as_integer(), Some(5));
    }

    #[test]
    fn test_divide_by_zero() {
        let a = Value::integer(10);
        let b = Value::integer(0);
        let result = a.div(&b);

        assert!(result.is_err());
    }

    #[test]
    fn test_compare_integers() {
        let a = Value::integer(10);
        let b = Value::integer(5);

        assert!(a.gt(&b).unwrap());
        assert!(b.lt(&a).unwrap());
        assert_eq!(a.compare(&b).unwrap(), std::cmp::Ordering::Greater);
    }

    #[test]
    fn test_compare_floats() {
        let a = Value::float(10.5);
        let b = Value::float(5.3);

        assert!(a.gt(&b).unwrap());
        assert!(b.lt(&a).unwrap());
    }

    #[test]
    fn test_compare_text() {
        let a = Value::text("apple");
        let b = Value::text("banana");

        assert!(a.lt(&b).unwrap());
        assert!(b.gt(&a).unwrap());
    }

    #[test]
    fn test_logical_and() {
        assert_eq!(Value::boolean(true).and(&Value::boolean(true)), true);
        assert_eq!(Value::boolean(true).and(&Value::boolean(false)), false);
        assert_eq!(Value::boolean(false).and(&Value::boolean(true)), false);
    }

    #[test]
    fn test_logical_or() {
        assert_eq!(Value::boolean(true).or(&Value::boolean(false)), true);
        assert_eq!(Value::boolean(false).or(&Value::boolean(false)), false);
    }

    #[test]
    fn test_logical_not() {
        assert_eq!(Value::boolean(true).not(), false);
        assert_eq!(Value::boolean(false).not(), true);
        assert_eq!(Value::null().not(), true);
    }

    #[test]
    fn test_is_truthy_falsy() {
        assert!(Value::boolean(true).is_truthy());
        assert!(Value::boolean(false).is_falsy());
        assert!(Value::null().is_falsy());
        assert!(Value::integer(0).is_falsy());
        assert!(Value::integer(42).is_truthy());
    }

    #[test]
    fn test_overflow() {
        let a = Value::integer(i64::MAX);
        let b = Value::integer(1);
        let result = a.add(&b);

        assert!(result.is_err());
    }

    // ==================== Merge Tests ====================

    #[test]
    fn test_merge_objects_simple() {
        use crate::collections::Object;

        let mut obj_a = Object::new();
        obj_a = obj_a.insert("x".to_string(), serde_json::json!(1));
        obj_a = obj_a.insert("y".to_string(), serde_json::json!(2));

        let mut obj_b = Object::new();
        obj_b = obj_b.insert("z".to_string(), serde_json::json!(3));
        obj_b = obj_b.insert("w".to_string(), serde_json::json!(4));

        let a = Value::Object(obj_a);
        let b = Value::Object(obj_b);

        let merged = a.merge(&b).unwrap();

        assert!(merged.is_object());
        if let Value::Object(obj) = merged {
            assert_eq!(obj.len(), 4);
            assert!(obj.contains_key("x"));
            assert!(obj.contains_key("y"));
            assert!(obj.contains_key("z"));
            assert!(obj.contains_key("w"));
        } else {
            panic!("Expected Object");
        }
    }

    #[test]
    fn test_merge_objects_overwrite() {
        use crate::collections::Object;

        let mut obj_a = Object::new();
        obj_a = obj_a.insert("x".to_string(), serde_json::json!(1));
        obj_a = obj_a.insert("y".to_string(), serde_json::json!(2));

        let mut obj_b = Object::new();
        obj_b = obj_b.insert("x".to_string(), serde_json::json!(100)); // Overwrites
        obj_b = obj_b.insert("z".to_string(), serde_json::json!(3));

        let a = Value::Object(obj_a);
        let b = Value::Object(obj_b);

        let merged = a.merge(&b).unwrap();

        assert!(merged.is_object());
        if let Value::Object(obj) = merged {
            assert_eq!(obj.len(), 3);
            assert_eq!(obj.get("x"), Some(&serde_json::json!(100)));
            assert_eq!(obj.get("y"), Some(&serde_json::json!(2)));
            assert_eq!(obj.get("z"), Some(&serde_json::json!(3)));
        }
    }

    #[test]
    fn test_merge_arrays() {
        use crate::collections::Array;

        let mut arr_a = Array::new();
        arr_a = arr_a.push(serde_json::json!(1));
        arr_a = arr_a.push(serde_json::json!(2));

        let mut arr_b = Array::new();
        arr_b = arr_b.push(serde_json::json!(3));
        arr_b = arr_b.push(serde_json::json!(4));

        let a = Value::Array(arr_a);
        let b = Value::Array(arr_b);

        let merged = a.merge(&b).unwrap();

        assert!(merged.is_array());
        if let Value::Array(arr) = merged {
            assert_eq!(arr.len(), 4);
            assert_eq!(arr.get(0), Some(&serde_json::json!(1)));
            assert_eq!(arr.get(1), Some(&serde_json::json!(2)));
            assert_eq!(arr.get(2), Some(&serde_json::json!(3)));
            assert_eq!(arr.get(3), Some(&serde_json::json!(4)));
        }
    }

    #[test]
    fn test_merge_non_objects() {
        let a = Value::integer(10);
        let b = Value::integer(20);

        let merged = a.merge(&b).unwrap();

        // Non-mergeable types: right overwrites left
        assert_eq!(merged.as_integer(), Some(20));
    }

    #[test]
    fn test_merge_shallow() {
        use crate::collections::Object;

        let mut obj_a = Object::new();
        obj_a = obj_a.insert("x".to_string(), serde_json::json!(1));

        let mut obj_b = Object::new();
        obj_b = obj_b.insert("y".to_string(), serde_json::json!(2));

        let a = Value::Object(obj_a);
        let b = Value::Object(obj_b);

        let merged = a.merge_shallow(&b).unwrap();

        assert!(merged.is_object());
        if let Value::Object(obj) = merged {
            assert_eq!(obj.len(), 2);
        }
    }

    #[test]
    fn test_merge_array_union() {
        use crate::collections::Array;

        let mut arr_a = Array::new();
        arr_a = arr_a.push(serde_json::json!(1));
        arr_a = arr_a.push(serde_json::json!(2));

        let mut arr_b = Array::new();
        arr_b = arr_b.push(serde_json::json!(2)); // Duplicate
        arr_b = arr_b.push(serde_json::json!(3));

        let a = Value::Array(arr_a);
        let b = Value::Array(arr_b);

        let merged = a.merge_array_union(&b).unwrap();

        assert!(merged.is_array());
        // Note: Currently just concat due to serde_json::Value limitation
        if let Value::Array(arr) = merged {
            assert_eq!(arr.len(), 4); // Will be 3 once proper union is implemented
        }
    }

    #[test]
    fn test_merge_with_type_mismatch() {
        use crate::collections::Object;

        let obj = Object::new();
        let a = Value::Object(obj);
        let b = Value::integer(42);

        // Merge with type mismatch: right overwrites
        let merged = a.merge(&b).unwrap();
        assert_eq!(merged.as_integer(), Some(42));
    }
}
