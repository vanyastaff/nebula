# Feature Specification: Production-Ready Storage Backends

**Feature Branch**: `002-storage-backends`  
**Created**: 2026-02-03  
**Status**: Draft  
**Input**: Implement Phase 2 of nebula-credential roadmap: Storage Backends including production-ready storage providers (Local, AWS Secrets Manager, Azure Key Vault, HashiCorp Vault, Kubernetes Secrets)

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Store Credentials in Local Encrypted Storage (Priority: P1)

A developer working on a local development machine needs to securely store API credentials without requiring cloud provider accounts, ensuring credentials are encrypted at rest with atomic write operations to prevent corruption.

**Why this priority**: Local storage is the foundation for all deployments and enables immediate MVP value without external dependencies. It's essential for development, testing, and on-premise deployments where cloud providers aren't available or desired.

**Independent Test**: Can be fully tested by storing a credential to a local directory, verifying the file is encrypted and has correct permissions (0600 on Unix), then reading it back and confirming the decrypted value matches. Test directory creation, atomic writes, and corruption resistance independently.

**Acceptance Scenarios**:

1. **Given** a LocalStorageProvider configured with directory `~/.nebula/credentials`, **When** a credential with ID "github_token" is stored, **Then** a file is created at `~/.nebula/credentials/github_token.enc` with permissions 0600 (owner-read-write only)
2. **Given** the storage directory does not exist, **When** LocalStorageProvider is initialized, **Then** the directory is automatically created with permissions 0700 (owner-only access)
3. **Given** a credential write is in progress, **When** the write operation completes, **Then** the system uses atomic rename (write-to-temp, then rename) to prevent partial writes or corruption
4. **Given** credentials are stored in local storage, **When** listing all credentials, **Then** the system returns all credential IDs without reading or decrypting the actual credential data
5. **Given** a credential exists in local storage, **When** deleting the credential, **Then** the file is securely deleted and subsequent retrieve operations return NotFound error

---

### User Story 2 - Integrate with AWS Secrets Manager (Priority: P2)

A platform engineer deploying to AWS needs to store credentials in AWS Secrets Manager to leverage AWS KMS encryption, automatic key rotation, and IAM-based access control without managing encryption keys directly.

**Why this priority**: AWS is a major cloud platform and provides enterprise-grade security features. This enables production deployments on AWS infrastructure while delegating encryption and key management to AWS KMS.

**Independent Test**: Can be fully tested by creating an AWS Secrets Manager client with test credentials, storing a secret, retrieving it, and verifying the value matches. Test IAM permission errors, network timeouts, and automatic retry with exponential backoff independently.

**Acceptance Scenarios**:

1. **Given** AWS credentials with secretsmanager:CreateSecret permission, **When** storing a credential, **Then** the credential is encrypted with AWS KMS and stored in AWS Secrets Manager with configurable tags
2. **Given** an AWS Secrets Manager credential exists, **When** retrieving the credential, **Then** the system automatically decrypts it using AWS KMS and returns the plaintext value
3. **Given** an AWS API call fails with a transient error (503, network timeout), **When** the operation is retried, **Then** the system uses exponential backoff starting at 100ms, doubling up to 5 times before failing
4. **Given** AWS credentials lack required IAM permissions, **When** attempting to store or retrieve a credential, **Then** the system returns a clear StorageError::PermissionDenied with the missing permission details
5. **Given** AWS Secrets Manager is unavailable, **When** operations timeout after 5 seconds, **Then** the system returns StorageError::Timeout with duration and operation details

---

### User Story 3 - Integrate with Azure Key Vault (Priority: P2)

A platform engineer deploying to Azure needs to store credentials in Azure Key Vault to leverage Azure-managed hardware security modules (HSMs), managed identity authentication, and compliance with Azure security standards.

**Why this priority**: Azure is a major cloud platform with strong enterprise adoption. Azure Key Vault provides HSM-backed encryption and integrates seamlessly with Azure managed identities, eliminating credential management for Azure workloads.

**Independent Test**: Can be fully tested by creating an Azure Key Vault client with managed identity or service principal, storing a secret, retrieving it, and verifying RBAC permissions are enforced. Test managed identity auth vs service principal auth independently.

**Acceptance Scenarios**:

