# Implementation Tasks: Production-Ready Storage Backends

**Feature**: 002-storage-backends  
**Branch**: `002-storage-backends`  
**Generated**: 2026-02-04  
**Status**: Ready for Implementation

---

## Task Summary

- **Total Tasks**: 47
- **Estimated LOC**: ~1,650 lines
- **Parallel Opportunities**: 12 tasks can run in parallel after foundations
- **Dependencies**: Phase 1 Core Credential Abstractions (001-credential-core-abstractions)

---

## Task Format

```
- [ ] [TASK-ID] [P] [US#] Description
      Location: file/path
      Dependencies: [TASK-ID, ...]
      Estimated Lines: XX
```

**Legend**:
- `[P]` = Can be parallelized with other [P] tasks at same level
- `[US#]` = User Story number (US1-US5)
- `Dependencies` = Must complete before starting this task

---

## Phase 0: Setup and Foundation (6 tasks)

### Workspace Configuration

- [x] [T001] Add storage backend dependencies to nebula-credential Cargo.toml
      Location: crates/nebula-credential/Cargo.toml
      Dependencies: None
      Estimated Lines: 30
      
      Add optional feature-gated dependencies:
      - aws-sdk-secretsmanager (feature: storage-aws)
      - azure_security_keyvault_secrets (feature: storage-azure)
      - vaultrs (feature: storage-vault)
      - kube + k8s-openapi (feature: storage-k8s)
      - atomicwrites, uuid, fs2, directories (always enabled)
      - windows-acl (Windows only)
      
      Define features: storage-local (default), storage-aws, storage-azure, storage-vault, storage-k8s, storage-all

- [x] [T002] Create provider module structure in nebula-credential
      Location: crates/nebula-credential/src/providers/mod.rs
      Dependencies: [T001]
      Estimated Lines: 20
      
      Create directory structure:
      - src/providers/mod.rs (re-exports)
      - src/providers/mock.rs (MockStorageProvider)
      - src/providers/local.rs (LocalStorageProvider)
      - src/providers/aws.rs (AwsSecretsManagerProvider)
      - src/providers/azure.rs (AzureKeyVaultProvider)
      - src/providers/vault.rs (HashiCorpVaultProvider)
      - src/providers/kubernetes.rs (KubernetesSecretsProvider)

- [x] [T003] Create utils module for shared retry logic
      Location: crates/nebula-credential/src/utils/mod.rs, src/utils/retry.rs
      Dependencies: [T001]
      Estimated Lines: 15
      
      Create:
      - src/utils/mod.rs (re-exports)
      - src/utils/retry.rs (stub, implemented in T010)

- [x] [T004] Add provider configuration types to lib.rs exports
      Location: crates/nebula-credential/src/lib.rs
      Dependencies: [T002, T003]
      Estimated Lines: 10
      
      Add to prelude:
      - All provider types
      - All config types
      - RetryPolicy
      - StorageMetrics

### Shared Infrastructure

- [x] [T005] Implement ProviderConfig trait
      Location: crates/nebula-credential/src/providers/config.rs
      Dependencies: [T002]
      Estimated Lines: 25
      
      Define base trait for provider configuration:
      - validate() -> Result<(), ConfigError>
      - provider_name() -> &'static str
      Document contract requirements

- [x] [T006] Implement StorageMetrics struct with atomic counters
      Location: crates/nebula-credential/src/providers/metrics.rs
      Dependencies: [T002]
      Estimated Lines: 80
      
      Implement:
      - AtomicU64 counters for operations and latencies
      - record_operation() method
      - record_retry() method
      - avg_store_latency_ms(), avg_retrieve_latency_ms()
      - error_rate() calculation
      Thread-safe metrics collection (foundation for Phase 8)

---

## Phase 1: Shared Utilities (4 tasks)

- [x] [T007] Implement RetryPolicy configuration struct
      Location: crates/nebula-credential/src/utils/retry.rs
      Dependencies: [T003]
      Estimated Lines: 60
      
      Implement:
      - RetryPolicy struct (max_retries, base_delay_ms, max_delay_ms, multiplier, jitter)
      - Default implementation (5 retries, 100ms base, 2x multiplier)
      - validate() method (0-10 retries, base < max delay, multiplier 1.0-10.0)
      - Serde Serialize + Deserialize

