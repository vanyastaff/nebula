---
title: SecurityFeatures
tags: [nebula, nebula-credential, docs]
status: published
created: 2025-08-24
---

# Security Features Reference

Comprehensive security features for credential protection.

## Overview

nebula-credential implements defense-in-depth security with multiple layers:

1. **Encryption** - At rest and in transit
2. **Access Control** - RBAC and policy-based
3. **Audit Logging** - Complete audit trail
4. **Secret Handling** - Secure memory management
5. **Validation** - Input validation and sanitization
6. **Compliance** - SOC2, HIPAA, PCI-DSS support

## Encryption

### Encryption at Rest

All credentials are encrypted before storage.

#### Configuration

```rust
pub struct EncryptionConfig {
    /// Encryption algorithm
    pub algorithm: EncryptionAlgorithm,
    
    /// Key derivation function
    pub key_derivation: KeyDerivation,
    
    /// Key rotation interval
    pub key_rotation_interval: Duration,
    
    /// Master key source
    pub key_source: KeySource,
    
    /// Additional authenticated data
    pub aad: Option<Vec<u8>>,
}

pub enum EncryptionAlgorithm {
    Aes256Gcm,
    Aes256Cbc,
    ChaCha20Poly1305,
    XChaCha20Poly1305,
}

pub enum KeyDerivation {
    Pbkdf2 { iterations: u32 },
    Argon2id { memory: u32, iterations: u32 },
    Scrypt { n: u32, r: u32, p: u32 },
}

pub enum KeySource {
    Generated,
    Derived { password: SecureString },
    External { provider: String },
    Hsm { slot: u32 },
}
```

#### Usage

```rust
let encryption = EncryptionConfig {
    algorithm: EncryptionAlgorithm::Aes256Gcm,
    key_derivation: KeyDerivation::Argon2id {
        memory: 65536,
        iterations: 3,
    },
    key_rotation_interval: Duration::from_days(90),
    key_source: KeySource::External {
        provider: "aws-kms".to_string(),
    },
    aad: Some(b"nebula-credential".to_vec()),
};

let manager = CredentialManager::builder()
    .encryption_config(encryption)
    .build()
    .await?;
```

### Encryption in Transit

All network communication is encrypted.

```rust
pub struct TlsConfig {
    /// Minimum TLS version
    pub min_version: TlsVersion,
    
    /// Cipher suites
    pub cipher_suites: Vec<CipherSuite>,
    
    /// Client certificate
    pub client_cert: Option<Certificate>,
    
    /// CA bundle
    pub ca_bundle: Option<Vec<Certificate>>,
    
    /// Verify server certificate
    pub verify_server: bool,
}

pub enum TlsVersion {
    Tls12,
    Tls13,
}
```

### Key Management

#### Key Rotation

```rust
pub struct KeyRotationPolicy {
    /// Rotation interval
    pub interval: Duration,
    
    /// Grace period for old keys
    pub grace_period: Duration,
    
    /// Number of old keys to retain
    pub retained_keys: usize,
    
    /// Automatic rotation
    pub automatic: bool,
}

// Configure key rotation
manager.set_key_rotation_policy(KeyRotationPolicy {
    interval: Duration::from_days(90),
    grace_period: Duration::from_days(7),
    retained_keys: 3,
    automatic: true,
}).await?;

// Manual key rotation
manager.rotate_encryption_keys().await?;
```

#### Hardware Security Module (HSM)

```rust
pub struct HsmConfig {
    /// HSM type
    pub hsm_type: HsmType,
    
    /// Connection parameters
    pub connection: HsmConnection,
    
    /// Key identifiers
    pub keys: HsmKeys,
}

pub enum HsmType {
    CloudHsm,
    YubiHsm,
    Thales,
    Utimaco,
}

// Use HSM for encryption
let hsm = HsmConfig {
    hsm_type: HsmType::CloudHsm,
    connection: HsmConnection::Network {
        host: "hsm.example.com".to_string(),
        port: 3000,
    },
    keys: HsmKeys {
        master_key_id: "master-key-001".to_string(),
        signing_key_id: "signing-key-001".to_string(),
    },
};

manager.configure_hsm(hsm).await?;
```

## Access Control

### Role-Based Access Control (RBAC)

