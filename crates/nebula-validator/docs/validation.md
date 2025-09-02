# Advanced Validation Patterns

> Master validation with combinators, custom validators, caching, and real-world patterns

## Quick Reference

```rust
use nebula_value::prelude::*;
use nebula_value::validation::*;

// Basic validation
text.validate( & MinLength::new(5)) ?;
age.validate( & InRange::new(18i64, 65i64)) ?;

// Combinators
password.validate( & MinLength::new(8).and(ContainsUppercase).and(ContainsDigit)) ?;

// Custom validators
text.validate( & | t: & Text| {
if t.as_ref().contains("spam") {
Err(ValidationError::new("spam_detected", "Spam not allowed"))
} else { Ok(()) }
}) ?;
```

## Core Validation Framework

### Validator Trait

All validators implement the `Validator<T>` trait:

```rust
pub trait Validator<T> {
    fn validate(&self, value: &T) -> Result<(), ValidationError>;

    // Combinators
    fn and<V: Validator<T>>(self, other: V) -> And<Self, V> { /* ... */ }
    fn or<V: Validator<T>>(self, other: V) -> Or<Self, V> { /* ... */ }
    fn not(self) -> Not<Self> { /* ... */ }
}
```

### ValidationError Structure

```rust
pub struct ValidationError {
    pub path: String,                           // "users[0].email"
    pub code: &'static str,                     // "min_length"
    pub message: String,                        // "Must be at least 8 characters"
    pub params: HashMap<&'static str, String>, // {"min": "8", "actual": "5"}
}

// Helper methods
error.get_param("min");                         // Some("8")
error.get_param_as::<usize>("actual");          // Some(5)
```

## Built-in Validators

### Length Validators

```rust
use nebula_value::validation::*;

let password = Text::new("secret");

// Basic length constraints
password.validate( & MinLength::new(8)) ?;         // At least 8 chars
password.validate( & MaxLength::new(128)) ?;       // At most 128 chars
password.validate( & ExactLength::new(10)) ?;      // Exactly 10 chars
password.validate( & LengthBetween::new(8, 128)) ?; // Between 8 and 128

// Collection length
let tags = Array::from(vec![Text::new("rust").into_value()]);
tags.validate( & MinItems::new(1)) ?;              // At least 1 item
tags.validate( & MaxItems::new(10)) ?;             // At most 10 items
tags.validate( & ItemsBetween::new(1, 10)) ?;      // Between 1 and 10 items
```

### Numeric Validators

```rust
let age = Integer::new(25);
let score = Float::new(87.5);

// Range validation
age.validate( & InRange::new(0i64, 120i64)) ?;
score.validate( & InRange::new(0.0, 100.0)) ?;

// Sign validation
age.validate( & Positive) ?;                       // > 0
age.validate( & NonNegative) ?;                    // >= 0
age.validate( & Negative) ?;                       // < 0
age.validate( & NonPositive) ?;                    // <= 0
age.validate( & NonZero) ?;                        // != 0

// Parity validation
let even_num = Integer::new(42);
even_num.validate( & Even) ?;                      // n % 2 == 0

let odd_num = Integer::new(43);
odd_num.validate( & Odd) ?;                        // n % 2 != 0

// Divisibility
let multiple = Integer::new(15);
multiple.validate( & DivisibleBy::new(5)) ?;       // n % 5 == 0

// Float-specific
score.validate( & Finite) ?;                       // Not NaN or infinite
score.validate( & NotNaN) ?;                       // Not NaN
score.validate( & NotInfinite) ?;                  // Not infinite
```

### Pattern Validators

