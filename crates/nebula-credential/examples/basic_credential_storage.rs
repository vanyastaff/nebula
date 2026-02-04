use nebula_credential::{
    CredentialContext, CredentialId, CredentialMetadata, EncryptionKey, SecretString,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create credential ID
    let id = CredentialId::new("github_token")?;

    // 2. Create secret (automatically zeros on drop)
    let secret = SecretString::new("ghp_xxxxxxxxxxxx");

    // 3. Derive encryption key from master password
    let salt = [0u8; 16]; // Load from secure storage in production
    let key = EncryptionKey::derive_from_password("master-pwd", &salt)?;

    // 4. Create request context
    let context = CredentialContext::new("user_123");

    // 5. Create metadata
    let metadata = CredentialMetadata::new();

    println!("Credential '{}' ready for storage", id);
    println!(
        "Context: owner={}, trace_id={}",
        context.owner_id, context.trace_id
    );
    println!("Metadata: created_at={}", metadata.created_at);

    // Demonstrate secret redaction
    println!("Secret (redacted): {:?}", secret);

    // Access secret safely
    secret.expose_secret(|value| {
        println!("Secret length: {}", value.len());
    });

    // Phase 2 adds: provider.store(&id, encrypted, metadata, &context).await?

    Ok(())
}
