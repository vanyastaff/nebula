//! Basic usage example for nebula-validator

use nebula_validator::prelude::*;
use serde_json::json;

fn main() {
    basic_string_validation();
    composed_string_validation();
    extension_style_validation();
    regex_and_conditional_validation();
    numeric_validation();
    collection_validation();
    network_validation();
    json_field_validation();
    profile_payload_validation();
    collect_all_errors_demo();
    validation_error_details();

    println!("\nnebula-validator examples completed successfully.");
}

fn basic_string_validation() {
    println!("=== Basic String Validation ===");

    let validator = min_length(5);

    match validator.validate("hello") {
        Ok(_) => println!("✓ 'hello' is valid (length >= 5)"),
        Err(e) => println!("✗ Error: {}", e),
    }

    match validator.validate("hi") {
        Ok(_) => println!("✓ 'hi' is valid"),
        Err(e) => println!("✗ 'hi' is invalid: {}", e),
    }
}

fn composed_string_validation() {
    println!("\n=== Composed String Validation ===");

    // Username: 3..20 chars and only alphanumeric symbols.
    let username = min_length(3).and(max_length(20)).and(alphanumeric());

    for sample in ["alice123", "ab", "john_doe"] {
        match username.validate(sample) {
            Ok(_) => println!("✓ '{sample}' passed username rules"),
            Err(e) => println!("✗ '{sample}' failed: {}", e),
        }
    }

    // OR example: either exact length 5 or exact length 8.
    let id_rule = exact_length(5).or(exact_length(8));
    for sample in ["ABCDE", "ABC", "ABCDEFGH"] {
        println!("id '{sample}': {}", status(&id_rule.validate(sample)));
    }
}

fn numeric_validation() {
    println!("\n=== Numeric Validation ===");

    let age = in_range(18_u32, 120_u32);
    for value in [16_u32, 30_u32, 140_u32] {
        println!("age {value}: {}", status(&age.validate(&value)));
    }
}

fn extension_style_validation() {
    println!("\n=== Extension Style Validation ===");

    let username_rule = min_length(3).and(max_length(16)).and(alphanumeric());

    for sample in ["neo", "x", "john_doe"] {
        println!(
            "'{sample}'.validate_with(username_rule): {}",
            status(&sample.validate_with(&username_rule))
        );
    }
}

fn regex_and_conditional_validation() {
    println!("\n=== Regex + Conditional Validation ===");

    let ticket = matches_regex(r"^[A-Z]{3}-\d{4}$").expect("valid regex pattern");
    for value in ["ABC-2026", "abc-2026", "BAD"] {
        println!("ticket '{value}': {}", status(&ticket.validate(value)));
    }

    // Validate URL only when the field is non-empty.
    let website_rule = url().when(|value: &str| !value.is_empty());
    for value in ["", "https://nebula.dev", "not-url"] {
        println!(
            "website '{value}': {}",
            status(&website_rule.validate(value))
        );
    }
}

fn collection_validation() {
    println!("\n=== Collection Validation ===");

    let tags = size_range::<&str>(1, 3);

    let a = vec!["rust"];
    let b = vec!["rust", "validator", "nebula", "extra"];

    println!("tags {:?}: {}", a, status(&tags.validate(&a)));
    println!("tags {:?}: {}", b, status(&tags.validate(&b)));
}

fn network_validation() {
    println!("\n=== Network Validation ===");

    let host = hostname();
    let ip = ip_addr();

    for value in ["api.nebula.local", "bad host"] {
        println!("hostname '{value}': {}", status(&host.validate(value)));
    }

    for value in ["127.0.0.1", "300.1.1.1"] {
        println!("ip '{value}': {}", status(&ip.validate(value)));
    }
}

fn json_field_validation() {
    println!("\n=== JSON Field Validation ===");

    let payload = json!({
        "user": {
            "name": "alice",
            "port": 8080
        }
    });

    let name_rule = json_field::<_, str>("/user/name", min_length(3));
    let port_rule = json_field::<_, i64>("/user/port", in_range(1_i64, 65535_i64));
    let email_optional = json_field_optional::<_, str>("/user/email", email());

    println!("/user/name: {}", status(&name_rule.validate(&payload)));
    println!("/user/port: {}", status(&port_rule.validate(&payload)));
    println!(
        "/user/email (optional): {}",
        status(&email_optional.validate(&payload))
    );
}

fn profile_payload_validation() {
    println!("\n=== Profile Payload Validation ===");

    let profile_validator = json_field::<_, str>("/username", min_length(3).and(max_length(20)))
        .and(json_field::<_, str>("/email", email()))
        .and(json_field::<_, i64>("/age", in_range(13_i64, 120_i64)))
        .and(json_field_optional::<_, str>("/website", url()));

    let ok = json!({
        "username": "alice123",
        "email": "alice@example.com",
        "age": 25,
        "website": "https://example.com"
    });

    let bad = json!({
        "username": "ab",
        "email": "not-an-email",
        "age": 9,
        "website": "not-url"
    });

    println!(
        "valid profile: {}",
        status(&profile_validator.validate(&ok))
    );
    println!(
        "invalid profile: {}",
        status(&profile_validator.validate(&bad))
    );
}

fn collect_all_errors_demo() {
    println!("\n=== Collect-All Errors Demo ===");

    let validator = all_of([
        AnyValidator::new(json_field::<_, str>("/username", min_length(3))),
        AnyValidator::new(json_field::<_, str>("/email", email())),
        AnyValidator::new(json_field::<_, i64>("/age", in_range(13_i64, 120_i64))),
    ])
    .with_mode(ValidationMode::CollectAll);

    let payload = json!({
        "username": "ab",
        "email": "bad-email",
        "age": 9
    });

    match validator.validate(&payload) {
        Ok(()) => println!("unexpected: payload is valid"),
        Err(e) => {
            println!("root code: {}", e.code);
            println!("nested errors: {}", e.nested().len());
            for nested in e.nested() {
                println!("  - [{}] {}", nested.code, nested.message);
            }
        },
    }
}

fn validation_error_details() {
    println!("\n=== Error Details ===");

    let validator = min_length(6);

    if let Err(e) = validator.validate("abc") {
        println!("code: {}", e.code);
        println!("message: {}", e.message);
        println!("flattened error count: {}", e.total_error_count());
    }
}

fn status<T>(result: &Result<T, ValidationError>) -> &'static str {
    if result.is_ok() { "ok" } else { "error" }
}