```rust
use nebula_value::validation::*;

let email = Text::new("user@example.com");
let phone = Text::new("+1-555-123-4567");
let url = Text::new("https://example.com");

// Built-in pattern validators
email.validate( & Email) ?;                        // Email format
phone.validate( & Phone) ?;                        // Phone number format
url.validate( & Url) ?;                            // URL format

// String content validators
let username = Text::new("user123");
username.validate( & AlphanumericOnly) ?;          // Only letters and digits
username.validate( & NoWhitespace) ?;              // No spaces or tabs
username.validate( & StartsWith::new("user")) ?;   // Starts with prefix
username.validate( & EndsWith::new("123")) ?;      // Ends with suffix
username.validate( & Contains::new("er1")) ?;      // Contains substring

// Custom regex
let custom_pattern = RegexValidator::new(r"^[A-Z]{2}\d{4}$") ?;
let code = Text::new("AB1234");
code.validate( & custom_pattern) ?;

// IP addresses
let ip = Text::new("192.168.1.1");
ip.validate( & IpAddress) ?;                       // IPv4 or IPv6
ip.validate( & IpV4Address) ?;                     // IPv4 only
ip.validate( & IpV6Address) ?;                     // IPv6 only
```

### Collection Validators

```rust
let numbers = Array::from(vec![
    Integer::new(1).into_value(),
    Integer::new(2).into_value(),
    Integer::new(3).into_value(),
]);

let tags = Array::from(vec![
    Text::new("rust").into_value(),
    Text::new("programming").into_value(),
    Text::new("rust").into_value(),  // Duplicate
]);

// Size constraints
numbers.validate( & MinItems::new(1)) ?;
numbers.validate( & MaxItems::new(10)) ?;
numbers.validate( & ItemsBetween::new(1, 10)) ?;

// Content validation
numbers.validate( & UniqueItems) ?;                // All items unique (fails for tags)
numbers.validate( & Sorted) ?;                     // Items in sorted order
numbers.validate( & NoEmptyItems) ?;               // No empty strings/arrays

// Map validation
let user_data = Map::new();
user_data.validate( & RequiredKeys::new(vec!["name", "email"])) ?;
user_data.validate( & MinSize::new(2)) ?;          // At least 2 keys
user_data.validate( & MaxSize::new(20)) ?;         // At most 20 keys
```

## Validator Combinators

### Logical Operations

```rust
use nebula_value::validation::*;

let password = Text::new("MySecurePass123!");

// AND - all must pass
let strong_password = MinLength::new(8)
.and(MaxLength::new(128))
.and(ContainsUppercase)
.and(ContainsLowercase)
.and(ContainsDigit)
.and(ContainsSpecialChar);

password.validate( & strong_password) ?;

// OR - at least one must pass
let identifier = Text::new("user@example.com");
let id_validator = Email.or(Phone).or(AlphanumericOnly);
identifier.validate( & id_validator) ?;

// NOT - must fail
let non_empty = Text::new("hello");
let not_empty_validator = ExactLength::new(0).not();
non_empty.validate( & not_empty_validator) ?;

// Complex combinations
let complex = MinLength::new(5)
.and(MaxLength::new(50))
.and(
ContainsDigit
.or(ContainsSpecialChar)
.or(ContainsUppercase)
);
```

### Conditional Validation

```rust
// IF-THEN validation
let user_type = Text::new("admin");
let permissions = Array::new();

let conditional_validator = IfThen::new(
| user_type: & Text| user_type.as_ref() == "admin",
MinItems::new(1)  // Admin must have at least 1 permission
);

// Usage with multiple fields requires custom validator
struct UserValidator;
impl Validator<User> for UserValidator {
    fn validate(&self, user: &User) -> Result<(), ValidationError> {
        if user.user_type.as_ref() == "admin" {
            user.permissions.validate(&MinItems::new(1))
                .map_err(|e| e.with_path("permissions"))?;
        }
        Ok(())
    }
}
```

### Quantifier Combinators

```rust
// ALL OF - every validator must pass
let all_checks = AllOf::new(vec![
    Box::new(MinLength::new(8)) as Box<dyn Validator<Text>>,
    Box::new(MaxLength::new(128)),
    Box::new(ContainsDigit),
    Box::new(ContainsUppercase),
]);

password.validate( & all_checks) ?;

// ANY OF - at least one validator must pass  
let any_format = AnyOf::new(vec![
    Box::new(Email) as Box<dyn Validator<Text>>,
    Box::new(Phone),
    Box::new(Url),
]);

contact_info.validate( & any_format) ?;

// NONE OF - no validator should pass (blacklist)
let forbidden_patterns = NoneOf::new(vec![
    Box::new(Contains::new("password")) as Box<dyn Validator<Text>>,
    Box::new(Contains::new("123456")),
    Box::new(Contains::new("admin")),
]);

username.validate( & forbidden_patterns) ?;
```

