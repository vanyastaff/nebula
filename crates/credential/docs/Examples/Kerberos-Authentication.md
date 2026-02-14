---
title: "Kerberos Authentication"
tags: [example, kerberos, windows, enterprise, advanced]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced]
estimated_reading: 10
priority: P3
---

# Kerberos Authentication

> **TL;DR**: Используйте `Credential` trait из nebula-credential для Kerberos ticket-based authentication с Windows domain integration.

## Обзор

Kerberos credentials в nebula-credential управляются через `Credential` trait с поддержкой:
- **Ticket-Based Authentication**: TGT (Ticket Granting Ticket) acquisition
- **Windows Integration**: Active Directory domains
- **SSO**: Single Sign-On для domain resources
- **Microsoft Entra**: Cloud-based Kerberos (2026)

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#kerberos-protocol]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#kerberos-security]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- Windows domain или Kerberos KDC

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
chrono = "0.4"
```

### Implementing Kerberos Credential

```rust
// File: examples/kerberos_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    TestableCredential, SecretString,
    TestResult, TestDetails, OwnerId,
};
use async_trait::async_trait;
use std::collections::HashMap;
use chrono::{DateTime, Utc, Duration};

/// Kerberos configuration
#[derive(Debug, Clone)]
pub struct KerberosConfig {
    pub realm: String,
    pub kdc_servers: Vec<String>,
    pub service_principal: String,
    pub ticket_lifetime: Duration,
}

/// Kerberos ticket
#[derive(Debug, Clone)]
pub struct KerberosTicket {
    pub principal: String,
    pub expires_at: DateTime<Utc>,
    pub ticket_data: SecretString,
}

/// Kerberos credential provider
pub struct KerberosCredentialProvider {
    config: KerberosConfig,
}

impl KerberosCredentialProvider {
    pub fn new(config: KerberosConfig) -> Self {
        Self { config }
    }

    async fn acquire_ticket(
        &self,
        username: &str,
        password: &str,
    ) -> Result<KerberosTicket, CredentialError> {
        // In production: Use libgssapi or similar
        // Acquire TGT from KDC
        // This is a simplified example
        
        let principal = format!("{}@{}", username, self.config.realm);
        let expires_at = Utc::now() + self.config.ticket_lifetime;

        Ok(KerberosTicket {
            principal,
            expires_at,
            ticket_data: SecretString::new("ticket_blob".to_string()),
        })
    }
}

#[async_trait]
impl Credential for KerberosCredentialProvider {
    type Output = KerberosTicket;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        let username = ctx.metadata.get("username")
            .ok_or_else(|| CredentialError::ConfigurationError(
                "Username not found in context".to_string()
            ))?;

        let password = ctx.metadata.get("password")
            .ok_or_else(|| CredentialError::ConfigurationError(
                "Password not found in context".to_string()
            ))?;

        self.acquire_ticket(username, password).await
    }

    fn credential_type(&self) -> &'static str {
        "kerberos"
    }

    fn supports_refresh(&self) -> bool {
        true // Kerberos tickets can be renewed
    }
}

#[async_trait]
impl TestableCredential for KerberosCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();

        match self.retrieve(ctx).await {
            Ok(ticket) => {
                let latency_ms = start.elapsed().as_millis() as u64;

                let details = TestDetails {
                    latency_ms,
                    endpoint_tested: self.config.kdc_servers.first()
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string()),
                    permissions_verified: vec!["kerberos:tgt".to_string()],
                    metadata: HashMap::from([
                        ("principal".to_string(), 
                         serde_json::json!(ticket.principal)),
                        ("expires_at".to_string(), 
                         serde_json::json!(ticket.expires_at.to_rfc3339())),
                    ]),
                };

                Ok(TestResult::success("Kerberos TGT acquired successfully")
                    .with_details(details))
            }
            Err(e) => Ok(TestResult::failure(format!("Kerberos authentication failed: {}", e))),
        }
    }

    fn test_description(&self) -> &str {
        "Testing Kerberos by acquiring TGT from KDC"
    }
}
```

## Key Concepts

### 1. Kerberos Flow

```
1. User → KDC: Authentication Request
2. KDC → User: TGT (Ticket Granting Ticket)
3. User → TGS: Service Ticket Request + TGT
4. TGS → User: Service Ticket
5. User → Service: Access with Service Ticket
```

### 2. Microsoft Entra Kerberos (2026)

```rust
// Cloud-only identities can use Kerberos
let config = KerberosConfig {
    realm: "CLOUDIDENTITY.ONMICROSOFT.COM".to_string(),
    kdc_servers: vec!["kerberos.microsoftonline.com".to_string()],
    // ...
};
```

## Security Best Practices

> [!warning] Ticket Protection
> Store Kerberos tickets securely в `SecretString`.

## Related Examples

- **Enterprise Auth**: [[LDAP-Authentication]] - LDAP bind authentication | [[SAML-Authentication]] - SAML SSO
- **Certificate Auth**: [[mTLS-Certificate]] - Mutual TLS certificates
- **Token Auth**: [[JWT-Validation]] - JWT token validation

## See Also

- [[Core-Concepts|Core Concepts]]
- [[API-Reference|API Reference]]

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#kerberos-protocol]]
- [[../../specs/001-credential-docs/security-spec.md#kerberos-security]]

## Sources

- [RFC 4120 - Kerberos](https://datatracker.ietf.org/doc/html/rfc4120)
- [nebula-credential API](../Reference/API-Reference.md)
