---
title: Building Custom Storage Providers
description: Complete guide to implementing custom StorageProvider backends for nebula-credential
tags: [advanced, storage, custom-provider, implementation, trait]
related:
  - "[[../Integrations/AWS-Secrets-Manager]]"
  - "[[../Integrations/HashiCorp-Vault]]"
  - "[[../Integrations/Local-Storage]]"
  - "[[../Reference/API-Reference]]"
  - "[[../Architecture]]"
status: published
version: 1.0.0
---

# Building Custom Storage Providers

Complete guide to implementing custom `StorageProvider` backends for nebula-credential, enabling integration with any secret storage system.

---

## Overview

The `StorageProvider` trait allows you to integrate nebula-credential with any storage backend:
- **Cloud providers** (Google Cloud Secret Manager, DigitalOcean Secrets)
- **Enterprise systems** (CyberArk, Thycotic, 1Password)
- **Custom solutions** (PostgreSQL, etcd, Consul)
- **Hybrid approaches** (Local cache + remote backend)

---

## StorageProvider Trait

### Complete Interface

```rust
use async_trait::async_trait;
use nebula_credential::prelude::*;

#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Store encrypted credential data
    async fn store(
        &self,
        id: &CredentialId,
        encrypted_data: &EncryptedData,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError>;
    
    /// Retrieve encrypted credential data
    async fn retrieve(
        &self,
        id: &CredentialId,
    ) -> Result<Option<EncryptedData>, StorageError>;
    
    /// Delete credential
    async fn delete(&self, id: &CredentialId) -> Result<(), StorageError>;
    
    /// List credentials with optional filtering
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
    ) -> Result<Vec<CredentialMetadata>, StorageError>;
    
    /// Update credential metadata only (no re-encryption)
    async fn update_metadata(
        &self,
        id: &CredentialId,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError>;
}
```

**Key Points**:
- All data passed to provider is **already encrypted** (AES-256-GCM)
- Provider only handles **storage/retrieval**, not encryption
- Must be `Send + Sync` for concurrent access
- All operations are async

---

## Example 1: PostgreSQL Provider

### Implementation

