//! Example: Using the universal `expr` attribute for custom validators
//!
//! This demonstrates how you can use ANY validator without modifying nebula-derive!

use nebula_derive::Validator;
use nebula_validator::prelude::*;

// Example 1: Using expr for new validators not yet in nebula-derive
#[derive(Validator)]
struct UserProfile {
    // Standard built-in syntax
    #[validate(min_length = 3, max_length = 20)]
    username: String,

    // Universal expr syntax - works with ANY validator!
    #[validate(expr = "nebula_validator::validators::string::email()")]
    email: String,

    // Complex composition with expr
    #[validate(expr = "nebula_validator::validators::numeric::in_range(18, 100)")]
    age: u8,
}

// Example 2: Using expr for complex validator chains
#[derive(Validator)]
struct ProductForm {
    // Complex validator chain using combinators
    #[validate(expr = r#"
        nebula_validator::validators::string::min_length(3)
            .and(nebula_validator::validators::string::max_length(50))
            .and(nebula_validator::validators::string::alphanumeric())
    "#)]
    product_code: String,

    // Short import path (if you have `use` statements)
    #[validate(expr = "min_length(1).and(max_length(200))")]
    description: String,
}

// Example 3: Using expr for validators that don't exist yet in derive
#[derive(Validator)]
struct FutureProofForm {
    // Let's say you add a new validator to nebula-validator
    // but haven't updated nebula-derive yet - no problem!
    #[validate(expr = "my_custom_validator()")]
    custom_field: String,

    // Or use an external validator from another crate
    #[validate(expr = "external_crate::special_validator()")]
    special_field: String,
}

// Helper function for custom validation
fn my_custom_validator() -> impl TypedValidator<Input = str, Output = (), Error = ValidationError> {
    MinLength { min: 5 }
}

struct MinLength {
    min: usize,
}

impl TypedValidator for MinLength {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.len() >= self.min {
            Ok(())
        } else {
            Err(ValidationError::new(
                "min_length",
                format!("Must be at least {} characters", self.min),
            ))
        }
    }
}

fn main() {
    println!("✅ Universal validator expressions allow using ANY validator!");
    println!("   No need to wait for nebula-derive updates!");
}