```rust
pub struct AccessControl {
    /// Roles and permissions
    pub roles: HashMap<String, Role>,
    
    /// User role assignments
    pub assignments: HashMap<UserId, Vec<String>>,
    
    /// Default permissions
    pub default_permissions: Permissions,
    
    /// Enforcement mode
    pub enforcement: EnforcementMode,
}

pub struct Role {
    pub name: String,
    pub permissions: Permissions,
    pub conditions: Vec<Condition>,
}

pub struct Permissions {
    pub create: bool,
    pub read: bool,
    pub update: bool,
    pub delete: bool,
    pub rotate: bool,
    pub export: bool,
}

pub enum EnforcementMode {
    Strict,
    Permissive,
    Audit,
}
```

#### Defining Roles

```rust
let access_control = AccessControl::new()
    .add_role("admin", Role {
        name: "Administrator".to_string(),
        permissions: Permissions::all(),
        conditions: vec![],
    })
    .add_role("developer", Role {
        name: "Developer".to_string(),
        permissions: Permissions {
            create: true,
            read: true,
            update: true,
            delete: false,
            rotate: false,
            export: false,
        },
        conditions: vec![
            Condition::Environment(vec!["development", "staging"]),
        ],
    })
    .add_role("viewer", Role {
        name: "Viewer".to_string(),
        permissions: Permissions::read_only(),
        conditions: vec![],
    });

manager.set_access_control(access_control).await?;
```

### Policy-Based Access Control

```rust
pub struct Policy {
    pub id: String,
    pub effect: Effect,
    pub principals: Vec<Principal>,
    pub actions: Vec<Action>,
    pub resources: Vec<Resource>,
    pub conditions: Vec<Condition>,
}

pub enum Effect {
    Allow,
    Deny,
}

pub enum Principal {
    User(UserId),
    Role(String),
    Service(String),
    Anyone,
}

pub enum Action {
    CreateCredential,
    GetCredential,
    UpdateCredential,
    DeleteCredential,
    RotateCredential,
    ExportCredential,
}

pub enum Condition {
    IpAddress(IpNetwork),
    Time(TimeRange),
    Environment(Vec<String>),
    MfaRequired,
    Custom(Box<dyn Fn(&Context) -> bool>),
}
```

#### Example Policy

```rust
let policy = Policy {
    id: "production-read-only".to_string(),
    effect: Effect::Allow,
    principals: vec![Principal::Role("developer".to_string())],
    actions: vec![Action::GetCredential],
    resources: vec![Resource::Pattern("prod-*".to_string())],
    conditions: vec![
        Condition::Environment(vec!["production"]),
        Condition::MfaRequired,
        Condition::Time(TimeRange::BusinessHours),
    ],
};

manager.add_policy(policy).await?;
```

## Audit Logging

### Audit Configuration

```rust
pub struct AuditConfig {
    /// Log level
    pub level: AuditLevel,
    
    /// Storage backend for logs
    pub storage: AuditStorage,
    
    /// Retention period
    pub retention: Duration,
    
    /// PII handling
    pub pii_handling: PiiHandling,
    
    /// Real-time streaming
    pub streaming: Option<StreamingConfig>,
}

pub enum AuditLevel {
    None,
    Basic,
    Detailed,
    Full,
}

pub enum AuditStorage {
    File { path: PathBuf },
    Database { url: String },
    Syslog { endpoint: String },
    CloudWatch { log_group: String },
    Custom(Box<dyn AuditLogger>),
}

pub enum PiiHandling {
    Include,
    Redact,
    Hash,
    Encrypt,
}
```

### Audit Events

```rust
pub struct AuditEvent {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub user_id: Option<UserId>,
    pub credential_id: Option<CredentialId>,
    pub ip_address: Option<IpAddr>,
    pub user_agent: Option<String>,
    pub action: Action,
    pub result: Result<(), String>,
    pub metadata: HashMap<String, Value>,
}

pub enum EventType {
    CredentialCreated,
    CredentialAccessed,
    CredentialUpdated,
    CredentialDeleted,
    CredentialRotated,
    CredentialExpired,
    AuthenticationFailed,
    PermissionDenied,
    PolicyViolation,
}
```

### Querying Audit Logs

```rust
// Query audit logs
let filter = AuditFilter::builder()
    .event_types(vec![EventType::CredentialAccessed])
    .user_id(Some("user123"))
    .date_range(Utc::now() - Duration::days(7), Utc::now())
    .build();

let events = manager.query_audit_logs(filter).await?;

for event in events {
    println!("{}: {} by {} - {:?}",
        event.timestamp,
        event.event_type,
        event.user_id.unwrap_or_default(),
        event.result
    );
}
```

## Secure Memory Management

### SecureString

