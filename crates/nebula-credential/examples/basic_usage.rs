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
    let _password = SecureString::new("super-secret-password");
    println!("   ✓ Created secure string (auto-zeroized on drop)");
    println!("   - SecureString protects sensitive data");
    println!();

    // 4. Credential metadata
    println!("4. Credential metadata...");
    let metadata = CredentialMetadata {
        id: "postgres-db",
        name: "Database Credentials",
        description: "PostgreSQL production database",
        supports_refresh: true,
        requires_interaction: false,
    };

    println!("   ✓ Metadata created:");
    println!("     - ID: {}", metadata.id);
    println!("     - Name: {}", metadata.name);
    println!("     - Description: {}", metadata.description);
    println!();

    // 5. Credential context
    println!("5. Credential context...");
    let _context = CredentialContext::new();
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
