# Implementation Plan: Production-Ready Storage Backends

**Branch**: `002-storage-backends` | **Date**: 2026-02-03 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/002-storage-backends/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Implement Phase 2 of nebula-credential roadmap: Production-ready storage provider implementations for local encrypted filesystem, AWS Secrets Manager, Azure Key Vault, HashiCorp Vault, and Kubernetes Secrets. Each provider implements the `StorageProvider` trait from Phase 1, enabling seamless switching between backends with a single configuration change. All providers support automatic retries with exponential backoff, comprehensive error handling with actionable messages, and provider-specific features (AWS KMS, Azure Managed Identity, Vault versioning, K8s RBAC).

## Technical Context

**Language/Version**: Rust 2024 Edition (MSRV: 1.92)
**Primary Dependencies**: Tokio async runtime, async-trait, serde, thiserror, aws-sdk-secretsmanager, azure_security_keyvault, vaultrs, kube
**Storage**: Multiple pluggable backends - local encrypted filesystem (AES-256-GCM), AWS Secrets Manager (KMS-encrypted), Azure Key Vault (HSM-backed), HashiCorp Vault (KV v2), Kubernetes Secrets (namespace-isolated)
**Testing**: `cargo test --workspace`, `#[tokio::test(flavor = "multi_thread")]` for async, integration tests with mock providers and testcontainers for real backend testing
**Target Platform**: Cross-platform (Windows primary development, Linux/macOS support) - filesystem operations must handle POSIX and Windows permissions
**Project Type**: Workspace (16 crates organized in architectural layers) - nebula-credential in Domain layer
**Performance Goals**: Local storage <10ms p95, AWS/Azure/Vault <500ms p95 (including network), K8s <300ms p95, all providers support concurrent operations without blocking
**Constraints**: AWS 64KB credential limit, K8s 1MB limit, Vault configurable limits, atomic write operations for local storage, provider-specific rate limits (AWS 5000 req/s, Azure 2000 req/s)
**Scale/Scope**: Support 1000s of credentials per deployment, hundreds of concurrent reads, handle transient network failures gracefully, support multi-tenant isolation (K8s namespaces, Vault namespaces)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with `.specify/memory/constitution.md` principles:

- [x] **Type Safety First**: Uses `CredentialId` newtype from Phase 1, `EncryptedData` struct, provider-specific config types; all public APIs use sized types (no raw `str`); RetryPolicy and ProviderConfig use builder pattern with type-safe fields
- [x] **Isolated Error Handling**: Reuses `StorageError` from Phase 1 (already defined in nebula-credential); provider-specific errors (AWS SDK, Azure SDK) converted to `StorageError` at provider boundaries with context
- [x] **Test-Driven Development**: Each provider has unit tests with mock backends (MockStorageProvider pattern), integration tests with testcontainers for AWS/Azure/Vault/K8s, TDD cycle for each CRUD operation per provider
- [x] **Async Discipline**: All StorageProvider methods are async with `#[async_trait]`; cloud providers use tokio::time::timeout (5s reads, 10s writes); retry logic uses tokio::time::sleep with exponential backoff; no blocking calls in async code
- [x] **Modular Architecture**: All implementations in nebula-credential crate (Domain layer); dependencies: aws-sdk-* (AWS only), azure_* (Azure only), vaultrs (Vault only), kube (K8s only) with optional feature flags; no new crates, no circular deps
- [x] **Observability**: Each provider emits tracing spans (provider_name, operation, credential_id); errors logged with context before returning; metrics struct prepared (operation latency, retry count, error rate) for Phase 8
- [x] **Simplicity**: Five providers justified by spec requirements (local for dev, AWS/Azure for cloud, Vault for enterprise, K8s for containers); each provider ~200-300 lines; shared retry logic extracted to utils module

## Project Structure

### Documentation (this feature)

