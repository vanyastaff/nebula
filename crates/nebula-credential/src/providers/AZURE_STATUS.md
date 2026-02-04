# Azure Key Vault Provider Status

## Current Status: ⚠️ BLOCKED BY SDK VERSION CONFLICT

### Problem

The Azure Key Vault provider implementation is **blocked** due to a fundamental dependency conflict:

- **AWS SDK** uses `azure_core` version **0.31**
- **azure_security_keyvault_secrets 0.10** requires `azure_core` version **0.10**

These two versions of `azure_core` are **incompatible** and cannot coexist in the same binary.

### What Has Been Implemented ✅

1. **AzureKeyVaultConfig** (T027) - Complete
   - Full configuration struct with all fields
   - Comprehensive validation (URL format, HTTPS, GUIDs, timeouts, tag limits)
   - Support for 3 authentication types:
     - Managed Identity (production)
     - Service Principal (CI/CD)
     - Developer Tools (local development)
   - Retry policy integration
   - Serde serialization/deserialization

2. **AzureKeyVaultProvider skeleton** (T028) - Partial
   - Provider struct definition
   - Helper methods (get_secret_name, metadata_to_azure_tags, validate_size)
   - StorageProvider trait implementation (all methods defined)
   - Proper error mapping for Azure-specific errors

3. **API Integration Research** (T029-T030)
   - Correct Azure SDK 0.10 API patterns identified
   - RequestContent and SetSecretParameters usage documented
   - Pagination with try_next() for list operations
   - Response.into_model() pattern for result extraction

### What's Missing ❌

1. **Client Initialization** - The SecretClient::new() requires credentials that implement
   `azure_core::credentials::TokenCredential` from version 0.10, but we cannot create
   them due to the azure_core conflict.

2. **Integration Tests** (T032) - Cannot be implemented without working client initialization.

### Workarounds

#### Option 1: Use Separate Features (Current Approach)

**Do NOT enable `storage-aws` and `storage-azure` simultaneously**

```toml
# OK - Only Azure
[dependencies]
nebula-credential = { version = "0.1", features = ["storage-azure"] }

# OK - Only AWS  
[dependencies]
nebula-credential = { version = "0.1", features = ["storage-aws"] }

# ❌ FAILS - Both together
[dependencies]
nebula-credential = { version = "0.1", features = ["storage-aws", "storage-azure"] }
```

#### Option 2: Wait for Azure SDK Update

Azure SDK for Rust is actively developed. Monitor for updates to `azure_security_keyvault_secrets`
that use `azure_core` 0.20+ or newer.

- GitHub: https://github.com/Azure/azure-sdk-for-rust
- Crates.io: https://crates.io/crates/azure_security_keyvault_secrets

#### Option 3: Use Alternative Providers

For immediate production use, consider:
- ✅ **LocalStorageProvider** - File-based with AES-256-GCM encryption
- ✅ **AwsSecretsManagerProvider** - AWS Secrets Manager (fully implemented)
- ⏳ **HashiCorpVaultProvider** - Vault KV v2 (Phase 6 - next)
- ⏳ **KubernetesSecretsProvider** - K8s Secrets (Phase 7 - planned)

### Next Steps

1. **Phase 6**: Implement HashiCorp Vault provider (no dependency conflicts)
2. **Phase 7**: Implement Kubernetes Secrets provider  
3. **Monitor Azure SDK**: Check quarterly for `azure_security_keyvault_secrets` updates
4. **Return to Azure**: Complete implementation once SDK conflict is resolved

### Files

- **Config**: `src/providers/azure.rs` (lines 1-230) - ✅ Complete
- **Provider**: `src/providers/azure.rs` (lines 231-656) - ⚠️ Skeleton only
- **Tests**: Not created (blocked by client init)
- **Documentation**: This file

### Technical Details

The specific error when enabling both features:

```
error[E0277]: the trait bound `ImdsManagedIdentityCredential: azure_core::credentials::TokenCredential` is not satisfied
note: there are multiple different versions of crate `azure_core` in the dependency graph
  --> azure_core-0.31.0/src/credentials.rs:99:1  (expected)
  --> azure_core-0.10.0/src/auth.rs:42:1         (found)
```

Cargo cannot resolve this conflict because `azure_core` 0.10 and 0.31 have incompatible trait definitions.

---

**Last Updated**: 2026-02-04  
**Blocked Since**: Phase 5 implementation  
**Resolution**: Pending Azure SDK update
