# Target Architecture (Refactoring Plan)

## Protocol Layer

```mermaid
flowchart TB
    subgraph plugin [Plugin Developer API]
        CredentialType
        CredentialResource
    end

    subgraph protocols [Protocol Layer]
        StaticProtocol
        FlowProtocol
        InteractiveCredential
    end

    subgraph static [StaticProtocol impls]
        ApiKeyProtocol
        BasicAuthProtocol
        HeaderAuthProtocol
        DatabaseProtocol
    end

    subgraph flow [FlowProtocol impls]
        OAuth2Protocol
        LdapProtocol
        SamlProtocol
        KerberosProtocol
        MtlsProtocol
    end

    subgraph erased [Erased Layer]
        ErasedProtocol
        ProtocolDriver
        ProtocolRegistry
    end

    CredentialType --> StaticProtocol
    CredentialType --> FlowProtocol
    CredentialType --> InteractiveCredential
    CredentialResource --> CredentialType
    StaticProtocol --> ApiKeyProtocol
    StaticProtocol --> BasicAuthProtocol
    StaticProtocol --> HeaderAuthProtocol
    StaticProtocol --> DatabaseProtocol
    FlowProtocol --> OAuth2Protocol
    FlowProtocol --> LdapProtocol
    FlowProtocol --> SamlProtocol
    FlowProtocol --> KerberosProtocol
    FlowProtocol --> MtlsProtocol
    CredentialType --> ErasedProtocol
    StaticProtocol --> ProtocolDriver
    FlowProtocol --> ProtocolDriver
    ProtocolDriver --> ErasedProtocol
    ErasedProtocol --> ProtocolRegistry
```

## Management Layer

```mermaid
flowchart TB
    subgraph consumers [Consumers]
        Action
        Resource
        Engine
        NebulaApi
    end

    subgraph provider [CredentialProvider trait]
        credential_type
        get_by_id
    end

    subgraph manager [CredentialManager]
        store
        retrieve
        delete
        list
        validate
        rotate_credential
    end

    subgraph storage [Storage]
        CacheLayer
        StorageProvider
    end

    subgraph rotation [Rotation Subsystem]
        RotationTransaction
        RotationPolicy
        GracePeriod
        BlueGreen
    end

    Action --> provider
    Resource --> provider
    Engine --> provider
    NebulaApi --> manager

    provider -.->|future impl| manager
    manager --> CacheLayer
    manager --> StorageProvider
    manager --> rotation
    rotation --> StorageProvider
    ProtocolRegistry --> manager
```

## Manager vs Provider API Gaps (Phase 4)

| API doc (nebula-api) | Current Manager | Status |
|----------------------|-----------------|--------|
| `create(type_id, input)` → `InitializeResult` | Stub (returns error) | **Stub** |
| `continue_flow(id, UserInput)` → `InitializeResult<Complete>` | Stub (returns error) | **Stub** |
| `list_types()` → `Vec<CredentialTypeSchema>` | Returns empty vec | **Stub** — requires `ProtocolRegistry` with registered `CredentialKey → ErasedProtocol` mappings |
| `list(filter)` → `Vec<CredentialMetadata>` | `list(context)` → `Vec<CredentialId>` | Partial |
| `get(id)` → `(Metadata, CredentialStatus)` | `retrieve(id, ctx)` → `(EncryptedData, Metadata)` | Partial |
| `CredentialManager` implements `CredentialProvider` | `get(id)` works with `encryption_key` | **Done** |

## Rotation Boundaries

- **Manager** calls high-level rotation entry points; does not implement state machine.
- **Rotation** owns: `RotationTransaction`, `RotationState`, `TransactionPhase`, `GracePeriodTracker`, `BlueGreenRotation`, `FailureHandler`, `TransactionLog`.
- **Public types**: `RotationResult`, `RotationError`, `TransactionLog`, `GracePeriodConfig`, `RotationPolicy`.

## Auth Scenarios

| Scenario | Protocols | CredentialResource |
|----------|------------|--------------------|
| HTTP APIs | ApiKey, BasicAuth, HeaderAuth, OAuth2 | HTTP client receives State, applies via `authorize` |
| Enterprise IdP | LdapProtocol, SamlProtocol, KerberosProtocol | LDAP/SAML/Kerberos clients |
| DB + mTLS | DatabaseProtocol, MtlsConfig | DB connection pools, TLS clients |

## Phased Refactoring

1. **Phase 1**: Document gaps (this file); no code changes.
2. **Phase 2**: Add `CredentialManager::create`, `continue`, `list_types` stubs; implement `CredentialProvider` for `CredentialManager` (id-based only initially).
3. **Phase 3**: Align `list`/`get` with API contract; add `CredentialStatus`, `CredentialTypeSchema`.
4. **Phase 4**: Full nebula-api integration; type registry for `credential<C>()`.
