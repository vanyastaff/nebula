# Credential Rotation Data Model

**Phase**: 1 (Data Model & Contracts)  
**Date**: 2026-02-04  
**Based on**: research.md technology selections

## Core Entities

### 1. RotationPolicy

**Purpose**: Defines when and how credentials should be rotated

**Variants**:

```rust
pub enum RotationPolicy {
    Periodic(PeriodicConfig),
    BeforeExpiry(BeforeExpiryConfig),
    Scheduled(ScheduledConfig),
    Manual,
}

pub struct PeriodicConfig {
    pub interval: Duration,           // Rotation interval (e.g., 90 days)
    pub grace_period: Duration,       // Overlap time (e.g., 24 hours)
    pub enable_jitter: bool,          // Randomize ±10% to prevent storms
}

pub struct BeforeExpiryConfig {
    pub threshold_percentage: f32,    // Rotate at % of TTL (e.g., 0.80 = 80%)
    pub minimum_time_before_expiry: Duration, // Safety buffer (e.g., 5 min)
    pub grace_period: Duration,       // Overlap time
}

pub struct ScheduledConfig {
    pub scheduled_at: DateTime<Utc>, // Exact rotation time
    pub grace_period: Duration,      // Overlap time
    pub notify_before: Option<Duration>, // Notification lead time (e.g., 24 hours)
}
```

