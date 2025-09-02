//! Enhanced validation example using the new nebula-validator features
//! 
//! This example demonstrates the advanced conditional validation, rule composition,
//! and performance features of the enhanced validation system.

use nebula_validator::prelude::*;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Enhanced Validation Example");
    println!("==============================\n");

    // Example 1: Complex conditional validation form
    example_complex_form_validation().await?;
    
    // Example 2: Performance-optimized validation
    example_performance_validation().await?;
    
    // Example 3: Rule composition with dependencies
    example_rule_composition().await?;
    
    // Example 4: Valid/Invalid system with proofs
    example_valid_invalid_system().await?;

    Ok(())
}

/// Example 1: Complex conditional validation form
async fn example_complex_form_validation() -> Result<(), Box<dyn std::error::Error>> {
    println!("📝 Example 1: Complex Form Validation");
    println!("------------------------------------");
    
    // Create a complex registration form validator
    let registration_validator = RuleComposer::new()
        // Basic fields always required
        .rule("username", 
            Required::new()
                .and(StringLength::new(3, 20))
                .and(Pattern::new(r"^[a-zA-Z0-9_]+$"))
        )
        .rule("email",
            Required::new()
                .and(Email::new())
        )
        .rule("password",
            Required::new()
                .and(StringLength::min(8))
                .and(PasswordStrength::medium())
        )
        
        // Conditional validation based on account type
        .rule("account_type_validation",
            WhenChain::new()
                .when(
                    field("account_type").equals(json!("business")),
                    EnhancedAll::new()
                        .add(Required::new().for_field("company_name"))
                        .add(Required::new().for_field("tax_id"))
                        .add(Optional::new(Url::new()).for_field("website"))
                )
                .when(
                    field("account_type").equals(json!("personal")),
                    EnhancedAll::new()
                        .add(Required::new().for_field("first_name"))
                        .add(Required::new().for_field("last_name"))
                        .add(Optional::new(Date::new()).for_field("birth_date"))
                )
                .otherwise(
                    AlwaysInvalid::new("Invalid account type")
                )
        )
        
        // Address validation if country is specified
        .rule("address_validation",
            When::new(
                field("country").exists(),
                AddressValidator::for_country(field("country"))
            )
        )
        
        // XOR: either email verified, or phone specified
        .rule("contact_verification",
            Xor::new()
                .add(field("email_verified").equals(json!(true)))
                .add(field("phone").exists().and(PhoneNumber::valid()))
                .expect(XorExpectation::ExactlyOne)
        )
        
        // Dependent rules
        .dependent_rule("premium_features",
            When::new(
                field("subscription").equals(json!("premium")),
                EnhancedAll::new()
                    .add(Required::new().for_field("payment_method"))
                    .add(Required::new().for_field("billing_address"))
            ),
            vec!["account_type_validation".to_string()]
        );

    // Test data
    let form_data = json!({
        "username": "john_doe",
        "email": "john@example.com",
        "password": "SecurePass123!",
        "account_type": "business",
        "company_name": "Acme Corp",
        "tax_id": "12-3456789",
        "country": "US",
        "subscription": "premium",
        "payment_method": "card",
        "billing_address": {
            "street": "123 Main St",
            "city": "New York",
            "state": "NY",
            "zip": "10001"
        }
    });

    println!("Validating business account form...");
    let result = registration_validator.validate(&form_data).await?;
    
    match result.into_validated() {
        Validated::Valid(valid) => {
            println!("✅ Form validation successful!");
            println!("   Proof: {:?}", valid.proof());
        },
        Validated::Invalid(invalid) => {
            println!("❌ Form validation failed:");
            for error in invalid.errors() {
                println!("   - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 2: Performance-optimized validation
async fn example_performance_validation() -> Result<(), Box<dyn std::error::Error>> {
    println!("⚡ Example 2: Performance-Optimized Validation");
    println!("---------------------------------------------");
    
    // Create a performance-optimized validator
    let performance_validator = EnhancedAll::new()
        .parallel()
        .max_concurrency(4)
        .fail_fast()
        .add(Memoized::new(Email::new(), Duration::from_secs(300)))
        .add(Throttled::new(Url::new(), 100))
        .add(Lazy::new(|| Box::new(PhoneNumber::new())))
        .add(Deferred::new());

    // Set the deferred validator
    let deferred_validator = performance_validator.clone();
    tokio::spawn(async move {
        // Simulate loading validator from external source
        tokio::time::sleep(Duration::from_millis(100)).await;
        deferred_validator.set(AlwaysValid::new()).await;
    });

    let test_data = json!({
        "email": "test@example.com",
        "website": "https://example.com",
        "phone": "+1234567890"
    });

    println!("Running performance-optimized validation...");
    let start = std::time::Instant::now();
    let result = performance_validator.validate(&test_data).await?;
    let duration = start.elapsed();
    
    match result.into_validated() {
        Validated::Valid(_) => println!("✅ Performance validation successful in {:?}", duration),
        Validated::Invalid(invalid) => {
            println!("❌ Performance validation failed:");
            for error in invalid.errors() {
                println!("   - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 3: Rule composition with dependencies
async fn example_rule_composition() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔗 Example 3: Rule Composition with Dependencies");
    println!("------------------------------------------------");
    
    // Create a rule composer with dependencies
    let composer = RuleComposer::new()
        .rule("basic_info", 
            EnhancedAll::new()
                .add(Required::new().for_field("name"))
                .add(Required::new().for_field("age"))
        )
        .rule("age_verification",
            Predicate::new(
                "adult_check",
                |v| v["age"].as_u64().map_or(false, |a| a >= 18),
                "Must be 18 or older"
            )
        )
        .dependent_rule("premium_access",
            Predicate::new(
                "premium_check",
                |v| v["subscription"].as_str().map_or(false, |s| s == "premium"),
                "Premium subscription required"
            ),
            vec!["age_verification".to_string()]
        )
        .dependent_rule("payment_required",
            Required::new().for_field("payment_method"),
            vec!["premium_access".to_string()]
        );

    // Test data
    let test_data = json!({
        "name": "John Doe",
        "age": 25,
        "subscription": "premium",
        "payment_method": "credit_card"
    });

    println!("Executing rule composition with dependencies...");
    let result = composer.validate(&test_data).await?;
    
    match result.into_validated() {
        Validated::Valid(_) => println!("✅ Rule composition successful!"),
        Validated::Invalid(invalid) => {
            println!("❌ Rule composition failed:");
            for error in invalid.errors() {
                println!("   - {}", error.message);
            }
        }
    }
    
    println!();
    Ok(())
}

/// Example 4: Valid/Invalid system with proofs
async fn example_valid_invalid_system() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Example 4: Valid/Invalid System with Proofs");
    println!("------------------------------------------------");
    
    // Create a validator that returns Valid/Invalid
    let validator = Predicate::new(
        "positive_number",
        |v| v.as_f64().map_or(false, |n| n > 0.0),
        "Number must be positive"
    );

    // Test with positive number
    let positive_data = json!(42.0);
    println!("Testing positive number validation...");
    let result = validator.validate(&positive_data).await?;
    
    match result.into_validated() {
        Validated::Valid(valid) => {
            println!("✅ Value is valid: {:?}", valid.value());
            println!("   Proof: {:?}", valid.proof());
            println!("   Expired: {}", valid.is_expired());
        },
        Validated::Invalid(invalid) => {
            println!("❌ Value is invalid:");
            for error in invalid.errors() {
                println!("   - {}", error.message);
            }
        }
    }

    // Test with negative number
    let negative_data = json!(-5.0);
    println!("\nTesting negative number validation...");
    let result = validator.validate(&negative_data).await?;
    
    match result.into_validated() {
        Validated::Valid(_) => println!("✅ Unexpected success"),
        Validated::Invalid(invalid) => {
            println!("❌ Value is invalid as expected:");
            for error in invalid.errors() {
                println!("   - {}", error.message);
            }
            
            // Try to fix the error
            println!("\nAttempting to fix the error...");
            let fixed = invalid.try_fix(|_value, _errors| async {
                // Logic to fix the error
                Ok(json!(0.0))
            }).await;
            
            match fixed {
                Validated::Valid(valid) => {
                    println!("✅ Error fixed! New value: {:?}", valid.value());
                    println!("   Proof type: {:?}", valid.proof().proof_type);
                },
                Validated::Invalid(invalid) => {
                    println!("❌ Could not fix error:");
                    for error in invalid.errors() {
                        println!("   - {}", error.message);
                    }
                }
            }
        }
    }
    
    println!();
    Ok(())
}

// Mock validators for the example
mod mock_validators {
    use super::*;
    
    pub struct Email;
    impl Email {
        pub fn new() -> Self { Self }
    }
    
    pub struct PasswordStrength;
    impl PasswordStrength {
        pub fn medium() -> Self { Self }
    }
    
    pub struct Url;
    impl Url {
        pub fn new() -> Self { Self }
    }
    
    pub struct Date;
    impl Date {
        pub fn new() -> Self { Self }
    }
    
    pub struct AddressValidator;
    impl AddressValidator {
        pub fn for_country(_country: FieldCondition) -> Self { Self }
    }
    
    pub struct PhoneNumber;
    impl PhoneNumber {
        pub fn valid() -> Self { Self }
        pub fn new() -> Self { Self }
    }
    
    pub struct StringLength;
    impl StringLength {
        pub fn new(_min: usize, _max: usize) -> Self { Self }
        pub fn min(_min: usize) -> Self { Self }
    }
    
    pub struct Pattern;
    impl Pattern {
        pub fn new(_pattern: &str) -> Self { Self }
    }
    
    pub struct ValidationBuilder;
    
    impl ValidationBuilder {
        pub fn for_field(_field: &str) -> Self { Self }
    }
    
    // Implement Validatable for mock validators
    #[async_trait]
    impl Validatable for Email {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("email", "Email validator", ValidatorCategory::Format)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
    
    #[async_trait]
    impl Validatable for PasswordStrength {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("password_strength", "Password strength validator", ValidatorCategory::Format)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
    
    #[async_trait]
    impl Validatable for Url {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("url", "URL validator", ValidatorCategory::Format)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
    
    #[async_trait]
    impl Validatable for Date {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("date", "Date validator", ValidatorCategory::Format)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
    
    #[async_trait]
    impl Validatable for AddressValidator {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("address", "Address validator", ValidatorCategory::Format)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
    
    #[async_trait]
    impl Validatable for PhoneNumber {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("phone", "Phone number validator", ValidatorCategory::Format)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
    
    #[async_trait]
    impl Validatable for StringLength {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("string_length", "String length validator", ValidatorCategory::Format)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
    
    #[async_trait]
    impl Validatable for Pattern {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("pattern", "Pattern validator", ValidatorCategory::Format)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
    
    #[async_trait]
    impl Validatable for ValidationBuilder {
        async fn validate(&self, _value: &Value) -> ValidationResult<()> {
            ValidationResult::success(())
        }
        
        fn metadata(&self) -> ValidatorMetadata {
            ValidatorMetadata::new("validation_builder", "Validation builder", ValidatorCategory::Basic)
        }
        
        fn complexity(&self) -> ValidationComplexity {
            ValidationComplexity::Simple
        }
    }
}

use mock_validators::*;
