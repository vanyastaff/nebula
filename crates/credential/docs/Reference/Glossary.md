---
title: Glossary - nebula-credential
tags: [glossary, reference, terminology]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
---

# Glossary

> [!NOTE] Purpose
> This glossary defines key terms used throughout the nebula-credential documentation. Terms are organized alphabetically with cross-references to related concepts.

---

## A

### Access Control List (ACL)
A list of permissions attached to a credential that specifies which users/services can perform which operations. nebula-credential supports 6 permission types: Read, Write, Delete, Rotate, Grant, Execute.

**See**: [[Advanced/Access-Control|Access Control]], [[Architecture#access-control-scoping|Architecture - Access Control]]

### AES-256-GCM
Advanced Encryption Standard with 256-bit key in Galois/Counter Mode. The symmetric encryption algorithm used by nebula-credential for encrypting credentials at rest. Provides both confidentiality and authenticity.

**See**: [[Security/Encryption|Encryption]], [[Advanced/Security-Architecture|Security Architecture]]

### API Key
A simple credential type consisting of a single secret string used to authenticate API requests. nebula-credential stores API keys hashed with BLAKE3 and supports rate limiting and rotation.

**See**: [[Examples/API-Key-Basic|API Key Example]], [[How-To/Rotate-Credentials|Credential Rotation]]

### Argon2id
A memory-hard key derivation function used to derive encryption keys from master passwords. nebula-credential uses Argon2id with 19 MiB memory cost for resistance against brute-force attacks.

**See**: [[Security/Encryption#key-derivation|Key Derivation]], [[Advanced/Key-Management|Key Management]]

### Async Trait
Rust trait with `async` methods, enabled by the `async_trait` macro or native async fn in trait (Rust 1.75+). All credential operations in nebula-credential are async for non-blocking I/O.

**See**: [[Architecture#async-trait-composition|Architecture - Async Traits]]

### Authentication
The process of verifying the identity of a user, service, or system. nebula-credential supports 7+ authentication protocols including OAuth2, SAML, LDAP, mTLS, JWT, API Keys, and Kerberos.

**See**: [[Getting-Started/Core-Concepts#authentication-vs-authorization|Core Concepts]], [[Examples/OAuth2-Flow|OAuth2 Authentication]]

### Authorization
The process of granting or denying access to resources after authentication. In nebula-credential, authorization is managed through scopes and ACLs.

**See**: [[Advanced/Access-Control|Access Control]], [[Getting-Started/Core-Concepts#authentication-vs-authorization|Core Concepts]]

### AWS Secrets Manager
Amazon Web Services managed service for storing and rotating secrets. One of five storage providers supported by nebula-credential.

**See**: [[Integrations/AWS-Secrets-Manager|AWS Secrets Manager Integration]], [[Integrations/Provider-Comparison|Provider Comparison]]

---

## B

### BLAKE3
A cryptographic hash function used by nebula-credential for hashing API keys and generating credential identifiers. Faster than SHA-256 with comparable security.

**See**: [[Security/Encryption#hashing|Cryptographic Hashing]]

### Blue-Green Rotation
A credential rotation pattern where both old (blue) and new (green) credentials remain valid during a grace period, then the old credential is revoked. Enables zero-downtime rotation.

**See**: [[How-To/Rotate-Credentials#blue-green-pattern|Blue-Green Rotation]], [[Examples/Database-Rotation|Database Rotation Example]]

### Builder Pattern
A creational design pattern for constructing complex objects step-by-step. nebula-credential uses builder pattern for all configuration types with compile-time validation.

**See**: [[Architecture#builder-pattern-for-configuration|Architecture - Builder Pattern]], [[Reference/Configuration-Options|Configuration Options]]

---

## C

### Cache
In-memory or distributed storage for frequently accessed credentials to reduce latency. nebula-credential implements write-through caching with TTL-based expiration and LRU eviction.

**See**: [[How-To/Configure-Caching|Caching Configuration]], [[Architecture#cache-layer|Cache Architecture]]

### Certificate
A digital document used to prove ownership of a public key, typically in X.509 format. Used for mTLS authentication in nebula-credential.

**See**: [[Examples/mTLS-Certificate|mTLS Certificate Example]], [[Examples/Certificate-Rotation|Certificate Rotation]]

### Circuit Breaker
A resilience pattern that prevents repeated calls to failing external services. nebula-credential uses circuit breakers for storage provider communication.

**See**: [[Architecture#error-handling-resilience|Error Handling]], [[Troubleshooting/Provider-Connectivity|Provider Connectivity Issues]]

### Client Credentials Flow
An OAuth 2.0 grant type for server-to-server authentication without user interaction. One of four OAuth2 flows supported by nebula-credential.

**See**: [[Examples/OAuth2-ClientCredentials|OAuth2 Client Credentials]], [[Examples/OAuth2-Flow#grant-types|OAuth2 Grant Types]]

### Compliance
Adherence to regulatory standards and frameworks. nebula-credential provides documentation for SOC 2, ISO 27001, HIPAA, and GDPR compliance.

**See**: [[Advanced/Compliance-SOC2|SOC 2 Compliance]], [[Advanced/Compliance-ISO27001|ISO 27001 Compliance]], [[Advanced/Compliance-HIPAA|HIPAA Compliance]], [[Advanced/Compliance-GDPR|GDPR Compliance]]

### Correlation ID
A unique identifier (UUID) propagated across all operations related to a single request, enabling distributed tracing. nebula-credential uses `trace_id` and `span_id` for observability.

**See**: [[How-To/Enable-Audit-Logging#correlation-ids|Correlation IDs]], [[Advanced/Observability-Guide|Observability Guide]]

### Credential
An authentication mechanism used to verify identity. In nebula-credential, this is represented by the `Credential` trait with `authenticate()`, `validate()`, and `refresh()` methods.

**See**: [[Getting-Started/Core-Concepts#what-is-a-credential|Core Concepts]], [[Architecture#trait-hierarchy|Trait Hierarchy]], [[Reference/API-Reference|API Reference]]

### CredentialTest Trait
A trait for validating credential functionality by making real authentication attempts. Inspired by n8n's test method, supports 4 testing strategies.

**See**: [[Advanced/Testing-Credentials|Testing Credentials]], [[Architecture#testing-validation|Testing Architecture]]

---

## D

### Distributed Locking
A synchronization mechanism to coordinate concurrent operations across multiple instances. nebula-credential uses Redlock algorithm for credential rotation coordination.

**See**: [[Architecture#concurrency-model|Concurrency Model]], [[Examples/Database-Rotation#distributed-coordination|Distributed Coordination]]

---

## E

### Encryption Key
A secret value used to encrypt and decrypt data. nebula-credential uses AES-256-GCM keys derived from master passwords via Argon2id, with key versioning for rotation.

**See**: [[Security/Encryption#encryption-keys|Encryption Keys]], [[Advanced/Key-Management|Key Management]]

### Error Hierarchy
A structured classification of error types. nebula-credential uses `thiserror` to define a comprehensive hierarchy: `CredentialError`, `StorageError`, `CryptoError`, and protocol-specific errors.

**See**: [[Architecture#error-handling-resilience|Error Handling]], [[Troubleshooting/Common-Errors|Common Errors]]

### Expiration
The point at which a credential becomes invalid. nebula-credential supports automatic refresh before expiration with configurable thresholds (default 5 minutes).

**See**: [[Architecture#credential-lifecycle-management|Credential Lifecycle]], [[Examples/OAuth2-Token-Refresh|Token Refresh]]

---

## G

### GAT (Generic Associated Type)
A Rust feature allowing trait associated types to be generic over lifetimes or other types. Used in nebula-credential for flexible async credential APIs.

**See**: [[Architecture#async-trait-composition|Architecture - Async Traits]]

### Grace Period
A time window during credential rotation where both old and new credentials remain valid. Default 24 hours in nebula-credential's blue-green rotation pattern.

**See**: [[How-To/Rotate-Credentials#grace-period|Grace Period Configuration]], [[Examples/Database-Rotation|Database Rotation]]

---

## H

### HashiCorp Vault
An identity-based secrets management system. One of five storage providers supported by nebula-credential, with support for Transit engine and KV v2.

**See**: [[Integrations/HashiCorp-Vault|HashiCorp Vault Integration]], [[Integrations/Provider-Comparison|Provider Comparison]]

### HSM (Hardware Security Module)
A physical device for managing and storing cryptographic keys. nebula-credential supports HSM integration via PKCS#11 interface for production key storage.

**See**: [[Advanced/Key-Management#hsm-integration|HSM Integration]], [[Advanced/Security-Architecture#key-storage|Key Storage]]

---

## I

### Interactive Credential
A credential flow requiring user interaction (e.g., OAuth2 authorization code flow). Represented by `InteractiveCredential` trait with `initialize()` and `resume()` methods.

**See**: [[Architecture#trait-hierarchy|Trait Hierarchy]], [[Examples/OAuth2-Flow|OAuth2 Interactive Flow]]

---

## J

### JWT (JSON Web Token)
A compact token format for representing claims between parties. nebula-credential supports JWT validation with HS256/RS256/ES256 algorithms and claims validation.

**See**: [[Examples/JWT-Validation|JWT Validation]], [[Architecture#protocol-support-matrix|Protocol Support]]

---

## K

### Kerberos
A network authentication protocol using tickets. nebula-credential supports Kerberos authentication with TGT (Ticket Granting Ticket) acquisition.

**See**: [[Examples/Kerberos-Authentication|Kerberos Example]], [[Architecture#protocol-support-matrix|Protocol Support]]

### Key Derivation
The process of deriving one or more secret keys from a master password using a key derivation function (KDF). nebula-credential uses Argon2id.

**See**: [[Security/Encryption#key-derivation|Key Derivation]], [[Advanced/Key-Management|Key Management]]

### Key Rotation
The process of replacing an encryption key with a new one and re-encrypting data. nebula-credential supports key versioning with multiple active keys during rotation.

**See**: [[Advanced/Key-Management#key-rotation|Key Rotation]], [[Security/Encryption#key-versioning|Key Versioning]]

---

## L

### LDAP (Lightweight Directory Access Protocol)
A protocol for accessing and maintaining distributed directory services. nebula-credential supports LDAP/Active Directory authentication with connection pooling and injection prevention.

**See**: [[Examples/LDAP-Authentication|LDAP Example]], [[Architecture#protocol-support-matrix|Protocol Support]]

### Lifecycle
The sequence of states a credential transitions through: Uninitialized → PendingInteraction → Authenticating → Active → Expired → Rotating → GracePeriod → Revoked. nebula-credential implements an 11-state state machine.

**See**: [[Advanced/Credential-Lifecycle|Credential Lifecycle]], [[Architecture#state-machine-architecture|State Machine]]

### LRU (Least Recently Used)
A cache eviction policy that removes least recently accessed items when capacity is reached. Used by nebula-credential cache with 1GB max size.

**See**: [[How-To/Configure-Caching#lru-eviction|LRU Eviction]], [[Architecture#cache-layer|Cache Architecture]]

---

## M

### Metadata
Information about a credential (owner, scope, creation time, etc.) stored separately from the encrypted credential payload.

**See**: [[Architecture#storage-abstraction|Storage Architecture]], [[Reference/API-Reference#credential-metadata|API Reference]]

### mTLS (Mutual TLS)
A TLS authentication method where both client and server present certificates. nebula-credential supports mTLS with X.509 certificate validation, CRL/OCSP checking.

**See**: [[Examples/mTLS-Certificate|mTLS Example]], [[Architecture#protocol-support-matrix|Protocol Support]]

---

## N

### Nonce
A number used once - a unique value for each encryption operation to prevent replay attacks. nebula-credential generates 96-bit nonces with collision prevention (monotonic counter + random + timestamp).

**See**: [[Security/Encryption#nonce-generation|Nonce Generation]], [[Advanced/Security-Architecture#replay-prevention|Replay Prevention]]

---

## O

### OAuth2 (OAuth 2.0)
An authorization framework enabling applications to obtain limited access to user accounts. nebula-credential supports all grant types: Authorization Code + PKCE, Client Credentials, Device Code, Refresh Token.

**See**: [[Examples/OAuth2-Flow|OAuth2 Flow]], [[Examples/OAuth2-GitHub|GitHub OAuth2]], [[Examples/OAuth2-Google|Google OAuth2]]

### Observability
The ability to measure system internal states through outputs (logs, metrics, traces). nebula-credential provides comprehensive audit logging, Prometheus metrics, and OpenTelemetry tracing.

**See**: [[Advanced/Observability-Guide|Observability Guide]], [[How-To/Enable-Audit-Logging|Audit Logging]]

### OpenTelemetry
A collection of APIs, SDKs, and tools for generating, collecting, and exporting telemetry data. nebula-credential integrates with OpenTelemetry for distributed tracing.

**See**: [[Advanced/Observability-Guide#opentelemetry-integration|OpenTelemetry Integration]]

### Ownership Model
A security model where every credential has a single owner (user/service account) with non-transferable ownership. Enforced by nebula-credential ACLs.

**See**: [[Advanced/Access-Control#ownership-model|Ownership Model]], [[Architecture#access-control-scoping|Access Control]]

---

## P

### Permission
An operation a principal can perform on a credential. nebula-credential defines 6 permission types: Read, Write, Delete, Rotate, Grant (modify ACL), Execute (use credential).

**See**: [[Advanced/Access-Control#permissions|Permission Types]], [[Architecture#access-control-scoping|Access Control]]

### PhantomData
A Rust type for adding type parameters to a struct without storing them. Used in nebula-credential's type-state pattern for compile-time state enforcement.

**See**: [[Advanced/Type-State-Pattern|Type-State Pattern]], [[Architecture#type-state-pattern|Architecture]]

### PKCE (Proof Key for Code Exchange)
An OAuth2 extension (RFC 7636) that prevents authorization code interception attacks. Mandatory for all Authorization Code flows in nebula-credential.

**See**: [[Examples/OAuth2-Flow#pkce|OAuth2 PKCE]], [[Security/Encryption#oauth2-security|OAuth2 Security]]

### Prometheus
An open-source monitoring and alerting toolkit. nebula-credential exports metrics in Prometheus format: latency histograms, operation counters, cache hit ratio gauge.

**See**: [[Advanced/Observability-Guide#prometheus-metrics|Prometheus Metrics]]

### Provider
A storage backend for encrypted credentials. nebula-credential supports 5 providers: Local (SQLite), AWS Secrets Manager, HashiCorp Vault, Azure Key Vault, Kubernetes Secrets.

**See**: [[Integrations/Provider-Comparison|Provider Comparison]], [[Architecture#storage-abstraction|Storage Architecture]]

---

## R

### Redlock
A distributed locking algorithm for Redis. Used by nebula-credential for coordinating concurrent credential rotation across multiple instances.

**See**: [[Architecture#concurrency-model|Concurrency Model]], [[Advanced/Rotation-Policies#distributed-locking|Distributed Locking]]

### Refresh Token
An OAuth2 token used to obtain new access tokens without user interaction. nebula-credential automatically refreshes access tokens before expiration.

**See**: [[Examples/OAuth2-Token-Refresh|Token Refresh]], [[Examples/OAuth2-Flow#refresh-flow|OAuth2 Refresh]]

### Rotation
The process of replacing a credential with a new one. nebula-credential supports 4 rotation policies: periodic, before-expiry, scheduled (cron), manual.

**See**: [[How-To/Rotate-Credentials|Credential Rotation]], [[Advanced/Rotation-Policies|Rotation Policies]]

### RotatableCredential Trait
A trait for credentials that support rotation. Extends `Credential` trait with `rotate()` method and `Policy` associated type.

**See**: [[Architecture#trait-hierarchy|Trait Hierarchy]], [[Reference/API-Reference#rotatablecredential|API Reference]]

---

## S

### SAML (Security Assertion Markup Language)
An XML-based standard for exchanging authentication and authorization data. nebula-credential supports SAML 2.0 with XML signature validation (RSA-SHA256, ECDSA-SHA256).

**See**: [[Examples/SAML-Authentication|SAML Example]], [[Architecture#protocol-support-matrix|Protocol Support]]

### Scope
A namespace that isolates credentials. nebula-credential supports three scope levels: workflow, organization, global, with hierarchical access validation.

**See**: [[Advanced/Access-Control#scope-isolation|Scope Isolation]], [[Getting-Started/Core-Concepts#scopes|Core Concepts]]

### SecretString
A zero-copy string type for sensitive data that implements `Zeroize` and `ZeroizeOnDrop`, ensuring memory is cleared when dropped. Prevents accidental logging via custom `Debug` impl.

**See**: [[Examples/SecretString-Usage|SecretString Example]], [[Architecture#zero-copy-secrets|Zero-Copy Secrets]]

### State Machine
A computational model with states and transitions. nebula-credential implements an 11-state lifecycle state machine with validated transitions.

**See**: [[Advanced/Credential-Lifecycle|Credential Lifecycle]], [[Architecture#state-machine-architecture|State Machine Architecture]]

### Storage Provider
An implementation of the `StorageProvider` trait that handles encrypted credential persistence to a specific backend.

**See**: [[Architecture#storage-abstraction|Storage Architecture]], [[Integrations/Provider-Comparison|Provider Comparison]]

---

## T

### TGT (Ticket Granting Ticket)
The initial ticket obtained from Kerberos authentication server, used to request service tickets. Managed by nebula-credential's Kerberos support.

**See**: [[Examples/Kerberos-Authentication|Kerberos Authentication]]

### TLS 1.3
The latest version of the Transport Layer Security protocol. nebula-credential enforces TLS 1.3 for all network communication with restricted cipher suites.

**See**: [[Advanced/Security-Architecture#network-security|Network Security]], [[Security/Encryption#tls-configuration|TLS Configuration]]

### TTL (Time To Live)
The duration a cached credential remains valid before expiration. Default 5 minutes in nebula-credential cache.

**See**: [[How-To/Configure-Caching#ttl-configuration|TTL Configuration]], [[Architecture#cache-layer|Cache Architecture]]

### Type-State Pattern
A Rust design pattern using `PhantomData` to encode state in the type system, enabling compile-time verification of state transitions.

**See**: [[Advanced/Type-State-Pattern|Type-State Pattern]], [[Architecture#type-state-pattern|Architecture]]

---

## Z

### Zeroization
The process of securely overwriting memory containing sensitive data before deallocation. nebula-credential uses the `zeroize` crate to zero all `SecretString` instances on drop.

**See**: [[Examples/SecretString-Usage#zeroization|Zeroization]], [[Architecture#zero-copy-secrets|Zero-Copy Secrets]], [[Advanced/Security-Architecture#memory-protection|Memory Protection]]

---

## Acronyms

| Acronym | Full Term | Definition |
|---------|-----------|------------|
| ACL | Access Control List | List of permissions for credential access |
| AES | Advanced Encryption Standard | Symmetric encryption algorithm |
| API | Application Programming Interface | Set of rules for software interaction |
| AWS | Amazon Web Services | Cloud computing platform |
| BLAKE3 | BLAKE Algorithm Version 3 | Cryptographic hash function |
| GCM | Galois/Counter Mode | Mode of operation for symmetric block ciphers |
| GAT | Generic Associated Type | Rust feature for generic trait types |
| GDPR | General Data Protection Regulation | EU data protection regulation |
| HIPAA | Health Insurance Portability and Accountability Act | US healthcare data regulation |
| HSM | Hardware Security Module | Physical crypto key storage device |
| IAM | Identity and Access Management | Framework for managing digital identities |
| ISO | International Organization for Standardization | Standards organization |
| JWT | JSON Web Token | Compact token format for claims |
| KDF | Key Derivation Function | Function for deriving keys from passwords |
| KMS | Key Management Service | Cloud service for managing encryption keys |
| LDAP | Lightweight Directory Access Protocol | Directory service protocol |
| LRU | Least Recently Used | Cache eviction policy |
| mTLS | Mutual Transport Layer Security | Two-way TLS authentication |
| NIST | National Institute of Standards and Technology | US standards agency |
| OAuth | Open Authorization | Authorization framework |
| OCSP | Online Certificate Status Protocol | Certificate revocation protocol |
| PKCE | Proof Key for Code Exchange | OAuth2 security extension |
| RBAC | Role-Based Access Control | Access control based on roles |
| RFC | Request for Comments | Internet standards document |
| SAML | Security Assertion Markup Language | XML authentication standard |
| SOC | Service Organization Control | Audit report standard |
| SSO | Single Sign-On | Authentication across multiple systems |
| TGT | Ticket Granting Ticket | Kerberos initial ticket |
| TLS | Transport Layer Security | Cryptographic protocol |
| TTL | Time To Live | Duration before expiration |
| UUID | Universally Unique Identifier | 128-bit identifier |
| XML | eXtensible Markup Language | Markup language for documents |

---

## See Also

- [[README|nebula-credential Documentation Home]]
- [[Getting-Started/Core-Concepts|Core Concepts]]
- [[Architecture|System Architecture]]
- [[Reference/API-Reference|API Reference]]
- [[Reference/Configuration-Options|Configuration Options]]
