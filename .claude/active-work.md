# Active Work
Updated: 2026-04-03

## In Progress
- **Desktop app** (Tauri): `apps/desktop/`

## Recently Completed
- **nebula-system overhaul** (04-03): Phase 1 cleanup (removed broken management API, dead features, stubs), Phase 2 test hardening (38 tests + 5 doctests), Phase 3 ProcessMonitor (per-PID sandbox tracking), Phase 4 SystemLoad (adaptive worker scaling). Deleted old `crates/system/design/`. Plan at `docs/plans/2026-04-03-nebula-system-overhaul.md`.
- **nebula-credential HLD v1.5** (04-01): Full architecture review: 10 adversarial rounds, 2 dev challenges, 2 town halls, 2 open conferences (10 external devs total incl. gaming, Airflow migration, SOC2 auditor, dev tooling, Vault skeptic). 33 v1 ship items. 17 bugs. SOC2 grades: CC6.1/CC7.2 CONDITIONAL, CC6.3 PASS. New in v1.5: DecryptedCacheLayer, DatabaseAuth.extensions, registry introspection. v1.1 deferred: CredentialStore dyn-compat, put_batch. HLD at `docs/plans/nebula-credential-hld-v1.md`.
- **nebula-credential DX excellence** (03-31): Typed `CredentialSnapshot` (`Box<dyn Any>` + `project::<S>()`), `credential_typed::<S>()` on ActionContext/TriggerContext, rotation feature-gated, `CredentialResolverRef` for composition, 285 tests + 19 doctests, missing Debug impls added, broken doctests fixed.
- **nebula-resilience deep invariant audit** (03-31): 9 bug fixes (Duration overflow panic, pipeline total_budget drop, SlidingWindow stale entries, hedge delay overflow, TokenBucket burst/reset/current_rate, LeakyBucket current_rate, AdaptiveHedge zero delay), CB counter dedup, 4 doc link fixes, all clippy --all-targets clean. 153 tests, 7 benchmark suites, 14 integration tests.
- **nebula-resilience full audit** (03-31): Bug fixes (burst sync, probe slot leak, jitter, retry budget), naming audit (8 renames per API Guidelines), interoperability (Debug/serde/non_exhaustive on all types), design patterns audit, 10-dimension code review. 139 tests.
- **nebula-credential v2 complete** (03-30): All 9 phases done — AuthScheme 13 types, Credential trait, 6 storage backends, 4 layers, OAuth2 migration, PendingStateStore, executor, RefreshCoordinator hardening, derive(Credential) macro, v1 deleted (~17.6K LOC)
- **nebula-error Classify migration** (03-30): all 21 crates implement Classify trait
- **nebula-error v1** (03-27): Classify trait, NebulaError<E>, ErrorDetails, derive macro
- **nebula-resource v2 + DX** (03-25/26)
- **nebula-parameter v3** (03-25)

## Blocked
- **nebula-engine**: needs credential DI (CredentialResolver into ActionContext) + storage Postgres backend
- **nebula-webhook**: uses deprecated v1 compat types
- **nebula-auth**: RFC phase — blocked on RFC convergence, no stable API yet
- **Desktop Phase 2+**: blocked on engine working end-to-end (Group 2 acceptance criteria)

## Cross-Crate Priority Order
1. `nebula-storage` Phase 1 (Postgres) — unblocks engine
2. `nebula-action` Phase 2 — finish context model (unblocks runtime + credential integration)
3. `nebula-resource` Phase 1 — contract docs + scope invariants
4. `nebula-runtime` Phase 1 — isolation routing + SpillToBlob
5. Desktop Phase 1 — typed IPC (independent of backend)
6. `nebula-engine` Phase 1 — wire to Postgres (needs items 1–3 done)

## Next Up
- **Fix pre-existing bugs B1-B9** from credential HLD review (B6 CRITICAL: verify_owner fails open)
- **Implement CredentialPhase + OwnerId** (unblocks state machine, scoping, 5 new error variants)
- **Implement StackBuilder** (encryption-mandatory composition)
- **Replace Provider(String) with ProviderError** (retryability signaling)
- **Add test-support feature** (FakeCredentialBackend, CredentialScenario)
- **Re-export ParameterValues/AuthScheme** from nebula-credential (single-crate plugin DX)
- Wire CredentialResolver into ActionContext (unblocks nebula-engine)
- nebula-credential-storage crate (SQLite backend)
- nebula-webhook migration to v2 types