## Custom Validators

### Function Validators

```rust
use nebula_value::validation::*;

// Simple closure validator
fn no_profanity(text: &Text) -> Result<(), ValidationError> {
    let profanity_list = ["spam", "scam", "fake"];
    let content = text.as_ref().to_lowercase();

    for word in &profanity_list {
        if content.contains(word) {
            return Err(ValidationError::new(
                "profanity_detected",
                format!("Content contains inappropriate word: {}", word)
            ).with_param("detected_word", word.to_string()));
        }
    }
    Ok(())
}

let message = Text::new("This is not spam");
message.validate( & no_profanity) ?;

// Parameterized function validator
fn min_words(min: usize) -> impl Validator<Text> {
    move |text: &Text| {
        let word_count = text.as_ref().split_whitespace().count();
        if word_count < min {
            Err(ValidationError::new(
                "min_words",
                format!("Must contain at least {} words, found {}", min, word_count)
            )
                .with_param("min", min.to_string())
                .with_param("actual", word_count.to_string()))
        } else {
            Ok(())
        }
    }
}

let description = Text::new("Short text");
description.validate( & min_words(5)) ?;
```

### Struct Validators

```rust
use nebula_value::validation::*;
use std::collections::HashSet;

// Stateful validator with configuration
struct UniqueInListValidator {
    existing_values: HashSet<String>,
    case_sensitive: bool,
}

impl UniqueInListValidator {
    pub fn new(existing: Vec<String>, case_sensitive: bool) -> Self {
        let existing_values = if case_sensitive {
            existing.into_iter().collect()
        } else {
            existing.into_iter().map(|s| s.to_lowercase()).collect()
        };

        Self { existing_values, case_sensitive }
    }
}

impl Validator<Text> for UniqueInListValidator {
    fn validate(&self, value: &Text) -> Result<(), ValidationError> {
        let check_value = if self.case_sensitive {
            value.as_ref().to_string()
        } else {
            value.as_ref().to_lowercase()
        };

        if self.existing_values.contains(&check_value) {
            Err(ValidationError::new(
                "duplicate_value",
                "This value already exists"
            ).with_param("value", value.as_ref().to_string()))
        } else {
            Ok(())
        }
    }
}

// Usage
let existing_usernames = vec!["alice".to_string(), "bob".to_string()];
let username_validator = UniqueInListValidator::new(existing_usernames, false);

let new_username = Text::new("Charlie");
new_username.validate( & username_validator) ?;  // OK

let duplicate_username = Text::new("ALICE");
duplicate_username.validate( & username_validator) ?;  // Error: case-insensitive match
```

### Cross-Field Validation

