//! Tests for text validators in derive macro

use nebula_derive::Validator;
use nebula_validator::core::TypedValidator;

#[test]
fn test_uuid_validator() {
    #[derive(Validator)]
    struct TestStruct {
        #[validate(uuid)]
        id: String,
    }

    // Valid UUID
    let valid = TestStruct {
        id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
    };
    assert!(valid.validate().is_ok());

    // Invalid UUID
    let invalid = TestStruct {
        id: "not-a-uuid".to_string(),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn test_datetime_validator() {
    #[derive(Validator)]
    struct TestStruct {
        #[validate(datetime)]
        created_at: String,
    }

    // Valid ISO 8601 datetime
    let valid = TestStruct {
        created_at: "2023-12-25T14:30:00Z".to_string(),
    };
    assert!(valid.validate().is_ok());

    // Valid date only
    let valid_date = TestStruct {
        created_at: "2023-12-25".to_string(),
    };
    assert!(valid_date.validate().is_ok());

    // Invalid datetime
    let invalid = TestStruct {
        created_at: "not-a-date".to_string(),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn test_json_validator() {
    #[derive(Validator)]
    struct TestStruct {
        #[validate(json)]
        data: String,
    }

    // Valid JSON object
    let valid_obj = TestStruct {
        data: r#"{"name": "John", "age": 30}"#.to_string(),
    };
    assert!(valid_obj.validate().is_ok());

    // Valid JSON array
    let valid_arr = TestStruct {
        data: r#"[1, 2, 3]"#.to_string(),
    };
    assert!(valid_arr.validate().is_ok());

    // Valid JSON string
    let valid_str = TestStruct {
        data: r#""hello""#.to_string(),
    };
    assert!(valid_str.validate().is_ok());

    // Invalid JSON
    let invalid = TestStruct {
        data: r#"{"name": "John"#.to_string(),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn test_slug_validator() {
    #[derive(Validator)]
    struct TestStruct {
        #[validate(slug)]
        url_slug: String,
    }

    // Valid slugs
    let valid = TestStruct {
        url_slug: "my-blog-post".to_string(),
    };
    assert!(valid.validate().is_ok());

    let valid2 = TestStruct {
        url_slug: "hello-world-123".to_string(),
    };
    assert!(valid2.validate().is_ok());

    // Invalid: uppercase
    let invalid = TestStruct {
        url_slug: "My-Blog-Post".to_string(),
    };
    assert!(invalid.validate().is_err());

    // Invalid: starts with hyphen
    let invalid2 = TestStruct {
        url_slug: "-hello".to_string(),
    };
    assert!(invalid2.validate().is_err());
}

#[test]
fn test_hex_validator() {
    #[derive(Validator)]
    struct TestStruct {
        #[validate(hex)]
        hash: String,
    }

    // Valid hex
    let valid = TestStruct {
        hash: "deadbeef".to_string(),
    };
    assert!(valid.validate().is_ok());

    let valid_with_prefix = TestStruct {
        hash: "0xabcdef123".to_string(),
    };
    assert!(valid_with_prefix.validate().is_ok());

    // Invalid hex
    let invalid = TestStruct {
        hash: "xyz123".to_string(),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn test_base64_validator() {
    #[derive(Validator)]
    struct TestStruct {
        #[validate(base64)]
        encoded: String,
    }

    // Valid base64
    let valid = TestStruct {
        encoded: "SGVsbG8gV29ybGQ=".to_string(), // "Hello World"
    };
    assert!(valid.validate().is_ok());

    let valid_no_padding = TestStruct {
        encoded: "YWJjZGVm".to_string(), // "abcdef"
    };
    assert!(valid_no_padding.validate().is_ok());

    // Invalid base64
    let invalid = TestStruct {
        encoded: "Not valid base64!".to_string(),
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn test_combined_text_validators() {
    #[derive(Validator)]
    struct UserForm {
        #[validate(uuid)]
        user_id: String,

        #[validate(slug)]
        username: String,

        #[validate(datetime)]
        created_at: String,

        #[validate(json)]
        metadata: String,
    }

    let valid = UserForm {
        user_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        username: "john-doe".to_string(),
        created_at: "2023-12-25T14:30:00Z".to_string(),
        metadata: r#"{"role": "admin"}"#.to_string(),
    };
    assert!(valid.validate().is_ok());

    let invalid_uuid = UserForm {
        user_id: "invalid".to_string(),
        username: "john-doe".to_string(),
        created_at: "2023-12-25T14:30:00Z".to_string(),
        metadata: r#"{"role": "admin"}"#.to_string(),
    };
    assert!(invalid_uuid.validate().is_err());
}

#[test]
fn test_text_validators_with_expr() {
    #[derive(Validator)]
    struct TestStruct {
        // Using expr for more complex validation
        #[validate(expr = "nebula_validator::validators::text::Uuid::new().lowercase_only()")]
        lowercase_uuid: String,

        #[validate(expr = "nebula_validator::validators::text::Hex::new().no_prefix()")]
        hex_no_prefix: String,
    }

    let valid = TestStruct {
        lowercase_uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        hex_no_prefix: "deadbeef".to_string(),
    };
    assert!(valid.validate().is_ok());

    // Uppercase UUID should fail with lowercase_only
    let invalid = TestStruct {
        lowercase_uuid: "550E8400-E29B-41D4-A716-446655440000".to_string(),
        hex_no_prefix: "deadbeef".to_string(),
    };
    assert!(invalid.validate().is_err());
}
