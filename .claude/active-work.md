# Active Work
Updated: 2026-03-31

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
- **nebula-resilience deep invariant audit** (03-31): 9 bug fixes (Duration overflow panic, pipeline total_budget drop, SlidingWindow stale entries, hedge delay overflow, TokenBucket burst/reset/current_rate, LeakyBucket current_rate, AdaptiveHedge zero delay), CB counter dedup, 4 doc link fixes, all clippy --all-targets clean. 153 tests, 7 benchmark suites, 14 integration tests.
- **nebula-resilience full audit** (03-31): Bug fixes (burst sync, probe slot leak, jitter, retry budget), naming audit (8 renames per API Guidelines), interoperability (Debug/serde/non_exhaustive on all types), design patterns audit, 10-dimension code review. 139 tests.
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
