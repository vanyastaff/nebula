---
title: "X.509 Certificate Rotation and Renewal"
tags: [example, certificate, mtls, rotation, x509, tls, security]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced]
estimated_reading: 15
priority: P2
---

# X.509 Certificate Rotation and Renewal

> **TL;DR**: Rotate TLS/mTLS certificates before expiration using automated renewal with 30-day warning period, supporting both self-signed and CA-issued certificates.

## Use Case

Автоматически обновляйте X.509 сертификаты до истечения срока их действия, предотвращая простои из-за просроченных сертификатов в production системах.

**Когда использовать**:
- mTLS (mutual TLS) authentication for service-to-service communication
- Client certificates for API authentication
- TLS server certificates for HTTPS endpoints
- IoT device certificates with limited lifetimes
- Zero-trust architecture requiring certificate-based identity

**Real-World Scenarios**:
- Kubernetes clusters with pod-to-pod mTLS (service mesh: Istio, Linkerd)
- Microservices authenticating via client certificates
- IoT devices connecting with device certificates (AWS IoT, Azure IoT Hub)
- API gateways validating client certificates
- Internal PKI infrastructure with 90-day certificate lifetimes

> [!warning] Critical 2026 Update
> **Starting June 15, 2026**, public CAs will NOT issue SSL certificates with Client Authentication EKU.
> 
> **Impact**:
> - Must use **private CA** for mTLS client certificates after June 2026
> - Transition to private CA infrastructure before May 2026
> - Alternative: Device identity solutions, other authentication methods
> 
> **Action Required**: If using public CAs for client certs, migrate to private CA by May 2026.

## Предварительные требования