```rust
use async_trait::async_trait;
use nebula_credential::prelude::*;
use sqlx::{PgPool, postgres::PgPoolOptions};
use serde_json;

pub struct PostgresProvider {
    pool: PgPool,
}

impl PostgresProvider {
    pub async fn new(database_url: &str) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        Ok(Self { pool })
    }
    
    pub async fn initialize_schema(&self) -> Result<(), StorageError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS credentials (
                id TEXT PRIMARY KEY,
                nonce BYTEA NOT NULL,
                ciphertext BYTEA NOT NULL,
                version SMALLINT NOT NULL,
                metadata JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            
            CREATE INDEX IF NOT EXISTS idx_credentials_owner 
                ON credentials ((metadata->>'owner_id'));
            
            CREATE INDEX IF NOT EXISTS idx_credentials_type 
                ON credentials ((metadata->>'credential_type'));
            
            CREATE INDEX IF NOT EXISTS idx_credentials_scope 
                ON credentials ((metadata->>'scope_id'));
            "#
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::SchemaInitFailed(e.to_string()))?;
        
        Ok(())
    }
}

#[async_trait]
impl StorageProvider for PostgresProvider {
    async fn store(
        &self,
        id: &CredentialId,
        encrypted: &EncryptedData,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        let metadata_json = serde_json::to_value(metadata)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        sqlx::query(
            r#"
            INSERT INTO credentials (id, nonce, ciphertext, version, metadata)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE
            SET nonce = EXCLUDED.nonce,
                ciphertext = EXCLUDED.ciphertext,
                version = EXCLUDED.version,
                metadata = EXCLUDED.metadata,
                updated_at = NOW()
            "#
        )
        .bind(id.as_str())
        .bind(&encrypted.nonce[..])
        .bind(&encrypted.ciphertext)
        .bind(encrypted.version as i16)
        .bind(metadata_json)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn retrieve(
        &self,
        id: &CredentialId,
    ) -> Result<Option<EncryptedData>, StorageError> {
        let row = sqlx::query(
            r#"
            SELECT nonce, ciphertext, version
            FROM credentials
            WHERE id = $1
            "#
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::ReadFailed(e.to_string()))?;
        
        match row {
            Some(row) => {
                let nonce_bytes: Vec<u8> = row.get("nonce");
                let mut nonce = [0u8; 12];
                nonce.copy_from_slice(&nonce_bytes);
                
                Ok(Some(EncryptedData {
                    nonce,
                    ciphertext: row.get("ciphertext"),
                    version: row.get::<i16, _>("version") as u8,
                }))
            }
            None => Ok(None),
        }
    }
    
    async fn delete(&self, id: &CredentialId) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM credentials WHERE id = $1")
            .bind(id.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
    ) -> Result<Vec<CredentialMetadata>, StorageError> {
        let mut query = String::from("SELECT metadata FROM credentials WHERE 1=1");
        
        if let Some(f) = filter {
            if let Some(owner) = &f.owner_id {
                query.push_str(&format!(" AND metadata->>'owner_id' = '{}'", owner.as_str()));
            }
            if let Some(cred_type) = &f.credential_type {
                query.push_str(&format!(" AND metadata->>'credential_type' = '{}'", cred_type));
            }
            if let Some(scope) = &f.scope_id {
                query.push_str(&format!(" AND metadata->>'scope_id' = '{}'", scope.as_str()));
            }
        }
        
        let rows = sqlx::query(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::ReadFailed(e.to_string()))?;
        
        let mut results = Vec::new();
        for row in rows {
            let metadata_json: serde_json::Value = row.get("metadata");
            let metadata: CredentialMetadata = serde_json::from_value(metadata_json)
                .map_err(|e| StorageError::DeserializationError(e.to_string()))?;
            results.push(metadata);
        }
        
        Ok(results)
    }
    
    async fn update_metadata(
        &self,
        id: &CredentialId,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        let metadata_json = serde_json::to_value(metadata)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        sqlx::query(
            r#"
            UPDATE credentials
            SET metadata = $1, updated_at = NOW()
            WHERE id = $2
            "#
        )
        .bind(metadata_json)
        .bind(id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::UpdateFailed(e.to_string()))?;
        
        Ok(())
    }
}
```

### Usage

```rust
use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize provider
    let provider = PostgresProvider::new("postgresql://user:pass@localhost/credentials").await?;
    provider.initialize_schema().await?;
    
    // Create manager with custom provider
    let manager = CredentialManager::new(Box::new(provider));
    
    // Use normally
    let api_key = ApiKeyCredential::new("sk_test_12345");
    let ctx = CredentialContext::new(OwnerId::new("alice"));
    let id = CredentialId::new();
    
    manager.store_credential(&id, &api_key, &ctx).await?;
    
    Ok(())
}
```

---

## Example 2: Redis Provider (Cached)