```rust
use nebula_value::prelude::*;
use nebula_value::validation::*;

struct User {
    email: Text,
    password: Text,
    confirm_password: Text,
    age: Integer,
    terms_accepted: Bool,
}

struct UserValidator;

impl Validator<User> for UserValidator {
    fn validate(&self, user: &User) -> Result<(), ValidationError> {
        let mut errors = ValidationErrors::new();

        // Individual field validation
        if let Err(e) = user.email.validate(&Email) {
            errors.add(e.with_path("email"));
        }

        if let Err(e) = user.password.validate(&MinLength::new(8).and(ContainsDigit)) {
            errors.add(e.with_path("password"));
        }

        if let Err(e) = user.age.validate(&InRange::new(13i64, 120i64)) {
            errors.add(e.with_path("age"));
        }

        if let Err(e) = user.terms_accepted.validate(&MustBeTrue) {
            errors.add(e.with_path("terms_accepted"));
        }

        // Cross-field validation
        if user.password.as_ref() != user.confirm_password.as_ref() {
            errors.add(ValidationError::new(
                "password_mismatch",
                "Password and confirmation do not match"
            ).with_path("confirm_password"));
        }

        // Password cannot contain email username
        if let Some(username) = user.email.as_ref().split('@').next() {
            if user.password.as_ref().to_lowercase().contains(&username.to_lowercase()) {
                errors.add(ValidationError::new(
                    "password_contains_email",
                    "Password cannot contain your email username"
                ).with_path("password"));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.into()) // Convert to single ValidationError
        }
    }
}

// Usage
let user = User {
email: Text::new("alice@example.com"),
password: Text::new("alice123"),          // Contains email username!
confirm_password: Text::new("alice123"),
age: Integer::new(25),
terms_accepted: Bool::new(true),
};

match user.validate( & UserValidator) {
Ok(()) => println ! ("User is valid"),
Err(error) => {
println ! ("Validation failed:");
for err in &error.errors {
println ! ("  {}: {}", err.path, err.message);
}
}
}
```

## Performance Optimization

### Cached Regex Validators

```rust
use nebula_value::validation::*;
use once_cell::sync::Lazy;

// Global cached regex for repeated use
static EMAIL_VALIDATOR: Lazy<CachedRegex> = Lazy::new(|| {
    CachedRegex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
        .expect("Valid email regex")
});

// Reuse across many validations (compiles once)
for email_str in user_emails {
let email = Text::new(email_str);
email.validate( & * EMAIL_VALIDATOR) ?;  // Zero allocation after first compile
}

// Custom cached validator
static PHONE_VALIDATOR: Lazy<CachedRegex> = Lazy::new(|| {
    CachedRegex::new(r"^\+?1?-?\.?\s?\(?(\d{3})\)?[-\.\s]?(\d{3})[-\.\s]?(\d{4})$")
        .expect("Valid phone regex")
});
```

### Validator Composition and Reuse

```rust
// Create reusable validator instances
struct ValidationRules {
    strong_password: Box<dyn Validator<Text>>,
    username: Box<dyn Validator<Text>>,
    email: Box<dyn Validator<Text>>,
}

impl ValidationRules {
    fn new() -> Self {
        Self {
            strong_password: Box::new(
                MinLength::new(8)
                    .and(MaxLength::new(128))
                    .and(ContainsUppercase)
                    .and(ContainsLowercase)
                    .and(ContainsDigit)
                    .and(ContainsSpecialChar)
            ),
            username: Box::new(
                MinLength::new(3)
                    .and(MaxLength::new(20))
                    .and(AlphanumericOnly)
            ),
            email: Box::new(Email),
        }
    }
}

// Reuse across application
let rules = ValidationRules::new();

for user_input in user_inputs {
user_input.username.validate( & * rules.username) ?;
user_input.email.validate( & * rules.email) ?;
user_input.password.validate( & * rules.strong_password) ?;
}
```

### Compile-time Validation

```rust
// For known constants, validate at compile time
macro_rules! validated_email {
    ($email:literal) => {{
        const _: () = {
            // Basic compile-time checks
            assert!(!$email.is_empty());
            assert!($email.contains('@'));
            assert!($email.contains('.'));
        };
        Text::new_unchecked($email)  // Skip runtime validation
    }};
}

// Zero runtime cost for constants
let admin_email = validated_email!("admin@example.com");
let support_email = validated_email!("support@example.com");
```

## Validation Patterns

### Layered Validation