1. **Given** Azure Managed Identity with Key Vault Secrets User role, **When** storing a credential, **Then** authentication happens automatically without explicit credentials and the secret is stored in Azure Key Vault
2. **Given** an Azure Key Vault secret exists, **When** retrieving the credential, **Then** the system automatically handles token refresh if the access token has expired
3. **Given** Azure Key Vault requires specific RBAC permissions, **When** insufficient permissions exist, **Then** the system returns StorageError::PermissionDenied with the required role (e.g., "Key Vault Secrets Officer")
4. **Given** Azure Key Vault operations require tags, **When** storing a credential with metadata, **Then** the metadata is converted to Azure Key Vault tags for organization and audit purposes
5. **Given** the Azure environment, **When** soft-delete is enabled, **Then** deleted credentials enter a recovery period and can be purged or recovered within the retention window

---

### User Story 4 - Integrate with HashiCorp Vault (Priority: P2)

A platform engineer using HashiCorp Vault needs to store credentials using Vault's KV v2 secrets engine to leverage versioning, audit logging, and Vault's policy-based access control across multi-cloud or on-premise infrastructure.

**Why this priority**: HashiCorp Vault is the industry-standard secret management solution for hybrid and multi-cloud deployments. It provides advanced features like versioning, audit logging, and dynamic secrets that are critical for enterprise environments.

**Independent Test**: Can be fully tested by connecting to a Vault server with a valid token, storing a secret in KV v2 engine, retrieving it, and verifying version metadata. Test token renewal, policy enforcement, and version history independently.

**Acceptance Scenarios**:

1. **Given** a Vault token with read/write permissions on path "secret/data/nebula/*", **When** storing a credential, **Then** the credential is stored in Vault KV v2 engine with automatic versioning
2. **Given** a Vault token is nearing expiration (TTL < 1 hour), **When** performing operations, **Then** the system automatically renews the token before it expires using Vault's token renewal API
3. **Given** Vault enforces policy restrictions, **When** attempting to access a path without permission, **Then** the system returns StorageError::PermissionDenied with the denied path and required capabilities
4. **Given** Vault KV v2 versioning is enabled, **When** listing credentials, **Then** the system returns the latest version number along with credential metadata
5. **Given** Vault requires namespace isolation (Vault Enterprise), **When** configuring the provider, **Then** the system supports specifying a namespace for multi-tenant deployments

---

### User Story 5 - Integrate with Kubernetes Secrets (Priority: P3)

A platform engineer deploying containerized workloads on Kubernetes needs to store credentials as Kubernetes Secrets to integrate with native Kubernetes RBAC, namespace isolation, and automatic secret mounting into pods.

**Why this priority**: Kubernetes is the dominant container orchestration platform. Native Kubernetes Secrets integration enables seamless credential distribution to pods, namespace-based isolation, and integration with service accounts. Lower priority because many teams use external secret stores (AWS, Vault) even in Kubernetes.

**Independent Test**: Can be fully tested by creating a Kubernetes client with appropriate kubeconfig, storing a secret in a namespace, retrieving it, and verifying RBAC permissions. Test namespace isolation and service account authentication independently.

**Acceptance Scenarios**:

1. **Given** Kubernetes service account credentials with secrets.create permission in namespace "nebula", **When** storing a credential, **Then** a Kubernetes Secret object is created in the specified namespace with type "Opaque"
2. **Given** Kubernetes enforces namespace isolation, **When** attempting to access a secret in a different namespace, **Then** the system returns StorageError::PermissionDenied with the namespace and required RBAC role
3. **Given** Kubernetes secrets support labels and annotations, **When** storing a credential with metadata, **Then** the metadata is converted to Kubernetes labels for filtering and annotations for additional context
4. **Given** a pod is running in Kubernetes, **When** retrieving a credential, **Then** the system automatically uses the pod's service account token for authentication via in-cluster configuration
5. **Given** Kubernetes API server is temporarily unavailable, **When** operations fail, **Then** the system retries with exponential backoff up to 3 times before returning StorageError::Unavailable

---

### Edge Cases