```rust
use async_trait::async_trait;
use nebula_credential::prelude::*;
use redis::{Client, AsyncCommands};
use serde_json;

pub struct RedisProvider {
    client: Client,
    ttl_seconds: usize,
}

impl RedisProvider {
    pub fn new(redis_url: &str, ttl_seconds: usize) -> Result<Self, StorageError> {
        let client = Client::open(redis_url)
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        Ok(Self { client, ttl_seconds })
    }
}

#[async_trait]
impl StorageProvider for RedisProvider {
    async fn store(
        &self,
        id: &CredentialId,
        encrypted: &EncryptedData,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        let mut conn = self.client.get_async_connection().await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        // Store encrypted data
        let data_key = format!("cred:{}:data", id.as_str());
        let data_json = serde_json::to_string(&encrypted)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        conn.set_ex(&data_key, data_json, self.ttl_seconds).await
            .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
        
        // Store metadata
        let meta_key = format!("cred:{}:meta", id.as_str());
        let meta_json = serde_json::to_string(metadata)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        conn.set_ex(&meta_key, meta_json, self.ttl_seconds).await
            .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
        
        // Add to index
        let index_key = format!("cred:owner:{}", metadata.owner_id.as_str());
        conn.sadd(index_key, id.as_str()).await
            .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn retrieve(
        &self,
        id: &CredentialId,
    ) -> Result<Option<EncryptedData>, StorageError> {
        let mut conn = self.client.get_async_connection().await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        let data_key = format!("cred:{}:data", id.as_str());
        let data_json: Option<String> = conn.get(&data_key).await
            .map_err(|e| StorageError::ReadFailed(e.to_string()))?;
        
        match data_json {
            Some(json) => {
                let encrypted: EncryptedData = serde_json::from_str(&json)
                    .map_err(|e| StorageError::DeserializationError(e.to_string()))?;
                Ok(Some(encrypted))
            }
            None => Ok(None),
        }
    }
    
    async fn delete(&self, id: &CredentialId) -> Result<(), StorageError> {
        let mut conn = self.client.get_async_connection().await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        let data_key = format!("cred:{}:data", id.as_str());
        let meta_key = format!("cred:{}:meta", id.as_str());
        
        conn.del(&[data_key, meta_key]).await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;
        
        Ok(())
    }
    
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
    ) -> Result<Vec<CredentialMetadata>, StorageError> {
        let mut conn = self.client.get_async_connection().await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        // Get all credential IDs for owner
        let owner_id = filter.and_then(|f| f.owner_id.as_ref())
            .ok_or_else(|| StorageError::ReadFailed("Owner filter required".to_string()))?;
        
        let index_key = format!("cred:owner:{}", owner_id.as_str());
        let ids: Vec<String> = conn.smembers(&index_key).await
            .map_err(|e| StorageError::ReadFailed(e.to_string()))?;
        
        let mut results = Vec::new();
        for id in ids {
            let meta_key = format!("cred:{}:meta", id);
            if let Some(meta_json) = conn.get::<_, Option<String>>(&meta_key).await
                .map_err(|e| StorageError::ReadFailed(e.to_string()))? 
            {
                let metadata: CredentialMetadata = serde_json::from_str(&meta_json)
                    .map_err(|e| StorageError::DeserializationError(e.to_string()))?;
                results.push(metadata);
            }
        }
        
        Ok(results)
    }
    
    async fn update_metadata(
        &self,
        id: &CredentialId,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        let mut conn = self.client.get_async_connection().await
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;
        
        let meta_key = format!("cred:{}:meta", id.as_str());
        let meta_json = serde_json::to_string(metadata)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;
        
        conn.set_ex(&meta_key, meta_json, self.ttl_seconds).await
            .map_err(|e| StorageError::UpdateFailed(e.to_string()))?;
        
        Ok(())
    }
}
```

---

## Example 3: Hybrid Provider (Local + Remote)

```rust
use async_trait::async_trait;
use nebula_credential::prelude::*;

pub struct HybridProvider {
    local: Box<dyn StorageProvider>,
    remote: Box<dyn StorageProvider>,
    cache_ttl: Duration,
}

impl HybridProvider {
    pub fn new(
        local: Box<dyn StorageProvider>,
        remote: Box<dyn StorageProvider>,
        cache_ttl: Duration,
    ) -> Self {
        Self { local, remote, cache_ttl }
    }
}

#[async_trait]
impl StorageProvider for HybridProvider {
    async fn store(
        &self,
        id: &CredentialId,
        encrypted: &EncryptedData,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        // Write to remote (source of truth)
        self.remote.store(id, encrypted, metadata).await?;
        
        // Cache locally
        self.local.store(id, encrypted, metadata).await?;
        
        Ok(())
    }
    
    async fn retrieve(
        &self,
        id: &CredentialId,
    ) -> Result<Option<EncryptedData>, StorageError> {
        // Try local cache first
        if let Some(data) = self.local.retrieve(id).await? {
            eprintln!("Cache hit for {id}");
            return Ok(Some(data));
        }
        
        // Fallback to remote
        eprintln!("Cache miss for {id}, fetching from remote");
        if let Some(data) = self.remote.retrieve(id).await? {
            // Populate cache
            let metadata = CredentialMetadata::new(
                id.clone(),
                "unknown", // Would need to store this
                OwnerId::new("unknown"),
            );
            self.local.store(id, &data, &metadata).await?;
            
            return Ok(Some(data));
        }
        
        Ok(None)
    }
    
    async fn delete(&self, id: &CredentialId) -> Result<(), StorageError> {
        // Delete from both
        self.remote.delete(id).await?;
        self.local.delete(id).await?;
        Ok(())
    }
    
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
    ) -> Result<Vec<CredentialMetadata>, StorageError> {
        // Always list from remote (source of truth)
        self.remote.list(filter).await
    }
    
    async fn update_metadata(
        &self,
        id: &CredentialId,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        // Update remote
        self.remote.update_metadata(id, metadata).await?;
        
        // Invalidate local cache
        self.local.delete(id).await?;
        
        Ok(())
    }
}
```