Zero-memory string for sensitive data.

```rust
pub struct SecureString {
    data: Pin<Box<[u8]>>,
    len: usize,
}

impl SecureString {
    pub fn new(value: impl Into<String>) -> Self {
        let bytes = value.into().into_bytes();
        let mut data = Pin::new(bytes.into_boxed_slice());
        
        // Mark memory as non-swappable
        unsafe {
            mlock(data.as_ptr(), data.len());
        }
        
        Self { data, len }
    }
    
    pub fn expose(&self) -> &str {
        unsafe {
            std::str::from_utf8_unchecked(&self.data[..self.len])
        }
    }
}

impl Drop for SecureString {
    fn drop(&mut self) {
        // Zero memory before deallocation
        unsafe {
            std::ptr::write_volatile(
                self.data.as_mut_ptr(),
                0u8,
                self.len
            );
            munlock(self.data.as_ptr(), self.data.len());
        }
    }
}
```

### Memory Protection

```rust
pub struct MemoryProtection {
    /// Disable memory swapping
    pub no_swap: bool,
    
    /// Lock memory pages
    pub lock_memory: bool,
    
    /// Guard pages
    pub guard_pages: bool,
    
    /// Secure allocator
    pub secure_allocator: bool,
}

// Configure memory protection
manager.set_memory_protection(MemoryProtection {
    no_swap: true,
    lock_memory: true,
    guard_pages: true,
    secure_allocator: true,
}).await?;
```

## Input Validation

### Credential Validation

```rust
pub struct ValidationRules {
    /// Minimum entropy for secrets
    pub min_entropy: f64,
    
    /// Password requirements
    pub password_rules: PasswordRules,
    
    /// API key format
    pub api_key_pattern: Option<Regex>,
    
    /// Certificate validation
    pub certificate_rules: CertificateRules,
}

pub struct PasswordRules {
    pub min_length: usize,
    pub require_uppercase: bool,
    pub require_lowercase: bool,
    pub require_digits: bool,
    pub require_special: bool,
    pub max_age: Option<Duration>,
    pub history_check: usize,
}

// Configure validation
manager.set_validation_rules(ValidationRules {
    min_entropy: 60.0,
    password_rules: PasswordRules {
        min_length: 12,
        require_uppercase: true,
        require_lowercase: true,
        require_digits: true,
        require_special: true,
        max_age: Some(Duration::days(90)),
        history_check: 5,
    },
    api_key_pattern: Some(Regex::new(r"^sk_[a-zA-Z0-9]{32}$")?),
    certificate_rules: CertificateRules::default(),
}).await?;
```

### Sanitization

```rust
pub struct Sanitizer {
    /// Remove sensitive patterns
    pub patterns: Vec<SensitivePattern>,
    
    /// Redaction character
    pub redaction_char: char,
    
    /// Keep partial visibility
    pub partial_redaction: bool,
}

pub struct SensitivePattern {
    pub name: String,
    pub regex: Regex,
    pub redaction_type: RedactionType,
}

pub enum RedactionType {
    Full,
    Partial { visible_start: usize, visible_end: usize },
    Hash,
}

// Sanitize logs
let sanitizer = Sanitizer::default()
    .add_pattern("api_key", r"sk_[a-zA-Z0-9]+")
    .add_pattern("password", r"password=\S+")
    .add_pattern("token", r"Bearer \S+");

let sanitized = sanitizer.sanitize(log_message)?;
```

## Compliance

### SOC2 Compliance

```rust
pub struct Soc2Config {
    /// Enable SOC2 controls
    pub enabled: bool,
    
    /// Control objectives
    pub controls: Vec<Soc2Control>,
    
    /// Evidence collection
    pub evidence_collection: bool,
    
    /// Audit reports
    pub audit_reports: bool,
}

pub enum Soc2Control {
    AccessControl,
    ChangeManagement,
    DataProtection,
    IncidentResponse,
    RiskAssessment,
}

// Enable SOC2 compliance
manager.enable_compliance(ComplianceFramework::Soc2(
    Soc2Config {
        enabled: true,
        controls: vec![
            Soc2Control::AccessControl,
            Soc2Control::DataProtection,
        ],
        evidence_collection: true,
        audit_reports: true,
    }
)).await?;
```

### HIPAA Compliance

