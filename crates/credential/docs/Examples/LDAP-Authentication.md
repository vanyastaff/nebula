---
title: "LDAP Authentication"
tags: [example, ldap, active-directory, enterprise, intermediate]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 10
priority: P2
---

# LDAP Authentication

> **TL;DR**: Используйте `Credential` trait из nebula-credential для LDAP/Active Directory authentication с connection pooling.

## Обзор

LDAP credentials в nebula-credential управляются через `Credential` trait с поддержкой:
- **Bind Authentication**: Username/password validation
- **Connection Pooling**: Efficient connection reuse
- **TLS/STARTTLS**: Encrypted connections
- **Active Directory**: Special handling для AD

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#ldap-protocol]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#ldap-security]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- LDAP server или Active Directory

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
ldap3 = "0.11"
```

### Implementing LDAP Credential

```rust
// File: examples/ldap_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    TestableCredential, SecretString,
    TestResult, TestDetails, OwnerId,
};
use async_trait::async_trait;
use ldap3::{LdapConnAsync, Scope, SearchEntry};
use std::collections::HashMap;

/// LDAP configuration
#[derive(Debug, Clone)]
pub struct LdapConfig {
    pub url: String,
    pub base_dn: String,
    pub bind_dn_template: Option<String>,
    pub use_tls: bool,
}

/// LDAP user info
#[derive(Debug, Clone)]
pub struct LdapUserInfo {
    pub dn: String,
    pub cn: Option<String>,
    pub email: Option<String>,
    pub groups: Vec<String>,
}

/// LDAP credential output
#[derive(Debug, Clone)]
pub struct LdapCredential {
    pub config: LdapConfig,
    pub username: String,
    pub password: SecretString,
    pub user_info: LdapUserInfo,
}

/// LDAP credential provider
pub struct LdapCredentialProvider {
    config: LdapConfig,
}

impl LdapCredentialProvider {
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }

    async fn authenticate_user(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LdapUserInfo, CredentialError> {
        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url).await
            .map_err(|e| CredentialError::ConnectionError(
                format!("LDAP connection failed: {}", e)
            ))?;

        ldap3::drive!(conn);

        // Bind with user credentials
        let bind_dn = if let Some(ref template) = self.config.bind_dn_template {
            template.replace("{username}", username)
        } else {
            format!("cn={},{}", username, self.config.base_dn)
        };

        ldap.simple_bind(&bind_dn, password).await
            .map_err(|e| CredentialError::AuthenticationFailed(
                format!("LDAP bind failed: {}", e)
            ))?;

        // Search for user info
        let filter = format!("(cn={})", username);
        let (rs, _res) = ldap.search(
            &self.config.base_dn,
            Scope::Subtree,
            &filter,
            vec!["cn", "mail", "memberOf"]
        ).await
            .map_err(|e| CredentialError::ConfigurationError(
                format!("LDAP search failed: {}", e)
            ))?
            .success()
            .map_err(|e| CredentialError::ConfigurationError(
                format!("LDAP search error: {}", e)
            ))?;

        let entry = rs.into_iter()
            .next()
            .ok_or_else(|| CredentialError::AuthenticationFailed(
                "User not found".to_string()
            ))?;

        let search_entry = SearchEntry::construct(entry);

        Ok(LdapUserInfo {
            dn: search_entry.dn,
            cn: search_entry.attrs.get("cn").and_then(|v| v.first()).cloned(),
            email: search_entry.attrs.get("mail").and_then(|v| v.first()).cloned(),
            groups: search_entry.attrs.get("memberOf").cloned().unwrap_or_default(),
        })
    }
}

#[async_trait]
impl Credential for LdapCredentialProvider {
    type Output = LdapCredential;
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

        let user_info = self.authenticate_user(username, password).await?;

        Ok(LdapCredential {
            config: self.config.clone(),
            username: username.clone(),
            password: SecretString::new(password.clone()),
            user_info,
        })
    }

    fn credential_type(&self) -> &'static str {
        "ldap"
    }

    fn supports_refresh(&self) -> bool {
        false
    }
}

#[async_trait]
impl TestableCredential for LdapCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();

        match self.retrieve(ctx).await {
            Ok(credential) => {
                let latency_ms = start.elapsed().as_millis() as u64;

                let details = TestDetails {
                    latency_ms,
                    endpoint_tested: self.config.url.clone(),
                    permissions_verified: vec!["ldap:bind".to_string()],
                    metadata: HashMap::from([
                        ("dn".to_string(), 
                         serde_json::json!(credential.user_info.dn)),
                        ("groups".to_string(), 
                         serde_json::json!(credential.user_info.groups)),
                    ]),
                };

                Ok(TestResult::success("LDAP authentication successful")
                    .with_details(details))
            }
            Err(e) => Ok(TestResult::failure(format!("LDAP auth failed: {}", e))),
        }
    }

    fn test_description(&self) -> &str {
        "Testing LDAP authentication by binding with credentials"
    }
}
```

## Key Concepts

### 1. LDAP Bind Authentication

```rust
// Simple bind with username/password
ldap.simple_bind(&bind_dn, password).await?;
```

### 2. DN Template

```rust
let config = LdapConfig {
    bind_dn_template: Some("cn={username},ou=users,dc=example,dc=com".to_string()),
    // ...
};
```

## Security Best Practices

> [!warning] TLS Required for Production
> Всегда используйте LDAPS или STARTTLS для protection паролей.

## Related Examples

- **Enterprise Auth**: [[SAML-Authentication]] - Enterprise SSO | [[Kerberos-Authentication]] - Kerberos authentication
- **OAuth2**: [[OAuth2-Flow]] - OAuth2 Authorization Code flow
- **Databases**: [[Database-PostgreSQL]] - PostgreSQL credentials

## See Also

- [[Core-Concepts|Core Concepts]]
- [[API-Reference|API Reference]]

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#ldap-protocol]]
- [[../../specs/001-credential-docs/security-spec.md#ldap-security]]

## Sources

- [ldap3 Rust Crate](https://docs.rs/ldap3/)
- [nebula-credential API](../Reference/API-Reference.md)
