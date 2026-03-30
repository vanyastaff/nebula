# Active Work
Updated: 2026-03-30

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
- **nebula-credential v1 deletion** (03-30): Phase 8 — deleted traits/, protocols/, providers/, manager/, v1 core types, ~19K LOC removed. Renamed CredentialStateV2 → CredentialState.
- **nebula-error Classify migration** (03-30): all 21 crates implement Classify trait
- **nebula-error v1** (03-27): Classify trait, NebulaError<E>, ErrorDetails, derive macro
- **nebula-credential v2 phases 1-7** (03-26-30): AuthScheme, Credential trait, storage, resolver, storage backends, layers
- **nebula-resource v2 + DX** (03-25/26)
- **nebula-parameter v3** (03-25)

## Blocked
- **nebula-engine**: needs credential DI
- **nebula-webhook**: uses deprecated v1 compat types

## Next Up
- Rewrite `#[derive(Credential)]` macro for v2 Credential trait
- Wire CredentialResolver into ActionContext
