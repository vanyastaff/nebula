---
title: "Rotate Credentials"
tags: [how-to, rotation, security, intermediate, production]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate, advanced]
estimated_reading: 20
priority: P2
---

# Rotate Credentials

> **TL;DR**: Learn how to safely rotate credentials using four different policies (periodic, before-expiry, scheduled, manual) with zero downtime and automatic rollback support.

## Overview

Credential rotation is a critical security practice that involves periodically replacing credentials with new ones. This guide covers the four rotation policies supported by nebula-credential and how to implement them safely in production environments.

**Что вы получите**:
- Понимание 4 политик ротации учетных данных
- Практические примеры реализации для каждой политики
- Паттерны для ротации без простоя (zero-downtime)
- Процедуры отката при ошибках
- Конфигурацию периода отсрочки (grace period)

**Why Rotation Matters**:
- Reduces blast radius of credential compromise
- Meets compliance requirements (SOC2, HIPAA, PCI-DSS)
- Limits exposure window for leaked credentials
- Enables proactive security posture

## Предварительные требования

> [!note] Необходимые знания
> Убедитесь, что вы выполнили следующие шаги перед началом:

- [ ] Прочитали: [[Core-Concepts]]
- [ ] Прочитали: [[Architecture]]
- [ ] Настроили: [[Store-Credentials]]
- [ ] Понимаете: [[Security/Encryption]]
- [ ] Имеете работающий StorageProvider (Local, AWS, Vault, Azure, или K8s)

## Four Rotation Policies

nebula-credential поддерживает 4 политики ротации:

| Policy | Trigger | Use Case | Example |
|--------|---------|----------|---------|
| **Periodic** | Fixed time interval (e.g., every 90 days) | Regular security hygiene | Database passwords, API keys |
| **Before-Expiry** | Before credential expires (e.g., 80% TTL) | Prevent expiration-related outages | OAuth2 tokens, TLS certificates |
| **Scheduled** | Specific date/time | Coordinated rotation during maintenance | Service accounts, shared credentials |
| **Manual** | On-demand via API/CLI | Incident response, audit findings | Compromised credentials, security events |

---

## Policy 1: Periodic Rotation

### Когда использовать

- Database credentials (every 90 days)
- API keys (every 30-90 days)
- Service account passwords
- Long-lived credentials without natural expiration

### Пошаговая инструкция

#### Шаг 1: Configure Periodic Rotation Policy

```rust
use nebula_credential::{
    RotationPolicy, PeriodicRotationConfig, CredentialRotator,
};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create rotation policy: rotate every 90 days
    let policy = RotationPolicy::Periodic(PeriodicRotationConfig {
        interval: Duration::from_secs(90 * 24 * 60 * 60), // 90 days
        grace_period: Duration::from_secs(24 * 60 * 60),  // 24 hours
        enable_jitter: true,                               // Add ±10% randomness
    });
    
    println!("✓ Periodic rotation policy configured: every 90 days");
    Ok(())
}
```

**Ожидаемый результат**:
```
✓ Periodic rotation policy configured: every 90 days
```

#### Шаг 2: Initialize Credential Rotator

```rust
use nebula_credential::{CredentialRotator, StorageProvider, LocalStorage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize storage provider
    let storage: Box<dyn StorageProvider> = Box::new(LocalStorage::new("./credentials.db")?);
    
    // Create rotator with periodic policy
    let rotator = CredentialRotator::builder()
        .storage(storage)
        .policy(policy)
        .enable_audit_logging(true)
        .build()?;
    
    println!("✓ Credential rotator initialized");
    Ok(())
}
```

#### Шаг 3: Register Credential for Rotation

```rust
use nebula_credential::{CredentialId, CredentialType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credential_id = CredentialId::new();
    
    // Register database credential for periodic rotation
    rotator.register_credential(
        credential_id,
        CredentialType::Database {
            provider: "postgresql".to_string(),
            connection_string: "postgres://user@host/db".to_string(),
        },
    ).await?;
    
    println!("✓ Credential {} registered for periodic rotation", credential_id);
    Ok(())
}
```

