# nebula-credential

`nebula-credential` is Nebula's secure credential management system.

It covers:
- credential lifecycle (store/retrieve/update/delete)
- multi-tenant scoped access via `CredentialContext`
- protocol abstractions (`OAuth2`, API key, basic/header auth, DB, LDAP, mTLS, SAML, Kerberos)
- pluggable storage providers (local/AWS/Vault/Kubernetes, feature-gated)
- caching, validation, and rotation orchestration

## Role in Platform

This crate is the security boundary for secrets in a Rust n8n-like platform.  
Actions/plugins should access credentials through provider abstractions, not direct secret storage internals.

## Main Modules

- `core` - ids, metadata, context, errors, references/provider primitives
- `manager` - high-level `CredentialManager` API
- `protocols` - reusable credential protocol definitions
- `providers` - storage backends
- `rotation` - rotation policies, transactions, blue-green/grace-period flows
- `traits` - storage/locking/credential behavior traits
- `utils` - crypto/secret/time/retry helpers

## Document Set

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [API.md](API.md)
- [DECISIONS.md](DECISIONS.md)
- [ROADMAP.md](ROADMAP.md)
- [PROPOSALS.md](PROPOSALS.md)
