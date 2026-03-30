# Active Work
Updated: 2026-03-30

## In Progress
- **nebula-credential v2** phases 5-6
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
- **nebula-error Classify migration** (03-30): all 21 crates implement Classify trait
- **nebula-error v1** (03-27): Classify trait, NebulaError<E>, ErrorDetails, derive macro
- **nebula-credential v2 phases 1-4** (03-26): AuthScheme, Credential trait, storage, resolver
- **nebula-resource v2 + DX** (03-25/26)
- **nebula-parameter v3** (03-25)

## Blocked
- **nebula-engine**: needs credential DI
- **nebula-webhook**: uses deprecated v1 compat types

## Next Up
- nebula-credential Phase 5-6 (storage backends + testing)
- Wire CredentialResolver into ActionContext
- Delete v1 credential code