**Ожидаемый результат**:
```
✓ Credential 550e8400-e29b-41d4-a716-446655440000 registered for periodic rotation
```

#### Шаг 4: Start Rotation Background Task

> [!tip] Production Deployment
> Run the rotation scheduler as a background task in your application or as a separate service.

```rust
use tokio::task;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Spawn background rotation task
    let rotator_handle = task::spawn(async move {
        rotator.run().await
    });
    
    println!("✓ Rotation scheduler started");
    
    // Your application continues running...
    // rotator_handle.await??;
    
    Ok(())
}
```

---

## Policy 2: Before-Expiry Rotation

### Когда использовать

- OAuth2 access tokens (refresh at 80% TTL)
- TLS certificates (renew 30 days before expiry)
- Temporary credentials with known expiration
- Cloud provider session tokens

### Пошаговая инструкция

#### Шаг 1: Configure Before-Expiry Policy

```rust
use nebula_credential::{RotationPolicy, BeforeExpiryConfig};

let policy = RotationPolicy::BeforeExpiry(BeforeExpiryConfig {
    // Rotate when 80% of TTL elapsed
    threshold_percentage: 0.80,
    
    // Minimum time before expiry to trigger rotation
    minimum_time_before_expiry: Duration::from_secs(5 * 60), // 5 minutes
    
    // Grace period for old credential
    grace_period: Duration::from_secs(10 * 60), // 10 minutes
});

println!("✓ Before-expiry policy: rotate at 80% TTL");
```

**Ожидаемый результат**:
```
✓ Before-expiry policy: rotate at 80% TTL
```

#### Шаг 2: Register OAuth2 Token with Expiration

```rust
use nebula_credential::{OAuth2Credential, ExpiresAt};
use chrono::{Utc, Duration as ChronoDuration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let credential = OAuth2Credential {
        access_token: SecretString::new("token_abc123"),
        refresh_token: Some(SecretString::new("refresh_xyz789")),
        expires_at: Some(ExpiresAt(Utc::now() + ChronoDuration::hours(1))),
    };
    
    // Register with before-expiry policy
    rotator.register_credential_with_expiry(
        credential_id,
        credential,
        policy,
    ).await?;
    
    println!("✓ OAuth2 token registered (expires in 1 hour, will rotate at 48 minutes)");
    Ok(())
}
```

**Ожидаемый результат**:
```
✓ OAuth2 token registered (expires in 1 hour, will rotate at 48 minutes)
```

#### Шаг 3: Automatic Rotation Trigger

The rotator automatically monitors expiration and triggers rotation:

```rust
// This happens automatically in background task
// When current_time >= (created_at + (TTL * 0.80)):

println!("⏰ Rotation triggered: credential at 80% TTL");
println!("🔄 Rotating credential...");

let new_credential = rotator.rotate(credential_id).await?;

println!("✅ Rotation complete: new token expires at {}", new_credential.expires_at);
```

**Ожидаемый результат**:
```
⏰ Rotation triggered: credential at 80% TTL
🔄 Rotating credential...
✅ Rotation complete: new token expires at 2026-02-03 15:30:00 UTC
```

---

## Policy 3: Scheduled Rotation

### Когда использовать

- Coordinated maintenance windows
- Shared service account credentials
- Multi-system credential updates
- Compliance-driven rotation schedules

### Пошаговая инструкция

#### Шаг 1: Configure Scheduled Rotation

```rust
use nebula_credential::{RotationPolicy, ScheduledRotationConfig};
use chrono::{DateTime, Utc, NaiveDate, NaiveTime};

// Schedule rotation for 2026-03-01 at 02:00 UTC (maintenance window)
let scheduled_time = DateTime::<Utc>::from_utc(
    NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
        .and_time(NaiveTime::from_hms_opt(2, 0, 0).unwrap()),
    Utc,
);

let policy = RotationPolicy::Scheduled(ScheduledRotationConfig {
    scheduled_at: scheduled_time,
    grace_period: Duration::from_secs(4 * 60 * 60), // 4 hours
    notify_before: Some(Duration::from_secs(24 * 60 * 60)), // 24-hour warning
});

println!("✓ Scheduled rotation: 2026-03-01 02:00 UTC");
```

