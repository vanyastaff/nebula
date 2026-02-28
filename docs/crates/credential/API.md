# API Reference (Human-Oriented)

## High-level Entry Point

- `CredentialManager`
  - CRUD operations
  - listing/filtering
  - validation checks
  - optional cache integration
  - batch and scoped operations

Created via `CredentialManager::builder()`.

## Core Types

- `CredentialId`, `ScopeId`
- `CredentialContext`
- `CredentialMetadata`
- `CredentialDescription`
- `CredentialFilter`
- `CredentialRef`, `CredentialProvider`
- `CredentialState`
- `CredentialError`, `StorageError`, `ManagerError`, `ValidationError`, `CryptoError`

## Traits

- `StorageProvider`, `StateStore`
- `DistributedLock`
- `CredentialResource`, `CredentialType`
- `FlowProtocol`, `StaticProtocol`, `InteractiveCredential`
- `Refreshable`, `Revocable`
- `RotatableCredential`, `TestableCredential`

## Built-in Protocols

- `ApiKeyProtocol`
- `BasicAuthProtocol`
- `HeaderAuthProtocol`
- `DatabaseProtocol`
- `OAuth2Protocol` (+ config/state/flow helpers)
- `LdapProtocol`
- `SamlConfig`
- `KerberosConfig`
- `MtlsConfig`

## Storage Providers

- `MockStorageProvider` (always available)
- `LocalStorageProvider` (`storage-local`)
- `AwsSecretsManagerProvider` (`storage-aws`)
- `HashiCorpVaultProvider` (`storage-vault`)
- `KubernetesSecretsProvider` (`storage-k8s`)

Provider configuration and metrics are exposed via `providers::config` and `providers::metrics`.

## Rotation APIs

- `RotationPolicy` (`Periodic`, `BeforeExpiry`, `Scheduled`, `Manual`)
- `RotationTransaction`, `TransactionPhase`, `RotationState`
- grace period and backup APIs
- blue/green helpers for zero-downtime scenarios
- retry and failure classification helpers

## Utilities

- `EncryptionKey`, `EncryptedData`, `encrypt`, `decrypt`
- `SecretString`
- retry and time helpers
- validation helpers for encrypted payload constraints