---

## Testing Your Provider

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_credential::prelude::*;
    
    #[tokio::test]
    async fn test_store_and_retrieve() {
        let provider = PostgresProvider::new("postgresql://localhost/test").await.unwrap();
        provider.initialize_schema().await.unwrap();
        
        let id = CredentialId::new();
        let encrypted = EncryptedData {
            nonce: [1u8; 12],
            ciphertext: vec![1, 2, 3, 4],
            version: 1,
        };
        let metadata = CredentialMetadata::new(
            id.clone(),
            "test",
            OwnerId::new("alice"),
        );
        
        // Store
        provider.store(&id, &encrypted, &metadata).await.unwrap();
        
        // Retrieve
        let retrieved = provider.retrieve(&id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().ciphertext, encrypted.ciphertext);
    }
    
    #[tokio::test]
    async fn test_list_with_filter() {
        let provider = PostgresProvider::new("postgresql://localhost/test").await.unwrap();
        
        let filter = CredentialFilter::new()
            .owner(OwnerId::new("alice"))
            .credential_type("api_key");
        
        let results = provider.list(Some(&filter)).await.unwrap();
        assert!(!results.is_empty());
    }
}
```

---

## Best Practices

### 1. Connection Pooling

```rust
// Use connection pools for database providers
let pool = PgPoolOptions::new()
    .max_connections(10)
    .min_connections(2)
    .connect(database_url)
    .await?;
```

### 2. Proper Indexing

```sql
-- Index commonly filtered fields
CREATE INDEX idx_owner ON credentials ((metadata->>'owner_id'));
CREATE INDEX idx_type ON credentials ((metadata->>'credential_type'));
CREATE INDEX idx_scope ON credentials ((metadata->>'scope_id'));
CREATE INDEX idx_created ON credentials (created_at);
```

### 3. Error Handling

```rust
async fn store(&self, ...) -> Result<(), StorageError> {
    sqlx::query(...)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            eprintln!("Store failed: {e}");
            StorageError::WriteFailed(e.to_string())
        })?;
    Ok(())
}
```

### 4. Transaction Support

```rust
async fn atomic_update(
    &self,
    id: &CredentialId,
    encrypted: &EncryptedData,
    metadata: &CredentialMetadata,
) -> Result<(), StorageError> {
    let mut tx = self.pool.begin().await
        .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
    
    // Perform operations in transaction
    sqlx::query("UPDATE credentials SET ...")
        .execute(&mut tx)
        .await?;
    
    tx.commit().await
        .map_err(|e| StorageError::WriteFailed(e.to_string()))?;
    
    Ok(())
}
```

---

## Related Documentation

- [[../Integrations/Local-Storage]] - SQLite reference implementation
- [[../Integrations/AWS-Secrets-Manager]] - Cloud provider example
- [[../Integrations/Migration-Guide]] - Migrating between providers
- [[../Reference/API-Reference]] - Complete API documentation
- [[../Architecture]] - System architecture overview

---

## Summary

Custom providers enable:
✅ Integration with any storage backend  
✅ Hybrid caching strategies  
✅ Custom indexing and query patterns  
✅ Provider-specific optimizations  
✅ Migration flexibility  

Key implementation points:
- Data is already encrypted before reaching provider
- Must be `Send + Sync + async`
- Proper error handling with `StorageError`
- Connection pooling for performance
- Transaction support for consistency