- [x] [T008] Implement exponential backoff with jitter
      Location: crates/nebula-credential/src/utils/retry.rs
      Dependencies: [T007]
      Estimated Lines: 40
      
      Implement:
      - calculate_delay(attempt: u32, policy: &RetryPolicy) -> Duration
      - apply_jitter(delay: Duration, policy: &RetryPolicy) -> Duration
      - ±25% jitter using rand crate
      Unit tests for delay calculation

- [x] [T009] Implement retry executor for async operations
      Location: crates/nebula-credential/src/utils/retry.rs
      Dependencies: [T008]
      Estimated Lines: 60
      
      Implement:
      - async fn retry_with_policy<F, T, E>(policy: &RetryPolicy, operation: F) -> Result<T, E>
      - where F: Fn() -> Future<Output = Result<T, E>>
      - Track retry attempts, sleep between retries
      - Log retry attempts with tracing::warn!
      - Return last error after max retries

- [x] [T010] Write unit tests for retry logic
      Location: crates/nebula-credential/tests/utils/retry_tests.rs
      Dependencies: [T009]
      Estimated Lines: 100
      
      Test cases:
      - Successful operation (no retries)
      - Transient failure then success (2 retries)
      - Permanent failure (max retries exhausted)
      - Jitter randomness (delays within expected range)
      - Exponential growth (delays double each attempt)

---

## Phase 2: Mock Provider for Testing (3 tasks)

- [X] [T011] Implement MockStorageProvider struct with in-memory storage
      Location: crates/nebula-credential/src/providers/mock.rs
      Dependencies: [T006]
      Estimated Lines: 40
      
      Implement:
      - HashMap<CredentialId, (EncryptedData, CredentialMetadata)> storage
      - Arc<RwLock<...>> for thread safety
      - should_fail: Arc<RwLock<Option<StorageError>>> for error simulation
      - new(), fail_next_with(), clear(), count() methods

- [X] [T012] Implement StorageProvider trait for MockStorageProvider
      Location: crates/nebula-credential/src/providers/mock.rs
      Dependencies: [T011]
      Estimated Lines: 100
      
      Implement all StorageProvider methods:
      - store() - insert into HashMap, check should_fail
      - retrieve() - get from HashMap, return NotFound if missing
      - delete() - remove from HashMap (idempotent)
      - list() - return keys, apply filter if provided
      - exists() - check HashMap contains_key

- [X] [T013] Write unit tests for MockStorageProvider
      Location: crates/nebula-credential/tests/mock_provider_tests.rs
      Dependencies: [T012]
      Estimated Lines: 150
      
      Test cases:
      - Store and retrieve
      - Retrieve NotFound
      - Delete idempotent
      - List with filter (tags)
      - Exists check
      - Simulated errors
      - Concurrent operations (100 parallel stores)

---

## Phase 3: US1 - Local Storage Provider (Priority P1) (7 tasks)

- [X] [T014] [P] [US1] Implement LocalStorageConfig struct
      Location: crates/nebula-credential/src/providers/local.rs
      Dependencies: [T005]
      Estimated Lines: 60
      
      Implement:
      - base_path: PathBuf, create_dir: bool, file_extension: String, enable_locking: bool
      - Default implementation (use directories crate for platform paths)
      - Validation (absolute path, no path separators in extension)
      - ProviderConfig trait implementation

- [X] [T015] [P] [US1] Implement CredentialFile serialization format
      Location: crates/nebula-credential/src/providers/local.rs
      Dependencies: [T014]
      Estimated Lines: 40
      
      Implement:
      - CredentialFile struct (version, encrypted_data, metadata, salt)
      - CURRENT_VERSION constant (1)
      - new(), needs_migration() methods
      - Serde Serialize + Deserialize

- [X] [T016] [US1] Implement atomic file write utility
      Location: crates/nebula-credential/src/providers/local.rs
      Dependencies: [T015]
      Estimated Lines: 50
      
      Implement:
      - async fn atomic_write(path: &Path, data: &[u8]) -> Result<(), io::Error>
      - Write to temp file with UUID suffix in same directory
      - Set permissions 0600 (Unix) or ACL (Windows)
      - Rename to final path (atomic operation)
      - Cleanup temp file on error

