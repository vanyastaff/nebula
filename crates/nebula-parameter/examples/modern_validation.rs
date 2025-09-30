//! Example demonstrating the modern validation system
//!
//! This example shows how to use the new modern validation system
//! that replaces the legacy condition-based validation.

use nebula_parameter::{ModernParameterValidation, HasModernValidation, RoutingParameter};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Modern Validation System Demo");
    println!("================================");

    // Example 1: Email validation
    println!("\n📧 Email Validation Example:");
    let email_validation = ModernParameterValidation::builder()
        .email()
        .message("Please provide a valid email address")
        .build();

    // Test valid email
    let valid_email = json!("user@example.com");
    match email_validation.validate(&valid_email).await {
        Ok(()) => println!("✅ Valid email: {}", valid_email),
        Err(e) => println!("❌ Invalid email: {}", e),
    }

    // Test invalid email
    let invalid_email = json!("not-an-email");
    match email_validation.validate(&invalid_email).await {
        Ok(()) => println!("✅ Valid email: {}", invalid_email),
        Err(e) => println!("❌ Invalid email: {}", e),
    }

    // Example 2: String length validation
    println!("\n📏 String Length Validation Example:");
    let name_validation = ModernParameterValidation::builder()
        .string_length(Some(2), Some(50))
        .message("Name must be between 2 and 50 characters")
        .build();

    // Test valid name
    let valid_name = json!("Alice");
    match name_validation.validate(&valid_name).await {
        Ok(()) => println!("✅ Valid name: {}", valid_name),
        Err(e) => println!("❌ Invalid name: {}", e),
    }

    // Test invalid name (too short)
    let invalid_name = json!("A");
    match name_validation.validate(&invalid_name).await {
        Ok(()) => println!("✅ Valid name: {}", invalid_name),
        Err(e) => println!("❌ Invalid name: {}", e),
    }

    // Example 3: Multiple validators (email + required)
    println!("\n🔗 Multiple Validators Example:");
    let combined_validation = ModernParameterValidation::builder()
        .required()
        .email()
        .message("A valid email address is required")
        .build();

    // Test empty value
    let empty_value = json!(null);
    match combined_validation.validate(&empty_value).await {
        Ok(()) => println!("✅ Valid value: {}", empty_value),
        Err(e) => println!("❌ Invalid value: {}", e),
    }

    // Test valid email
    let valid_email = json!("admin@company.com");
    match combined_validation.validate(&valid_email).await {
        Ok(()) => println!("✅ Valid value: {}", valid_email),
        Err(e) => println!("❌ Invalid value: {}", e),
    }

    // Example 4: Numeric range validation
    println!("\n🔢 Numeric Range Validation Example:");
    let age_validation = ModernParameterValidation::builder()
        .numeric_range(Some(18.0), Some(120.0))
        .message("Age must be between 18 and 120")
        .build();

    // Test valid age
    let valid_age = json!(25);
    match age_validation.validate(&valid_age).await {
        Ok(()) => println!("✅ Valid age: {}", valid_age),
        Err(e) => println!("❌ Invalid age: {}", e),
    }

    // Test invalid age (too young)
    let invalid_age = json!(16);
    match age_validation.validate(&invalid_age).await {
        Ok(()) => println!("✅ Valid age: {}", invalid_age),
        Err(e) => println!("❌ Invalid age: {}", e),
    }

    // Example 5: Using modern validation with routing parameter
    println!("\n🛤️  Routing Parameter with Modern Validation:");
    let mut routing_param = RoutingParameter::new(
        "api_endpoint",
        "API Endpoint",
        "Routing endpoint for API calls",
        None
    )?;

    // Add connection validation
    let connection_validation = ModernParameterValidation::builder()
        .url(true) // require HTTPS
        .required()
        .message("A valid HTTPS URL is required for API endpoint")
        .build();

    routing_param.set_modern_validation(Some(connection_validation));

    // Test validation through the trait
    let valid_url = json!("https://api.example.com/v1");
    match routing_param.validate_modern(&valid_url).await {
        Ok(()) => println!("✅ Valid API endpoint: {}", valid_url),
        Err(e) => println!("❌ Invalid API endpoint: {}", e),
    }

    let invalid_url = json!("http://insecure.com"); // HTTP instead of HTTPS
    match routing_param.validate_modern(&invalid_url).await {
        Ok(()) => println!("✅ Valid API endpoint: {}", invalid_url),
        Err(e) => println!("❌ Invalid API endpoint: {}", e),
    }

    // Example 6: Using utility functions
    println!("\n🛠️  Utility Functions Example:");

    // Password validation
    let password_validation = nebula_parameter::utils::password(8);
    let weak_password = json!("123");
    match password_validation.validate(&weak_password).await {
        Ok(()) => println!("✅ Strong password"),
        Err(e) => println!("❌ Weak password: {}", e),
    }

    let strong_password = json!("MyP@ssw0rd123");
    match password_validation.validate(&strong_password).await {
        Ok(()) => println!("✅ Strong password: {}", strong_password),
        Err(e) => println!("❌ Weak password: {}", e),
    }

    // Phone number validation
    let phone_validation = nebula_parameter::utils::phone();
    let valid_phone = json!("+1-555-123-4567");
    match phone_validation.validate(&valid_phone).await {
        Ok(()) => println!("✅ Valid phone: {}", valid_phone),
        Err(e) => println!("❌ Invalid phone: {}", e),
    }

    println!("\n✨ Modern validation system demonstration completed!");
    println!("This system provides clean, composable validation that replaces legacy validation.");

    Ok(())
}