```text
specs/[###-feature]/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
crates/
├── [crate-name]/          # Identify which crate(s) this feature modifies/adds
│   ├── src/
│   │   ├── lib.rs
│   │   └── [modules]/
│   ├── tests/             # Integration tests
│   ├── examples/          # Usage examples
│   └── Cargo.toml
└── ...

# Existing workspace crates by layer:
# Core: nebula-core, nebula-value, nebula-log
# Domain: nebula-parameter, nebula-action, nebula-expression, nebula-validator, nebula-credential
# UI: nebula-ui, nebula-parameter-ui
# System: nebula-config, nebula-memory, nebula-resilience, nebula-resource, nebula-system
# Tooling: nebula-derive
```

**Structure Decision**: 
- **Primary crate**: `nebula-credential` (Domain layer) - all storage provider implementations
- **No new crates**: Storage providers are implementations of existing `StorageProvider` trait
- **Module structure** within nebula-credential:
  ```
  src/
  ├── providers/
  │   ├── mod.rs           # Re-exports all providers
  │   ├── local.rs         # LocalStorageProvider
  │   ├── aws.rs           # AwsSecretsManagerProvider
  │   ├── azure.rs         # AzureKeyVaultProvider
  │   ├── vault.rs         # HashiCorpVaultProvider
  │   └── kubernetes.rs    # KubernetesSecretsProvider
  └── utils/
      └── retry.rs         # Shared retry logic with exponential backoff
  ```
- **Dependency management**: Cloud provider SDKs added as optional features to minimize binary size for users who only need local storage
- **Justification**: Follows Principle V - providers are implementation details of the credential management domain, not separate concerns requiring dedicated crates

## Complexity Tracking

**No violations** - All constitution principles satisfied without exceptions.

---

## Phase Completion Summary

### Phase 0: Research ✅ COMPLETE

**Generated**: `research.md` (72KB comprehensive research)

**Findings**:
- **AWS Secrets Manager**: aws-sdk-secretsmanager with automatic credential chain, 64KB limit, KMS encryption, built-in retry
- **Azure Key Vault**: azure_security_keyvault with Managed Identity, 25KB limit, HSM-backed, RBAC integration
- **HashiCorp Vault**: vaultrs v0.7+ with KV v2 versioning, token renewal, AppRole auth, policy-based access
- **Kubernetes Secrets**: kube 0.87+ with namespace isolation, 1MB limit, RBAC, watch streams with backoff
- **Local Storage**: Atomic writes (atomicwrites/uuid), 0600 permissions (Unix), ACL (Windows), AES-256-GCM encryption

**Key Decisions**:
- Shared retry logic: exponential backoff with jitter (100ms base, 2x multiplier, 5 max retries)
- Provider configs: Builder pattern with validation
- Metrics foundation: StorageMetrics struct for Phase 8 observability
- Testing: MockStorageProvider for unit tests, testcontainers for integration tests

### Phase 1: Design & Contracts ✅ COMPLETE

**Generated**:
- `data-model.md` - Complete data model with all entities, configurations, validation rules
- `contracts/storage-provider-trait.rs` - StorageProvider trait contract with implementation requirements
- `quickstart.md` - Quick start guide for all 5 providers with code examples

**Entities Designed**:
- **Configurations**: LocalStorageConfig, AwsSecretsManagerConfig, AzureKeyVaultConfig, VaultConfig, KubernetesSecretsConfig
- **Shared Utilities**: RetryPolicy, StorageMetrics, CredentialFile (local only)
- **Providers**: LocalStorageProvider, AwsSecretsManagerProvider, AzureKeyVaultProvider, HashiCorpVaultProvider, KubernetesSecretsProvider

**Validation Rules**:
- Size limits per provider (AWS 64KB, Azure 25KB, Vault configurable, K8s 1MB, Local unlimited)
- Metadata conversion (AWS tags max 50, Azure tags max 15, K8s labels max 63 chars)
- Config validation (URL formats, namespace names, timeout ranges)

