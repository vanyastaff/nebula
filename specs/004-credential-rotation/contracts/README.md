# Credential Rotation API Contracts

**Feature**: Phase 4 - Credential Rotation  
**Primary Contract**: Rust API (this phase)  
**Secondary Contract**: HTTP REST API (future work in `nebula-api`)

---

## PRIMARY: Rust API (This Implementation)

The `nebula-credential` crate is a **Rust library** in the Node Layer. The primary contract is the **Rust public API** that other crates will use.

### Core Rotation API

Implemented as methods on `CredentialManager`:

```rust
// In crates/nebula-credential/src/rotation/mod.rs

impl CredentialManager {
    /// Configure automatic rotation policy for a credential
    pub async fn set_rotation_policy(
        &self,
        id: &CredentialId,
        policy: RotationPolicy,
    ) -> Result<(), RotationError>;

    /// Manually trigger rotation immediately (Manual policy)
    pub async fn rotate_now(
        &self,
        id: &CredentialId,
        reason: RotationReason,
        no_grace_period: bool,
    ) -> Result<RotationTransaction, RotationError>;

    /// Get current rotation status for a credential
    pub async fn get_rotation_status(
        &self,
        id: &CredentialId,
    ) -> Result<Option<RotationTransaction>, RotationError>;

    /// Manually rollback an in-progress rotation
    pub async fn rollback_rotation(
        &self,
        id: &CredentialId,
    ) -> Result<(), RotationError>;

    /// Get rotation history (audit log) for a credential
    pub async fn get_rotation_history(
        &self,
        id: &CredentialId,
    ) -> Result<Vec<RotationEvent>, RotationError>;
    
    /// Retrieve encrypted backup for disaster recovery
    pub async fn get_rotation_backup(
        &self,
        backup_id: &BackupId,
    ) -> Result<RotationBackup, RotationError>;
}
```

### Scheduler API

Background task management for automatic rotation:

```rust
// In crates/nebula-credential/src/rotation/scheduler.rs

pub struct RotationScheduler {
    // Internal state
}

impl RotationScheduler {
    /// Create new scheduler with reference to credential manager
    pub fn new(manager: Arc<CredentialManager>) -> Self;

    /// Start background scheduler (monitors policies, triggers rotations)
    pub async fn start(&self) -> Result<(), RotationError>;

    /// Stop background scheduler gracefully
    pub async fn stop(&self) -> Result<(), RotationError>;
    
    /// Get list of credentials approaching rotation (observability)
    pub async fn pending_rotations(&self) -> Result<Vec<PendingRotation>, RotationError>;
}
```

### Key Types

All types defined in `../data-model.md`:

- `RotationPolicy` - Periodic | BeforeExpiry | Scheduled | Manual
- `RotationTransaction` - State machine for rotation execution
- `RotationState` - Pending → Creating → Validating → Committing → Committed | RolledBack
- `RotationEvent` - Audit log entry
- `RotationBackup` - Encrypted backup of rotated credential
- `RotationReason` - SecurityIncident | ComplianceAudit | PersonnelChange | UserRequested | Testing

---

## SECONDARY: HTTP REST API (Future - Out of Scope)

**IMPORTANT**: HTTP endpoints are **NOT part of this phase**. They will be implemented in `nebula-api` crate (Presentation Layer) in future work.

### Future HTTP Endpoints (Reference Only)

These endpoints will call the Rust API defined above:

```http
# Rotation Policy Management
PUT    /api/credentials/{id}/rotation/policy
GET    /api/credentials/{id}/rotation/policy

# Manual Rotation Control
POST   /api/credentials/{id}/rotation/rotate
POST   /api/credentials/{id}/rotation/rollback

# Status & Observability
GET    /api/credentials/{id}/rotation/status
GET    /api/credentials/{id}/rotation/history
GET    /api/credentials/{id}/rotation/backups/{backup_id}

# Scheduler Control (admin only)
GET    /api/rotation/scheduler/status
POST   /api/rotation/scheduler/start
POST   /api/rotation/scheduler/stop
GET    /api/rotation/scheduler/pending
```

### Example Implementation (Future Work)

This code belongs in `nebula-api` crate, NOT `nebula-credential`:

```rust
// In crates/nebula-api/src/routes/rotation.rs (FUTURE WORK)

#[post("/api/credentials/{id}/rotation/rotate")]
async fn trigger_rotation(
    id: Path<CredentialId>,
    payload: Json<RotationRequest>,
    credential_manager: Data<Arc<CredentialManager>>, // From nebula-credential
) -> Result<Json<RotationResponse>> {
    // Validate request
    let reason = payload.reason.clone();
    let no_grace_period = payload.no_grace_period;
    
    // Call business logic from nebula-credential
    let transaction = credential_manager
        .rotate_now(&id, reason, no_grace_period)
        .await
        .map_err(|e| ApiError::from(e))?;
    
    // Return HTTP response
    Ok(Json(RotationResponse::from(transaction)))
}
```

---

## Architecture Boundaries

```
┌───────────────────────────────────────────────┐
│         nebula-api (Presentation Layer)       │
│              FUTURE WORK                      │
│  - HTTP endpoints                             │
│  - Request/Response serialization             │
│  - Authentication/Authorization               │
│  - Calls nebula-credential Rust API ↓         │
└───────────────────────────────────────────────┘
                      ↓
┌───────────────────────────────────────────────┐
│        nebula-credential (Node Layer)         │
│              THIS PHASE                       │
│  - Rust public API (CredentialManager)        │
│  - Business logic (rotation policies)         │
│  - Validation, grace period, transactions     │
│  - Uses nebula-storage StorageProvider ↓      │
└───────────────────────────────────────────────┘
                      ↓
┌───────────────────────────────────────────────┐
│      nebula-storage (Infrastructure Layer)    │
│            EXTENDS PHASE 2 WORK               │
│  - StorageProvider trait extensions           │
│  - SQL migrations (rotation_transactions)     │
│  - Database queries (save/load state)         │
└───────────────────────────────────────────────┘
```

---

## Contract Stability

**Rust API**: Stable public API following semantic versioning
- Breaking changes require major version bump
- Deprecation warnings before removal
- Extension methods preferred over breaking changes

**HTTP API**: Not yet defined (future work)
- Will follow REST best practices
- Will use OpenAPI 3.0 specification
- Will version endpoints (/api/v1/...)

---

## See Also

- `../data-model.md` - Complete type definitions and validation rules
- `../quickstart.md` - Usage examples with Rust API
- `../plan.md` - Architecture decisions and crate responsibilities
