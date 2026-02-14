---
title: Decryption Failures
description: Diagnostic guide for encryption and decryption errors in nebula-credential
tags: [troubleshooting, encryption, decryption, security, diagnostics]
related:
  - "[[Common-Errors]]"
  - "[[../Advanced/Key-Management]]"
  - "[[../Advanced/Security-Architecture]]"
  - "[[../Security/Encryption]]"
  - "[[Debugging-Checklist]]"
status: published
version: 1.0.0
---

# Decryption Failures

This guide provides comprehensive troubleshooting for encryption and decryption errors in `nebula-credential`.

---

## Overview

**Encryption Stack**:
- **Algorithm**: AES-256-GCM (AEAD)
- **Key Derivation**: Argon2id
- **Hash**: BLAKE3
- **Nonce**: 96-bit (12 bytes), unique per encryption
- **Tag**: 128-bit (16 bytes), authentication

**Common Error Types**:
- `CryptoError::DecryptionFailed` - Decryption operation failed
- `CryptoError::NoValidKey` - No key could decrypt data
- `CryptoError::InvalidKey` - Key format or content invalid
- `CryptoError::UnsupportedVersion` - Unsupported algorithm version

---

## Quick Diagnosis Decision Tree

```
DecryptionFailed
├─ Wrong Key? → Try key rotation manager with historical keys
├─ Corrupted Data? → Check ciphertext integrity, re-authenticate
├─ Version Mismatch? → Migrate encryption version
└─ Tampered Nonce? → Verify storage integrity, re-encrypt
```

---

## 1. `DecryptionFailed` - Root Cause Analysis

### 1.1 Wrong Encryption Key

**Symptom**: `DecryptionFailed("Authentication tag verification failed")`

**Cause**: Data encrypted with different key than current decryption key.

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

// Check if key rotation occurred
let encrypted_data = storage.retrieve(&id).await?;
let metadata = storage.get_metadata(&id).await?;

eprintln!("Credential created: {}", metadata.created_at);
eprintln!("Last updated: {}", metadata.updated_at);
eprintln!("Encryption version: {}", encrypted_data.version);

// Check key rotation history
let key_manager = KeyRotationManager::new(current_key);
eprintln!("Current key version: {}", key_manager.current_version());
eprintln!("Previous keys: {:#?}", key_manager.previous_key_versions());
```

**Solution**: Use `KeyRotationManager` to try all historical keys:

```rust
use nebula_credential::prelude::*;

pub async fn decrypt_with_rotation(
    encrypted: &EncryptedData,
    rotation_manager: &KeyRotationManager,
) -> Result<Vec<u8>, CryptoError> {
    // Try current key first
    match rotation_manager.decrypt_with_current(encrypted).await {
        Ok(plaintext) => {
            eprintln!("Decrypted with current key");
            return Ok(plaintext);
        }
        Err(e) => eprintln!("Current key failed: {e}"),
    }
    
    // Try all previous keys
    for (key_id, key) in rotation_manager.previous_keys() {
        match rotation_manager.decrypt_with_key(encrypted, key).await {
            Ok(plaintext) => {
                eprintln!("Decrypted with previous key: {key_id}");
                
                // Re-encrypt with current key
                let re_encrypted = rotation_manager.encrypt(&plaintext).await?;
                storage.store(&id, &re_encrypted, &metadata).await?;
                eprintln!("Re-encrypted with current key");
                
                return Ok(plaintext);
            }
            Err(_) => continue,
        }
    }
    
    Err(CryptoError::NoValidKey)
}
```

**Prevention**:

```rust
use nebula_credential::prelude::*;