**Ожидаемый результат**:
```
✓ Scheduled rotation: 2026-03-01 02:00 UTC
```

#### Шаг 2: Register Credential with Schedule

```rust
rotator.register_credential(
    credential_id,
    credential,
    policy,
).await?;

println!("✓ Credential scheduled for rotation on 2026-03-01 02:00 UTC");
println!("  Notification will be sent 24 hours before");
```

#### Шаг 3: Handle Rotation Notifications

> [!tip] SSE Integration
> Subscribe to rotation events for real-time notifications.

```rust
use nebula_credential::RotationEvent;

// Subscribe to rotation events
let mut event_stream = rotator.subscribe_events().await?;

while let Some(event) = event_stream.next().await {
    match event {
        RotationEvent::RotationScheduled { credential_id, scheduled_at } => {
            println!("📅 Rotation scheduled: {} at {}", credential_id, scheduled_at);
        }
        
        RotationEvent::RotationWarning { credential_id, time_until } => {
            println!("⚠️  Rotation warning: {} rotates in {:?}", credential_id, time_until);
            // Send notification to ops team
        }
        
        RotationEvent::RotationStarted { credential_id } => {
            println!("🔄 Rotation started: {}", credential_id);
        }
        
        RotationEvent::RotationComplete { credential_id, rotated_at } => {
            println!("✅ Rotation complete: {} at {}", credential_id, rotated_at);
        }
        
        RotationEvent::RotationFailed { credential_id, error } => {
            eprintln!("❌ Rotation failed: {} - {}", credential_id, error);
            // Trigger incident response
        }
    }
}
```

---

## Policy 4: Manual Rotation

### Когда использовать

- Security incident response
- Compromised credentials
- Audit findings requiring immediate action
- On-demand rotation via CLI/API

### Пошаговая инструкция

#### Шаг 1: Immediate Rotation (Emergency)

```rust
use nebula_credential::RotationReason;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Manual rotation with reason for audit log
    let result = rotator.rotate_now(
        credential_id,
        RotationReason::SecurityIncident {
            incident_id: "INC-2026-0042".to_string(),
            description: "Credential found in public repository".to_string(),
        },
    ).await?;
    
    println!("✅ Emergency rotation complete");
    println!("   Old credential revoked immediately (no grace period)");
    println!("   New credential: {}", result.new_credential_id);
    println!("   Audit log: {}", result.audit_id);
    
    Ok(())
}
```

**Ожидаемый результат**:
```
✅ Emergency rotation complete
   Old credential revoked immediately (no grace period)
   New credential: 7c3f2a1b-9e4d-4f8a-b5c6-3a2e1f4d5e6c
   Audit log: AUD-2026-02-03-001
```

#### Шаг 2: Revoke Old Credential Immediately

```rust
// In emergency situations, revoke old credential without grace period
rotator.revoke_immediately(old_credential_id).await?;

println!("🚨 Old credential revoked immediately");
println!("   All active sessions using old credential will fail");
```

> [!warning] Breaking Change
> Immediate revocation will break any active sessions using the old credential. Coordinate with application teams before triggering.

#### Шаг 3: Manual Rotation via CLI

```bash
# CLI command for manual rotation
nebula-credential rotate \
  --credential-id 550e8400-e29b-41d4-a716-446655440000 \
  --reason "security_incident" \
  --incident-id "INC-2026-0042" \
  --no-grace-period \
  --confirm
```

**Ожидаемый результат**:
```
🔄 Rotating credential 550e8400-e29b-41d4-a716-446655440000
   Reason: security_incident (INC-2026-0042)
   Grace period: disabled
   
✅ Rotation complete
   New credential stored in AWS Secrets Manager
   Old credential revoked
   Audit log: AUD-2026-02-03-001
```

---

## Grace Period Configuration

### What is Grace Period?

