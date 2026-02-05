# Implementation Plan: Credential Rotation

**Branch**: `004-credential-rotation` | **Date**: 2026-02-04 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/004-credential-rotation/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Implement automatic credential rotation with zero downtime, supporting four rotation policies (Periodic, Before-Expiry, Scheduled, Manual) with grace period management. The system validates new credentials before committing rotation, automatically rolls back on validation failure, and maintains encrypted backups for disaster recovery. Rotation is observable through comprehensive metrics and audit logging, with support for blue-green deployment patterns for high-availability credentials (databases, certificates, OAuth2 tokens).

**Primary Technical Approach**: Background scheduler monitors credential TTL and policy conditions, triggers rotation transactions using two-phase commit pattern (create+validate → commit or rollback), maintains both old and new credentials during configurable grace period, and supports credential-specific validation tests following n8n pattern (actual functionality testing via provider-specific endpoints).

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)

**Primary Dependencies**: 
- Tokio async runtime (background scheduler, async rotation operations)
- chrono (timestamp handling, TTL calculations, expiration tracking)
- tokio-cron-scheduler or similar (periodic policy scheduling with jitter)
- serde (rotation policy serialization, backup storage)
- thiserror (rotation-specific error types)

**Storage**: Builds on Phase 2 storage providers (requires durable storage for rotation state, backups, audit logs)

**Testing**: 
- `cargo test --workspace` for unit tests
- `#[tokio::test(flavor = "multi_thread")]` for async rotation tests
- `tokio::time::pause()` and `tokio::time::advance()` for time-based policy simulation
- Integration tests with mock credentials and storage

**Target Platform**: Cross-platform (Windows primary development, Linux/macOS support)

**Project Type**: Workspace - adds rotation capability to `nebula-credential` crate (Phase 4 builds on Phases 1-3)

**Performance Goals**: 
- Rotation completes in <5 minutes for database credentials (SC-004)
- Before-Expiry rotation triggers within 60 seconds of threshold (SC-003)
- System handles 1000 concurrent rotations with <10% performance degradation (SC-005)
- Validation completes within 30 seconds timeout (FR-057)

**Constraints**:
- Zero downtime requirement: 100% authentication success during grace period (SC-002)
- Rotation must be atomic: never leave credentials in invalid intermediate state
- Rollback must complete within 30 seconds of validation failure (SC-007)
- Storage operations must use durable storage (no in-memory-only state for active rotations)

**Scale/Scope**:
- Support 10,000+ credentials under rotation management (SC-010)
- Grace periods from 0 seconds (emergency) to 90 days (client migrations)
- Rotation policies support intervals from hours (OAuth2 tokens) to years (certificates)
- Backup retention minimum 30 days with encrypted storage (FR-039)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with `.specify/memory/constitution.md` principles:

- [x] **Type Safety First**: 
  - ✅ Feature uses newtype patterns: `RotationId`, `BackupId`, `GracePeriodDuration`
  - ✅ Enums for exhaustive state: `RotationState` (Pending→Creating→Validating→Committing→Committed/RolledBack)
  - ✅ Custom types avoid String: `RotationPolicy`, `RetryPolicy`, `ValidationTest`
  - ✅ Sized types in aliases (no `str` usage)

- [x] **Isolated Error Handling**: 
  - ✅ `nebula-credential` crate defines `RotationError` with thiserror
  - ✅ Errors converted at boundaries: storage errors → rotation errors with context
  - ✅ Actionable messages required (FR-064: failure reason + remediation steps)

- [x] **Test-Driven Development**: 
  - ✅ Test strategy defined:
    - Unit tests: Policy evaluation logic (periodic interval, TTL calculations)
    - Integration tests: Full rotation cycle with mock credentials
    - Time-based tests: Grace period expiration using `tokio::time` control
    - Rollback tests: Validation failure scenarios trigger automatic rollback

- [x] **Async Discipline**: 
  - ✅ Cancellation: Rotation scheduler uses `tokio::select!` for graceful shutdown
  - ✅ Timeouts: Validation 30s (FR-057), storage operations 5s, HTTP requests 10s
  - ✅ `JoinSet` for parallel validation of multiple credentials
  - ✅ Bounded channels for rotation queue (prevent memory exhaustion during rotation storms)
  - ✅ `RwLock` for rotation state (read-heavy: many readers check state, few writers update)

- [x] **Modular Architecture**: 
  - ✅ Extends `nebula-credential` crate (Phase 4 builds on Phases 1-3)
  - ✅ Dependencies respect layers: uses `nebula-core` (CredentialId), `nebula-log` (tracing)
  - ✅ No circular dependencies: rotation depends on storage (Phase 2), not vice versa