// Always maintain key rotation history
impl KeyRotationManager {
    pub fn rotate_key(&mut self) -> Result<KeyVersion, CryptoError> {
        let new_key = EncryptionKey::generate();
        let new_version = self.next_version();
        
        // Keep old key accessible
        self.previous_keys.insert(self.current_version, self.current_key.clone());
        
        // Set new key
        self.current_key = new_key;
        self.current_version = new_version;
        
        Ok(new_version)
    }
}
```

See: [[../Advanced/Key-Management#Key-Rotation]]

---

### 1.2 Corrupted Ciphertext

**Symptom**: `DecryptionFailed("Ciphertext corrupted")`

**Causes**:
- Storage provider corruption
- Network transmission error
- Partial write during crash
- Manual editing of encrypted data

**Diagnosis**:

```rust
use nebula_credential::prelude::*;
use blake3::Hasher;

let encrypted = storage.retrieve(&id).await?;

// Check basic structure
eprintln!("Nonce length: {} (expected 12)", encrypted.nonce.len());
eprintln!("Ciphertext length: {}", encrypted.ciphertext.len());
eprintln!("Version: {} (current: 1)", encrypted.version);

// Check for suspicious patterns
if encrypted.ciphertext.len() < 16 {
    eprintln!("⚠️  Ciphertext too short, likely corrupted");
}

if encrypted.ciphertext.iter().all(|&b| b == 0) {
    eprintln!("⚠️  Ciphertext all zeros, likely corrupted");
}

// Verify integrity if checksum stored
if let Some(stored_checksum) = metadata.tags.get("checksum") {
    let mut hasher = Hasher::new();
    hasher.update(&encrypted.nonce);
    hasher.update(&encrypted.ciphertext);
    let computed_checksum = hex::encode(hasher.finalize().as_bytes());
    
    if stored_checksum != &computed_checksum {
        eprintln!("⚠️  Checksum mismatch, data corrupted!");
        eprintln!("   Stored:   {stored_checksum}");
        eprintln!("   Computed: {computed_checksum}");
    }
}
```

**Solution**: Re-authenticate and create new credential

```rust
use nebula_credential::prelude::*;

pub async fn recover_from_corruption(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    eprintln!("Credential {id} corrupted, re-authenticating...");
    
    // Delete corrupted credential
    manager.delete_credential(id, ctx).await?;
    
    // Re-authenticate (implementation depends on credential type)
    let new_credential = authenticate_user(ctx).await?;
    
    // Store with checksum
    let mut metadata = CredentialMetadata::new(
        id.clone(),
        new_credential.credential_type(),
        ctx.owner_id.clone(),
    );
    
    let encrypted = manager.encrypt(&new_credential).await?;
    
    // Add integrity checksum
    let mut hasher = blake3::Hasher::new();
    hasher.update(&encrypted.nonce);
    hasher.update(&encrypted.ciphertext);
    let checksum = hex::encode(hasher.finalize().as_bytes());
    metadata = metadata.with_tag("checksum".to_string(), checksum);
    
    manager.store_credential(id, &new_credential, ctx).await?;
    
    Ok(())
}
```

**Prevention**:

```rust
use nebula_credential::prelude::*;

// Add checksums to all encrypted data
pub async fn store_with_integrity(
    storage: &impl StorageProvider,
    id: &CredentialId,
    encrypted: &EncryptedData,
    metadata: &CredentialMetadata,
) -> Result<(), StorageError> {
    // Compute checksum
    let mut hasher = blake3::Hasher::new();
    hasher.update(&encrypted.nonce);
    hasher.update(&encrypted.ciphertext);
    let checksum = hex::encode(hasher.finalize().as_bytes());
    
    // Add to metadata
    let mut meta_with_checksum = metadata.clone();
    meta_with_checksum = meta_with_checksum.with_tag("checksum".to_string(), checksum);
    
    // Store
    storage.store(id, encrypted, &meta_with_checksum).await?;
    
    Ok(())
}

