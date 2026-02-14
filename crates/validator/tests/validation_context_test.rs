use nebula_validator::core::{
    ContextualValidator, ValidationContext, ValidationContextBuilder, ValidationError,
};

#[test]
fn test_context_insert_get() {
    let mut ctx = ValidationContext::new();
    ctx.insert("key", 42usize);

    assert_eq!(ctx.get::<usize>("key"), Some(&42));
    assert_eq!(ctx.get::<String>("key"), None); // Wrong type
    assert_eq!(ctx.get::<usize>("missing"), None);
}

#[test]
fn test_context_contains() {
    let mut ctx = ValidationContext::new();
    ctx.insert("key", 42usize);

    assert!(ctx.contains("key"));
    assert!(!ctx.contains("missing"));
}

#[test]
fn test_context_field_path() {
    let mut ctx = ValidationContext::new();
    ctx.push_field("user");
    ctx.push_field("address");
    ctx.push_field("zipcode");

    assert_eq!(ctx.field_path(), "user.address.zipcode");

    ctx.pop_field();
    assert_eq!(ctx.field_path(), "user.address");

    ctx.clear_path();
    assert_eq!(ctx.field_path(), "");
}

#[test]
fn test_context_parent() {
    let mut parent = ValidationContext::new();
    parent.insert("parent_key", 100usize);

    let mut child = ValidationContext::with_parent(std::sync::Arc::new(parent));
    child.insert("child_key", 200usize);

    assert_eq!(child.get::<usize>("child_key"), Some(&200));
    assert_eq!(child.get::<usize>("parent_key"), Some(&100));
}

#[test]
fn test_context_builder() {
    let ctx = ValidationContextBuilder::new()
        .with("max", 100usize)
        .with("min", 5usize)
        .build();

    assert_eq!(ctx.get::<usize>("max"), Some(&100));
    assert_eq!(ctx.get::<usize>("min"), Some(&5));
}

#[test]
fn test_context_len_empty() {
    let mut ctx = ValidationContext::new();
    assert!(ctx.is_empty());
    assert_eq!(ctx.len(), 0);

    ctx.insert("key", 42);
    assert!(!ctx.is_empty());
    assert_eq!(ctx.len(), 1);
}

// Cross-field validation example
struct PasswordMatch;

impl ContextualValidator for PasswordMatch {
    type Input = User;

    fn validate_with_context(
        &self,
        input: &User,
        _ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        if input.password != input.password_confirmation {
            return Err(ValidationError::new(
                "password_mismatch",
                "Passwords do not match",
            ));
        }
        Ok(())
    }
}

struct User {
    password: String,
    password_confirmation: String,
}

#[test]
fn test_cross_field_validation() {
    let validator = PasswordMatch;
    let ctx = ValidationContext::new();

    let valid_user = User {
        password: "secret123".to_string(),
        password_confirmation: "secret123".to_string(),
    };

    let invalid_user = User {
        password: "secret123".to_string(),
        password_confirmation: "different".to_string(),
    };

    assert!(validator.validate_with_context(&valid_user, &ctx).is_ok());
    assert!(
        validator
            .validate_with_context(&invalid_user, &ctx)
            .is_err()
    );
}

// Date range validation example
struct DateRange {
    start: i64,
    end: i64,
}

struct DateRangeValidator;

impl ContextualValidator for DateRangeValidator {
    type Input = DateRange;

    fn validate_with_context(
        &self,
        input: &DateRange,
        _ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        if input.start >= input.end {
            return Err(ValidationError::new(
                "invalid_date_range",
                "Start date must be before end date",
            ));
        }
        Ok(())
    }
}

#[test]
fn test_date_range_validation() {
    let validator = DateRangeValidator;
    let ctx = ValidationContext::new();

    let valid_range = DateRange {
        start: 100,
        end: 200,
    };

    let invalid_range = DateRange {
        start: 200,
        end: 100,
    };

    assert!(validator.validate_with_context(&valid_range, &ctx).is_ok());
    assert!(
        validator
            .validate_with_context(&invalid_range, &ctx)
            .is_err()
    );
}

// Conditional validation example
struct ConditionalRequired;

impl ContextualValidator for ConditionalRequired {
    type Input = Form;

    fn validate_with_context(
        &self,
        input: &Form,
        ctx: &ValidationContext,
    ) -> Result<(), ValidationError> {
        // Get "require_email" flag from context
        let require_email = ctx.get::<bool>("require_email").copied().unwrap_or(false);

        if require_email && input.email.is_empty() {
            return Err(ValidationError::new(
                "email_required",
                "Email is required when flag is set",
            ));
        }

        Ok(())
    }
}

struct Form {
    email: String,
}

#[test]
fn test_conditional_validation_with_context() {
    let validator = ConditionalRequired;

    // Context with require_email = true
    let ctx_required = ValidationContextBuilder::new()
        .with("require_email", true)
        .build();

    // Context with require_email = false
    let ctx_optional = ValidationContextBuilder::new()
        .with("require_email", false)
        .build();

    let form_empty = Form {
        email: String::new(),
    };

    let form_filled = Form {
        email: "test@example.com".to_string(),
    };

    // Should fail when email required and empty
    assert!(
        validator
            .validate_with_context(&form_empty, &ctx_required)
            .is_err()
    );

    // Should pass when email optional and empty
    assert!(
        validator
            .validate_with_context(&form_empty, &ctx_optional)
            .is_ok()
    );

    // Should always pass when email is filled
    assert!(
        validator
            .validate_with_context(&form_filled, &ctx_required)
            .is_ok()
    );
    assert!(
        validator
            .validate_with_context(&form_filled, &ctx_optional)
            .is_ok()
    );
}

// Test with nested field paths
#[test]
fn test_nested_field_paths() {
    let mut ctx = ValidationContext::new();

    ctx.push_field("user");
    assert_eq!(ctx.field_path(), "user");

    ctx.push_field("profile");
    assert_eq!(ctx.field_path(), "user.profile");

    ctx.push_field("email");
    assert_eq!(ctx.field_path(), "user.profile.email");

    // Pop fields
    assert_eq!(ctx.pop_field(), Some("email".to_string()));
    assert_eq!(ctx.field_path(), "user.profile");

    assert_eq!(ctx.pop_field(), Some("profile".to_string()));
    assert_eq!(ctx.field_path(), "user");

    assert_eq!(ctx.pop_field(), Some("user".to_string()));
    assert_eq!(ctx.field_path(), "");
}

// Test child context
#[test]
fn test_child_context() {
    let mut parent = ValidationContext::new();
    parent.push_field("parent");
    parent.insert("key", 42usize);

    let (_parent_arc, child) = parent.child();

    // Child should inherit field path
    assert_eq!(child.field_path(), "parent");

    // Child can look up parent data through Arc reference — no data loss
    assert_eq!(child.get::<usize>("key"), Some(&42));
}