- What happens when storage provider initialization fails (e.g., AWS credentials invalid, Vault unreachable)? (Provider should fail fast during initialization with clear error message indicating configuration problem)
- How does the system handle concurrent writes to the same credential ID across different storage providers? (Each provider handles atomicity internally - local uses file locks, cloud providers use their native concurrency controls)
- What happens when retrieving a credential that was deleted externally (e.g., manually deleted from AWS Secrets Manager)? (Return NotFound error without exposing whether credential existed previously)
- How does the system handle large credentials exceeding provider limits? (AWS: 64KB, Vault: configurable, K8s: 1MB - validate size before attempting storage and return clear error with limit)
- What happens when switching storage providers with existing credentials? (Providers are independent - migration requires explicit export/import, not automatic)
- How does the system handle storage provider API version changes? (Use stable SDK versions, pin dependencies, provide upgrade path through semantic versioning)
- What happens when network connectivity is intermittent during cloud provider operations? (Use exponential backoff with jitter for retries, respect provider rate limits, fail after reasonable timeout)

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST implement a LocalStorageProvider that persists encrypted credentials to filesystem with atomic write operations (write-to-temp, then rename)
- **FR-002**: LocalStorageProvider MUST set file permissions to 0600 (owner read/write only) on Unix systems and equivalent restricted ACLs on Windows
- **FR-003**: LocalStorageProvider MUST automatically create the storage directory with permissions 0700 if it does not exist during initialization
- **FR-004**: System MUST implement an AwsSecretsManagerProvider that integrates with aws-sdk-secretsmanager for credential operations
- **FR-005**: AwsSecretsManagerProvider MUST implement automatic retry with exponential backoff (100ms base, 2x multiplier, max 5 retries) for transient errors (503, network timeout)
- **FR-006**: AwsSecretsManagerProvider MUST support IAM-based authentication using AWS credential chain (environment variables, instance profile, IAM role)
- **FR-007**: System MUST implement an AzureKeyVaultProvider that integrates with azure_security_keyvault SDK
- **FR-008**: AzureKeyVaultProvider MUST support Azure Managed Identity authentication for passwordless credential access
- **FR-009**: AzureKeyVaultProvider MUST automatically refresh Azure access tokens when they expire (before operations, not during)
- **FR-010**: System MUST implement a HashiCorpVaultProvider that integrates with Vault KV v2 secrets engine
- **FR-011**: HashiCorpVaultProvider MUST implement automatic token renewal when token TTL drops below 1 hour
- **FR-012**: HashiCorpVaultProvider MUST preserve secret versioning metadata when storing and retrieving credentials
- **FR-013**: System MUST implement a KubernetesSecretsProvider that integrates with kube-rs client library
- **FR-014**: KubernetesSecretsProvider MUST support namespace isolation and enforce Kubernetes RBAC permissions
- **FR-015**: KubernetesSecretsProvider MUST support both in-cluster authentication (service account) and out-of-cluster authentication (kubeconfig)
- **FR-016**: All storage providers MUST implement the StorageProvider trait from Phase 1 without breaking the trait interface
- **FR-017**: All storage providers MUST convert CredentialMetadata to provider-specific tagging/labeling mechanisms (AWS tags, Azure tags, Vault metadata, K8s labels)
- **FR-018**: All storage providers MUST handle provider-specific size limits and return clear errors when credentials exceed limits
- **FR-019**: System MUST validate storage provider configuration at initialization time and fail fast with actionable error messages
- **FR-020**: All cloud storage providers MUST support configurable timeout values (default: 5 seconds for reads, 10 seconds for writes)

### Key Entities