- [x] **Observability**: 
  - ✅ Logging strategy: `tracing` with structured fields (credential_id, rotation_id, policy_type, state)
  - ✅ Metrics: rotation_duration, success_rate, active_grace_periods, validation_failures (FR-028 enhanced)
  - ✅ Audit trail: All rotation events logged with timestamp, actor, reason (FR-030)
  - ✅ Trace state transitions: Pending→Creating→Validating→Committing→Committed

- [x] **Simplicity**: 
  - ✅ Complexity justified:
    - Two-phase commit: Required for atomic rotation with rollback (prevents invalid credential states)
    - Background scheduler: Required for automatic policy-driven rotation (manual-only would violate compliance requirements)
    - Grace period: Required for zero-downtime (production requirement, not speculative)
    - Four rotation policies: Driven by compliance (SOC2, PCI-DSS) and operational needs (certificates, OAuth2)
  - ✅ No speculative features: MFA, SSH keys, approval workflows explicitly marked out of scope

**Constitution Compliance**: ✅ PASSED - All principles satisfied, complexity justified by concrete requirements

## Project Structure

### Documentation (this feature)

```text
specs/004-credential-rotation/
├── plan.md              # This file (/speckit.plan command output)
├── spec.md              # Feature specification (complete - 64 FRs, 9 user stories)
├── checklists/
│   └── requirements.md  # Specification quality checklist (validated)
├── research.md          # Phase 0 output - COMPLETED
├── data-model.md        # Phase 1 output - COMPLETED
├── quickstart.md        # Phase 1 output - COMPLETED
├── contracts/           # Phase 1 output - COMPLETED
└── tasks.md             # Phase 2 output (/speckit.tasks command) - NOT YET
```

### Source Code - Crate Responsibilities

**IMPORTANT**: Rotation feature spans multiple crates following Nebula architecture layers.

#### `nebula-credential` (Node Layer) ← **PRIMARY SCOPE OF THIS PHASE**

**Responsibility**: Business logic for credential rotation

```text
crates/nebula-credential/
├── src/
│   ├── lib.rs                          # Re-export rotation module
│   ├── rotation/                       # NEW: Phase 4 module
│   │   ├── mod.rs                      # Module exports
│   │   ├── policy.rs                   # Rotation policies (Periodic, BeforeExpiry, Scheduled, Manual)
│   │   ├── scheduler.rs                # Background task monitoring credentials
│   │   ├── transaction.rs              # Two-phase commit rotation logic
│   │   ├── state.rs                    # RotationState enum and transitions
│   │   ├── grace_period.rs             # Grace period management
│   │   ├── validation.rs               # Credential validation tests (n8n pattern)
│   │   ├── backup.rs                   # Rotation backups for disaster recovery
│   │   ├── blue_green.rs               # Blue-green deployment support
│   │   ├── retry.rs                    # Retry policy and exponential backoff
│   │   └── metrics.rs                  # Rotation metrics and observability
│   ├── types/                          # Existing from Phase 1
│   ├── storage/                        # Existing from Phase 2
│   ├── manager.rs                      # Existing from Phase 3 - MODIFY for rotation integration
│   └── error.rs                        # Existing - EXTEND with RotationError variants
├── tests/
│   └── rotation_tests.rs               # NEW: Integration tests for rotation
├── examples/
│   ├── database_rotation.rs            # NEW: Blue-green database rotation example
│   ├── oauth2_refresh.rs               # NEW: OAuth2 token refresh example
│   └── certificate_renewal.rs          # NEW: X.509 certificate rotation example
└── Cargo.toml                          # UPDATE dependencies (chrono, tokio)
```

**Public API** (Rust functions for other crates):
```rust
// In nebula-credential/src/rotation/mod.rs
impl CredentialManager {
    pub async fn set_rotation_policy(&self, id: &CredentialId, policy: RotationPolicy) -> Result<()>;
    pub async fn rotate_now(&self, id: &CredentialId, reason: RotationReason, no_grace_period: bool) -> Result<RotationTransaction>;
    pub async fn get_rotation_status(&self, id: &CredentialId) -> Result<Option<RotationTransaction>>;
    pub async fn rollback_rotation(&self, id: &CredentialId) -> Result<()>;
    pub async fn get_rotation_history(&self, id: &CredentialId) -> Result<Vec<RotationEvent>>;
}

pub struct RotationScheduler { /* ... */ }
impl RotationScheduler {
    pub fn new(manager: Arc<CredentialManager>) -> Self;
    pub async fn start(&self) -> Result<()>;
    pub async fn stop(&self) -> Result<()>;
}
```