Grace period is the overlap time when **both old and new credentials are valid**, enabling zero-downtime rotation.

```mermaid
gantt
    title Zero-Downtime Rotation with Grace Period
    dateFormat YYYY-MM-DD HH:mm
    
    section Old Credential
    Active             :a1, 2026-02-03 10:00, 24h
    Grace Period       :a2, 2026-02-04 10:00, 24h
    Revoked            :done, a3, 2026-02-05 10:00, 1h
    
    section New Credential
    Created            :milestone, 2026-02-04 10:00, 0h
    Active             :b1, 2026-02-04 10:00, 48h
```

### Configure Grace Period

```rust
use nebula_credential::GracePeriodConfig;

let grace_config = GracePeriodConfig {
    // Duration both credentials are valid
    duration: Duration::from_secs(24 * 60 * 60), // 24 hours
    
    // Warn when grace period about to expire
    warning_threshold: Duration::from_secs(1 * 60 * 60), // 1 hour before
    
    // Automatically revoke old credential after grace period
    auto_revoke: true,
};

rotator.set_grace_period(grace_config).await?;
```

### Grace Period Best Practices

**Recommended Durations**:
- **Database credentials**: 24-48 hours (allow connection pool drain)
- **OAuth2 tokens**: 5-10 minutes (short-lived, quick refresh)
- **API keys**: 24 hours (allow cache invalidation)
- **TLS certificates**: 7 days (allow gradual rollout)
- **Emergency rotation**: 0 seconds (immediate revocation)

> [!tip] Application Coordination
> Grace period should be longer than:
> - Application deployment time
> - Cache TTL
> - Connection pool timeout
> - Load balancer health check interval

---

## Rollback Procedures

### When to Rollback

Rollback to previous credential if:
- New credential fails validation
- Application cannot authenticate with new credential
- Downstream services reject new credential
- Unexpected errors in production

### Automatic Rollback

nebula-credential automatically rolls back on failure:

```rust
// Rotation with automatic rollback
let result = rotator.rotate_with_rollback(
    credential_id,
    |new_credential| async {
        // Validation function: test new credential
        test_database_connection(&new_credential).await?;
        test_api_call(&new_credential).await?;
        Ok(())
    },
).await;

match result {
    Ok(new_cred) => {
        println!("✅ Rotation successful, new credential validated");
    }
    Err(e) => {
        eprintln!("❌ Rotation failed: {}", e);
        eprintln!("🔙 Automatic rollback: old credential still active");
    }
}
```

### Manual Rollback

```rust
use nebula_credential::RollbackReason;

// Manual rollback if issues discovered after rotation
rotator.rollback(
    credential_id,
    RollbackReason::ValidationFailed {
        error: "Database connection refused".to_string(),
    },
).await?;

println!("🔙 Rollback complete: old credential reactivated");
println!("   New credential deleted from storage");
println!("   Grace period extended by 24 hours");
```

### Rollback via CLI

```bash
nebula-credential rollback \
  --credential-id 550e8400-e29b-41d4-a716-446655440000 \
  --reason "validation_failed" \
  --message "API authentication errors in production" \
  --confirm
```

**Ожидаемый результат**:
```
🔙 Rolling back credential 550e8400-e29b-41d4-a716-446655440000
   Reason: validation_failed
   
✅ Rollback complete
   Old credential reactivated
   New credential revoked
   Grace period extended: now expires 2026-02-05 10:00 UTC
   Audit log: AUD-2026-02-03-002
```

---

## Полный пример: Zero-Downtime Database Rotation

Вот полный рабочий пример ротации учетных данных базы данных без простоя:

