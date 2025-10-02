//! Basic credential management example
//!
//! Demonstrates:
//! - Creating credential IDs
//! - Secure string handling
//! - Credential metadata

use nebula_credential::prelude::*;

fn main() {
    println!("=== Nebula Credential - Basic Usage ===\n");

    // 1. Create credential IDs
    println!("1. Creating credential IDs...");
    let cred_id_1 = CredentialId::new();
    let cred_id_2 = CredentialId::from_string("my-database-creds");

    println!("   ✓ Random ID: {}", cred_id_1);
    println!("   ✓ Named ID: {}", cred_id_2);
    println!();

    // 2. Credential ID operations
    println!("2. Credential ID operations...");
    println!("   - ID as string: {}", cred_id_2.as_str());
    println!("   - ID equality: {}", cred_id_1 == cred_id_1);
    println!("   - Different IDs: {}", cred_id_1 != cred_id_2);
    println!();

    // 3. Secure string handling
    println!("3. Secure string example...");
    let password = SecureString::from("super-secret-password");
    println!("   ✓ Created secure string (auto-zeroized on drop)");
    println!("   - SecureString protects sensitive data");
    println!();

    // 4. Credential metadata
    println!("4. Credential metadata...");
    let metadata = CredentialMetadata {
        id: cred_id_2.clone(),
        name: "Database Credentials".to_string(),
        description: Some("PostgreSQL production database".to_string()),
    };

    println!("   ✓ Metadata created:");
    println!("     - ID: {}", metadata.id);
    println!("     - Name: {}", metadata.name);
    if let Some(desc) = &metadata.description {
        println!("     - Description: {}", desc);
    }
    println!();

    // 5. Credential context
    println!("5. Credential context...");
    let context = CredentialContext::new();
    println!("   ✓ Context created for credential operations");
    println!("   - Used for tracing and audit logging");
    println!();

    println!("=== Key Features ===");
    println!("✓ Type-safe credential IDs");
    println!("✓ Secure string with automatic zeroization");
    println!("✓ Metadata tracking");
    println!("✓ Context for observability");
    println!();

    println!("=== Example completed successfully! ===");
}
