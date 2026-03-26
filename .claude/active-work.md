# Active Work
Updated: 2026-03-26

## In Progress
- **nebula-credential v2** phases 5-6 (adapt storage backends, testing infra)
- **nebula-engine/runtime**: blocked on credential DI
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
- **nebula-credential v2 phases 1-4** (2026-03-26): AuthScheme in core, Credential trait, storage layers, resolver, RefreshCoordinator. Resource::Auth rename done.
- **nebula-resource v2 DX** (2026-03-26): Debug, convenience register/acquire, health_check, docs rewrite, safety fixes
- **nebula-resource v2** (2026-03-25): all 6 phases done
- **nebula-parameter v3** (2026-03-25): full rewrite, consumers migrated

## Blocked
- **nebula-engine execution**: needs credential DI (partially unblocked by credential v2 phases 1-4)
- **nebula-webhook migration**: uses deprecated v1 compat types

## Next Up
- nebula-credential Phase 5 (adapt v1 storage backends to CredentialStoreV2)
- nebula-credential Phase 6 (testing infrastructure)
- Wire CredentialResolver into ActionContext (unblocks engine)
- Delete v1 credential code after Phase 6