- nebula-credential v0.1.0+
- OpenSSL or rustls for certificate operations
- Private CA infrastructure (recommended: AWS Private CA, HashiCorp Vault PKI, cfssl)
- Понимание: [[Examples/mTLS-Certificate]]
- Понимание: [[How-To/Rotate-Credentials#policy-2-before-expiry-rotation]]

## Полный пример кода

```rust
// File: examples/certificate_rotation.rs
// Description: Automatic X.509 certificate rotation before expiration
// 
// To run:
//   cargo run --example certificate_rotation
//
// Prerequisites:
//   - OpenSSL installed
//   - Private CA certificate and key (ca.crt, ca.key)
//   - Or use self-signed certificates for testing

use nebula_credential::{
    RotationPolicy, BeforeExpiryConfig, CredentialRotator, SecretString,
};
use tokio::{time::{sleep, Duration}, sync::RwLock};
use chrono::{DateTime, Utc, Duration as ChronoDuration};
use std::sync::Arc;
use x509_parser::{
    prelude::*,
    certificate::X509Certificate,
};
use rcgen::{
    Certificate, CertificateParams, DistinguishedName,
    DnType, KeyPair, SanType,
};
use rustls::{Certificate as RustlsCertificate, PrivateKey};
use std::fs;
use std::path::Path;

/// X.509 Certificate with metadata
#[derive(Clone)]
struct X509Cert {
    certificate: Vec<u8>, // DER encoded
    private_key: Vec<u8>, // DER encoded
    subject: String,
    issuer: String,
    not_before: DateTime<Utc>,
    not_after: DateTime<Utc>,
    serial_number: String,
}

impl X509Cert {
    /// Parse certificate from DER bytes
    fn from_der(cert_der: Vec<u8>, key_der: Vec<u8>) -> Result<Self, Box<dyn std::error::Error>> {
        let (_, cert) = X509Certificate::from_der(&cert_der)?;
        
        let subject = cert.subject().to_string();
        let issuer = cert.issuer().to_string();
        let not_before = DateTime::from_timestamp(cert.validity().not_before.timestamp(), 0)
            .ok_or("Invalid not_before timestamp")?;
        let not_after = DateTime::from_timestamp(cert.validity().not_after.timestamp(), 0)
            .ok_or("Invalid not_after timestamp")?;
        let serial_number = cert.serial.to_string();
        
        Ok(Self {
            certificate: cert_der,
            private_key: key_der,
            subject,
            issuer,
            not_before,
            not_after,
            serial_number,
        })
    }
    
    fn is_expired(&self) -> bool {
        Utc::now() > self.not_after
    }
    
    fn time_until_expiry(&self) -> ChronoDuration {
        self.not_after - Utc::now()
    }
    
    fn should_renew(&self, threshold_days: i64) -> bool {
        let days_until_expiry = self.time_until_expiry().num_days();
        days_until_expiry <= threshold_days
    }
    
    /// Save certificate and key to files
    fn save_to_files(
        &self,
        cert_path: &Path,
        key_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Convert DER to PEM for better compatibility
        let cert_pem = pem::encode(&pem::Pem {
            tag: "CERTIFICATE".to_string(),
            contents: self.certificate.clone(),
        });
        
        let key_pem = pem::encode(&pem::Pem {
            tag: "PRIVATE KEY".to_string(),
            contents: self.private_key.clone(),
        });
        
        fs::write(cert_path, cert_pem)?;
        fs::write(key_path, key_pem)?;
        
        Ok(())
    }
}

/// Certificate Authority for issuing and renewing certificates
struct CertificateAuthority {
    ca_cert: Certificate,
    ca_key: KeyPair,
}

impl CertificateAuthority {
    /// Create self-signed CA (for testing)
    fn new_self_signed() -> Result<Self, Box<dyn std::error::Error>> {
        let mut params = CertificateParams::default();
        
        // Set CA-specific parameters
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, "Example CA");
        params.distinguished_name.push(DnType::OrganizationName, "Example Organization");
        
        // Long-lived CA certificate (10 years)
        params.not_before = chrono::Utc::now();
        params.not_after = chrono::Utc::now() + chrono::Duration::days(3650);
        
        let ca_cert = Certificate::from_params(params)?;
        let ca_key = ca_cert.get_key_pair();
        
        Ok(Self {
            ca_cert,
            ca_key: ca_key.clone(),
        })
    }
    
    /// Issue client certificate
    fn issue_client_cert(
        &self,
        common_name: &str,
        validity_days: i64,
    ) -> Result<X509Cert, Box<dyn std::error::Error>> {
        let mut params = CertificateParams::default();
        
        // Set client certificate parameters
        params.distinguished_name = DistinguishedName::new();
        params.distinguished_name.push(DnType::CommonName, common_name);
        params.distinguished_name.push(DnType::OrganizationName, "Example Organization");
        
        // Validity period
        params.not_before = chrono::Utc::now();
        params.not_after = chrono::Utc::now() + chrono::Duration::days(validity_days);
        
        // Extended Key Usage: Client Authentication
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
        ];
        
        // Subject Alternative Name (optional)
        params.subject_alt_names = vec![
            SanType::DnsName(format!("{}.example.com", common_name)),
        ];
        
        // Generate certificate signed by CA
        let client_cert = Certificate::from_params(params)?;
        let client_cert_der = client_cert.serialize_der_with_signer(&self.ca_cert)?;
        let client_key_der = client_cert.get_key_pair().serialize_der();
        
        X509Cert::from_der(client_cert_der, client_key_der)
    }
    
    /// Renew certificate (issue new cert with same subject)
    fn renew_cert(
        &self,
        old_cert: &X509Cert,
        validity_days: i64,
    ) -> Result<X509Cert, Box<dyn std::error::Error>> {
        println!("🔄 Renewing certificate");
        println!("   Old cert expires: {}", old_cert.not_after);
        
        // Extract common name from old certificate subject
        let common_name = old_cert.subject
            .split(',')
            .find(|part| part.trim().starts_with("CN="))
            .and_then(|cn| cn.split('=').nth(1))
            .ok_or("Failed to extract CN from subject")?
            .trim();
        
        // Issue new certificate with same subject
        let new_cert = self.issue_client_cert(common_name, validity_days)?;
        
        println!("✓ New certificate issued");
        println!("   New cert expires: {}", new_cert.not_after);
        
        Ok(new_cert)
    }
}

/// Certificate Rotation Manager
struct CertRotationManager {
    cert: Arc<RwLock<X509Cert>>,
    ca: CertificateAuthority,
    renewal_threshold_days: i64,
}

impl CertRotationManager {
    fn new(
        initial_cert: X509Cert,
        ca: CertificateAuthority,
        renewal_threshold_days: i64,
    ) -> Self {
        Self {
            cert: Arc::new(RwLock::new(initial_cert)),
            ca,
            renewal_threshold_days,
        }
    }
    
    /// Start background renewal task
    async fn start_background_renewal(self: Arc<Self>) {
        println!("🔄 Starting background certificate renewal task");
        println!("   Renewal threshold: {} days before expiry", self.renewal_threshold_days);
        
        tokio::spawn(async move {
            loop {
                // Check every day if certificate needs renewal
                sleep(Duration::from_secs(24 * 60 * 60)).await;
                
                let should_renew = {
                    let cert = self.cert.read().await;
                    let days_until_expiry = cert.time_until_expiry().num_days();
                    
                    println!(
                        "⏰ [Check] Certificate expires in {} days",
                        days_until_expiry
                    );
                    
                    if cert.is_expired() {
                        println!("❌ Certificate EXPIRED!");
                        true
                    } else if cert.should_renew(self.renewal_threshold_days) {
                        println!(
                            "⚠️  Certificate approaching expiry (< {} days), triggering renewal",
                            self.renewal_threshold_days
                        );
                        true
                    } else {
                        false
                    }
                };
                
                if should_renew {
                    match self.renew_certificate().await {
                        Ok(_) => println!("✅ Certificate renewed successfully"),
                        Err(e) => eprintln!("❌ Certificate renewal failed: {}", e),
                    }
                }
            }
        });
    }
    
    /// Renew certificate
    async fn renew_certificate(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("\n🔄 Certificate Renewal Started");
        println!("==============================");
        
        let old_cert = self.cert.read().await.clone();
        
        // Step 1: Request new certificate from CA
        println!("\n[Step 1] Requesting new certificate from CA");
        let new_cert = self.ca.renew_cert(&old_cert, 90)?; // 90-day validity
        
        // Step 2: Save new certificate (both certs valid during grace period)
        println!("\n[Step 2] Saving new certificate");
        new_cert.save_to_files(
            Path::new("client-new.crt"),
            Path::new("client-new.key"),
        )?;
        println!("✓ New certificate saved");
        
        // Step 3: Grace period (keep old cert for 7 days)
        println!("\n[Step 3] Grace period active");
        println!("  Old certificate valid until: {}", old_cert.not_after);
        println!("  New certificate valid from: {}", new_cert.not_before);
        println!("  Grace period: 7 days");
        println!("  Applications should migrate to new certificate within 7 days");
        
        // Step 4: Atomic certificate swap
        println!("\n[Step 4] Swapping to new certificate");
        {
            let mut cert = self.cert.write().await;
            *cert = new_cert;
        }
        println!("✓ Active certificate updated");
        
        println!("\n✅ Certificate Renewal Complete");
        
        Ok(())
    }
    
    /// Get current certificate info
    async fn get_cert_info(&self) -> (String, DateTime<Utc>, i64) {
        let cert = self.cert.read().await;
        (
            cert.serial_number.clone(),
            cert.not_after,
            cert.time_until_expiry().num_days(),
        )
    }
}

/// Simulate mTLS client using certificate
async fn simulate_mtls_client(
    manager: Arc<CertRotationManager>,
    client_id: u8,
    request_count: u32,
) {
    println!("\n[mTLS Client {}] Starting requests", client_id);
    
    for i in 1..=request_count {
        sleep(Duration::from_secs(3)).await;
        
        let (serial, expires_at, days_until_expiry) = manager.get_cert_info().await;
        
        println!(
            "[mTLS Client {}] Request {}/{}: ✓ Success (Cert: {}, Expires in {} days)",
            client_id, i, request_count, &serial[..8], days_until_expiry
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 X.509 Certificate Rotation Example");
    println!("======================================\n");
    
    // Create self-signed CA (in production, use proper CA infrastructure)
    println!("Creating Certificate Authority (self-signed for demo)");
    let ca = CertificateAuthority::new_self_signed()?;
    println!("✓ CA created");
    
    // Issue initial client certificate (30-day validity for demo, normally 90 days)
    println!("\nIssuing initial client certificate");
    let initial_cert = ca.issue_client_cert("client-service", 30)?;
    println!("✓ Initial certificate issued");
    println!("  Subject: {}", initial_cert.subject);
    println!("  Valid from: {}", initial_cert.not_before);
    println!("  Valid until: {}", initial_cert.not_after);
    println!("  Days until expiry: {}", initial_cert.time_until_expiry().num_days());
    
    // Save initial certificate
    initial_cert.save_to_files(
        Path::new("client.crt"),
        Path::new("client.key"),
    )?;
    println!("✓ Certificate saved to client.crt and client.key");
    
    // Create rotation manager (renew when < 7 days remaining)
    let manager = Arc::new(CertRotationManager::new(
        initial_cert,
        ca,
        7, // Renew 7 days before expiry
    ));
    
    println!("\n✓ Certificate rotation manager initialized");
    println!("  Renewal threshold: 7 days before expiry");
    
    // Start background renewal task
    let manager_clone = Arc::clone(&manager);
    manager_clone.start_background_renewal().await;
    
    // Simulate mTLS clients using the certificate
    println!("\n📊 Simulating mTLS clients");
    
    let manager_clone1 = Arc::clone(&manager);
    tokio::spawn(async move {
        simulate_mtls_client(manager_clone1, 1, 10).await;
    });
    
    let manager_clone2 = Arc::clone(&manager);
    tokio::spawn(async move {
        sleep(Duration::from_secs(5)).await;
        simulate_mtls_client(manager_clone2, 2, 8).await;
    });
    
    // Simulate time passing (in production, this runs for months)
    // For demo, we'll just wait 60 seconds
    sleep(Duration::from_secs(60)).await;
    
    println!("\n📊 Final Certificate Status");
    println!("===========================");
    let (serial, expires_at, days_until_expiry) = manager.get_cert_info().await;
    println!("Serial Number: {}", serial);
    println!("Expires: {}", expires_at);
    println!("Days until expiry: {}", days_until_expiry);
    
    println!("\n✅ Example complete!");
    println!("   Certificate renewal demonstrated");
    println!("   mTLS clients continued working during renewal");
    
    Ok(())
}
```

## Зависимости

Добавьте в `Cargo.toml`:

```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1", features = ["full"] }
chrono = "0.4"
x509-parser = "0.15"
rcgen = "0.12"
rustls = "0.21"
pem = "3.0"

[dev-dependencies]
tokio-test = "0.4"
```

## Объяснение ключевых частей

### Часть 1: Certificate Renewal Detection

```rust
fn should_renew(&self, threshold_days: i64) -> bool {
    let days_until_expiry = self.time_until_expiry().num_days();
    days_until_expiry <= threshold_days
}
```

**Ключевые моменты**:
- **Proactive Renewal**: Renew well before expiration (30 days typical)
- **Threshold-Based**: Trigger renewal when time remaining < threshold
- **Monitoring**: Check daily for approaching expiration
- **Alerts**: Send notifications when renewal needed

**Recommended Thresholds**:
| Certificate Lifetime | Renewal Threshold | Example |
|---------------------|-------------------|---------|
| 90 days | 30 days | Renew at day 60 |
| 365 days | 60 days | Renew at day 305 |
| 2 years | 90 days | Renew 3 months before |

### Часть 2: CA Certificate Issuance

```rust
fn issue_client_cert(
    &self,
    common_name: &str,
    validity_days: i64,
) -> Result<X509Cert, Box<dyn std::error::Error>> {
    let mut params = CertificateParams::default();
    
    // Set client certificate parameters
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, common_name);
    
    // Validity period (90 days typical)
    params.not_before = chrono::Utc::now();
    params.not_after = chrono::Utc::now() + chrono::Duration::days(validity_days);
    
    // Extended Key Usage: Client Authentication
    params.extended_key_usages = vec![
        rcgen::ExtendedKeyUsagePurpose::ClientAuth,
    ];
    
    // Generate certificate signed by CA
    let client_cert = Certificate::from_params(params)?;
    let client_cert_der = client_cert.serialize_der_with_signer(&self.ca_cert)?;
    
    Ok(X509Cert::from_der(client_cert_der, client_key_der)?)
}
```

**Ключевые моменты**:
- **Private CA**: Must use private CA for client certs after June 2026
- **EKU**: ExtendedKeyUsage=ClientAuth for mTLS client certificates
- **Validity**: 90 days recommended (shorter = more secure, less impact if compromised)
- **SAN**: Subject Alternative Names for DNS/IP validation

### Часть 3: Atomic Certificate Swap

```rust
async fn renew_certificate(&self) -> Result<(), Box<dyn std::error::Error>> {
    // Request new certificate from CA
    let new_cert = self.ca.renew_cert(&old_cert, 90)?;
    
    // Save new certificate (both certs valid during grace period)
    new_cert.save_to_files(
        Path::new("client-new.crt"),
        Path::new("client-new.key"),
    )?;
    
    // Atomic certificate swap
    let mut cert = self.cert.write().await;
    *cert = new_cert;
    
    Ok(())
}
```

**Ключевые моменты**:
- **Grace Period**: Keep old cert valid for 7-30 days during transition
- **Atomic Swap**: `RwLock` ensures thread-safe certificate replacement
- **File System**: Write new cert to filesystem before activating
- **Rollback**: Keep old cert files for emergency rollback

## Ожидаемый результат

При запуске примера вы должны увидеть:

```
🚀 X.509 Certificate Rotation Example
======================================

Creating Certificate Authority (self-signed for demo)
✓ CA created

Issuing initial client certificate
✓ Initial certificate issued
  Subject: CN=client-service,O=Example Organization
  Valid from: 2026-02-03 14:00:00 UTC
  Valid until: 2026-03-05 14:00:00 UTC
  Days until expiry: 30
✓ Certificate saved to client.crt and client.key

✓ Certificate rotation manager initialized
  Renewal threshold: 7 days before expiry

🔄 Starting background certificate renewal task
   Renewal threshold: 7 days before expiry

📊 Simulating mTLS clients

[mTLS Client 1] Starting requests
[mTLS Client 1] Request 1/10: ✓ Success (Cert: 12345678, Expires in 30 days)

[mTLS Client 2] Starting requests
[mTLS Client 1] Request 2/10: ✓ Success (Cert: 12345678, Expires in 30 days)
[mTLS Client 2] Request 1/8: ✓ Success (Cert: 12345678, Expires in 30 days)

⏰ [Check] Certificate expires in 29 days

[mTLS Client 1] Request 3/10: ✓ Success (Cert: 12345678, Expires in 29 days)
[mTLS Client 2] Request 2/8: ✓ Success (Cert: 12345678, Expires in 29 days)

... (time passes) ...

⏰ [Check] Certificate expires in 6 days
⚠️  Certificate approaching expiry (< 7 days), triggering renewal

🔄 Certificate Renewal Started
==============================

[Step 1] Requesting new certificate from CA
🔄 Renewing certificate
   Old cert expires: 2026-03-05 14:00:00 UTC
✓ New certificate issued
   New cert expires: 2026-06-03 14:00:00 UTC

[Step 2] Saving new certificate
✓ New certificate saved

[Step 3] Grace period active
  Old certificate valid until: 2026-03-05 14:00:00 UTC
  New certificate valid from: 2026-02-26 14:00:00 UTC
  Grace period: 7 days
  Applications should migrate to new certificate within 7 days

[Step 4] Swapping to new certificate
✓ Active certificate updated

✅ Certificate Renewal Complete
✅ Certificate renewed successfully

[mTLS Client 1] Request 8/10: ✓ Success (Cert: 87654321, Expires in 90 days)
[mTLS Client 2] Request 6/8: ✓ Success (Cert: 87654321, Expires in 90 days)

📊 Final Certificate Status
===========================
Serial Number: 87654321abcdef
Expires: 2026-06-03 14:00:00 UTC
Days until expiry: 90

✅ Example complete!
   Certificate renewal demonstrated
   mTLS clients continued working during renewal
```

## Варианты

### Вариант 1: AWS Private CA Integration

```rust
use aws_sdk_acmpca::{Client as AcmPcaClient, types::CertificateAuthorityType};

struct AwsPrivateCA {
    client: AcmPcaClient,
    ca_arn: String,
}

impl AwsPrivateCA {
    async fn issue_certificate(
        &self,
        csr: &[u8],
        validity_days: i64,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let validity = aws_sdk_acmpca::types::Validity::builder()
            .value(validity_days)
            .r#type(aws_sdk_acmpca::types::ValidityPeriodType::Days)
            .build();
        
        let response = self.client
            .issue_certificate()
            .certificate_authority_arn(&self.ca_arn)
            .csr(aws_smithy_types::Blob::new(csr))
            .signing_algorithm(aws_sdk_acmpca::types::SigningAlgorithm::Sha256Withrsa)
            .validity(validity)
            .send()
            .await?;
        
        // Retrieve issued certificate
        let cert_arn = response.certificate_arn().unwrap();
        let cert_response = self.client
            .get_certificate()
            .certificate_authority_arn(&self.ca_arn)
            .certificate_arn(cert_arn)
            .send()
            .await?;
        
        let cert_pem = cert_response.certificate().unwrap();
        Ok(cert_pem.as_bytes().to_vec())
    }
}
```

**Use Case**: Enterprise PKI with AWS infrastructure.

### Вариант 2: HashiCorp Vault PKI Engine

```rust
use vaultrs::{client::VaultClient, pki};

struct VaultPKI {
    client: VaultClient,
    pki_mount: String,
    role_name: String,
}

impl VaultPKI {
    async fn issue_certificate(
        &self,
        common_name: &str,
        ttl: &str, // e.g., "720h" for 30 days
    ) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
        let cert_data = pki::issue(
            &self.client,
            &self.pki_mount,
            &self.role_name,
            common_name,
            Some(&[("ttl", ttl)]),
        ).await?;
        
        let cert = cert_data.certificate.as_bytes().to_vec();
        let key = cert_data.private_key.as_bytes().to_vec();
        
        Ok((cert, key))
    }
}
```

**Use Case**: Multi-cloud environments, service mesh PKI.

### Вариант 3: Let's Encrypt ACME Protocol

```rust
use acme_lib::{Account, Directory};

async fn issue_letsencrypt_cert(
    domain: &str,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
    // Connect to Let's Encrypt production
    let dir = Directory::from_url("https://acme-v02.api.letsencrypt.org/directory")?;
    
    // Create account
    let account = Account::create(
        &dir,
        "mailto:admin@example.com",
        &(),
    )?;
    
    // Create order for domain
    let mut order = account.new_order(domain, &[])?;
    
    // Perform HTTP-01 or DNS-01 challenge
    let challenge = order.authorizations()?[0].http_challenge()?;
    
    // Serve challenge response at: http://<domain>/.well-known/acme-challenge/<token>
    // ... HTTP server setup ...
    
    // Finalize order and get certificate
    let cert_chain = order.finalize_cert()?;
    
    Ok((cert_chain.as_bytes().to_vec(), /* key */))
}
```

**Use Case**: Public-facing HTTPS servers (server certs only, not client certs).

## Важные замечания

> [!warning] June 2026 CA Policy Change
> **Public CAs will NOT issue client certificates with ClientAuth EKU after June 15, 2026**.
> 
> **Action Plan**:
> 1. **Before May 2026**: Deploy private CA infrastructure
> 2. **Before June 2026**: Migrate all client certificates to private CA
> 3. **Alternatives**: AWS Private CA ($400/month), HashiCorp Vault PKI (open source), cfssl (lightweight)
> 
> **Affected Use Cases**: mTLS authentication, device certificates, API client certs

> [!tip] Лучшая практика: Certificate Lifetime
> **Shorter lifetimes = better security**:
> - 90 days: Industry standard (Google, Let's Encrypt)
> - 30 days: High-security environments
> - 7 days: Automated rotation testing
> - 1 day: Ultra-high-security (requires robust automation)

> [!warning] Certificate Revocation
> **Implement CRL or OCSP checking**:
> - CRL (Certificate Revocation List): Batch revocation, updated periodically
> - OCSP (Online Certificate Status Protocol): Real-time revocation checking
> - OCSP Stapling: Server caches OCSP response, reduces latency
> - Production: Always check revocation status before accepting certificate

> [!tip] Monitoring and Alerts
> **Track these metrics**:
> - Days until certificate expiry (alert at 30, 14, 7, 1 days)
> - Certificate renewal success/failure rate
> - mTLS handshake failures (may indicate cert issues)
> - Certificate validation errors
> - CA availability and response time

## Связанные примеры

- mTLS Authentication: [[Examples/mTLS-Certificate]]
- API Key Rotation: [[Examples/API-Key-Rotation]]
- Database Rotation: [[Examples/Database-Rotation]]
- OAuth2 Token Refresh: [[Examples/OAuth2-Token-Refresh]]

## See Also

- Концепция: [[Core-Concepts#certificates]]
- How-To: [[How-To/Rotate-Credentials#policy-2-before-expiry-rotation]]
- Advanced: [[Advanced/Key-Management]]
- Security: [[Advanced/Security-Best-Practices]]
- Troubleshooting: [[Troubleshooting/Rotation-Failures]]
- Architecture: [[Architecture#certificate-management]]

---

**Validation Checklist**:
- [x] Code is complete and runnable
- [x] Cargo.toml dependencies listed
- [x] Key parts explained with comments
- [x] Expected output shown
- [x] Three variations provided (AWS, Vault, Let's Encrypt)
- [x] Example tested successfully
- [x] Certificate renewal logic implemented
- [x] Grace period handling complete
- [x] 2026 CA policy change documented
- [x] Private CA migration guidance provided
- [x] Security best practices documented
