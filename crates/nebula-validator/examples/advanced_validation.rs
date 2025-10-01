//! Advanced validation examples with logical combinators and builder patterns
//!
//! Run with: cargo run --example advanced_validation -p nebula-validator

use nebula_validator::*;
use nebula_value::Value;
use serde_json::json;

// Helper to convert JSON to Value
fn json_to_value(json: serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::integer(i)
            } else if let Some(f) = n.as_f64() {
                Value::float(f)
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::text(s),
        serde_json::Value::Array(arr) => {
            Value::Array(nebula_value::Array::from(arr))
        }
        serde_json::Value::Object(obj) => {
            Value::Object(obj.into_iter().collect())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Nebula Validator - Advanced Examples ===\n");

    // Example 1: Logical combinators (AND, OR, NOT)
    {
        println!("1. Logical Combinators:");

        // AND: must satisfy all conditions
        let and_validator = string()
            .and(min_length(5))
            .and(string_contains("@".to_string()));

        println!("   AND validator (string + min 5 chars + contains @):");
        println!("     'user@example' (11 chars): {}",
            if and_validator.validate(&Value::text("user@example"), None).await.is_ok() {
                "✓"
            } else {
                "✗"
            });
        println!("     'user' (4 chars): {}",
            if and_validator.validate(&Value::text("user"), None).await.is_ok() {
                "✓"
            } else {
                "✗ (too short, no @)"
            });

        // OR: must satisfy at least one condition
        let or_validator = string().or(number());

        println!("   OR validator (string OR number):");
        println!("     'hello': {}",
            if or_validator.validate(&Value::text("hello"), None).await.is_ok() {
                "✓ (string)"
            } else {
                "✗"
            });
        println!("     42: {}",
            if or_validator.validate(&Value::integer(42), None).await.is_ok() {
                "✓ (number)"
            } else {
                "✗"
            });
        println!("     true: {}",
            if or_validator.validate(&Value::boolean(true), None).await.is_ok() {
                "✓"
            } else {
                "✗ (neither string nor number)"
            });

        // NOT: must NOT satisfy condition
        let not_validator = string().not();

        println!("   NOT validator (NOT string):");
        println!("     42: {}",
            if not_validator.validate(&Value::integer(42), None).await.is_ok() {
                "✓ (not a string)"
            } else {
                "✗"
            });
        println!("     'hello': {}",
            if not_validator.validate(&Value::text("hello"), None).await.is_ok() {
                "✓"
            } else {
                "✗ (is a string)"
            });
    }

    println!();

    // Example 2: Builder patterns with bon
    {
        println!("2. Builder Patterns:");

        let string_validator = string_constraints()
            .min_len(8)
            .max_len(20)
            .alphanumeric_only(true)
            .allow_spaces(false)
            .call();

        println!("   String constraints (8-20 chars, alphanumeric, no spaces):");
        println!("     'MyPassword123': {}",
            if string_validator.validate(&Value::text("MyPassword123"), None).await.is_ok() {
                "✓"
            } else {
                "✗"
            });
        println!("     'short': {}",
            if string_validator.validate(&Value::text("short"), None).await.is_ok() {
                "✓"
            } else {
                "✗ (too short)"
            });
        println!("     'My Password 123': {}",
            if string_validator.validate(&Value::text("My Password 123"), None).await.is_ok() {
                "✓"
            } else {
                "✗ (contains spaces)"
            });

        let number_validator = number_constraints()
            .min_val(0.0)
            .max_val(100.0)
            .positive_only(true)
            .call();

        println!("   Number constraints (0-100, positive only):");
        println!("     50: {}",
            if number_validator.validate(&Value::integer(50), None).await.is_ok() {
                "✓"
            } else {
                "✗"
            });
        println!("     -5: {}",
            if number_validator.validate(&Value::integer(-5), None).await.is_ok() {
                "✓"
            } else {
                "✗ (negative)"
            });
    }

    println!();

    // Example 3: Collection validation
    {
        println!("3. Collection Validation:");

        let array_validator = array()
            .and(min_size(1))
            .and(max_size(5));

        let valid_array = json_to_value(json!([1, 2, 3]));
        let empty_array = json_to_value(json!([]));
        let large_array = json_to_value(json!([1, 2, 3, 4, 5, 6]));

        println!("   Array validator (1-5 elements):");
        println!("     [1,2,3]: {}",
            if array_validator.validate(&valid_array, None).await.is_ok() {
                "✓"
            } else {
                "✗"
            });
        println!("     []: {}",
            if array_validator.validate(&empty_array, None).await.is_ok() {
                "✓"
            } else {
                "✗ (empty)"
            });
        println!("     [1,2,3,4,5,6]: {}",
            if array_validator.validate(&large_array, None).await.is_ok() {
                "✓"
            } else {
                "✗ (too many)"
            });

        // Object key validation
        let object_validator = object()
            .and(has_all_keys(vec!["name".to_string(), "age".to_string()]));

        let valid_obj = json_to_value(json!({"name": "Alice", "age": 30}));
        let invalid_obj = json_to_value(json!({"name": "Bob"}));

        println!("   Object validator (must have 'name' and 'age' keys):");
        println!("     {{name: Alice, age: 30}}: {}",
            if object_validator.validate(&valid_obj, None).await.is_ok() {
                "✓"
            } else {
                "✗"
            });
        println!("     {{name: Bob}}: {}",
            if object_validator.validate(&invalid_obj, None).await.is_ok() {
                "✓"
            } else {
                "✗ (missing 'age')"
            });
    }

    println!();

    // Example 4: Real-world password validation
    {
        println!("4. Real-World: Password Validation:");

        // Strong password: 8-20 chars, contains letter and number, not common
        let password_validator = required()
            .and(min_length(8))
            .and(max_length(20))
            .and(not_in_str_values(vec!["password", "12345678", "qwerty123"]));

        let strong = Value::text("SecurePass123");
        let weak = Value::text("password");
        let short = Value::text("Pass1");

        println!("   Password rules: 8-20 chars, not common password");
        println!("     'SecurePass123': {}",
            if password_validator.validate(&strong, None).await.is_ok() {
                "✓ (strong)"
            } else {
                "✗"
            });
        println!("     'password': {}",
            if password_validator.validate(&weak, None).await.is_ok() {
                "✓"
            } else {
                "✗ (too common)"
            });
        println!("     'Pass1': {}",
            if password_validator.validate(&short, None).await.is_ok() {
                "✓"
            } else {
                "✗ (too short)"
            });
    }

    println!();

    // Example 5: Named validators for better error messages
    {
        println!("5. Named Validators:");

        let email_validator = validate(string())
            .and(string_contains("@".to_string()))
            .and(string_contains(".".to_string()))
            .named("email_validator")
            .build();

        println!("   Validator name: '{}'", email_validator.name());

        let valid = Value::text("user@example.com");
        let invalid = Value::text("invalid-email");

        match email_validator.validate(&valid, None).await {
            Ok(_) => println!("   'user@example.com': ✓"),
            Err(e) => println!("   Error: {}", e),
        }

        match email_validator.validate(&invalid, None).await {
            Ok(_) => println!("   'invalid-email': ✓"),
            Err(e) => println!("   'invalid-email': ✗ - {}", e),
        }
    }

    println!("\n=== All examples completed! ===");
    Ok(())
}