**Validation Rules**:
- `interval` > 1 hour (prevent too-frequent rotation)
- `grace_period` ≤ `interval` for Periodic (overlap can't exceed rotation period)
- `threshold_percentage` ∈ [0.5, 0.95] (rotate between 50%-95% of TTL)
- `scheduled_at` must be future timestamp

---

### 2. RotationTransaction

**Purpose**: Tracks state of single rotation operation

```rust
pub struct RotationTransaction {
    pub id: RotationId,              // UUID
    pub credential_id: CredentialId,
    pub state: RotationState,
    pub old_version: u32,
    pub new_version: Option<u32>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub validation_result: Option<ValidationResult>,
    pub backup_id: Option<BackupId>,
    pub grace_period_end: Option<DateTime<Utc>>,
}

pub enum RotationState {
    Pending,      // Queued for rotation
    Creating,     // Generating new credential
    Validating,   // Testing new credential
    Committing,   // Storing new credential
    Committed,    // Rotation complete, grace period active
    RolledBack,   // Validation failed, restored old credential
}
```

**State Transitions**:
```
Pending → Creating → Validating → Committing → Committed
    ↓         ↓           ↓            ↓
    → RolledBack ← ← ← ← ← (failure at any stage)
```

**Validation Rules**:
- State transitions must follow allowed paths (enforced by state machine)
- `new_version` = `old_version + 1` when Created
- `completed_at` required when state = Committed or RolledBack
- `grace_period_end` = `completed_at + policy.grace_period` when Committed

---

### 3. RotationBackup

**Purpose**: Encrypted backup for disaster recovery

```rust
pub struct RotationBackup {
    pub id: BackupId,                 // UUID
    pub credential_id: CredentialId,
    pub version: u32,
    pub encrypted_data: Vec<u8>,      // AES-256-GCM encrypted credential
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,    // created_at + 30 days minimum
    pub transaction_id: RotationId,
}
```

**Validation Rules**:
- `expires_at` ≥ `created_at + 30 days` (minimum retention)
- `encrypted_data` non-empty
- Backup created BEFORE rotation starts (can rollback)

---

### 4. ValidationTest

**Purpose**: Credential-type-specific functionality test

```rust
pub struct ValidationTest {
    pub test_method: TestMethod,
    pub endpoint: String,              // URL or query
    pub expected_criteria: SuccessCriteria,
    pub timeout: Duration,             // Default 30s
    pub retry_policy: RetryPolicy,
}

pub enum TestMethod {
    HttpRequest { method: HttpMethod, headers: HashMap<String, String> },
    DatabaseQuery { query: String },
    TlsHandshake { server_name: String },
}

pub enum SuccessCriteria {
    HttpStatus(u16),                   // 2xx = success
    QueryResult,                       // Non-empty result = success
    ValidHandshake,                    // TLS connection established = success
}

pub enum ValidationResult {
    Success,
    Failure { error: ValidationError },
}
```

---

### 5. RetryPolicy

**Purpose**: Exponential backoff configuration

```rust
pub struct RetryPolicy {
    pub max_attempts: u32,             // Default 5
    pub initial_backoff: Duration,     // Default 100ms
    pub backoff_multiplier: f32,       // Default 2.0
    pub max_backoff: Duration,         // Default 32s
    pub retriable_errors: Vec<ErrorType>,
}

pub enum ErrorType {
    NetworkTimeout,
    ServiceUnavailable,
    RateLimitExceeded,
    // Non-retriable: AuthenticationFailed, InvalidCredentialFormat
}
```

**Backoff Calculation**:
```
attempt_1: 100ms
attempt_2: 200ms
attempt_3: 400ms
attempt_4: 800ms
attempt_5: 1600ms (capped at max_backoff if 1600ms > 32s)
```

---

### 6. BlueGreenState

**Purpose**: Track blue-green deployment for databases

```rust
pub struct BlueGreenState {
    pub blue_credential: CredentialId,           // Old
    pub green_credential: Option<CredentialId>,  // New
    pub active_color: Color,
    pub traffic_percentage: HashMap<Color, u8>,  // Blue: 50%, Green: 50%
    pub error_rate: HashMap<Color, f32>,         // Blue: 0.01, Green: 0.02
    pub instance_migration: HashMap<InstanceId, Color>,
}

pub enum Color {
    Blue,   // Old credential
    Green,  // New credential
}
```

**Validation Rules**:
- `traffic_percentage.values().sum() == 100`
- `error_rate` ∈ [0.0, 1.0] (percentage as decimal)
- Rollback triggered if `error_rate[Green] > 0.05` (5% threshold)

---

### 7. RotationEvent (Audit Log)

**Purpose**: Immutable audit trail

```rust
pub struct RotationEvent {
    pub id: EventId,                   // UUID
    pub event_type: EventType,
    pub credential_id: CredentialId,
    pub rotation_id: RotationId,
    pub timestamp: DateTime<Utc>,
    pub actor: Actor,
    pub reason: Option<RotationReason>,
    pub metadata: HashMap<String, String>,
}

pub enum EventType {
    RotationStarted,
    NewCredentialCreated,
    ValidationSucceeded,
    ValidationFailed,
    RotationCommitted,
    RotationRolledBack,
    GracePeriodExpired,
    OldCredentialRevoked,
}

pub enum Actor {
    System,
    User(UserId),
}

pub enum RotationReason {
    SecurityIncident { incident_id: String, description: String },
    ComplianceAudit { auditor: String, finding_id: String },
    PersonnelChange { employee_id: String, change_type: String },
    UserRequested { user_id: String, reason: Option<String> },
    Testing { test_id: String, environment: String },
}
```

---

## Relationships

```
RotationPolicy (1) ── manages ──> (N) CredentialId
CredentialId (1) ── has current ──> (1) RotationTransaction [where state = Committed]
RotationTransaction (1) ── creates ──> (1) RotationBackup
RotationTransaction (1) ── uses ──> (1) ValidationTest
RotationTransaction (1) ── generates ──> (N) RotationEvent
ValidationTest (1) ── has ──> (1) RetryPolicy
BlueGreenState (1) ── references ──> (2) CredentialId [blue + green]
```

---

## Storage Schema

**Keys** (using storage provider):

- `credentials:{credential_id}` → Latest version
- `credentials:{credential_id}:v{version}` → Specific version
- `rotation:policies:{credential_id}` → RotationPolicy
- `rotation:transactions:{rotation_id}` → RotationTransaction
- `rotation:backups:{backup_id}` → RotationBackup
- `rotation:events:{event_id}` → RotationEvent
- `rotation:locks:{credential_id}` → Optimistic lock version

**Indexes** (for queries):

- By credential: `rotation:transactions:by_credential:{credential_id}` → List<RotationId>
- By state: `rotation:transactions:by_state:{state}` → List<RotationId>
- By timestamp: `rotation:events:by_timestamp:{date}` → List<EventId>

---

## Type Safety Guarantees

**Newtype patterns**:
```rust
pub struct RotationId(Uuid);
pub struct BackupId(Uuid);
pub struct EventId(Uuid);
pub struct GracePeriodDuration(Duration); // Custom type with validation

impl GracePeriodDuration {
    pub fn new(duration: Duration) -> Result<Self, ValidationError> {
        if duration > Duration::from_secs(90 * 24 * 60 * 60) {
            return Err(ValidationError::GracePeriodTooLong);
        }
        Ok(Self(duration))
    }
}
```

**Enum exhaustiveness**:
- All state transitions checked at compile time via match expressions
- Adding new RotationState variant requires updating all state machine logic

---

**Status**: Data model complete, ready for contract generation