- **LocalStorageProvider**: File-based storage implementation that persists encrypted credentials to disk with atomic write guarantees, owner-only file permissions, and automatic directory creation
- **AwsSecretsManagerProvider**: AWS Secrets Manager integration that handles IAM authentication, KMS encryption delegation, automatic retries, and AWS-specific error mapping
- **AzureKeyVaultProvider**: Azure Key Vault integration that handles managed identity authentication, token refresh, RBAC enforcement, and Azure-specific error mapping
- **HashiCorpVaultProvider**: HashiCorp Vault integration that handles token-based authentication, automatic token renewal, KV v2 versioning, and Vault policy enforcement
- **KubernetesSecretsProvider**: Kubernetes Secrets integration that handles service account authentication, namespace isolation, RBAC enforcement, and Kubernetes-specific error mapping
- **ProviderConfig**: Configuration struct containing provider-specific settings (directory path for local, region for AWS, vault URL for Vault, namespace for K8s)
- **RetryPolicy**: Retry configuration with exponential backoff parameters (base delay, multiplier, max attempts, max delay) used by all cloud providers
- **StorageMetrics**: Per-provider metrics tracking operation latency, retry counts, error rates, and cache hit/miss ratios (foundation for Phase 8 observability)

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Developers can switch between storage providers by changing 1 configuration line without modifying application code
- **SC-002**: Local storage operations complete in under 10 milliseconds (p95 latency) for credentials under 10KB
- **SC-003**: AWS Secrets Manager operations complete in under 500 milliseconds (p95 latency) accounting for network round-trip and KMS encryption
- **SC-004**: All cloud storage providers successfully recover from transient network failures through automatic retry without user intervention
- **SC-005**: Storage provider initialization fails within 2 seconds when credentials or configuration are invalid, providing actionable error messages
- **SC-006**: Concurrent operations on the same credential ID from different threads/tasks are handled safely without corruption or race conditions
- **SC-007**: All storage providers enforce provider-specific permissions (file permissions, IAM policies, RBAC) preventing unauthorized credential access
- **SC-008**: Credential data survives process restart and system reboot when using persistent storage providers (local, cloud)
- **SC-009**: Zero credentials are lost or corrupted during storage operations under normal conditions (verified by integration tests simulating crashes)

## Dependencies *(mandatory)*

### External Dependencies

- **tokio** (v1.49+): Async runtime for all async storage operations
- **async-trait** (v0.1+): Async trait support for StorageProvider implementations
- **serde** (v1.0+): Serialization for credential metadata and provider configurations
- **serde_json** (v1.0+): JSON serialization for local storage file format
- **uuid** (v1.7+): Generating unique filenames for atomic write operations
- **aws-sdk-secretsmanager** (latest): AWS Secrets Manager SDK for AWS provider
- **aws-config** (latest): AWS credential chain and region configuration
- **aws-types** (latest): AWS common types for error handling
- **azure_security_keyvault** (latest): Azure Key Vault SDK for Azure provider
- **azure_identity** (latest): Azure authentication including managed identity support
- **vaultrs** (v0.7+): HashiCorp Vault client library for Vault provider
- **kube** (v0.87+): Kubernetes client library for K8s provider
- **k8s-openapi** (v0.20+): Kubernetes API types (version matching cluster version)

### Internal Dependencies

- **nebula-credential (Phase 1)**: Core abstractions including StorageProvider trait, CredentialId, CredentialMetadata, EncryptionManager, and error types

### Assumptions

- **A-001**: All storage providers have network connectivity to their respective backend services (AWS endpoints, Azure endpoints, Vault server, K8s API server)
- **A-002**: Cloud provider credentials (AWS keys, Azure service principal, Vault token, K8s kubeconfig) are configured correctly before provider initialization
- **A-003**: Local storage directory path is writable and has sufficient disk space for expected credential volume
- **A-004**: Cloud provider APIs maintain backward compatibility within major versions (AWS SDK v1, Azure SDK v1, Vault API v1)
- **A-005**: Kubernetes clusters run version 1.21+ supporting stable Secrets API
- **A-006**: For production deployments, cloud providers are configured with appropriate redundancy and backup strategies (not managed by nebula-credential)
- **A-007**: AWS KMS keys, Azure Key Vault vaults, and Vault KV mounts are pre-provisioned and accessible before storage operations

## Out of Scope

The following are explicitly **not** included in this Phase 2 implementation:

- **Credential caching layer** - Phase 3 (cache sits above storage providers)
- **Multi-provider federation** - Phase 6 (routing credentials to different providers)
- **Provider-to-provider migration tools** - Phase 6 (bulk export/import)
- **Audit logging** - Phase 7 (tracking who accessed which credentials)
- **Metrics and observability** - Phase 8 (detailed performance monitoring)
- **Credential rotation** - Phase 4 (automatic credential refresh)
- **Dynamic secrets** (Vault database secrets, AWS temporary credentials) - Future consideration
- **Custom storage provider SDK** - Future consideration (documenting how third parties can add providers)
- **Secret versioning API** - Limited support (Vault preserves versions, but no unified versioning API across providers)
- **Backup and disaster recovery** - Delegated to underlying storage providers
- **Cross-region replication** - Delegated to underlying cloud providers (AWS multi-region, Azure geo-redundancy)

