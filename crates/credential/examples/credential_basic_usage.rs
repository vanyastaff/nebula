//! Basic credential management usage example
//!
//! Demonstrates US1: CRUD Operations
//! - Store credentials with encryption
//! - Retrieve credentials
//! - Delete credentials
//! - List all credentials

use nebula_credential::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Credential Manager: Basic Usage Example ===\n");

    // 1. Create a credential manager with mock storage
    println!("1. Creating credential manager...");
    let storage = Arc::new(MockStorageProvider::new());
    let manager = CredentialManager::builder().storage(storage).build();
    println!("   ✓ Manager created\n");

    // 2. Create encryption key and context
    let key = EncryptionKey::from_bytes([0u8; 32]);
    let context = CredentialContext::new("user-alice");

    // 3. Store a credential
    println!("2. Storing a credential...");
    let id = CredentialId::new("github-token")?;
    let secret_data = b"ghp_1234567890abcdefghijklmnopqrstuvwxyz";
    let encrypted = encrypt(&key, secret_data)?;
    let metadata = CredentialMetadata::new();

    manager.store(&id, encrypted, metadata, &context).await?;
    println!("   ✓ Stored credential: {}\n", id);

    // 4. Retrieve the credential
    println!("3. Retrieving credential...");
    if let Some((encrypted_data, metadata)) = manager.retrieve(&id, &context).await? {
        let decrypted = decrypt(&key, &encrypted_data)?;
        let secret = String::from_utf8_lossy(&decrypted);
        println!("   ✓ Retrieved: {} chars", secret.len());
        println!("   ✓ Created at: {}", metadata.created_at);
    }
    println!();

    // 5. List all credentials
    println!("4. Listing all credentials...");
    let all_creds = manager.list(&context).await?;
    println!("   ✓ Found {} credential(s)", all_creds.len());
    for cred_id in &all_creds {
        println!("     - {}", cred_id);
    }
    println!();

    // 6. Delete the credential
    println!("5. Deleting credential...");
    manager.delete(&id, &context).await?;
    println!("   ✓ Deleted: {}\n", id);

    // 7. Verify deletion
    println!("6. Verifying deletion...");
    let result = manager.retrieve(&id, &context).await?;
    if result.is_none() {
        println!("   ✓ Credential successfully deleted\n");
    }

    println!("=== Example completed successfully! ===");
    Ok(())
}
