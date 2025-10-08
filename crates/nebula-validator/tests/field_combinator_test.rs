use nebula_validator::combinators::field::*;
use nebula_validator::core::{TypedValidator, ValidationError};

struct User {
    name: String,
    email: String,
    age: u32,
}

struct MinValue {
    min: u32,
}

impl TypedValidator for MinValue {
    type Input = u32;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &u32) -> Result<(), ValidationError> {
        if *input >= self.min {
            Ok(())
        } else {
            Err(ValidationError::new(
                "min_value",
                format!("Must be at least {}", self.min),
            ))
        }
    }
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

#[test]
fn test_field_basic() {
    let user = User {
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        age: 25,
    };

    // Test with u32 field (simple case)
    let age_validator = field(MinValue { min: 18 }, |u: &User| &u.age);

    assert!(age_validator.validate(&user).is_ok());
}

#[test]
fn test_field_named() {
    let user = User {
        name: "Al".to_string(),
        email: "alice@example.com".to_string(),
        age: 15,
    };

    let age_validator = named_field("age", MinValue { min: 18 }, |u: &User| &u.age);

    let err = age_validator.validate(&user).unwrap_err();
    assert_eq!(err.field_name(), Some("age"));
    assert!(err.to_string().contains("field 'age'"));
}

#[test]
fn test_field_error_display() {
    let error = FieldError::new(
        Some("email"),
        ValidationError::new("invalid", "Invalid email"),
    );

    let display = format!("{}", error);
    assert!(display.contains("field 'email'"));
    assert!(display.contains("Invalid email"));
}

#[test]
fn test_field_extension_trait() {
    let user = User {
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        age: 25,
    };

    let age_validator = MinValue { min: 18 }.for_field("age", |u: &User| &u.age);

    assert!(age_validator.validate(&user).is_ok());
}

#[test]
fn test_field_name_accessor() {
    let validator = named_field("age", MinValue { min: 18 }, |u: &User| &u.age);

    assert_eq!(validator.field_name(), Some("age"));
}

#[test]
fn test_field_multiple_fields() {
    let user = User {
        name: "Alice".to_string(),
        email: "a@b.c".to_string(),
        age: 25,
    };

    let age_validator1 = named_field("age", MinValue { min: 18 }, |u: &User| &u.age);
    let age_validator2 = named_field("age", MinValue { min: 30 }, |u: &User| &u.age);

    assert!(age_validator1.validate(&user).is_ok());
    assert!(age_validator2.validate(&user).is_err());
}
