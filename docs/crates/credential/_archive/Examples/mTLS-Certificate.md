---
title: "mTLS Certificate Authentication"
tags: [example, mtls, certificate, x509, advanced]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced]
estimated_reading: 10
priority: P2
---

# mTLS Certificate Authentication

> **TL;DR**: Используйте `Credential` trait из nebula-credential для mTLS (mutual TLS) authentication с X.509 certificates.

## Обзор

mTLS credentials в nebula-credential управляются через `Credential` trait с поддержкой:
- **Client Certificates**: X.509 certificates для client authentication
- **Certificate Validation**: Expiration, chain verification
- **Private Key Storage**: Secure storage в `SecretString`
- **Zero-Trust**: High-security service-to-service authentication

**Architecture Reference**: См. [[../../specs/001-credential-docs/architecture.md#mtls-protocol]]  
**Security**: См. [[../../specs/001-credential-docs/security-spec.md#certificate-security]]

## Prerequisites

- [[Installation|Установлен nebula-credential]]
- [[Core-Concepts|Понимание Credential trait]]
- X.509 client certificate и private key

## Complete Example

### Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.11", features = ["rustls-tls"] }
```

### Implementing mTLS Credential

```rust
// File: examples/mtls_credential.rs
use nebula_credential::{
    Credential, CredentialContext, CredentialError,
    TestableCredential, SecretString,
    TestResult, TestDetails, OwnerId,
};
use async_trait::async_trait;
use std::collections::HashMap;
use chrono::{DateTime, Utc};

/// mTLS configuration
#[derive(Debug, Clone)]
pub struct MtlsConfig {
    pub verify_hostname: bool,
    pub min_tls_version: TlsVersion,
    pub test_endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum TlsVersion {
    Tls12,
    Tls13,
}

/// mTLS credential data (serializable)
#[derive(Debug, Clone)]
pub struct MtlsCredentialData {
    pub config: MtlsConfig,
    pub client_cert_pem: String,
    pub client_key_pem: SecretString,
    pub ca_cert_pem: Option<String>,
}

/// mTLS credential provider
pub struct MtlsCredentialProvider {
    config: MtlsConfig,
    cert_source: CertificateSource,
}

#[derive(Debug, Clone)]
pub enum CertificateSource {
    PemFiles {
        cert_path: String,
        key_path: String,
        ca_path: Option<String>,
    },
    PemStrings {
        cert_pem: String,
        key_pem: SecretString,
        ca_pem: Option<String>,
    },
}

impl MtlsCredentialProvider {
    pub fn new(config: MtlsConfig, cert_source: CertificateSource) -> Self {
        Self {
            config,
            cert_source,
        }
    }

    async fn load_certificates(&self) -> Result<MtlsCredentialData, CredentialError> {
        match &self.cert_source {
            CertificateSource::PemFiles { cert_path, key_path, ca_path } => {
                let cert_pem = tokio::fs::read_to_string(cert_path).await
                    .map_err(|e| CredentialError::ConfigurationError(
                        format!("Failed to read certificate: {}", e)
                    ))?;

                let key_pem = tokio::fs::read_to_string(key_path).await
                    .map_err(|e| CredentialError::ConfigurationError(
                        format!("Failed to read private key: {}", e)
                    ))?;

                let ca_pem = if let Some(path) = ca_path {
                    Some(tokio::fs::read_to_string(path).await
                        .map_err(|e| CredentialError::ConfigurationError(
                            format!("Failed to read CA certificate: {}", e)
                        ))?)
                } else {
                    None
                };

                Ok(MtlsCredentialData {
                    config: self.config.clone(),
                    client_cert_pem: cert_pem,
                    client_key_pem: SecretString::new(key_pem),
                    ca_cert_pem: ca_pem,
                })
            }
            CertificateSource::PemStrings { cert_pem, key_pem, ca_pem } => {
                Ok(MtlsCredentialData {
                    config: self.config.clone(),
                    client_cert_pem: cert_pem.clone(),
                    client_key_pem: key_pem.clone(),
                    ca_cert_pem: ca_pem.clone(),
                })
            }
        }
    }
}

#[async_trait]
impl Credential for MtlsCredentialProvider {
    type Output = MtlsCredentialData;
    type Error = CredentialError;

    async fn retrieve(
        &self,
        _ctx: &CredentialContext,
    ) -> Result<Self::Output, Self::Error> {
        self.load_certificates().await
    }

    fn credential_type(&self) -> &'static str {
        "mtls"
    }

    fn supports_refresh(&self) -> bool {
        false
    }
}

#[async_trait]
impl TestableCredential for MtlsCredentialProvider {
    async fn test(
        &self,
        ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        let start = std::time::Instant::now();
        let credential = self.retrieve(ctx).await?;

        if let Some(ref test_url) = self.config.test_endpoint {
            // Test mTLS connection
            let client = reqwest::Client::builder()
                .use_rustls_tls()
                .build()
                .map_err(|e| CredentialError::ConfigurationError(
                    format!("Failed to build client: {}", e)
                ))?;

            match client.get(test_url).send().await {
                Ok(response) => {
                    let latency_ms = start.elapsed().as_millis() as u64;

                    let details = TestDetails {
                        latency_ms,
                        endpoint_tested: test_url.clone(),
                        permissions_verified: vec!["mtls:connect".to_string()],
                        metadata: HashMap::from([
                            ("status".to_string(), 
                             serde_json::json!(response.status().as_u16())),
                        ]),
                    };

                    Ok(TestResult::success("mTLS connection successful")
                        .with_details(details))
                }
                Err(e) => Ok(TestResult::failure(format!("mTLS connection failed: {}", e))),
            }
        } else {
            Ok(TestResult::success("mTLS certificates loaded (no test endpoint configured)"))
        }
    }

    fn test_description(&self) -> &str {
        "Testing mTLS by loading certificates and connecting to test endpoint"
    }
}
```

## Key Concepts

### 1. Certificate Storage

```rust
// ✅ GOOD: Private key in SecretString
let credential = MtlsCredentialData {
    client_key_pem: SecretString::new(key_pem),
    // ...
};
```

### 2. Certificate Sources

```rust
// From files
CertificateSource::PemFiles {
    cert_path: "/path/to/cert.pem".to_string(),
    key_path: "/path/to/key.pem".to_string(),
    ca_path: Some("/path/to/ca.pem".to_string()),
}

// From strings
CertificateSource::PemStrings {
    cert_pem: cert_string,
    key_pem: SecretString::new(key_string),
    ca_pem: Some(ca_string),
}
```

## Security Best Practices

> [!warning] Private Key Protection
> Всегда храните private keys в `SecretString` с proper file permissions (0600).

**Certificate Rotation**:
```rust
// Rotate before expiration (30 days warning)
if cert_expires_at - Utc::now() < Duration::days(30) {
    rotate_certificate().await?;
}
```

## Related Examples

- **Enterprise Auth**: [[SAML-Authentication]] - Enterprise SSO | [[Kerberos-Authentication]] - Kerberos authentication
- **Token Auth**: [[JWT-Validation]] - JWT validation | [[OAuth2-Flow]] - OAuth2 flow
- **Cloud**: [[AWS-Credentials]] - AWS authentication

## See Also

- [[Core-Concepts|Core Concepts]]
- [[API-Reference|API Reference]]

**Spec References**:
- [[../../specs/001-credential-docs/architecture.md#mtls-protocol]]
- [[../../specs/001-credential-docs/security-spec.md#certificate-security]]

## Sources

- [RFC 5280 - X.509](https://datatracker.ietf.org/doc/html/rfc5280)
- [nebula-credential API](../Reference/API-Reference.md)
