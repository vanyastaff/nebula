//! Validating JSON data with nebula-validator.
//!
//! Run: `cargo run -p nebula-validator --features serde --example json_validation`

use nebula_validator::combinators::{json_field, json_field_optional, not, when, with_message};
use nebula_validator::foundation::{Validate, ValidateExt};
use nebula_validator::validators::is_true;
use nebula_validator::validators::{contains, email, max_length, min_length};
use nebula_validator::validators::{greater_than, in_range};
use serde_json::{Value, json};

fn main() {
    direct_value_validation();
    field_level_validation();
    composed_validators();
    real_world_config_validation();
    error_reporting();
}

/// Validate raw JSON values using `validate_any`.
fn direct_value_validation() {
    println!("=== Direct Value Validation ===\n");

    let min3 = min_length(3);

    // String validation
    let result = min3.validate_any(&json!("hello"));
    println!("min_length(3) on \"hello\": {}", status(&result));

    let result = min3.validate_any(&json!("hi"));
    println!("min_length(3) on \"hi\":    {}", status(&result));

    // Numeric validation
    let range = in_range::<i64>(1, 65535);
    let result = range.validate_any(&json!(8080));
    println!("in_range(1, 65535) on 8080: {}", status(&result));

    // Boolean validation
    let result = is_true().validate_any(&json!(true));
    println!("is_true() on true: {}", status(&result));

    // Type mismatch — number passed to string validator
    let result = min3.validate_any(&json!(42));
    println!("min_length(3) on 42: {}", status(&result));

    println!();
}

/// Validate fields within JSON objects using `json_field`.
fn field_level_validation() {
    println!("=== Field-Level Validation ===\n");

    let data = json!({
        "server": {
            "host": "localhost",
            "port": 8080
        }
    });

    // Required field
    let v = json_field("/server/host", min_length(1));
    println!("/server/host: {}", status(&v.validate(&data)));

    // Nested numeric field
    let v = json_field("/server/port", in_range::<i64>(1, 65535));
    println!("/server/port: {}", status(&v.validate(&data)));

    // Optional field — missing is ok
    let v = json_field_optional("/server/tls", min_length(1));
    println!(
        "/server/tls (optional, missing): {}",
        status(&v.validate(&data))
    );

    // Missing required field
    let v = json_field("/server/name", min_length(1));
    println!(
        "/server/name (required, missing): {}",
        status(&v.validate(&data))
    );

    println!();
}

/// Compose validators with `and`, `or`, `not`, `when`.
fn composed_validators() {
    println!("=== Composed Validators ===\n");

    // AND: both host and port must be valid
    let v = json_field("/host", min_length(1)).and(json_field("/port", in_range::<i64>(1, 65535)));

    let good = json!({"host": "localhost", "port": 8080});
    println!("host AND port (valid): {}", status(&v.validate(&good)));

    let bad = json!({"host": "", "port": 8080});
    println!("host AND port (empty host): {}", status(&v.validate(&bad)));

    // OR: field is either a string or a number > 0
    let v = json_field("/value", min_length(1)).or(json_field("/value", greater_than::<i64>(0)));

    println!(
        "string OR number (\"hello\"): {}",
        status(&v.validate(&json!({"value": "hello"})))
    );
    println!(
        "string OR number (42):      {}",
        status(&v.validate(&json!({"value": 42})))
    );

    // NOT: status must not contain "error"
    let v = not(json_field("/status", contains("error")));
    println!(
        "NOT contains error (\"ok\"): {}",
        status(&v.validate(&json!({"status": "ok"})))
    );

    // WHEN: validate email only when notify is true
    let v = when(json_field("/email", email()), |v: &Value| {
        v.get("notify").and_then(|n| n.as_bool()).unwrap_or(false)
    });

    let data = json!({"notify": false, "email": "bad"});
    println!("email WHEN notify=false: {}", status(&v.validate(&data)));

    let data = json!({"notify": true, "email": "user@example.com"});
    println!("email WHEN notify=true:  {}", status(&v.validate(&data)));

    println!();
}

/// Validate a complete server configuration.
fn real_world_config_validation() {
    println!("=== Real-World Config Validation ===\n");

    let validator = json_field("/host", min_length(1))
        .and(json_field("/port", in_range::<i64>(1, 65535)))
        .and(json_field("/workers", greater_than::<i64>(0)))
        .and(json_field_optional("/log_level", min_length(1)))
        .and(json_field("/database/url", min_length(10)))
        .and(json_field_optional(
            "/database/pool_size",
            in_range::<i64>(1, 100),
        ));

    let config = json!({
        "host": "0.0.0.0",
        "port": 8080,
        "workers": 4,
        "log_level": "info",
        "database": {
            "url": "postgres://localhost:5432/app",
            "pool_size": 10
        }
    });
    println!("Full config: {}", status(&validator.validate(&config)));

    let minimal = json!({
        "host": "localhost",
        "port": 3000,
        "workers": 1,
        "database": {
            "url": "sqlite:///tmp/app.db"
        }
    });
    println!("Minimal config: {}", status(&validator.validate(&minimal)));

    let bad = json!({
        "host": "",
        "port": 0,
        "workers": -1,
        "database": {
            "url": "short"
        }
    });
    println!("Bad config: {}", status(&validator.validate(&bad)));

    println!();
}

/// Inspect validation error details.
fn error_reporting() {
    println!("=== Error Reporting ===\n");

    let validator = json_field("/name", min_length(1))
        .and(json_field("/email", email()))
        .and(json_field("/age", in_range::<i64>(13, 120)))
        .and(with_message(
            json_field("/name", max_length(100)),
            "Name is too long",
        ));

    let data = json!({
        "name": "",
        "email": "not-email",
        "age": 5
    });

    // AND combinator short-circuits on first error
    if let Err(e) = validator.validate(&data) {
        println!("Error code:    {}", e.code);
        println!("Error message: {}", e.message);
        if let Some(field) = &e.field {
            println!("Error field:   {}", field);
        }
        for (k, v) in &e.params {
            println!("  param {}: {}", k, v);
        }
    }

    // Validate all fields independently to collect all errors
    let validators: Vec<(&str, Box<dyn Validate<Input = Value>>)> = vec![
        ("name", Box::new(json_field("/name", min_length(1)))),
        ("email", Box::new(json_field("/email", email()))),
        (
            "age",
            Box::new(json_field("/age", in_range::<i64>(13, 120))),
        ),
    ];

    println!("\nAll errors:");
    for (label, v) in &validators {
        if let Err(e) = v.validate(&data) {
            println!("  {}: [{}] {}", label, e.code, e.message);
        }
    }
}

fn status(result: &Result<(), nebula_validator::foundation::ValidationError>) -> &'static str {
    match result {
        Ok(()) => "PASS",
        Err(_) => "FAIL",
    }
}
