# nebula-credential
AES-256-GCM credential storage, manager, rotation, and protocol implementations.

## Invariants
- Credentials are **always encrypted at rest**. `encrypt`/`decrypt` in `utils` module use AES-256-GCM. No plaintext credential storage.
- `SecretString` zeroizes memory on drop. Never convert to `String` unless absolutely necessary.
- Never add a direct import from nebula-credential to nebula-resource or vice versa — use `EventBus<CredentialRotatedEvent>`.

## Key Decisions
- `CredentialProvider` trait = DI for actions. Actions declare credential needs via `ActionDependencies`; the engine injects via `CredentialAccessor` in `Context`. Never inject `CredentialManager` directly.
- `CredentialManager` wraps storage + cache layer + rotation. Use `CredentialManagerBuilder` to construct.
- Phases: 1-2 done (core types, manager, storage providers). 3-7 planned (derive macros, provider adapters, moka cache, test infra, protocol stubs).
- Built-in protocols: `ApiKeyProtocol`, `OAuth2Protocol`, `BasicAuthProtocol`, `HeaderAuthProtocol`, `DatabaseProtocol`, `LdapProtocol`, mTLS, SAML, Kerberos.

## Traps
- Circular dep with nebula-resource: the crates are at the same Business Logic layer and must not import each other. Signal credential rotation via EventBus only.
- Storage providers are feature-gated: `storage-local`, `storage-aws`, `storage-postgres`, `storage-vault`, `storage-k8s`.
- `CredentialId` here is a `nebula_core::CredentialId` re-export — not a separate type.

## Relations
- Depends on nebula-core (IDs), nebula-eventbus (rotation events). Peer with nebula-resource (no import between them).
