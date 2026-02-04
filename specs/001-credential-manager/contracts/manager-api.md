# Credential Manager API Contract

**Feature**: Credential Manager  
**Version**: 1.0.0  
**Date**: 2026-02-04

## Core CRUD Operations

### store
**Purpose**: Store credential with encryption  
**Input**: `(CredentialId, Credential)`  
**Output**: `Result<(), ManagerError>`  
**Behavior**: Encrypts credential, stores via provider, invalidates cache  
**Errors**: `StorageError` if provider fails

### retrieve
**Purpose**: Retrieve credential (cache-aside pattern)  
**Input**: `CredentialId`  
**Output**: `Result<Option<Credential>, ManagerError>`  
**Behavior**: Check cache → on miss, fetch from storage → populate cache  
**Performance**: <10ms cache hit, <100ms cache miss  
**Errors**: `StorageError` if provider fails

### delete
**Purpose**: Delete credential and invalidate cache  
**Input**: `CredentialId`  
**Output**: `Result<(), ManagerError>`  
**Behavior**: Delete from storage, invalidate cache entry  
**Errors**: `StorageError` if provider fails

### list
**Purpose**: List all credential IDs  
**Input**: None  
**Output**: `Result<Vec<CredentialId>, ManagerError>`  
**Behavior**: Delegate to storage provider (no caching)  
**Errors**: `StorageError` if provider fails

## Scope Operations

### retrieve_scoped
**Purpose**: Retrieve credential within scope  
**Input**: `(CredentialId, ScopeId)`  
**Output**: `Result<Option<Credential>, ManagerError>`  
**Behavior**: Retrieve + validate scope matches  
**Errors**: `ScopeViolation` if scope mismatch

### list_scoped
**Purpose**: List credentials in scope hierarchy  
**Input**: `ScopeId`  
**Output**: `Result<Vec<CredentialId>, ManagerError>`  
**Behavior**: Filter by scope prefix

## Batch Operations

### store_batch
**Purpose**: Store multiple credentials in parallel  
**Input**: `Vec<(CredentialId, Credential)>`  
**Output**: `Vec<Result<(), ManagerError>>`  
**Behavior**: JoinSet with bounded concurrency (default 10)  
**Performance**: 50%+ faster than sequential

### retrieve_batch
**Purpose**: Retrieve multiple credentials  
**Input**: `Vec<CredentialId>`  
**Output**: `Vec<Result<Option<Credential>, ManagerError>>`  
**Behavior**: Check cache for all, batch fetch misses

### delete_batch
**Purpose**: Delete multiple credentials  
**Input**: `Vec<CredentialId>`  
**Output**: `Vec<Result<(), ManagerError>>`  
**Behavior**: Parallel deletion + cache invalidation

## Validation

### validate
**Purpose**: Validate single credential  
**Input**: `CredentialId`  
**Output**: `Result<ValidationResult, ManagerError>`  
**Behavior**: Check expiration, format, rotation need

### validate_batch
**Purpose**: Validate multiple credentials  
**Input**: `Vec<CredentialId>`  
**Output**: `Vec<ValidationResult>`  
**Behavior**: Parallel validation

## Cache Management

### clear_cache
**Purpose**: Clear all cached credentials  
**Output**: `Result<(), ManagerError>`

### clear_cache_for
**Purpose**: Clear specific credential from cache  
**Input**: `CredentialId`  
**Output**: `Result<(), ManagerError>`

### cache_stats
**Purpose**: Get cache performance metrics  
**Output**: `Option<CacheStats>`  
**Returns**: `None` if caching disabled

## Builder API

### CredentialManager::builder()
**Purpose**: Create builder instance  
**Output**: `CredentialManagerBuilder<No>`

### builder.storage(provider)
**Purpose**: Set storage provider (required)  
**Input**: `Arc<dyn StorageProvider>`  
**Output**: `CredentialManagerBuilder<Yes>`  
**Compile Error**: Cannot build without this

### builder.cache_ttl(duration)
**Purpose**: Enable cache with TTL  
**Input**: `Duration`  
**Output**: `Self` (chainable)

### builder.cache_max_size(size)
**Purpose**: Set cache capacity  
**Input**: `usize`  
**Output**: `Self` (chainable)

### builder.batch_concurrency(limit)
**Purpose**: Set batch operation concurrency  
**Input**: `usize` (default 10)  
**Output**: `Self` (chainable)

### builder.build()
**Purpose**: Construct manager  
**Output**: `CredentialManager`  
**Requirement**: Must have storage set (compile-time check)