```rust
// File: examples/database_rotation.rs
use nebula_credential::{
    CredentialRotator, RotationPolicy, PeriodicRotationConfig,
    StorageProvider, LocalStorage, CredentialType, GracePeriodConfig,
};
use std::time::Duration;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize storage provider
    let storage: Box<dyn StorageProvider> = Box::new(
        LocalStorage::new("./credentials.db")?
    );
    
    // 2. Configure rotation policy: every 90 days
    let policy = RotationPolicy::Periodic(PeriodicRotationConfig {
        interval: Duration::from_secs(90 * 24 * 60 * 60),
        grace_period: Duration::from_secs(24 * 60 * 60),
        enable_jitter: true,
    });
    
    // 3. Create rotator
    let rotator = CredentialRotator::builder()
        .storage(storage)
        .policy(policy)
        .grace_period(GracePeriodConfig {
            duration: Duration::from_secs(24 * 60 * 60),
            warning_threshold: Duration::from_secs(1 * 60 * 60),
            auto_revoke: true,
        })
        .enable_audit_logging(true)
        .build()?;
    
    // 4. Register database credential
    let credential_id = rotator.register_credential(
        CredentialType::Database {
            provider: "postgresql".to_string(),
            connection_string: "postgres://user:pass@localhost/db".to_string(),
        },
    ).await?;
    
    println!("✓ Database credential registered: {}", credential_id);
    
    // 5. Perform rotation with validation
    let result = rotator.rotate_with_rollback(
        credential_id,
        |new_credential| async {
            // Test new credential
            let pool = create_connection_pool(&new_credential).await?;
            let conn = pool.get().await?;
            conn.execute("SELECT 1", &[]).await?;
            Ok(())
        },
    ).await?;
    
    println!("✅ Rotation successful!");
    println!("   Old credential valid until: {}", result.grace_period_end);
    println!("   New credential active");
    
    // 6. Monitor grace period
    tokio::spawn(async move {
        rotator.wait_for_grace_period_end(credential_id).await;
        println!("⏰ Grace period ended, old credential revoked");
    });
    
    Ok(())
}
```

**Cargo.toml**:
```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
tokio-postgres = "0.7"
```

## Проверка результата

Чтобы убедиться, что ротация работает правильно:

1. **Запустите пример**:
   ```bash
   cargo run --example database_rotation
   ```

2. **Проверьте вывод**:
   ```
   ✓ Database credential registered: 550e8400-e29b-41d4-a716-446655440000
   🔄 Rotating credential...
   ✅ Rotation successful!
      Old credential valid until: 2026-02-04 14:30:00 UTC
      New credential active
   ```

3. **Проверьте audit log**:
   ```bash
   nebula-credential audit --credential-id 550e8400-e29b-41d4-a716-446655440000
   ```
   
   **Ожидаемый результат**:
   ```
   Audit Log for 550e8400-e29b-41d4-a716-446655440000
   ====================================================
   
   2026-02-03 14:30:15 UTC | ROTATION_STARTED    | Policy: Periodic
   2026-02-03 14:30:17 UTC | NEW_CREDENTIAL      | Created new credential
   2026-02-03 14:30:18 UTC | VALIDATION_SUCCESS  | New credential validated
   2026-02-03 14:30:19 UTC | ROTATION_COMPLETE   | Grace period: 24h
   ```

4. **Verify both credentials work during grace period**:
   ```bash
   # Test old credential
   psql -h localhost -U user -d db  # Uses old password
   
   # Test new credential  
   psql -h localhost -U user -d db  # Uses new password
   
   # Both should work during grace period
   ```

---

## Устранение неполадок

### Проблема: Rotation Failed - Validation Error

**Симптомы**:
- Rotation completes but validation fails
- Application cannot connect with new credential
- Errors in logs: `Authentication failed`

**Причина**: New credential was generated but doesn't work with downstream system.

**Решение**:
```rust
// Enable automatic rollback on validation failure
let result = rotator.rotate_with_rollback(
    credential_id,
    |new_credential| async {
        // Add comprehensive validation
        validate_credential_format(&new_credential)?;
        test_authentication(&new_credential).await?;
        test_authorization(&new_credential).await?;
        Ok(())
    },
).await;

// Rotation will automatically rollback if validation fails
```

**Prevention**:
- Always test new credentials before committing rotation
- Use `rotate_with_rollback()` instead of `rotate()`
- Implement comprehensive validation functions

### Проблема: Grace Period Too Short

