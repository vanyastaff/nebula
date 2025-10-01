//! Cross-field validation operations using the unified validator macro

use crate::{validator, validator_fn, ValueExt};
use nebula_value::Value;

// ==================== CROSS-FIELD VALIDATORS ====================

validator! {
    /// Validator that ensures current field equals another field
    pub struct EqualsField {
        other_field: String
    }
    impl {
        fn check_with_context(value: &Value, ctx: &ValidationContext, other_field: &String) -> bool {
            {
                if let Some(other_value) = ctx.get_sibling(other_field).or_else(|| ctx.get_field(other_field)) {
                    value == other_value
                } else {
                    false
                }
            }
        }
        fn error(other_field: &String) -> String {
            { format!("Value must equal field '{}'", other_field) }
        }
        const DESCRIPTION: &str = "Field value must equal another field";
    }
}

validator! {
    /// Validator that ensures current field is different from another field
    pub struct DifferentFrom {
        other_field: String
    }
    impl {
        fn check_with_context(value: &Value, ctx: &ValidationContext, other_field: &String) -> bool {
            {
                if let Some(other_value) = ctx.get_sibling(other_field).or_else(|| ctx.get_field(other_field)) {
                    value != other_value
                } else {
                    true // If other field doesn't exist, we consider it "different"
                }
            }
        }
        fn error(other_field: &String) -> String {
            { format!("Value must be different from field '{}'", other_field) }
        }
        const DESCRIPTION: &str = "Field value must be different from another field";
    }
}

validator! {
    /// Validator that makes current field required if another field has a specific value
    pub struct RequiredIfField {
        other_field: String,
        other_value: Value
    }
    impl {
        fn check_with_context(value: &Value, ctx: &ValidationContext, other_field: &String, other_value: &Value) -> bool {
            {
                let other_field_value = ctx.get_sibling(other_field)
                    .or_else(|| ctx.get_field(other_field));

                // If the other field has the specified value, current field becomes required
                if let Some(other_val) = other_field_value {
                    if other_val == other_value {
                        // Field is required, check if it's empty
                        if value.is_null() {
                            return false;
                        } else if value.is_collection() || value.is_text() {
                            return !value.is_empty();
                        } else {
                            return true;
                        }
                    }
                }

                // If other field doesn't have the specified value, current field is not required
                true
            }
        }
        fn error(other_field: &String, other_value: &Value) -> String {
            { format!("Field is required when '{}' equals '{}'", other_field, other_value) }
        }
        const DESCRIPTION: &str = "Field is required when another field has a specific value";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn equals_field(other_field: String) -> EqualsField);
validator_fn!(pub fn different_from(other_field: String) -> DifferentFrom);
validator_fn!(pub fn required_if_field(other_field: String, other_value: Value) -> RequiredIfField);

// String-specific convenience functions with &str input
pub fn equals_field_str(other_field: &str) -> EqualsField {
    EqualsField::new(other_field.to_string())
}

pub fn different_from_str(other_field: &str) -> DifferentFrom {
    DifferentFrom::new(other_field.to_string())
}

pub fn required_if_field_str(other_field: &str, other_value: Value) -> RequiredIfField {
    RequiredIfField::new(other_field.to_string(), other_value)
}