**State Transitions**:
- Provider lifecycle: Uninitialized → Configured → Connected → Operating → Retrying → Disconnected
- Token renewal (Vault): Active → NeedsRenewal → Renewing → Active (or Expired)
- Soft-delete recovery (Azure/Vault): Active → Soft-Deleted → Recovered/Purged

### Phase 2: Tasks Generation (NEXT STEP)

**Command**: `/speckit.tasks` (NOT part of /speckit.plan)

**Expected Output**: `tasks.md` with actionable, dependency-ordered implementation tasks

---

## Post-Design Constitution Re-Check ✅ PASS

All principles remain satisfied after design phase:

- ✅ **Type Safety First**: ProviderConfig trait, builder patterns, no raw str types
- ✅ **Isolated Error Handling**: StorageError reused from Phase 1, provider errors mapped at boundaries
- ✅ **Test-Driven Development**: Mock and integration test strategy defined
- ✅ **Async Discipline**: Timeouts, retry logic, no blocking in async
- ✅ **Modular Architecture**: Single crate (nebula-credential), optional features for cloud SDKs
- ✅ **Observability**: Tracing spans, metrics foundation, structured logging
- ✅ **Simplicity**: ~200-300 lines per provider, shared retry utility

**No architecture changes required** - design fits within existing Phase 1 abstractions.

---

## Implementation Readiness

**Branch**: `002-storage-backends` (current branch)  
**Plan Location**: `specs/002-storage-backends/plan.md` (this file)  
**Generated Artifacts**:
- ✅ research.md - Technology research complete
- ✅ data-model.md - Data model complete
- ✅ contracts/storage-provider-trait.rs - API contract defined
- ✅ quickstart.md - User guide with examples

**Next Steps**:
1. Run `/speckit.tasks` to generate implementation task list
2. Review tasks.md for dependency ordering
3. Begin TDD implementation (tests first, then implementation)
4. Run quality gates after each phase completion:
   ```bash
   cargo fmt --all
   cargo clippy --workspace -- -D warnings
   cargo check --workspace
   cargo doc --no-deps --workspace
   ```

**Estimated Implementation Scope**:
- Local Storage: ~250 lines (atomic writes, permissions, locking)
- AWS Provider: ~300 lines (SDK integration, retry, error mapping)
- Azure Provider: ~300 lines (Managed Identity, token refresh, RBAC)
- Vault Provider: ~350 lines (token renewal, KV v2, versioning)
- K8s Provider: ~300 lines (namespace isolation, RBAC, watch)
- Shared Utils: ~150 lines (retry logic, metrics)
- **Total**: ~1,650 lines across 6 files

**Dependencies Added** (with optional features):
```toml
[dependencies]
# Existing Phase 1 deps...

# New for Phase 2:
aws-sdk-secretsmanager = { version = "1.0", optional = true }
aws-config = { version = "1.0", optional = true }
azure_security_keyvault_secrets = { version = "0.20", optional = true }
azure_identity = { version = "0.20", optional = true }
vaultrs = { version = "0.7", optional = true }
kube = { version = "0.87", optional = true, features = ["runtime", "derive"] }
k8s-openapi = { version = "0.20", optional = true, features = ["latest"] }
atomicwrites = "0.4"
uuid = { version = "1.0", features = ["v4", "fast-rng"] }
fs2 = "0.4"
directories = "5.0"

[target.'cfg(windows)'.dependencies]
windows-acl = "0.3"

[features]
default = ["storage-local"]
storage-local = []
storage-aws = ["aws-sdk-secretsmanager", "aws-config"]
storage-azure = ["azure_security_keyvault_secrets", "azure_identity"]
storage-vault = ["vaultrs"]
storage-k8s = ["kube", "k8s-openapi"]
storage-all = ["storage-aws", "storage-azure", "storage-vault", "storage-k8s"]
```

---

## Planning Complete ✅

**Status**: Ready for implementation task generation via `/speckit.tasks`

**Command to proceed**:
```bash
/speckit.tasks
```