**Does NOT contain**:
- ❌ HTTP endpoints (that's `nebula-api`)
- ❌ SQL schema definitions (that's `nebula-storage`)
- ❌ Database queries (uses `StorageProvider` trait from Phase 2)

---

#### `nebula-storage` (Infrastructure Layer) ← **OUT OF SCOPE** (Phase 2 work)

**Responsibility**: Database persistence for rotation state

```text
crates/nebula-storage/
├── migrations/
│   └── 004_rotation_tables.sql         # NEW: CREATE TABLE rotation_transactions, rotation_backups, rotation_events
├── src/
│   ├── rotation/
│   │   ├── transaction_store.rs        # Impl StorageProvider for RotationTransaction
│   │   ├── backup_store.rs             # Impl StorageProvider for RotationBackup
│   │   └── event_store.rs              # Impl StorageProvider for RotationEvent
│   └── lib.rs
└── Cargo.toml
```

**Example implementation** (for context, not part of this phase):
```rust
// In nebula-storage
impl StorageProvider for PostgresStorage {
    async fn save_rotation_transaction(&self, tx: &RotationTransaction) -> Result<()> {
        sqlx::query!(
            "INSERT INTO rotation_transactions (id, credential_id, state, ...) 
             VALUES ($1, $2, $3, ...) 
             ON CONFLICT (id) DO UPDATE SET state = $3, ...",
            tx.id.as_str(), tx.credential_id.as_str(), tx.state.to_string()
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
```

**Note**: Phase 2 `StorageProvider` trait already exists. This phase extends it with rotation-specific methods.

---

#### `nebula-api` (Presentation Layer) ← **OUT OF SCOPE** (future work)

**Responsibility**: REST API endpoints for rotation

```text
crates/nebula-api/
├── src/
│   ├── routes/
│   │   └── credentials.rs              # NEW: POST /credentials/{id}/rotation/rotate
│   │   └── rotation.rs                 # NEW: GET /credentials/{id}/rotation/status
│   └── lib.rs
└── Cargo.toml                          # Depends on nebula-credential
```

**Example endpoint** (for context, not part of this phase):
```rust
// In nebula-api
#[post("/credentials/{id}/rotation/rotate")]
async fn trigger_rotation(
    id: Path<CredentialId>,
    payload: Json<RotationRequest>,
    credential_manager: Data<Arc<CredentialManager>>,  // From nebula-credential
) -> Result<Json<RotationResponse>> {
    // Call business logic from nebula-credential
    let transaction = credential_manager
        .rotate_now(&id, payload.reason, payload.no_grace_period)
        .await?;
    
    Ok(Json(RotationResponse::from(transaction)))
}
```

---

### Additional Documentation (existing from Phases 1-3)

```text
crates/nebula-credential/docs/
├── How-To/Rotate-Credentials.md        # Already exists - reference material
├── Advanced/Rotation-Policies.md       # Already exists - reference material
├── Examples/*.md                       # Already exists - reference examples
└── Troubleshooting/Rotation-Failures.md # Already exists - reference material
```

---

### Structure Decision

**Chosen Approach**: Extend existing `nebula-credential` crate with `rotation/` module (modular within single crate)

**Rationale**:
- ✅ Follows Constitution Principle V (Modular Architecture): Rotation is a cohesive feature within the credential management domain
- ✅ Avoids creating 17th crate: Would violate simplicity (rotation is not independently usable without credential manager)
- ✅ Respects layer boundaries: Rotation depends on storage (Phase 2) and manager (Phase 3), extends credential functionality
- ✅ Single deployment unit: Rotation and credential management evolve together (rotation policies tied to credential types)

**Architectural Layer**: Node Layer (`nebula-credential`) - credential lifecycle management including rotation

**Why not a separate `nebula-rotation` crate?**
- Rotation requires deep integration with `CredentialManager` (state transitions, storage access, validation)
- Separating would create tight coupling requiring frequent cross-crate changes
- Rotation is not independently useful (no rotation without credentials to rotate)
- Module-level separation provides sufficient boundaries without deployment overhead

**Cross-Crate Integration**:
- `nebula-credential` provides Rust API → `nebula-api` calls it for HTTP endpoints
- `nebula-credential` uses `StorageProvider` trait → `nebula-storage` implements persistence
- `nebula-credential` emits events → `nebula-eventbus` distributes to subscribers
- `nebula-credential` uses metrics → `nebula-metrics` collects and exports

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

*No violations - Constitution Check passed cleanly. All complexity (two-phase commit, background scheduler, grace period, four policies) justified in Principle VII check above with concrete requirements from spec (compliance, zero-downtime, validation safety).*

---

## Phase 0: Research & Technical Discovery

**Objective**: Resolve all unknowns from Technical Context and make technology selection decisions

### Research Tasks

#### RT-001: Rotation Scheduling Library Selection

**Question**: Which Rust cron/scheduling library best supports rotation policies with jitter and distributed coordination?

**Options to evaluate**:
1. `tokio-cron-scheduler` - Tokio-based cron with async support
2. `cron` + custom `tokio::time` intervals - Manual scheduling
3. `apscheduler` port or similar - Advanced scheduling patterns

**Evaluation criteria**:
- Jitter support (±10% randomization for periodic policy)
- Async/await compatibility
- Distributed locking integration (prevent concurrent rotation)
- Dynamic schedule updates (policies change at runtime)

**Research outcome**: NEEDS RESEARCH

#### RT-002: Credential-Specific Validation Patterns

**Question**: How do we implement extensible validation tests following n8n pattern for different credential types?

**Design considerations**:
- Trait-based validation: `CredentialValidator` trait with type-specific implementations
- Test endpoint configuration: Where to store provider-specific test endpoints (config file, database, hardcoded)?
- Timeout handling: Per-credential-type timeouts (database 5s, OAuth2 10s, certificate 15s)
- Mock validation for testing: How to inject test doubles for validation without production API calls

**Research outcome**: NEEDS RESEARCH

#### RT-003: Two-Phase Commit State Persistence

**Question**: How to persist rotation transaction state for crash recovery (process restarts mid-rotation)?

**Design considerations**:
- State storage: Store transaction state in same StorageProvider as credentials, or separate?
- Recovery on restart: How to detect incomplete transactions and resume/rollback?
- Idempotency: Ensure retrying partial rotation doesn't create duplicate credentials
- State machine persistence: Serialize `RotationState` enum with transaction metadata

**Research outcome**: NEEDS RESEARCH

#### RT-004: Grace Period Implementation Strategy

**Question**: How to maintain two active credentials (old + new) during grace period without breaking existing code?

**Design considerations**:
- Credential versioning: Does `CredentialManager.retrieve()` return latest version only, or support version parameter?
- Backward compatibility: Existing code expects single credential, how to handle grace period transparently?
- Revocation mechanism: How to mark old credential for deletion without immediate removal?
- Storage schema: Store versions as separate entries or single entry with version array?

**Research outcome**: NEEDS RESEARCH

#### RT-005: Blue-Green Rotation for Databases

**Question**: How to create new database user with identical privileges to old user across different database systems?

**Design considerations**:
- Privilege introspection: SQL queries to enumerate grants for old user (PostgreSQL `pg_roles`, MySQL `SHOW GRANTS`)
- DDL generation: Generate `GRANT` statements for new user mirroring old user
- Database-specific differences: PostgreSQL roles vs MySQL users, schema ownership
- Atomic privilege transfer: Transaction support for user creation + grants

**Research outcome**: NEEDS RESEARCH

#### RT-006: Certificate Renewal CA Integration

**Question**: How to integrate with multiple CA types (AWS Private CA, Vault PKI, self-signed) using unified interface?

**Design considerations**:
- CA abstraction: Trait for certificate issuance (`CertificateAuthority` trait?)
- Provider implementations: AWS SDK, Vault HTTP API, OpenSSL for self-signed
- Certificate validation: Chain verification, trust store integration
- Rate limiting: Handle CA rate limits (Let's Encrypt 50 certs/week) with retry backoff

**Research outcome**: NEEDS RESEARCH

#### RT-007: Rotation Metrics and Observability

**Question**: Which metrics library to use and what metrics structure provides actionable operational visibility?

**Options**:
1. `prometheus` crate - Direct Prometheus integration
2. `metrics` crate - Vendor-agnostic metrics facade
3. Custom metrics via `tracing` events

**Key metrics to track** (from FR-028):
- Rotation duration histogram (p50, p95, p99)
- Success/failure counters by policy type
- Active grace periods gauge
- Credentials approaching expiry gauge
- Validation failure rate counter
- Rollback count counter

**Research outcome**: NEEDS RESEARCH

#### RT-008: Distributed Locking for Concurrent Rotation Prevention

**Question**: How to prevent multiple instances from rotating same credential simultaneously (FR-021)?

**Options**:
1. Storage-provider-based locking: Use StorageProvider conditional writes
2. External distributed lock: Redis/Consul/etcd
3. Database advisory locks: PostgreSQL `pg_advisory_lock`
4. Optimistic locking: Version numbers with compare-and-swap

**Evaluation criteria**:
- Works with all storage providers (Local, AWS, Azure, Vault, K8s)
- Lock timeout and automatic release
- Fault tolerance (lock holder crashes)

**Research outcome**: NEEDS RESEARCH

### Research Deliverables

File: `specs/004-credential-rotation/research.md`

Expected structure:
```markdown
# Rotation Implementation Research

## Technology Selections

### Scheduling Library
**Decision**: [chosen library]
**Rationale**: [why chosen]
**Alternatives considered**: [what else evaluated]
**Integration approach**: [how to integrate]

### Validation Pattern
**Decision**: [trait design]
**Rationale**: [extensibility vs simplicity]
**Example implementations**: [database, OAuth2, certificate]

### State Persistence
**Decision**: [storage approach]
**Rationale**: [recovery guarantees]
**Recovery algorithm**: [on restart, how to resume]

### Grace Period Implementation
**Decision**: [versioning approach]
**Rationale**: [backward compatibility]
**API changes**: [CredentialManager modifications]

### Blue-Green Database Rotation
**Decision**: [privilege enumeration approach]
**Rationale**: [cross-database compatibility]
**Supported databases**: [PostgreSQL, MySQL, others]

### CA Integration
**Decision**: [abstraction design]
**Rationale**: [extensibility for new CAs]
**Initial implementations**: [AWS, Vault, self-signed]

### Metrics Library
**Decision**: [chosen library]
**Rationale**: [ecosystem fit]
**Metrics spec**: [specific metrics to track]

### Distributed Locking
**Decision**: [locking mechanism]
**Rationale**: [works with all storage providers]
**Fallback strategy**: [if lock acquisition fails]
```

---

## Phase 1: Data Model & Contracts

**Objective**: Define types, state machines, and API contracts based on research decisions

### Data Model

File: `specs/004-credential-rotation/data-model.md`

**Expected entities** (from spec.md Key Entities section):

1. **RotationPolicy** - When to rotate
   - Periodic: interval, grace_period, enable_jitter
   - BeforeExpiry: threshold_percentage, minimum_time_before_expiry, grace_period
   - Scheduled: scheduled_at, grace_period, notify_before
   - Manual: reason (enum: SecurityIncident, ComplianceAudit, PersonnelChange, UserRequested, Testing)

2. **RotationTransaction** - State tracking
   - transaction_id: RotationId
   - credential_id: CredentialId
   - state: RotationState (enum)
   - old_credential_version: u32
   - new_credential_version: Option<u32>
   - started_at: DateTime<Utc>
   - completed_at: Option<DateTime<Utc>>
   - validation_result: Option<ValidationResult>
   - backup_id: Option<BackupId>

3. **RotationState** - State machine
   - Pending: Queued for rotation
   - Creating: Generating new credential
   - Validating: Testing new credential
   - Committing: Storing new credential
   - Committed: Rotation complete, grace period active
   - RolledBack: Validation failed, restored old credential
   - Transitions: Pending→Creating→Validating→Committing→Committed OR →RolledBack at any stage

4. **GracePeriodConfig**
   - duration: Duration (0 seconds to 90 days)
   - warning_threshold: Duration (alert before expiry)
   - auto_revoke: bool (automatic old credential deletion)

5. **RotationBackup**
   - backup_id: BackupId
   - credential_id: CredentialId
   - credential_version: u32
   - encrypted_data: Vec<u8>
   - created_at: DateTime<Utc>
   - expires_at: DateTime<Utc> (30-day minimum retention)
   - transaction_id: RotationId

6. **ValidationTest** - Credential-specific test
   - test_method: TestMethod (HttpRequest, DatabaseQuery, TlsHandshake)
   - endpoint: String (test URL or query)
   - expected_criteria: SuccessCriteria (2xx response, query result, valid handshake)
   - timeout: Duration
   - retry_policy: RetryPolicy

7. **RetryPolicy**
   - max_attempts: u32
   - initial_backoff: Duration
   - backoff_multiplier: f32
   - max_backoff: Duration
   - retriable_errors: Vec<ErrorType>

8. **BlueGreenState** (for database rotation)
   - blue_credential: CredentialId (old)
   - green_credential: Option<CredentialId> (new)
   - active_color: Color (Blue | Green)
   - traffic_percentage: HashMap<Color, u8>
   - error_rate: HashMap<Color, f32>
   - instance_migration: HashMap<InstanceId, Color>

9. **RotationEvent** - Audit log
   - event_id: EventId
   - event_type: EventType (Started, Completed, Failed, RolledBack, GracePeriodExpired)
   - credential_id: CredentialId
   - rotation_id: RotationId
   - timestamp: DateTime<Utc>
   - actor: Actor (System | User(UserId))
   - reason: Option<RotationReason>
   - metadata: HashMap<String, String>

### API Contracts

**PRIMARY CONTRACT**: Rust API (this phase)

File: `specs/004-credential-rotation/contracts/README.md`

The primary contract for credential rotation is the **Rust public API** in `nebula-credential` crate:

```rust
// Core rotation API
impl CredentialManager {
    pub async fn set_rotation_policy(
        &self, 
        id: &CredentialId, 
        policy: RotationPolicy
    ) -> Result<(), RotationError>;
    
    pub async fn rotate_now(
        &self, 
        id: &CredentialId, 
        reason: RotationReason, 
        no_grace_period: bool
    ) -> Result<RotationTransaction, RotationError>;
    
    pub async fn get_rotation_status(
        &self, 
        id: &CredentialId
    ) -> Result<Option<RotationTransaction>, RotationError>;
    
    pub async fn rollback_rotation(
        &self, 
        id: &CredentialId
    ) -> Result<(), RotationError>;
    
    pub async fn get_rotation_history(
        &self, 
        id: &CredentialId
    ) -> Result<Vec<RotationEvent>, RotationError>;
}

// Scheduler API
pub struct RotationScheduler {
    // ...
}

impl RotationScheduler {
    pub fn new(manager: Arc<CredentialManager>) -> Self;
    pub async fn start(&self) -> Result<(), RotationError>;
    pub async fn stop(&self) -> Result<(), RotationError>;
}
```

**SECONDARY CONTRACT**: HTTP REST API (future work in `nebula-api`)

File: `specs/004-credential-rotation/contracts/http-api-reference.md` (for documentation only)

**NOTE**: HTTP endpoints will be implemented in `nebula-api` crate (Presentation Layer) in a future phase. They will call the Rust API defined above.

Example HTTP endpoints (for reference, NOT part of this implementation):

```http
PUT    /api/credentials/{id}/rotation/policy     # Configure rotation policy
POST   /api/credentials/{id}/rotation/rotate     # Trigger manual rotation
GET    /api/credentials/{id}/rotation/status     # Get rotation status
POST   /api/credentials/{id}/rotation/rollback   # Rollback rotation
GET    /api/credentials/{id}/rotation/history    # Get rotation history
GET    /api/credentials/{id}/rotation/backups/{backup_id}  # Retrieve backup
```

These HTTP endpoints will be implemented by calling the Rust API:

```rust
// Example from nebula-api (future work, for context only)
#[post("/api/credentials/{id}/rotation/rotate")]
async fn trigger_rotation(
    id: Path<CredentialId>,
    payload: Json<RotationRequest>,
    credential_manager: Data<Arc<CredentialManager>>,
) -> Result<Json<RotationResponse>> {
    let transaction = credential_manager
        .rotate_now(&id, payload.reason, payload.no_grace_period)
        .await?;
    Ok(Json(RotationResponse::from(transaction)))
}
```

### Quickstart Guide

File: `specs/004-credential-rotation/quickstart.md`

**Expected content**:
```markdown
# Credential Rotation Quickstart

## Basic Periodic Rotation (5 minutes)

1. Configure 90-day rotation policy:
   ```rust
   use nebula_credential::rotation::*;
   
   let policy = RotationPolicy::Periodic(PeriodicConfig {
       interval: Duration::from_secs(90 * 24 * 60 * 60),
       grace_period: Duration::from_secs(24 * 60 * 60),
       enable_jitter: true,
   });
   
   manager.set_rotation_policy(credential_id, policy).await?;
   ```

2. Start background scheduler:
   ```rust
   let scheduler = RotationScheduler::new(manager);
   scheduler.start().await?;
   ```

3. Monitor rotation events:
   ```rust
   let events = manager.get_rotation_history(credential_id).await?;
   for event in events {
       println!("{:?}", event);
   }
   ```

## Manual Emergency Rotation (1 minute)

1. Trigger immediate rotation:
   ```rust
   manager.rotate_now(
       credential_id,
       RotationReason::SecurityIncident {
           incident_id: "INC-2026-042".to_string(),
           description: "Credential leaked in GitHub".to_string(),
       },
       true, // no_grace_period
   ).await?;
   ```

## OAuth2 Token Refresh (Before-Expiry)

1. Configure 80% TTL rotation:
   ```rust
   let policy = RotationPolicy::BeforeExpiry(BeforeExpiryConfig {
       threshold_percentage: 0.80,
       minimum_time_before_expiry: Duration::from_secs(5 * 60),
       grace_period: Duration::from_secs(10 * 60),
   });
   ```

2. System automatically refreshes when token reaches 48 minutes (80% of 1-hour TTL)
```

---

## Phase 2: Task Breakdown

**NOT EXECUTED BY /speckit.plan - Use /speckit.tasks command instead**

This phase is handled by `/speckit.tasks` command which:
- Reads this plan.md and research.md
- Generates detailed implementation tasks in tasks.md
- Creates dependency-ordered tasks with parallel execution groups
- Includes testing checkpoints and quality gates

---

## Implementation Notes

### Key Design Decisions (from research phase)

*To be filled during Phase 0 research*

### Known Constraints

- Storage provider must support atomic read-modify-write for transaction safety (checked during research)
- Time synchronization required across distributed systems for accurate scheduling (NTP dependency)
- Validation tests require network access to credential providers (database, OAuth2, CA)
- Backup encryption uses same encryption key as credentials (Phase 1 dependency)

### Risk Mitigation

**Risk**: Rotation scheduler becomes single point of failure
**Mitigation**: Scheduler uses heartbeat mechanism, restart detection, transaction recovery

**Risk**: Grace period expiration during system downtime causes credential revocation
**Mitigation**: Grace period extension API, warning notifications 1 hour before expiry

**Risk**: Validation test false negatives (network blip fails valid credential)
**Mitigation**: Retry validation 3 times before rollback, distinguish network vs auth errors

**Risk**: Rotation storms when many credentials share same schedule
**Mitigation**: Jitter distribution (±10%), global rotation rate limit

### Dependencies on Previous Phases & Other Crates

**Phase 1 (Core Abstractions)** - REQUIRED:
- CredentialId, CredentialMetadata, CredentialData types
- EncryptionManager for backup encryption
- SecretString for secure password handling

**Phase 2 (Storage Backends)** - REQUIRED:
- At least one StorageProvider implementation
- Atomic read-modify-write for transaction state
- Backup storage with retention policies
- **EXTENDS StorageProvider trait**: Add rotation-specific methods (save_rotation_transaction, save_backup, etc.)

**Phase 3 (Credential Manager)** - REQUIRED:
- CredentialManager CRUD operations
- Credential state machine (Active, Expired, Revoked)
- Caching layer (impacts grace period credential lookup)

---

**Cross-Crate Dependencies** (Nebula workspace):

```text
nebula-credential (THIS PHASE)
├── uses → nebula-core (CredentialId, Scope, ExecutionId)
├── uses → nebula-value (Value types for credential data)
├── uses → nebula-log (tracing, structured logging)
├── uses → nebula-metrics (rotation metrics collection)
├── uses → nebula-eventbus (rotation event publishing)
├── uses → nebula-config (rotation policy configuration)
├── uses → nebula-resilience (retry policies, circuit breakers)
└── extends → nebula-storage (StorageProvider trait - Phase 2)

nebula-api (FUTURE - Presentation Layer)
└── uses → nebula-credential (calls Rust API for HTTP endpoints)

nebula-storage (FUTURE - Infrastructure Layer)
└── implements → StorageProvider rotation methods (SQL migrations, queries)
```

**Trait Extensions Required**:

```rust
// In nebula-storage (Phase 2 work, extends existing trait)
#[async_trait]
pub trait StorageProvider: Send + Sync {
    // ... existing methods from Phase 2 ...
    
    // NEW: Rotation-specific methods
    async fn save_rotation_transaction(&self, tx: &RotationTransaction) -> Result<()>;
    async fn get_rotation_transaction(&self, id: &RotationId) -> Result<Option<RotationTransaction>>;
    async fn update_rotation_state(&self, id: &RotationId, state: RotationState) -> Result<()>;
    async fn save_rotation_backup(&self, backup: &RotationBackup) -> Result<()>;
    async fn get_rotation_backup(&self, id: &BackupId) -> Result<Option<RotationBackup>>;
    async fn save_rotation_event(&self, event: &RotationEvent) -> Result<()>;
    async fn list_rotation_events(&self, credential_id: &CredentialId) -> Result<Vec<RotationEvent>>;
}
```

**External Dependencies** (outside Nebula):
- Certificate rotation: CA infrastructure (AWS Private CA, Vault PKI, or OpenSSL)
- OAuth2 rotation: OAuth2 provider with refresh token support
- Database rotation: Database admin privileges for user creation
- Notifications: Webhook/email service for rotation alerts

### Testing Strategy

**Unit Tests** (tests within rotation module):
- Policy evaluation: Periodic interval calculations, TTL threshold checks
- State machine transitions: Valid state progressions, invalid transition rejection
- Validation test configuration: Timeout handling, retry logic
- Grace period calculations: Duration arithmetic, expiration detection

**Integration Tests** (tests/ directory):
- Full rotation cycle: Create credential → Configure policy → Trigger rotation → Verify grace period → Confirm revocation
- Rollback scenarios: Validation failure → Automatic rollback → Old credential restored
- Blue-green rotation: Database user creation → Privilege grant → Traffic shift → Old user cleanup
- Time-based tests: Use `tokio::time::pause()` to simulate 90-day intervals in seconds

**Property-Based Tests** (if using proptest):
- Grace period math: For any duration D, grace_period_end = rotation_time + D (invariant)
- Jitter distribution: For any interval I, rotation_time ∈ [scheduled_time - I*0.1, scheduled_time + I*0.1]
- State machine: No transitions bypass validation state (state machine correctness)

**Example Tests**:
```rust
#[tokio::test]
async fn test_periodic_rotation_90_days() {
    tokio::time::pause();
    
    let policy = RotationPolicy::Periodic(/*90 days*/);
    let scheduler = RotationScheduler::new(/*...*/);
    
    // Advance time by 89 days - should NOT trigger
    tokio::time::advance(Duration::from_secs(89 * 24 * 60 * 60)).await;
    assert_eq!(scheduler.pending_rotations().await.len(), 0);
    
    // Advance time by 1 more day - should trigger
    tokio::time::advance(Duration::from_secs(1 * 24 * 60 * 60)).await;
    assert_eq!(scheduler.pending_rotations().await.len(), 1);
}

#[tokio::test]
async fn test_validation_failure_triggers_rollback() {
    let mut mock_validator = MockValidator::new();
    mock_validator.expect_validate()
        .returning(|_| Err(ValidationError::AuthenticationFailed));
    
    let result = transaction.execute(mock_validator).await;
    
    assert!(result.is_err());
    assert_eq!(transaction.state(), RotationState::RolledBack);
    assert!(old_credential_still_active());
}
```

### Performance Considerations

- Background scheduler check interval: 60 seconds default (configurable based on policy density)
- Validation timeout: 30 seconds default, configurable per credential type
- Concurrent rotation limit: 100 simultaneous rotations (prevent resource exhaustion)
- Backup retention cleanup: Daily background task removes expired backups (>30 days old)

### Security Considerations

- Rotation transactions logged with audit trail (who, what, when, why)
- Validation tests never log credential values (only test results)
- Backup encryption uses same key as active credentials
- Grace period credentials marked for deletion (not immediately destroyed for forensics)
- Manual rotation requires authentication and authorization checks

---

## Phase 1 Completion Checklist

- [x] research.md generated with all technology selections documented
- [x] data-model.md created with entity definitions and state machine
- [x] contracts/ directory created with Rust API contracts
- [x] quickstart.md created with 5-minute getting started guide
- [x] Agent context updated (`.specify/scripts/bash/update-agent-context.sh`)
- [x] Constitution Check re-evaluated (confirm no new violations)
- [x] Plan updated with crate responsibility boundaries (nebula-credential vs nebula-api vs nebula-storage)
- [ ] Quality gates passed (run after code implementation):
  - [ ] `cargo fmt --all`
  - [ ] `cargo clippy --workspace -- -D warnings`
  - [ ] `cargo check --workspace`
  - [ ] `cargo doc --no-deps --workspace`

---

**Status**: Phase 1 (Data Model & Contracts) - ✅ COMPLETED

**Phase 1 Deliverables**:
- ✅ `research.md` - 8 technology decisions documented
- ✅ `data-model.md` - 9 entities with state machine
- ✅ `contracts/README.md` - Rust API contract (primary) + HTTP reference (future)
- ✅ `quickstart.md` - 5 working examples
- ✅ `plan.md` - Updated with crate boundaries clarification

**Key Clarifications Made**:
1. **Scope**: `nebula-credential` crate implements business logic ONLY (Rust API)
2. **Out of Scope**: HTTP endpoints (`nebula-api`), SQL schema (`nebula-storage`)
3. **Primary Contract**: Rust public API functions (not HTTP REST)
4. **Integration**: Extends `StorageProvider` trait, uses existing Nebula crates

**Next Command**: `/speckit.tasks` to generate implementation tasks
