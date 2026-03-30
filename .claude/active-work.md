# Active Work
Updated: 2026-03-30

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
- **nebula-credential v2 complete** (03-30): All 9 phases done — AuthScheme 13 types, Credential trait, 6 storage backends, 4 layers, OAuth2 migration, PendingStateStore, executor, RefreshCoordinator hardening, derive(Credential) macro, v1 deleted (~17.6K LOC)
- **nebula-error Classify migration** (03-30): all 21 crates implement Classify trait
- **nebula-error v1** (03-27): Classify trait, NebulaError<E>, ErrorDetails, derive macro
- **nebula-resource v2 + DX** (03-25/26)
- **nebula-parameter v3** (03-25)

## Blocked
- **nebula-engine**: needs credential DI (CredentialResolver into ActionContext)
- **nebula-webhook**: uses deprecated v1 compat types

## Next Up
- Wire CredentialResolver into ActionContext (unblocks nebula-engine)
- nebula-webhook migration to v2 types
- nebula-credential Phase 5-6 storage backends: adapt production providers (Postgres, Vault, AWS, K8s) to real cloud SDKs (current impls have type-safe interfaces, some with SDK stubs)