```rust
pub struct HipaaConfig {
    /// Enable HIPAA controls
    pub enabled: bool,
    
    /// Encryption requirements
    pub encryption: HipaaEncryption,
    
    /// Access controls
    pub access_controls: HipaaAccessControls,
    
    /// Audit logging
    pub audit_logging: HipaaAuditLogging,
}

// Enable HIPAA compliance
manager.enable_compliance(ComplianceFramework::Hipaa(
    HipaaConfig {
        enabled: true,
        encryption: HipaaEncryption::Required,
        access_controls: HipaaAccessControls::Strict,
        audit_logging: HipaaAuditLogging::Complete,
    }
)).await?;
```

### PCI-DSS Compliance

```rust
pub struct PciDssConfig {
    /// PCI-DSS level
    pub level: PciDssLevel,
    
    /// Encryption requirements
    pub encryption: PciDssEncryption,
    
    /// Key management
    pub key_management: PciDssKeyManagement,
    
    /// Network segmentation
    pub network_segmentation: bool,
}

pub enum PciDssLevel {
    Level1,
    Level2,
    Level3,
    Level4,
}
```

## Threat Protection

### Rate Limiting

```rust
pub struct RateLimitConfig {
    /// Requests per second
    pub requests_per_second: u32,
    
    /// Burst size
    pub burst_size: u32,
    
    /// Per-user limits
    pub per_user: bool,
    
    /// Action on limit
    pub action: RateLimitAction,
}

pub enum RateLimitAction {
    Reject,
    Delay(Duration),
    Queue,
}

// Configure rate limiting
manager.set_rate_limit(RateLimitConfig {
    requests_per_second: 100,
    burst_size: 200,
    per_user: true,
    action: RateLimitAction::Delay(Duration::from_millis(100)),
}).await?;
```

### Anomaly Detection

```rust
pub struct AnomalyDetection {
    /// Detection algorithms
    pub algorithms: Vec<DetectionAlgorithm>,
    
    /// Threshold for alerts
    pub alert_threshold: f64,
    
    /// Action on detection
    pub action: AnomalyAction,
}

pub enum DetectionAlgorithm {
    StatisticalAnalysis,
    MachineLearning,
    RuleBased,
}

pub enum AnomalyAction {
    Alert,
    Block,
    RequireMfa,
    Investigate,
}

// Enable anomaly detection
manager.enable_anomaly_detection(AnomalyDetection {
    algorithms: vec![
        DetectionAlgorithm::StatisticalAnalysis,
        DetectionAlgorithm::MachineLearning,
    ],
    alert_threshold: 0.95,
    action: AnomalyAction::Alert,
}).await?;
```

## Security Best Practices

### Configuration Checklist

- ✅ Enable encryption at rest
- ✅ Use TLS 1.3 for transit
- ✅ Configure access control
- ✅ Enable audit logging
- ✅ Set up key rotation
- ✅ Implement rate limiting
- ✅ Configure validation rules
- ✅ Enable anomaly detection
- ✅ Regular security audits
- ✅ Incident response plan

### Example Secure Configuration

```rust
let manager = CredentialManager::builder()
    // Encryption
    .encryption_config(EncryptionConfig {
        algorithm: EncryptionAlgorithm::Aes256Gcm,
        key_derivation: KeyDerivation::Argon2id {
            memory: 65536,
            iterations: 3,
        },
        key_rotation_interval: Duration::from_days(90),
        key_source: KeySource::Hsm { slot: 1 },
        aad: Some(b"nebula".to_vec()),
    })
    // Access Control
    .access_control(AccessControl::strict())
    // Audit
    .audit_config(AuditConfig {
        level: AuditLevel::Full,
        storage: AuditStorage::Database {
            url: "postgresql://audit".to_string(),
        },
        retention: Duration::from_days(2555), // 7 years
        pii_handling: PiiHandling::Redact,
        streaming: None,
    })
    // Memory Protection
    .memory_protection(MemoryProtection {
        no_swap: true,
        lock_memory: true,
        guard_pages: true,
        secure_allocator: true,
    })
    // Rate Limiting
    .rate_limit(RateLimitConfig {
        requests_per_second: 100,
        burst_size: 200,
        per_user: true,
        action: RateLimitAction::Delay(Duration::from_millis(100)),
    })
    // Compliance
    .enable_compliance(ComplianceFramework::Soc2(Soc2Config::default()))
    .build()
    .await?;
```

## Related

- [Credential Manager](https://claude.ai/chat/CredentialManager.md)
- [Storage Backends](https://claude.ai/chat/StorageBackends.md)
- [Configuration](https://claude.ai/chat/Configuration.md)
- [Error Types](https://claude.ai/chat/ErrorTypes.md)
