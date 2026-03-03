# nebula-credential

`nebula-credential` is Nebula's secure credential management system.

## Scope

- **In scope:** Credential lifecycle (store/retrieve/update/delete); multi-tenant scoped access; protocol abstractions (OAuth2, API key, basic/header auth, DB, LDAP, mTLS, SAML, Kerberos); pluggable storage providers (local/AWS/Vault/Kubernetes); caching; validation; rotation orchestration
- **Out of scope:** Workflow business logic; API/CLI transport; UI rendering

## Current State

- **maturity:** Production-oriented; manager, providers, rotation subsystem implemented
- **key strengths:** Provider abstraction; scope isolation; encryption at rest; protocol extensibility
- **key risks:** Wide feature matrix; rotation state machine complexity; provider capability variance

## Target State

- **production criteria:** Stable API subsets; formal scope enforcement; provider capability matrix; audit coverage
- **compatibility guarantees:** See MIGRATION.md

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

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](ARCHITECTURE.md)
- [API.md](API.md)
- [INTERACTIONS.md](INTERACTIONS.md)
- [PROTOCOLS.md](PROTOCOLS.md) — protocol layer (StaticProtocol/FlowProtocol, configs, states)
- [TARGET_ARCHITECTURE.md](TARGET_ARCHITECTURE.md) — target architecture, diagrams, refactoring plan
- [DECISIONS.md](DECISIONS.md)
- [ROADMAP.md](ROADMAP.md)
- [PROPOSALS.md](PROPOSALS.md)
- [SECURITY.md](SECURITY.md)
- [RELIABILITY.md](RELIABILITY.md)
- [TEST_STRATEGY.md](TEST_STRATEGY.md)
- [MIGRATION.md](MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