// Verify on retrieval
pub async fn retrieve_with_verification(
    storage: &impl StorageProvider,
    id: &CredentialId,
) -> Result<EncryptedData, StorageError> {
    let encrypted = storage.retrieve(id).await?
        .ok_or_else(|| StorageError::ReadFailed("Not found".to_string()))?;
    
    let metadata = storage.get_metadata(id).await?;
    
    // Verify checksum if present
    if let Some(stored_checksum) = metadata.tags.get("checksum") {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&encrypted.nonce);
        hasher.update(&encrypted.ciphertext);
        let computed = hex::encode(hasher.finalize().as_bytes());
        
        if stored_checksum != &computed {
            return Err(StorageError::ReadFailed("Checksum verification failed".to_string()));
        }
    }
    
    Ok(encrypted)
}
```

---

### 1.3 Tampered Nonce

**Symptom**: `DecryptionFailed("Invalid nonce")`

**Cause**: Nonce modified after encryption (security issue or corruption).

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let encrypted = storage.retrieve(&id).await?;

eprintln!("Nonce: {:?}", encrypted.nonce);
eprintln!("Nonce length: {}", encrypted.nonce.len());

// Check for zero nonce (invalid)
if encrypted.nonce.iter().all(|&b| b == 0) {
    eprintln!("⚠️  SECURITY: Nonce is all zeros!");
}

// Check for reused nonce (if tracking nonces)
if nonce_tracker.contains(&encrypted.nonce) {
    eprintln!("⚠️  SECURITY: Nonce reuse detected!");
}
```

**Solution**: Storage integrity issue, re-encrypt data

```rust
use nebula_credential::prelude::*;

pub async fn recover_from_nonce_tampering(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    eprintln!("⚠️  Nonce tampering detected for credential {id}");
    
    // This is a security issue - audit immediately
    audit_log::log_security_event(AuditEvent::NonceTamperingDetected {
        credential_id: id.clone(),
        detected_at: Utc::now(),
    }).await;
    
    // Cannot recover - must re-authenticate
    eprintln!("Re-authentication required");
    manager.delete_credential(id, ctx).await?;
    
    Ok(())
}
```

**Prevention**: Use immutable storage providers (e.g., AWS Secrets Manager versioning)

---

### 1.4 Version Mismatch

**Symptom**: `UnsupportedVersion(2)` (current version is 1)

**Cause**: Data encrypted with newer library version than current runtime.

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

const CURRENT_VERSION: u8 = 1;

let encrypted = storage.retrieve(&id).await?;

if encrypted.version > CURRENT_VERSION {
    eprintln!("⚠️  Data encrypted with version {}, current version is {CURRENT_VERSION}",
        encrypted.version);
    eprintln!("    Library upgrade required");
} else if encrypted.version < CURRENT_VERSION {
    eprintln!("ℹ️  Data encrypted with old version {}, migration available",
        encrypted.version);
}
```

**Solution**: Upgrade library or migrate data

```rust
use nebula_credential::prelude::*;

pub async fn migrate_encryption_version(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    // Retrieve with old version
    let old_encrypted = storage.retrieve(id).await?;
    
    if old_encrypted.version == 1 {
        // Decrypt with version 1 algorithm
        let plaintext = decrypt_v1(&old_encrypted, &key)?;
        
        // Re-encrypt with current version
        let new_encrypted = encrypt_current_version(&plaintext, &key)?;
        
        // Store
        storage.store(id, &new_encrypted, &metadata).await?;
        
        eprintln!("Migrated credential {id} from v{} to v{}",
            old_encrypted.version, new_encrypted.version);
    }
    
    Ok(())
}
```

---

## 2. `NoValidKey` - Key Management Issues

### 2.1 Lost Encryption Keys

**Symptom**: `NoValidKey` after trying all available keys

**Causes**:
- Key backup lost
- HSM/KMS unavailable
- Key rotation without migration
- Accidental key deletion

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let rotation_manager = KeyRotationManager::load()?;

eprintln!("Available keys:");
eprintln!("  Current: version {}", rotation_manager.current_version());
for (key_id, version) in rotation_manager.previous_keys() {
    eprintln!("  Previous: {key_id} (version {version})");
}

// Try all keys
let encrypted = storage.retrieve(&id).await?;
let mut all_failed = true;

for (key_id, key) in rotation_manager.all_keys() {
    match decrypt_with_key(&encrypted, key) {
        Ok(_) => {
            eprintln!("✓ Key {key_id} can decrypt");
            all_failed = false;
        }
        Err(_) => eprintln!("✗ Key {key_id} failed"),
    }
}

if all_failed {
    eprintln!("⚠️  No keys can decrypt this data - keys lost or data encrypted elsewhere");
}
```

