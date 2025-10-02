use nebula_error::{NebulaError, Result};

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("⚠️  Simple Error Example");

    // Creating different types of errors
    println!("\n🏗️  Creating errors:");

    // Validation error
    let validation_err = NebulaError::validation("Email format is invalid");
    println!("   Validation: {}", validation_err);

    // Not found error
    let not_found_err = NebulaError::not_found("user", "user-123");
    println!("   Not found: {}", not_found_err);

    // Internal error
    let internal_err = NebulaError::internal("Database connection failed");
    println!("   Internal: {}", internal_err);

    // Check error properties
    println!("\n🔍 Error properties:");
    println!(
        "   Validation is client error: {}",
        validation_err.is_client_error()
    );
    println!(
        "   Internal is server error: {}",
        internal_err.is_server_error()
    );

    // Function that returns an error
    match risky_function() {
        Ok(value) => println!("   Success: {}", value),
        Err(e) => println!("   Error: {}", e),
    }

    Ok(())
}

fn risky_function() -> Result<String> {
    Err(NebulaError::internal("Something went wrong"))
}
