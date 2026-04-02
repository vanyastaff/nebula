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

| Document | Contents |
|----------|----------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Module map, state machine, type-state patterns, concurrency, data flows |
| [API.md](API.md) | Public API surface, core types, traits, interactive flows |
| [PROTOCOLS.md](PROTOCOLS.md) | Protocol support matrix, StaticProtocol/FlowProtocol, macro DX, state types |
| [TARGET_ARCHITECTURE.md](TARGET_ARCHITECTURE.md) | Target architecture diagrams, refactoring plan |
| [ROADMAP.md](ROADMAP.md) | 8-phase roadmap to v1.0 (~7-9 months) |
| [SCOPE_ENFORCEMENT.md](SCOPE_ENFORCEMENT.md) | Scope enforcement behavior, retrieve vs retrieve_scoped, provider matrix |
| [MIGRATION.md](MIGRATION.md) | Versioning policy, breaking changes, rollout/rollback plans |