- [X] [T017] [US1] Implement LocalStorageProvider struct and initialization
      Location: crates/nebula-credential/src/providers/local.rs
      Dependencies: [T016]
      Estimated Lines: 50
      
      Implement:
      - LocalStorageProvider struct (config, metrics)
      - new(config) -> Self
      - ensure_directory_exists() - create with 0700 permissions
      - get_file_path(id) -> PathBuf

- [X] [T018] [US1] Implement StorageProvider trait for LocalStorageProvider
      Location: crates/nebula-credential/src/providers/local.rs
      Dependencies: [T017]
      Estimated Lines: 120
      
      Implement all methods:
      - store() - serialize to JSON, atomic write, record metrics
      - retrieve() - read file with shared lock, deserialize, record metrics
      - delete() - remove file (ignore NotFound)
      - list() - scan directory, filter by metadata
      - exists() - check file existence

- [X] [T019] [US1] Write unit tests for LocalStorageProvider
      Location: crates/nebula-credential/tests/local_provider_tests.rs
      Dependencies: [T018]
      Estimated Lines: 80
      
      Test cases (using TempDir):
      - Store and retrieve
      - Directory autocreate
      - Atomic write (no temp files left)
      - Delete idempotent
      - List with filter

- [ ] [T020] [US1] Write integration tests for local storage
      Location: crates/nebula-credential/tests/integration/local_storage_integration.rs
      Dependencies: [T018]
      Estimated Lines: 200
      
      Test cases:
      - Atomic writes (no corruption)
      - Unix permissions 0600 (#[cfg(unix)])
      - Windows ACL (#[cfg(windows)])
      - Concurrent writes (file locking)
      - File corruption recovery
      - Directory autocreate with nested paths

---

## Phase 4: US2 - AWS Secrets Manager Provider (Priority P2) (6 tasks)

- [ ] [T021] [P] [US2] Implement AwsSecretsManagerConfig struct
      Location: crates/nebula-credential/src/providers/aws.rs
      Dependencies: [T005, T007]
      Estimated Lines: 70
      
      Implement:
      - region: Option<String>, secret_prefix: String, timeout: Duration
      - retry_policy: RetryPolicy, kms_key_id: Option<String>, default_tags: HashMap
      - Default implementation (auto-detect region, 5s timeout)
      - Validation (prefix max 512 chars, no invalid chars, timeout 1-60s)
      - ProviderConfig trait implementation

- [ ] [T022] [P] [US2] Implement AwsSecretsManagerProvider struct and initialization
      Location: crates/nebula-credential/src/providers/aws.rs
      Dependencies: [T021]
      Estimated Lines: 60
      
      Implement:
      - AwsSecretsManagerProvider struct (client, config, metrics)
      - async fn new(config) -> Result<Self, StorageError>
      - Initialize AWS SDK client with region and credential chain
      - Validate connection (optional DescribeSecret call)

- [ ] [T023] [US2] Implement metadata to AWS tags conversion
      Location: crates/nebula-credential/src/providers/aws.rs
      Dependencies: [T022]
      Estimated Lines: 40
      
      Implement:
      - fn metadata_to_aws_tags(metadata: &CredentialMetadata) -> Vec<Tag>
      - Convert tags Vec to AWS Tag format (max 50 tags)
      - Handle tag key/value length limits (128/256 chars)
      - Skip invalid tags with warning log

- [ ] [T024] [US2] Implement StorageProvider trait for AwsSecretsManagerProvider
      Location: crates/nebula-credential/src/providers/aws.rs
      Dependencies: [T023, T009]
      Estimated Lines: 150
      
      Implement all methods with retry logic:
      - store() - CreateSecret or UpdateSecret, apply KMS encryption, validate size limit (64KB)
      - retrieve() - GetSecretValue, deserialize JSON
      - delete() - DeleteSecret with recovery window
      - list() - ListSecrets with pagination, filter by prefix
      - exists() - DescribeSecret (lighter than GetSecretValue)
      Map AWS SDK errors to StorageError with context

- [ ] [T025] [US2] Write unit tests for AWS provider (with mocks)
      Location: crates/nebula-credential/tests/providers/aws_tests.rs
      Dependencies: [T024]
      Estimated Lines: 60
      
      Test cases (mocking AWS SDK):
      - Config validation
      - Metadata to tags conversion
      - Size limit validation (64KB)
      - Error mapping (ResourceNotFoundException -> NotFound)

- [ ] [T026] [US2] Write integration tests for AWS provider (LocalStack)
      Location: crates/nebula-credential/tests/integration/localstack_integration.rs
      Dependencies: [T024]
      Estimated Lines: 150
      
      Test cases (using testcontainers + LocalStack):
      - CRUD operations
      - Size limit validation (exceeds 64KB)
      - Retry on transient errors (simulate 503)
      - Tag application
      Requires docker-compose.test.yml configuration

---

## Phase 5: US3 - Azure Key Vault Provider (Priority P2) (6 tasks)

- [ ] [T027] [P] [US3] Implement AzureKeyVaultConfig struct
      Location: crates/nebula-credential/src/providers/azure.rs
      Dependencies: [T005, T007]
      Estimated Lines: 90
      
      Implement:
      - vault_url: String, credential_type: AzureCredentialType, timeout: Duration
      - retry_policy: RetryPolicy, secret_prefix: String, default_tags: HashMap
      - AzureCredentialType enum (ManagedIdentity, ServicePrincipal, DeveloperTools)
      - Default implementation
      - Validation (URL format, HTTPS, timeout 1-60s, GUID format for service principal)
      - ProviderConfig trait implementation

- [ ] [T028] [P] [US3] Implement AzureKeyVaultProvider struct and initialization
      Location: crates/nebula-credential/src/providers/azure.rs
      Dependencies: [T027]
      Estimated Lines: 80
      
      Implement:
      - AzureKeyVaultProvider struct (client, config, metrics)
      - async fn new(config) -> Result<Self, StorageError>
      - Initialize azure_security_keyvault SecretClient
      - Handle Managed Identity vs Service Principal auth
      - Automatic token refresh logic

- [ ] [T029] [US3] Implement metadata to Azure tags conversion
      Location: crates/nebula-credential/src/providers/azure.rs
      Dependencies: [T028]
      Estimated Lines: 30
      
      Implement:
      - fn metadata_to_azure_tags(metadata: &CredentialMetadata) -> HashMap<String, String>
      - Convert tags Vec to Azure tag format (max 15 tags)
      - Handle tag limits

- [ ] [T030] [US3] Implement StorageProvider trait for AzureKeyVaultProvider
      Location: crates/nebula-credential/src/providers/azure.rs
      Dependencies: [T029, T009]
      Estimated Lines: 140
      
      Implement all methods with retry logic:
      - store() - SetSecret with tags, validate size limit (25KB)
      - retrieve() - GetSecret, handle token refresh
      - delete() - DeleteSecret (soft-delete)
      - list() - ListSecrets with pagination
      - exists() - GetSecret metadata-only
      Map Azure errors to StorageError with RBAC context

- [ ] [T031] [US3] Write unit tests for Azure provider (with mocks)
      Location: crates/nebula-credential/tests/providers/azure_tests.rs
      Dependencies: [T030]
      Estimated Lines: 60
      
      Test cases:
      - Config validation
      - Metadata to tags conversion (max 15)
      - Size limit validation (25KB)
      - Credential type handling

- [ ] [T032] [US3] Write integration tests for Azure provider (Lowkey Vault)
      Location: crates/nebula-credential/tests/integration/azure_lowkey_vault_integration.rs
      Dependencies: [T030]
      Estimated Lines: 250
      
      Test cases (using testcontainers + Lowkey Vault):
      - CRUD operations
      - Soft-delete recovery
      - Metadata tags preservation
      - Concurrent operations (50 parallel)
      Requires docker-compose.test.yml with lowkey-vault service

---

## Phase 6: US4 - HashiCorp Vault Provider (Priority P2) (7 tasks)

- [ ] [T033] [P] [US4] Implement VaultConfig struct
      Location: crates/nebula-credential/src/providers/vault.rs
      Dependencies: [T005, T007]
      Estimated Lines: 100
      
      Implement:
      - address: String, auth_method: VaultAuthMethod, mount_path: String, path_prefix: String
      - namespace: Option<String>, timeout: Duration, retry_policy: RetryPolicy
      - tls_verify: bool, token_renewal_threshold: Duration
      - VaultAuthMethod enum (Token, AppRole)
      - Default implementation (from env vars)
      - Validation (URL format, TLS enforcement for HTTPS, path format)
      - ProviderConfig trait implementation

- [ ] [T034] [P] [US4] Implement HashiCorpVaultProvider struct and initialization
      Location: crates/nebula-credential/src/providers/vault.rs
      Dependencies: [T033]
      Estimated Lines: 70
      
      Implement:
      - HashiCorpVaultProvider struct (client, config, metrics, token_renewal_task)
      - async fn new(config) -> Result<Self, StorageError>
      - Initialize vaultrs VaultClient
      - Handle Token vs AppRole authentication
      - Start token renewal background task (if needed)

- [ ] [T035] [US4] Implement token renewal background task
      Location: crates/nebula-credential/src/providers/vault.rs
      Dependencies: [T034]
      Estimated Lines: 60
      
      Implement:
      - async fn token_renewal_loop(client, threshold) -> JoinHandle<()>
      - Check token TTL every 5 minutes
      - Renew token if TTL < threshold (default 1 hour)
      - Log renewal success/failure
      - Handle AppRole re-authentication on token expiry

- [ ] [T036] [US4] Implement StorageProvider trait for HashiCorpVaultProvider
      Location: crates/nebula-credential/src/providers/vault.rs
      Dependencies: [T035, T009]
      Estimated Lines: 130
      
      Implement all methods with retry logic:
      - store() - kv2::set(), preserve versioning
      - retrieve() - kv2::read() latest version
      - delete() - kv2::delete_metadata() (permanent)
      - list() - kv2::list() with path prefix
      - exists() - kv2::read() metadata endpoint
      Map Vault errors to StorageError with policy context

- [ ] [T037] [US4] Write unit tests for Vault provider (with mocks)
      Location: crates/nebula-credential/tests/providers/vault_tests.rs
      Dependencies: [T036]
      Estimated Lines: 70
      
      Test cases:
      - Config validation
      - Path construction (mount + prefix + id)
      - Token vs AppRole auth handling
      - Error mapping

- [ ] [T038] [US4] Write integration tests for Vault provider (Docker)
      Location: crates/nebula-credential/tests/integration/vault_integration.rs
      Dependencies: [T036]
      Estimated Lines: 200
      
      Test cases (using testcontainers + Vault):
      - CRUD operations
      - Versioning (store twice, retrieve latest)
      - Token renewal (requires long-running test)
      - Policy enforcement (permission denied)
      Requires docker-compose.test.yml with vault service

- [ ] [T039] [US4] Implement graceful shutdown for token renewal task
      Location: crates/nebula-credential/src/providers/vault.rs
      Dependencies: [T036]
      Estimated Lines: 30
      
      Implement:
      - Drop trait for HashiCorpVaultProvider
      - Cancel token_renewal_task JoinHandle
      - Wait for task completion with timeout

---

## Phase 7: US5 - Kubernetes Secrets Provider (Priority P3) (6 tasks)

- [ ] [T040] [P] [US5] Implement KubernetesSecretsConfig struct
      Location: crates/nebula-credential/src/providers/kubernetes.rs
      Dependencies: [T005, T007]
      Estimated Lines: 80
      
      Implement:
      - namespace: String, kubeconfig_path: Option<PathBuf>, secret_prefix: String
      - timeout: Duration, retry_policy: RetryPolicy
      - default_labels: HashMap, default_annotations: HashMap
      - Default implementation (from env vars, default namespace)
      - Validation (namespace format max 63 chars, label format)
      - ProviderConfig trait implementation

- [ ] [T041] [P] [US5] Implement KubernetesSecretsProvider struct and initialization
      Location: crates/nebula-credential/src/providers/kubernetes.rs
      Dependencies: [T040]
      Estimated Lines: 70
      
      Implement:
      - KubernetesSecretsProvider struct (client, secrets_api, config, metrics)
      - async fn new(config) -> Result<Self, StorageError>
      - Initialize kube::Client (in-cluster or kubeconfig)
      - Create Api<Secret> for namespace

- [ ] [T042] [US5] Implement metadata to K8s labels/annotations conversion
      Location: crates/nebula-credential/src/providers/kubernetes.rs
      Dependencies: [T041]
      Estimated Lines: 50
      
      Implement:
      - fn metadata_to_labels(metadata: &CredentialMetadata) -> BTreeMap<String, String>
      - fn metadata_to_annotations(metadata: &CredentialMetadata) -> BTreeMap<String, String>
      - Sanitize labels (max 63 chars, valid DNS subdomain format)
      - Store full metadata in annotations

- [ ] [T043] [US5] Implement StorageProvider trait for KubernetesSecretsProvider
      Location: crates/nebula-credential/src/providers/kubernetes.rs
      Dependencies: [T042, T009]
      Estimated Lines: 140
      
      Implement all methods with retry logic:
      - store() - Create or Update Secret, base64 encode, validate size limit (1MB)
      - retrieve() - Get Secret, base64 decode
      - delete() - Delete Secret (idempotent)
      - list() - List Secrets with label selector
      - exists() - Get Secret metadata-only
      Map K8s errors to StorageError with RBAC context

- [ ] [T044] [US5] Write unit tests for K8s provider (with mocks)
      Location: crates/nebula-credential/tests/providers/kubernetes_tests.rs
      Dependencies: [T043]
      Estimated Lines: 70
      
      Test cases:
      - Config validation
      - Label sanitization (63 char limit)
      - Metadata to labels/annotations conversion
      - Size limit validation (1MB)

- [ ] [T045] [US5] Write integration tests for K8s provider (kind)
      Location: crates/nebula-credential/tests/integration/kubernetes_integration.rs
      Dependencies: [T043]
      Estimated Lines: 150
      
      Test cases (using kind cluster):
      - CRUD operations
      - Namespace isolation (store in ns1, not visible in ns2)
      - RBAC enforcement (permission denied)
      - Label filtering
      Requires scripts/setup-kind-for-tests.sh

---

## Phase 8: Testing Infrastructure (2 tasks)

- [ ] [T046] Setup Docker Compose for integration tests
      Location: crates/nebula-credential/docker-compose.test.yml
      Dependencies: None (can run parallel with implementation)
      Estimated Lines: 60
      
      Create docker-compose.test.yml with services:
      - vault (HashiCorp Vault dev mode)
      - localstack (AWS Secrets Manager emulator)
      - lowkey-vault (Azure Key Vault emulator v7.1.0)
      - postgres (for future Phase 3 caching)
      Add health checks for all services

- [ ] [T047] Create test execution script
      Location: scripts/run-all-tests.sh
      Dependencies: [T046]
      Estimated Lines: 80
      
      Create bash script:
      - Run unit tests (cargo test --lib)
      - Start Docker services
      - Run integration tests (vault, localstack, lowkey-vault)
      - Run local storage tests
      - Run K8s tests (if kind installed)
      - Cleanup Docker containers
      Make script executable, add to CI/CD

---

## Dependency Graph

```
Setup Phase (T001-T006)
    ├─> T001 (Cargo.toml) ─┬─> T002 (module structure) ─> T004 (lib.rs exports)
    │                      └─> T003 (utils module)
    │
    ├─> T005 (ProviderConfig trait)
    └─> T006 (StorageMetrics)

Shared Utilities (T007-T010)
    T003 ─> T007 (RetryPolicy) ─> T008 (backoff) ─> T009 (retry executor) ─> T010 (tests)

Mock Provider (T011-T013)
    T006 ─> T011 (struct) ─> T012 (trait impl) ─> T013 (tests)

Local Storage [P1] (T014-T020) - CAN START AFTER T005, T006
    T005 ─> T014 (config) ─> T015 (CredentialFile) ─> T016 (atomic write)
                                                    ─> T017 (provider struct)
                                                    ─> T018 (trait impl)
                                                    ─> T019 (unit tests)
                                                    ─> T020 (integration tests)

AWS Provider [P2] (T021-T026) - CAN START AFTER T005, T007, T009
    T005, T007 ─> T021 (config) ─> T022 (provider struct) ─> T023 (metadata conversion)
    T009 ──────────────────────────┘                      ─> T024 (trait impl)
                                                           ─> T025 (unit tests)
                                                           ─> T026 (integration tests)

Azure Provider [P2] (T027-T032) - CAN START AFTER T005, T007, T009
    T005, T007 ─> T027 (config) ─> T028 (provider struct) ─> T029 (metadata conversion)
    T009 ──────────────────────────┘                      ─> T030 (trait impl)
                                                           ─> T031 (unit tests)
                                                           ─> T032 (integration tests)

Vault Provider [P2] (T033-T039) - CAN START AFTER T005, T007, T009
    T005, T007 ─> T033 (config) ─> T034 (provider struct) ─> T035 (token renewal)
    T009 ──────────────────────────┘                      ─> T036 (trait impl)
                                                           ─> T037 (unit tests)
                                                           ─> T038 (integration tests)
                                                           ─> T039 (graceful shutdown)

K8s Provider [P3] (T040-T045) - CAN START AFTER T005, T007, T009
    T005, T007 ─> T040 (config) ─> T041 (provider struct) ─> T042 (metadata conversion)
    T009 ──────────────────────────┘                      ─> T043 (trait impl)
                                                           ─> T044 (unit tests)
                                                           ─> T045 (integration tests)

Test Infrastructure (T046-T047) - CAN START ANYTIME
    T046 (docker-compose.test.yml) ─> T047 (run-all-tests.sh)
```

---

## Parallelization Strategy

**After completing foundations (T001-T010)**, these tasks can run in parallel:

1. **Local Storage** (T014-T020) - 7 tasks, P1 priority
2. **AWS Provider** (T021-T026) - 6 tasks, P2 priority
3. **Azure Provider** (T027-T032) - 6 tasks, P2 priority
4. **Vault Provider** (T033-T039) - 7 tasks, P2 priority
5. **K8s Provider** (T040-T045) - 6 tasks, P3 priority

**Total parallel work**: 32 tasks across 5 independent streams after 10 sequential foundation tasks.

---

## Testing Checklist

After completing all tasks, verify:

- [ ] All unit tests pass: `cargo test --lib --package nebula-credential`
- [ ] All integration tests pass: `./scripts/run-all-tests.sh`
- [ ] Code coverage > 80%: `cargo tarpaulin --package nebula-credential`
- [ ] No clippy warnings: `cargo clippy --workspace -- -D warnings`
- [ ] Code formatted: `cargo fmt --all -- --check`
- [ ] Documentation builds: `cargo doc --no-deps --package nebula-credential`
- [ ] Examples run successfully: `cargo run --example local-storage`

---

## Success Criteria Validation

Map to spec.md Success Criteria:

- **SC-001** (Switch providers with 1 config line): Verify quickstart examples work
- **SC-002** (Local storage <10ms p95): Benchmark with criterion
- **SC-003** (AWS <500ms p95): Benchmark with LocalStack
- **SC-004** (Retry recovery): Integration tests simulate failures
- **SC-005** (Init fails fast <2s): Unit tests with invalid config
- **SC-006** (Concurrent safety): Integration tests with 50 parallel ops
- **SC-007** (Permission enforcement): Integration tests verify RBAC
- **SC-008** (Data survives restart): Integration tests store, kill, restart, retrieve
- **SC-009** (Zero corruption): Integration tests simulate crashes during writes

---

## Estimated Timeline

**Sequential foundation**: ~1 day (T001-T010)  
**Parallel implementation**: ~3 days (T014-T045 across 5 providers)  
**Testing infrastructure**: ~0.5 day (T046-T047)  
**Integration & validation**: ~0.5 day

**Total**: ~5 days with parallelization (single developer)  
**Total**: ~2-3 days with team of 3 (Local + AWS + Azure in parallel)

---

## Implementation Notes

1. **TDD Approach**: Write tests (T013, T019, T025, T031, T037, T044) before implementation
2. **Feature Flags**: Test each provider independently with `--features storage-aws` etc.
3. **Docker First**: Setup T046 early to enable integration testing during development
4. **Metrics Foundation**: T006 prepares for Phase 8 observability, keep it simple (atomic counters only)
5. **Error Context**: All StorageError variants must include actionable remediation in messages
6. **Security**: Never log credential values, use "[REDACTED]" in tracing spans

---

## Ready for Implementation ✅

All tasks defined, dependencies mapped, success criteria clear.

**Next Step**: Begin with T001 (Cargo.toml dependencies) following TDD workflow.