## Technical Constraints

- **TC-001**: Must compile with Rust 1.92+ (project MSRV)
- **TC-002**: Must work on Windows, Linux, and macOS (filesystem operations must be cross-platform)
- **TC-003**: All async operations must use Tokio runtime (no blocking calls in async code)
- **TC-004**: Cloud provider SDKs must support TLS 1.2+ for all network communications
- **TC-005**: Local storage must support filesystems without POSIX (Windows NTFS, FAT32 limitations)
- **TC-006**: All providers must implement StorageProvider trait without breaking changes to the Phase 1 interface
- **TC-007**: Provider initialization must not perform network calls in constructors (use explicit async `init()` or `connect()` methods)
- **TC-008**: All public APIs must be documented with rustdoc including usage examples for each provider

## Security Considerations

- **SEC-001**: Local storage files MUST have restrictive permissions (0600 Unix, DACL on Windows) to prevent unauthorized access
- **SEC-002**: AWS provider MUST use HTTPS for all AWS API calls with certificate validation enabled
- **SEC-003**: Azure provider MUST validate access tokens before each operation, refreshing expired tokens automatically
- **SEC-004**: Vault provider MUST use HTTPS for all Vault API calls with configurable TLS certificate verification
- **SEC-005**: Kubernetes provider MUST validate service account tokens and enforce namespace isolation
- **SEC-006**: Provider configuration containing sensitive data (AWS keys, Azure secrets, Vault tokens) MUST use SecretString type with automatic zeroization
- **SEC-007**: All providers MUST log operations without including credential values (IDs only, with "[REDACTED]" for secrets)
- **SEC-008**: Local storage atomic writes MUST use temporary files in the same directory (not /tmp) to prevent cross-filesystem vulnerabilities
- **SEC-009**: Error messages MUST NOT leak credential values, authentication tokens, or internal paths that could aid attackers
- **SEC-010**: Cloud provider retry logic MUST implement jitter to prevent thundering herd attacks during outages

## Future Considerations

- **FC-001**: Design provider configuration to support connection pooling for high-throughput scenarios (multiple concurrent requests)
- **FC-002**: Consider implementing read replicas for cloud providers to reduce latency in geographically distributed deployments
- **FC-003**: Add support for Google Cloud Secret Manager as an additional cloud provider
- **FC-004**: Design error types to include provider-specific error details while maintaining common error interface
- **FC-005**: Consider adding provider health checks and circuit breakers for failing providers
- **FC-006**: Plan for secret versioning API that abstracts over provider-specific versioning mechanisms (Vault versions, AWS version stages)
- **FC-007**: Consider adding bulk operations (store_many, retrieve_many) to optimize performance for providers that support batch APIs
- **FC-008**: Design migration strategy for credentials when changing encryption algorithms or upgrading provider SDK versions

## Reference Documentation

The following nebula-credential documentation provides additional context for Phase 2 implementation:

### Integration Guides
- **Integrations/AWS-Secrets-Manager.md**: Complete guide for AWS Secrets Manager integration including IAM permissions, KMS configuration, and error handling
- **Integrations/Azure-Key-Vault.md**: Azure Key Vault setup including managed identity configuration, RBAC roles, and HSM options
- **Integrations/HashiCorp-Vault.md**: Vault setup including KV v2 engine configuration, token authentication, and policy examples
- **Integrations/Kubernetes-Secrets.md**: K8s Secrets integration including RBAC configuration, namespace isolation, and service account setup
- **Integrations/Local-Storage.md**: Local storage implementation details including directory structure, file format, and permissions
- **Integrations/Provider-Comparison.md**: Feature comparison matrix across all providers to guide provider selection

### Architecture
- **Meta/TECHNICAL-DESIGN.md**: Low-level implementation details for storage provider patterns, retry logic, and error handling
- **Reference/StorageBackends.md**: Complete reference for StorageProvider trait and implementation requirements

### Examples
- **How-To/Store-Credentials.md**: Multi-provider examples showing identical code working with different providers
- **How-To/Retrieve-Credentials.md**: Provider-specific retrieval patterns and error handling examples