**Solutions**:

1. **Restore from backup**:
   ```rust
   // Restore key from HSM
   let restored_key = hsm_client.get_key("credential-master-key").await?;
   rotation_manager.add_key(KeyId::new(), restored_key);
   ```

2. **Restore from KMS**:
   ```rust
   use aws_sdk_kms::Client as KmsClient;
   
   let kms_client = KmsClient::new(&aws_config);
   let decrypt_output = kms_client
       .decrypt()
       .ciphertext_blob(encrypted_key_blob)
       .send()
       .await?;
   
   let key = EncryptionKey::from_bytes(
       decrypt_output.plaintext().unwrap().as_ref().try_into()?
   );
   rotation_manager.add_key(KeyId::new(), key);
   ```

3. **Last resort - re-authenticate**:
   ```rust
   eprintln!("Cannot recover encrypted data, re-authentication required");
   manager.delete_credential(&id, &ctx).await?;
   let new_cred = authenticate_user(&ctx).await?;
   manager.store_credential(&id, &new_cred, &ctx).await?;
   ```

**Prevention**: Key Backup Strategy

```rust
use nebula_credential::prelude::*;

pub struct KeyBackupStrategy {
    hsm: HsmClient,
    kms: KmsClient,
    local_encrypted: PathBuf,
}

impl KeyBackupStrategy {
    pub async fn backup_key(&self, key: &EncryptionKey) -> Result<(), BackupError> {
        // 1. Store in HSM
        self.hsm.store_key("credential-master-key", key.as_bytes()).await?;
        
        // 2. Encrypt with KMS and store
        let encrypted_key = self.kms.encrypt()
            .key_id("alias/credential-kms-key")
            .plaintext(key.as_bytes())
            .send()
            .await?;
        
        tokio::fs::write(
            &self.local_encrypted,
            encrypted_key.ciphertext_blob().unwrap()
        ).await?;
        
        // 3. Print recovery instructions
        println!("Key backed up:");
        println!("  HSM: credential-master-key");
        println!("  KMS: {}", self.local_encrypted.display());
        
        Ok(())
    }
    
    pub async fn restore_key(&self) -> Result<EncryptionKey, BackupError> {
        // Try HSM first
        if let Ok(key_bytes) = self.hsm.get_key("credential-master-key").await {
            return Ok(EncryptionKey::from_bytes(key_bytes.try_into()?));
        }
        
        // Fallback to KMS
        let ciphertext = tokio::fs::read(&self.local_encrypted).await?;
        let plaintext = self.kms.decrypt()
            .ciphertext_blob(ciphertext)
            .send()
            .await?;
        
        Ok(EncryptionKey::from_bytes(
            plaintext.plaintext().unwrap().as_ref().try_into()?
        ))
    }
}
```