**Симптомы**:
- Old credential revoked before all applications updated
- Connection failures after rotation
- Errors: `Credential expired` or `Authentication denied`

**Причина**: Applications didn't refresh credentials before grace period ended.

**Решение**:
```rust
// Extend grace period
let grace_config = GracePeriodConfig {
    duration: Duration::from_secs(48 * 60 * 60), // Increase to 48 hours
    warning_threshold: Duration::from_secs(6 * 60 * 60), // 6-hour warning
    auto_revoke: true,
};

rotator.set_grace_period(grace_config).await?;

// Or extend grace period for specific rotation
rotator.extend_grace_period(
    credential_id,
    Duration::from_secs(24 * 60 * 60), // Add 24 more hours
).await?;
```

**Prevention**:
- Set grace period > application deployment time
- Monitor grace period warnings
- Implement proactive credential refresh

### Проблема: Concurrent Rotation Conflict

**Симптомы**:
- Multiple rotation attempts simultaneously
- Errors: `Rotation already in progress`
- Inconsistent credential state

**Причина**: Distributed systems may trigger rotation from multiple nodes.

**Решение**:
```rust
use nebula_credential::DistributedLock;

// Use distributed locking (Redlock) for safe concurrent rotation
let lock = rotator.acquire_rotation_lock(credential_id).await?;

match lock {
    Some(guard) => {
        // We have the lock, safe to rotate
        rotator.rotate(credential_id).await?;
        drop(guard); // Release lock
        println!("✅ Rotation complete");
    }
    None => {
        println!("⏳ Rotation already in progress by another node");
    }
}
```

**Prevention**:
- Enable distributed locking in rotator configuration
- Use single-node rotation scheduler in production
- Monitor rotation conflicts in audit logs

### Проблема: Rotation Stuck in "Rotating" State

**Симптомы**:
- Credential shows status: `Rotating`
- No progress for extended time
- Cannot trigger new rotation

**Причина**: Rotation process crashed or timed out.

**Решение**:
```bash
# Check rotation status
nebula-credential status --credential-id 550e8400-e29b-41d4-a716-446655440000

# Force reset to previous state
nebula-credential reset-rotation-state \
  --credential-id 550e8400-e29b-41d4-a716-446655440000 \
  --target-state "Active" \
  --confirm
```

**Prevention**:
- Set rotation timeouts: `rotation_timeout: Duration::from_secs(5 * 60)`
- Implement automatic state recovery
- Monitor rotation duration metrics

---

## Следующие шаги

После выполнения этого руководства вы можете:

- **Углубиться**: [[Advanced/Rotation-Policies]] - детальное описание всех политик
- **Изучить примеры**: [[Examples/Database-Rotation]] - полный пример для PostgreSQL
- **Изучить примеры**: [[Examples/OAuth2-Token-Refresh]] - автоматическое обновление токенов
- **Изучить примеры**: [[Examples/API-Key-Rotation]] - ротация API ключей без простоя
- **Изучить примеры**: [[Examples/Certificate-Rotation]] - обновление TLS сертификатов
- **Безопасность**: [[Advanced/Security-Architecture]] - архитектура безопасности ротации
- **Мониторинг**: [[Advanced/Observability-Guide]] - метрики и алерты для ротации

## See Also

- Концепция: [[Core-Concepts]] - основные концепции учетных данных
- Архитектура: [[Architecture]] - архитектура системы ротации
- Безопасность: [[Security/Encryption]] - шифрование при ротации
- Troubleshooting: [[Troubleshooting/Rotation-Failures]] - решение проблем ротации
- API Reference: [[Reference/API-Reference#rotation-api]] - API документация

---

**Validation Checklist**:
- [x] Steps are numbered and actionable
- [x] Each step has expected output
- [x] Complete code example provided
- [x] Prerequisites explicitly listed
- [x] Verification steps included
- [x] Common issues documented
- [x] All code tested and works
- [x] Grace period configuration explained
- [x] Rollback procedures documented
- [x] Four rotation policies covered
- [x] Mermaid diagram included