```rust
use nebula_value::prelude::*;

// Layer 1: Basic format validation
fn validate_format(input: &str) -> Result<Text, ValidationError> {
    let text = Text::new(input);
    text.validate(&MinLength::new(1).and(MaxLength::new(100)))?;
    Ok(text)
}

// Layer 2: Domain-specific validation
fn validate_username(text: Text) -> Result<Text, ValidationError> {
    text.validate(&AlphanumericOnly.and(MinLength::new(3)))?;
    Ok(text)
}

// Layer 3: Business rule validation  
fn validate_unique_username(text: Text, existing: &HashSet<String>) -> Result<Text, ValidationError> {
    if existing.contains(text.as_ref()) {
        return Err(ValidationError::new(
            "username_taken",
            "This username is already taken"
        ));
    }
    Ok(text)
}

// Apply layers
fn process_username_input(input: &str, existing: &HashSet<String>) -> Result<Text, ValidationError> {
    let formatted = validate_format(input)?;
    let domain_valid = validate_username(formatted)?;
    let unique = validate_unique_username(domain_valid, existing)?;
    Ok(unique)
}
```

### Validation with Normalization

```rust
// Normalize then validate
fn validate_and_normalize_email(input: &str) -> Result<Text, ValidationError> {
    // Normalize: trim and lowercase
    let normalized = input.trim().to_lowercase();
    let email = Text::new(normalized);

    // Validate normalized version
    email.validate(&Email)?;

    Ok(email)
}

// Validate with transformations
fn validate_phone_number(input: &str) -> Result<Text, ValidationError> {
    // Remove common formatting characters
    let digits_only: String = input.chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();

    let phone = Text::new(digits_only);
    phone.validate(&Phone)?;

    Ok(phone)
}
```

### Batch Validation

```rust
use nebula_value::prelude::*;

struct BatchValidator<T> {
    validators: Vec<Box<dyn Validator<T>>>,
}

impl<T> BatchValidator<T> {
    fn new() -> Self {
        Self { validators: Vec::new() }
    }

    fn add_validator(mut self, validator: Box<dyn Validator<T>>) -> Self {
        self.validators.push(validator);
        self
    }

    fn validate_all(&self, values: &[T]) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();

        for (index, value) in values.iter().enumerate() {
            for validator in &self.validators {
                if let Err(mut error) = validator.validate(value) {
                    error.prepend_index(index);
                    errors.add(error);
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

// Usage
let emails = vec![
    Text::new("valid@example.com"),
    Text::new("invalid-email"),
    Text::new("another@test.org"),
];

let batch_validator = BatchValidator::new()
.add_validator(Box::new(Email))
.add_validator(Box::new(MinLength::new(5)));

match batch_validator.validate_all( & emails) {
Ok(()) => println ! ("All emails valid"),
Err(errors) => {
for error in & errors.errors {
println ! ("emails[{}]: {}", error.path, error.message);
}
}
}
```

## Integration with Web Frameworks

See [Integration Guide](integration/) for framework-specific patterns:

- [Axum Integration](integration/axum.md)
- [Actix Web Integration](integration/actix.md)
- [Warp Integration](integration/warp.md)

## Testing Validation

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_value::prelude::*;

    #[test]
    fn test_password_validation() {
        let strong_password = MinLength::new(8)
            .and(ContainsUppercase)
            .and(ContainsDigit);

        // Valid password
        assert!(Text::new("StrongPass123").validate(&strong_password).is_ok());

        // Too short
        let result = Text::new("Weak1").validate(&strong_password);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, "min_length");

        // No uppercase
        let result = Text::new("weakpass123").validate(&strong_password);
        assert!(result.is_err());

        // No digit
        let result = Text::new("WeakPassword").validate(&strong_password);
        assert!(result.is_err());
    }

    #[test]
    fn test_custom_validator() {
        fn no_spaces(text: &Text) -> Result<(), ValidationError> {
            if text.as_ref().contains(' ') {
                Err(ValidationError::new("no_spaces", "Spaces not allowed"))
            } else {
                Ok(())
            }
        }

        assert!(Text::new("nospaces").validate(&no_spaces).is_ok());
        assert!(Text::new("has spaces").validate(&no_spaces).is_err());
    }
}
```

## Next Steps

- [Async Validation](validation-async.md) - Database uniqueness checks and external API validation
- [Error Handling](error-handling.md) - Advanced error handling patterns
- [Custom Types](custom-types.md) - Building domain-specific validated types
- [Performance Guide](performance.md) - Optimization techniques for high-throughput validation