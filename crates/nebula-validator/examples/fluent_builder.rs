//! Examples of using the Fluent Builder API for Nebula Validator
//! 
//! This demonstrates the type-safe, fluent interface for building validators.

use nebula_validator::{
    string, numeric, collection, custom,
    ValidationBuilder, CompositeValidator,
    Validatable, ValidationResult,
};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Nebula Validator Fluent Builder Examples\n");

    // Example 1: String validation
    example_string_validation().await?;
    
    // Example 2: Numeric validation
    example_numeric_validation().await?;
    
    // Example 3: Collection validation
    example_collection_validation().await?;
    
    // Example 4: Custom validation
    example_custom_validation().await?;
    
    // Example 5: Complex validation chains
    example_complex_validation().await?;
    
    // Example 6: Builder composition
    example_builder_composition().await?;

    println!("\n✅ All examples completed successfully!");
    Ok(())
}

/// Example 1: String validation with fluent builder
async fn example_string_validation() -> Result<(), Box<dyn std::error::Error>> {
    println!("📝 Example 1: String Validation");
    
    // Build a string validator with multiple constraints
    let username_validator = string()
        .min_length(3)
        .max_length(20)
        .pattern(r"^[a-zA-Z0-9_]+$")
        .required()
        .build();
    
    // Test valid username
    let valid_username = Value::String("john_doe".to_string());
    let result = username_validator.validate(&valid_username).await;
    match result {
        Ok(()) => println!("  ✅ Valid username: 'john_doe'"),
        Err(errors) => {
            println!("  ❌ Username validation failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid username (too short)
    let invalid_username = Value::String("ab".to_string());
    let result = username_validator.validate(&invalid_username).await;
    match result {
        Ok(()) => println!("  ✅ Invalid username passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Username validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid username (contains invalid characters)
    let invalid_username2 = Value::String("john@doe".to_string());
    let result = username_validator.validate(&invalid_username2).await;
    match result {
        Ok(()) => println!("  ✅ Invalid username passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Username validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 2: Numeric validation with fluent builder
async fn example_numeric_validation() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔢 Example 2: Numeric Validation");
    
    // Build a numeric validator for age
    let age_validator = numeric()
        .min(18.0)
        .max(120.0)
        .required()
        .build();
    
    // Test valid age
    let valid_age = Value::Number(serde_json::Number::from_f64(25.0).unwrap());
    let result = age_validator.validate(&valid_age).await;
    match result {
        Ok(()) => println!("  ✅ Valid age: 25"),
        Err(errors) => {
            println!("  ❌ Age validation failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid age (too young)
    let invalid_age = Value::Number(serde_json::Number::from_f64(15.0).unwrap());
    let result = age_validator.validate(&invalid_age).await;
    match result {
        Ok(()) => println!("  ✅ Invalid age passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Age validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid age (too old)
    let invalid_age2 = Value::Number(serde_json::Number::from_f64(150.0).unwrap());
    let result = age_validator.validate(&invalid_age2).await;
    match result {
        Ok(()) => println!("  ✅ Invalid age passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Age validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 3: Collection validation with fluent builder
async fn example_collection_validation() -> Result<(), Box<dyn std::error::Error>> {
    println!("📦 Example 3: Collection Validation");
    
    // Build a collection validator for tags
    let tags_validator = collection()
        .min_length(1)
        .max_length(10)
        .required()
        .build();
    
    // Test valid tags collection
    let valid_tags = Value::Array(vec![
        Value::String("rust".to_string()),
        Value::String("async".to_string()),
        Value::String("validation".to_string()),
    ]);
    let result = tags_validator.validate(&valid_tags).await;
    match result {
        Ok(()) => println!("  ✅ Valid tags collection: 3 tags"),
        Err(errors) => {
            println!("  ❌ Tags validation failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid tags collection (empty)
    let invalid_tags = Value::Array(vec![]);
    let result = tags_validator.validate(&invalid_tags).await;
    match result {
        Ok(()) => println!("  ✅ Invalid tags passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Tags validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid tags collection (too many)
    let invalid_tags2 = Value::Array((0..15).map(|i| Value::String(format!("tag{}", i))).collect());
    let result = tags_validator.validate(&invalid_tags2).await;
    match result {
        Ok(()) => println!("  ✅ Invalid tags passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Tags validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 4: Custom validation with fluent builder
async fn example_custom_validation() -> Result<(), Box<dyn std::error::Error>> {
    println!("🎯 Example 4: Custom Validation");
    
    // Build a string validator with custom validation
    let password_validator = string()
        .min_length(8)
        .max_length(100)
        .custom("no_common_passwords", |password| {
            let common_passwords = ["password", "123456", "qwerty", "admin"];
            if common_passwords.contains(&password) {
                Err("Password is too common".to_string())
            } else {
                Ok(())
            }
        })
        .custom("must_contain_uppercase", |password| {
            if password.chars().any(|c| c.is_uppercase()) {
                Ok(())
            } else {
                Err("Password must contain at least one uppercase letter".to_string())
            }
        })
        .custom("must_contain_digit", |password| {
            if password.chars().any(|c| c.is_numeric()) {
                Ok(())
            } else {
                Err("Password must contain at least one digit".to_string())
            }
        })
        .required()
        .build();
    
    // Test valid password
    let valid_password = Value::String("SecurePass123".to_string());
    let result = password_validator.validate(&valid_password).await;
    match result {
        Ok(()) => println!("  ✅ Valid password: 'SecurePass123'"),
        Err(errors) => {
            println!("  ❌ Password validation failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid password (too common)
    let invalid_password = Value::String("password".to_string());
    let result = password_validator.validate(&invalid_password).await;
    match result {
        Ok(()) => println!("  ✅ Invalid password passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Password validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid password (no uppercase)
    let invalid_password2 = Value::String("securepass123".to_string());
    let result = password_validator.validate(&invalid_password2).await;
    match result {
        Ok(()) => println!("  ✅ Invalid password passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Password validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 5: Complex validation chains
async fn example_complex_validation() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔗 Example 5: Complex Validation Chains");
    
    // Build a complex validator for a user profile
    let profile_validator = string()
        .min_length(2)
        .max_length(50)
        .custom("no_profanity", |name| {
            let profanity = ["badword", "inappropriate"];
            if profanity.iter().any(|&word| name.to_lowercase().contains(word)) {
                Err("Name contains inappropriate content".to_string())
            } else {
                Ok(())
            }
        })
        .build();
    
    // Test valid profile name
    let valid_name = Value::String("John Doe".to_string());
    let result = profile_validator.validate(&valid_name).await;
    match result {
        Ok(()) => println!("  ✅ Valid profile name: 'John Doe'"),
        Err(errors) => {
            println!("  ❌ Profile name validation failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid profile name (too short)
    let invalid_name = Value::String("J".to_string());
    let result = profile_validator.validate(&invalid_name).await;
    match result {
        Ok(()) => println!("  ✅ Invalid name passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Profile name validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test invalid profile name (contains profanity)
    let invalid_name2 = Value::String("John Badword".to_string());
    let result = profile_validator.validate(&invalid_name2).await;
    match result {
        Ok(()) => println!("  ✅ Invalid name passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Profile name validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 6: Builder composition
async fn example_builder_composition() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧩 Example 6: Builder Composition");
    
    // Create multiple validators
    let email_validator = string()
        .email()
        .required()
        .build();
    
    let age_validator = numeric()
        .min(18.0)
        .max(120.0)
        .required()
        .build();
    
    let tags_validator = collection()
        .min_length(1)
        .max_length(5)
        .required()
        .build();
    
    // Compose them into a composite validator
    let composite_validator = CompositeValidator::new(vec![
        email_validator.into_validator(),
        age_validator.into_validator(),
        tags_validator.into_validator(),
    ]);
    
    // Test with valid data
    let valid_data = Value::Object(serde_json::Map::from_iter(vec![
        ("email".to_string(), Value::String("user@example.com".to_string())),
        ("age".to_string(), Value::Number(serde_json::Number::from_f64(25.0).unwrap())),
        ("tags".to_string(), Value::Array(vec![Value::String("rust".to_string())])),
    ]));
    
    let result = composite_validator.validate(&valid_data).await;
    match result {
        Ok(()) => println!("  ✅ Composite validation successful"),
        Err(errors) => {
            println!("  ❌ Composite validation failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test with invalid data
    let invalid_data = Value::Object(serde_json::Map::from_iter(vec![
        ("email".to_string(), Value::String("invalid-email".to_string())),
        ("age".to_string(), Value::Number(serde_json::Number::from_f64(15.0).unwrap())),
        ("tags".to_string(), Value::Array(vec![])),
    ]));
    
    let result = composite_validator.validate(&invalid_data).await;
    match result {
        Ok(()) => println!("  ✅ Invalid data passed (unexpected)"),
        Err(errors) => {
            println!("  ❌ Composite validation correctly failed:");
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 7: Error handling and debugging
async fn example_error_handling() -> Result<(), Box<dyn std::error::Error>> {
    println!("🐛 Example 7: Error Handling and Debugging");
    
    // Build a validator that will fail
    let strict_validator = string()
        .min_length(10)
        .max_length(15)
        .pattern(r"^[A-Z]+$")
        .custom("must_be_palindrome", |s| {
            if s.chars().rev().collect::<String>() == s {
                Ok(())
            } else {
                Err("String must be a palindrome".to_string())
            }
        })
        .build();
    
    // Test with a string that will fail multiple validations
    let test_string = Value::String("hello".to_string());
    let result = strict_validator.validate(&test_string).await;
    
    match result {
        Ok(()) => println!("  ✅ String passed validation (unexpected)"),
        Err(errors) => {
            println!("  ❌ String validation failed with {} errors:", errors.len());
            for (i, error) in errors.iter().enumerate() {
                println!("    Error {}: {}", i + 1, error.message);
                if let Some(path) = &error.path {
                    println!("      Path: {}", path);
                }
                println!("      Code: {:?}", error.code);
                if let Some(suggestion) = &error.suggestion {
                    println!("      Suggestion: {}", suggestion);
                }
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 8: Performance testing
async fn example_performance_testing() -> Result<(), Box<dyn std::error::Error>> {
    println!("⚡ Example 8: Performance Testing");
    
    // Build a complex validator
    let complex_validator = string()
        .min_length(5)
        .max_length(100)
        .pattern(r"^[a-zA-Z0-9\s]+$")
        .custom("no_repeated_chars", |s| {
            let mut chars = s.chars().collect::<Vec<_>>();
            chars.sort();
            chars.dedup();
            if chars.len() == s.len() {
                Ok(())
            } else {
                Err("String contains repeated characters".to_string())
            }
        })
        .custom("balanced_parentheses", |s| {
            let mut stack = Vec::new();
            for c in s.chars() {
                match c {
                    '(' | '[' | '{' => stack.push(c),
                    ')' => {
                        if stack.pop() != Some('(') {
                            return Err("Unmatched closing parenthesis".to_string());
                        }
                    }
                    ']' => {
                        if stack.pop() != Some('[') {
                            return Err("Unmatched closing bracket".to_string());
                        }
                    }
                    '}' => {
                        if stack.pop() != Some('{') {
                            return Err("Unmatched closing brace".to_string());
                        }
                    }
                    _ => {}
                }
            }
            if stack.is_empty() {
                Ok(())
            } else {
                Err("Unmatched opening brackets".to_string())
            }
        })
        .build();
    
    // Test with a valid string
    let valid_string = Value::String("Hello World (123)".to_string());
    
    let start = std::time::Instant::now();
    let result = complex_validator.validate(&valid_string).await;
    let duration = start.elapsed();
    
    match result {
        Ok(()) => println!("  ✅ Complex validation successful in {:?}", duration),
        Err(errors) => {
            println!("  ❌ Complex validation failed in {:?}:", duration);
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    // Test with an invalid string
    let invalid_string = Value::String("Hello (World".to_string());
    
    let start = std::time::Instant::now();
    let result = complex_validator.validate(&invalid_string).await;
    let duration = start.elapsed();
    
    match result {
        Ok(()) => println!("  ✅ Invalid string passed (unexpected) in {:?}", duration),
        Err(errors) => {
            println!("  ❌ Complex validation correctly failed in {:?}:", duration);
            for error in errors {
                println!("    - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}