See: [[../Advanced/Key-Management#Backup-and-Recovery]]

---

## 3. `InvalidKey` - Key Format Issues

### 3.1 Wrong Key Derivation Parameters

**Symptom**: `InvalidKey("Key derivation failed")`

**Cause**: Argon2id parameters mismatch between encryption and decryption.

**Diagnosis**:

```rust
use nebula_credential::prelude::*;
use argon2::Argon2;

// Check derivation config
let config = KeyDerivationConfig {
    memory_cost: 19456, // 19 MiB
    time_cost: 2,
    parallelism: 1,
};

eprintln!("Key derivation config:");
eprintln!("  Memory: {} KiB", config.memory_cost);
eprintln!("  Time: {} iterations", config.time_cost);
eprintln!("  Parallelism: {}", config.parallelism);

// Verify salt
eprintln!("Salt length: {} (expected 16)", salt.len());
```

**Solution**: Use consistent derivation parameters

```rust
use nebula_credential::prelude::*;
use argon2::{Argon2, Params};

pub fn derive_key_consistent(
    password: &str,
    salt: &[u8; 16],
    config: &KeyDerivationConfig,
) -> Result<EncryptionKey, CryptoError> {
    let params = Params::new(
        config.memory_cost,
        config.time_cost,
        config.parallelism,
        Some(32), // Output length
    ).map_err(|e| CryptoError::InvalidKey(e.to_string()))?;
    
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        params,
    );
    
    let mut key_bytes = [0u8; 32];
    argon2.hash_password_into(password.as_bytes(), salt, &mut key_bytes)
        .map_err(|e| CryptoError::KeyDerivation(e.to_string()))?;
    
    Ok(EncryptionKey::from_bytes(key_bytes))
}
```

**Prevention**: Store derivation parameters with encrypted data

```rust
use nebula_credential::prelude::*;

#[derive(Serialize, Deserialize)]
pub struct EncryptedDataV2 {
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
    pub version: u8,
    
    // Include derivation params
    pub kdf_params: KeyDerivationConfig,
    pub salt: [u8; 16],
}
```

---

### 3.2 Raw Key Corruption

**Symptom**: `InvalidKey("Key length must be 32 bytes")`

**Cause**: Key data corrupted or truncated.

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let key_bytes = load_key_from_storage()?;

eprintln!("Key length: {} (expected 32)", key_bytes.len());

if key_bytes.len() != 32 {
    eprintln!("⚠️  Key corrupted - invalid length");
}

// Check for zero key
if key_bytes.iter().all(|&b| b == 0) {
    eprintln!("⚠️  Key is all zeros - likely uninitialized");
}
```

**Solution**: Restore key from secure backup (see Section 2.1)

---

## 4. Advanced Debugging

### 4.1 Enable Cryptographic Tracing

```rust
use nebula_credential::prelude::*;
use tracing::{debug, instrument};

#[instrument(skip(key, plaintext))]
pub async fn encrypt_traced(
    key: &EncryptionKey,
    plaintext: &[u8],
) -> Result<EncryptedData, CryptoError> {
    debug!("Encrypting {} bytes", plaintext.len());
    
    let nonce = generate_nonce();
    debug!("Generated nonce: {:?}", nonce);
    
    let cipher = Aes256Gcm::new(Key::from_slice(key.as_bytes()));
    
    let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|e| {
            debug!("Encryption failed: {e}");
            CryptoError::EncryptionFailed(e.to_string())
        })?;
    
    debug!("Encryption successful, ciphertext length: {}", ciphertext.len());
    
    Ok(EncryptedData {
        nonce,
        ciphertext,
        version: 1,
    })
}

#[instrument(skip(key, encrypted))]
pub async fn decrypt_traced(
    key: &EncryptionKey,
    encrypted: &EncryptedData,
) -> Result<Vec<u8>, CryptoError> {
    debug!("Decrypting {} bytes with version {}", encrypted.ciphertext.len(), encrypted.version);
    debug!("Nonce: {:?}", encrypted.nonce);
    
    let cipher = Aes256Gcm::new(Key::from_slice(key.as_bytes()));
    
    let plaintext = cipher.decrypt(
        Nonce::from_slice(&encrypted.nonce),
        encrypted.ciphertext.as_ref()
    ).map_err(|e| {
        debug!("Decryption failed: {e}");
        CryptoError::DecryptionFailed(e.to_string())
    })?;
    
    debug!("Decryption successful, plaintext length: {}", plaintext.len());
    
    Ok(plaintext)
}
```

### 4.2 Compare Encryption Implementations

```rust
use nebula_credential::prelude::*;

pub async fn compare_implementations(
    plaintext: &[u8],
    key: &EncryptionKey,
) -> Result<(), CryptoError> {
    // Encrypt
    let encrypted1 = encrypt_v1(plaintext, key)?;
    let encrypted2 = encrypt_v1(plaintext, key)?;
    
    // Nonces must be different
    assert_ne!(encrypted1.nonce, encrypted2.nonce, "Nonce reuse detected!");
    
    // Ciphertext lengths should be equal
    assert_eq!(encrypted1.ciphertext.len(), encrypted2.ciphertext.len());
    
    // Decrypt both
    let decrypted1 = decrypt_v1(&encrypted1, key)?;
    let decrypted2 = decrypt_v1(&encrypted2, key)?;
    
    // Plaintexts must match original
    assert_eq!(decrypted1, plaintext);
    assert_eq!(decrypted2, plaintext);
    
    eprintln!("✓ Encryption implementation verified");
    
    Ok(())
}
```

### 4.3 Audit Encryption History

```rust
use nebula_credential::prelude::*;

pub struct EncryptionAudit {
    pub credential_id: CredentialId,
    pub encrypted_at: DateTime<Utc>,
    pub key_version: KeyVersion,
    pub algorithm_version: u8,
    pub ciphertext_length: usize,
}

pub async fn audit_credential_encryption(
    storage: &impl StorageProvider,
    id: &CredentialId,
) -> Result<Vec<EncryptionAudit>, StorageError> {
    let metadata = storage.get_metadata(id).await?;
    let encrypted = storage.retrieve(id).await?.unwrap();
    
    let audit = EncryptionAudit {
        credential_id: id.clone(),
        encrypted_at: metadata.updated_at,
        key_version: KeyVersion(encrypted.version.into()),
        algorithm_version: encrypted.version,
        ciphertext_length: encrypted.ciphertext.len(),
    };
    
    eprintln!("Encryption Audit for {id}:");
    eprintln!("  Encrypted: {}", audit.encrypted_at);
    eprintln!("  Key version: {}", audit.key_version);
    eprintln!("  Algorithm: v{}", audit.algorithm_version);
    eprintln!("  Size: {} bytes", audit.ciphertext_length);
    
    Ok(vec![audit])
}
```

---

## 5. Recovery Procedures

### 5.1 Systematic Recovery Workflow

```rust
use nebula_credential::prelude::*;

pub async fn recover_decryption_failure(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    eprintln!("=== Decryption Failure Recovery for {id} ===\n");
    
    // Step 1: Retrieve encrypted data
    eprintln!("Step 1: Retrieving encrypted data...");
    let encrypted = match manager.storage.retrieve(id).await {
        Ok(Some(data)) => {
            eprintln!("✓ Retrieved {} bytes", data.ciphertext.len());
            data
        }
        Ok(None) => {
            eprintln!("✗ Credential not found");
            return Err(CredentialError::NotFound(id.clone()));
        }
        Err(e) => {
            eprintln!("✗ Storage error: {e}");
            return Err(e.into());
        }
    };
    
    // Step 2: Verify data integrity
    eprintln!("\nStep 2: Verifying data integrity...");
    if encrypted.ciphertext.len() < 16 {
        eprintln!("✗ Ciphertext too short - corrupted");
        return Err(CredentialError::InvalidFormat("Corrupted ciphertext".to_string()));
    }
    eprintln!("✓ Basic structure valid");
    
    // Step 3: Try key rotation manager
    eprintln!("\nStep 3: Attempting decryption with all keys...");
    let rotation_manager = manager.key_rotation_manager();
    
    match decrypt_with_rotation(&encrypted, rotation_manager).await {
        Ok(plaintext) => {
            eprintln!("✓ Decryption successful");
            return Ok(());
        }
        Err(CryptoError::NoValidKey) => {
            eprintln!("✗ All keys failed");
        }
        Err(e) => {
            eprintln!("✗ Decryption error: {e}");
        }
    }
    
    // Step 4: Attempt key recovery
    eprintln!("\nStep 4: Attempting key recovery from backup...");
    if let Ok(restored_key) = restore_key_from_backup().await {
        eprintln!("✓ Key restored from backup");
        rotation_manager.add_key(KeyId::new(), restored_key);
        
        // Retry decryption
        match rotation_manager.decrypt(&encrypted).await {
            Ok(_) => {
                eprintln!("✓ Decryption successful with restored key");
                return Ok(());
            }
            Err(_) => eprintln!("✗ Restored key also failed"),
        }
    } else {
        eprintln!("✗ Key recovery failed");
    }
    
    // Step 5: Last resort - re-authentication
    eprintln!("\nStep 5: Recovery failed - re-authentication required");
    eprintln!("   Deleting corrupted credential...");
    manager.delete_credential(id, ctx).await?;
    
    eprintln!("   Please re-authenticate to create new credential");
    
    Err(CredentialError::InvalidFormat(
        "Unrecoverable decryption failure".to_string()
    ))
}
```

---

## 6. Prevention Best Practices

### 6.1 Robust Encryption Implementation

```rust
use nebula_credential::prelude::*;
use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::Aead;

pub struct RobustEncryptor {
    key: EncryptionKey,
    nonce_generator: NonceGenerator,
}

impl RobustEncryptor {
    pub fn new(key: EncryptionKey) -> Self {
        Self {
            key,
            nonce_generator: NonceGenerator::new(),
        }
    }
    
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedData, CryptoError> {
        // Generate unique nonce
        let nonce = self.nonce_generator.generate();
        
        // Create cipher
        let cipher = Aes256Gcm::new(Key::from_slice(self.key.as_bytes()));
        
        // Encrypt with AEAD
        let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;
        
        // Return with version
        Ok(EncryptedData {
            nonce,
            ciphertext,
            version: 1,
        })
    }
    
    pub fn decrypt(&self, encrypted: &EncryptedData) -> Result<Vec<u8>, CryptoError> {
        // Verify version
        if encrypted.version != 1 {
            return Err(CryptoError::UnsupportedVersion(encrypted.version));
        }
        
        // Verify nonce length
        if encrypted.nonce.len() != 12 {
            return Err(CryptoError::DecryptionFailed("Invalid nonce length".to_string()));
        }
        
        // Create cipher
        let cipher = Aes256Gcm::new(Key::from_slice(self.key.as_bytes()));
        
        // Decrypt and authenticate
        let plaintext = cipher.decrypt(
            Nonce::from_slice(&encrypted.nonce),
            encrypted.ciphertext.as_ref()
        ).map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;
        
        Ok(plaintext)
    }
}
```

### 6.2 Integrity Protection

```rust
use nebula_credential::prelude::*;

pub async fn store_with_integrity_protection(
    storage: &impl StorageProvider,
    id: &CredentialId,
    encrypted: &EncryptedData,
    metadata: &CredentialMetadata,
) -> Result<(), StorageError> {
    // Compute BLAKE3 hash
    let mut hasher = blake3::Hasher::new();
    hasher.update(&encrypted.version.to_le_bytes());
    hasher.update(&encrypted.nonce);
    hasher.update(&encrypted.ciphertext);
    
    let hash = hasher.finalize();
    let checksum = hex::encode(hash.as_bytes());
    
    // Store checksum in metadata
    let mut protected_metadata = metadata.clone();
    protected_metadata = protected_metadata.with_tag("checksum".to_string(), checksum);
    protected_metadata = protected_metadata.with_tag("checksum_algo".to_string(), "blake3".to_string());
    
    // Store
    storage.store(id, encrypted, &protected_metadata).await
}
```

---

## Related Documentation

- [[Common-Errors]] - All error types catalog
- [[../Advanced/Key-Management]] - Key rotation and management
- [[../Advanced/Security-Architecture]] - Security design
- [[../Security/Encryption]] - Encryption implementation details
- [[Debugging-Checklist]] - Systematic debugging
- [[Provider-Connectivity]] - Storage provider issues

---

## Summary

This guide covers:

✅ **Root cause analysis** for decryption failures  
✅ **Diagnostic procedures** for each error type  
✅ **Key rotation** troubleshooting and recovery  
✅ **Data corruption** detection and remediation  
✅ **Advanced debugging** with tracing and auditing  
✅ **Recovery workflows** for systematic resolution  
✅ **Prevention strategies** for robust encryption  

For quick diagnosis, start with the decision tree at the top, then follow the relevant section for detailed troubleshooting.
