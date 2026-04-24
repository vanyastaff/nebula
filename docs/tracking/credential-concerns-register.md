---
name: credential concerns register
status: living document — updated as sub-specs land and concerns surface
seeded: 2026-04-24 (Checkpoint 2)
last-updated: 2026-04-24
maintainer: credential-redesign workstream
---

# Credential Concerns Register

Living document. Tracks concerns surfaced during credential redesign from multiple sources (draft 2026-04-24, critique rounds, user's strategy-concerns list). Each row has a 6-label classification (see schema below) plus an 8-value status enum; labels are mutually exclusive by intent.

**Not authoritative for decisions** — the Strategy Document, Tech Spec, and ADRs are authoritative. This register is a tracking surface to ensure no concern is silently dropped and to give each concern a traceable resolution pointer.

## Schema

Each row:

- **ID** — `source:number` (e.g., `draft-f1`, `critique-c3`, `user-lifecycle-1`).
- **Category** — domain area.
- **Concern** — one-line description.
- **Label** — one of:
  - `strategy-blocking` — decision needed before prototype spike dispatch (resolved in Strategy §2/§3)
  - `tech-spec-material` — decision post-spike, affects trait/impl directly
  - `sub-spec` — requires separate design document
  - `implementation-phase` — execution detail, no design decision needed
  - `product-policy` — orthogonal to type shape (sealed/open, SOC 2, deployment mode policy, GDPR)
  - `process` — findings about the redesign workstream itself (budget, spike scope, inter-iteration checkpoints, success criteria) rather than about credential design concerns
- **Status** — `decided` / `locked-post-spike` / `pending-sub-spec` / `in-implementation` / `proposed` / `policy-frozen` / `open` / `out-of-scope`.
  - `proposed` — sub-spec or artefact started but not landed (draft status, work ongoing).
  - `out-of-scope` — concern exists but is owned outside credential scope (e.g. Plane A per ADR-0033).
- **Resolution** — pointer to where decided (§ of Strategy, Tech Spec, sub-spec file), or `TBD`.
- **Notes** — optional.

## Sources

- `draft-f{N}` — finding N in [drafts/2026-04-24-credential-redesign/05-known-gaps.md](../superpowers/drafts/2026-04-24-credential-redesign/05-known-gaps.md).
- `critique-c{N}` — numbered finding N in critique round (conversation 2026-04-24, mapped here).
- `user-{category}-{N}` — user's strategy-concerns list (2026-04-24).
- `arch-{name}` — architectural concern surfaced during conversation that does not match a numbered source (e.g., `arch-signing-infra` for signed-manifest infrastructure identified during sealed-policy discussion).

---

## Type system

All strategy-blocking findings resolved in Checkpoint 1 or deferred to spike validation.

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f1 | `ctx.credential::<C>()` ambiguity on multiple slots of same type | strategy-blocking | locked-post-spike | Spike Q2 validates hand-expanded macro output |
| draft-f2 | `ctx.credential::<dyn AcceptsBearer>()` conflict with sig (`AcceptsBearer` ≠ `Credential`) | strategy-blocking | decided | Strategy §3.2 — `dyn` nominal bound vs vtable clarified; runtime type-erased through `AnyCredential` |
| draft-f3 | `dyn Credential` / `dyn BitbucketCredential` not dyn-safe (4 assoc types) | strategy-blocking | decided | Strategy §3.2 — type-erased runtime path via `AnyCredential` + downcast; `dyn BitbucketBearer` is nominal bound only |
| draft-f4 | `CredentialRef<C>` runtime shape — PhantomData only | strategy-blocking | locked-post-spike | Spike Q3 three hypotheses H1/H2/H3 (Strategy §3.4) |
| draft-f5 | Pattern 2 as default for multi-auth, not Pattern 1 | strategy-blocking | decided | Strategy §2.2 table — Pattern 2 default for multi-auth |
| draft-f14 | Multi-credential resource (mTLS + Bearer in one HTTP client) | strategy-blocking | locked-post-spike | Spike Q4 — DualAuth<A, B> |
| draft-f32 | `CredentialGuard<C::Scheme>` projection through dyn service trait | strategy-blocking | locked-post-spike | Spike Q1 — pure marker + blanket sub-trait pattern (Strategy §3.3) |
| critique-c2 | §3.2 dyn-safety framing reduction incorrect (`AnyCredential` ≠ mechanism for `dyn Credential` being dyn-safe) | strategy-blocking | decided | Strategy §3.2 rewrite (Checkpoint 1 edit round 1) |
| critique-c6 | Bitbucket AppPassword vs Bearer service trait conflict | strategy-blocking | decided | Strategy §3.2–§3.3 — pure marker trait + blanket sub-trait with capability bound |
| critique-c10 | Triggers / multi-step / refresh-race compat not sketched in spike | strategy-blocking | locked-post-spike | Spike requires 5 compat sketches in NOTES.md (Strategy §Spike plan) |
| critique-c11 | `Credential` trait heaviness un-flagged | tech-spec-material | decided | Strategy §3.6 — addition discipline policy (ADR/alt/dyn-impact required per new assoc type / method / flag) |
| arch-phantom-shim-convention | Two-trait phantom-shim pattern with per-crate sealed placement for capability traits in `dyn` positions | tech-spec-material | decided | [ADR-0035](../adr/0035-phantom-shim-capability-pattern.md) — supersedes Strategy §3.2/§3.3 portions; spike iter-1 validated first instance (commit `acfec719`) |

## Sealed / plugin / registration

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f8 | Sealed trait prevents plugin impls vs 400+ plugin goal | product-policy | policy-frozen | Strategy §2.1 — sealed for API surface cleanliness (not security); `#[plugin_credential]` escape hatch |
| draft-f9 | Capability markers tie resource writers to credential crate | tech-spec-material | decided | Strategy §2.4 — accepted trade-off; markers live in credential crate |
| critique-c16 | Plugin registration mechanism for 3rd-party | tech-spec-material | decided | Explicit `register::<C>()` in plugin init (Strategy §2.1); `inventory`-style rejected (cross-crate unreliable) |
| arch-signing-infra | Signed manifest infrastructure (desktop / self-hosted / cloud trust anchors) | sub-spec | pending-sub-spec | Separate sub-spec per Strategy §2.1; macro works without signing until infra lands |

## Patterns and service grouping

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f6 | Wrapper type vs ProviderRegistry duplicate info | tech-spec-material | locked-post-spike | Tech Spec after ProviderRegistry sub-spec lands |
| draft-f7 | Generic OAuth2 fallback vs per-service types | tech-spec-material | decided | Strategy §2.2 — `GenericOAuth2Credential` as Pattern 3 (`dyn AcceptsBearer`) consumer |
| user-evolution-p1p2 | Pattern 1 → Pattern 2 promotion path (breaking change?) | tech-spec-material | decided | Strategy §2.2 — policy (a) breaking change per semver; (b) defensive Pattern 2 always rejected due to boilerplate cost |

## Resource-per-capability

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| critique-a-resource | Service with multiple auth capabilities — one Resource vs multiple | strategy-blocking | decided | Strategy §2.3 — one Resource per capability (BitbucketBearerClient + BitbucketBasicClient); not builder polymorphism |
| critique-b-macro-check | `#[action]` macro mechanism for capability ↔ resource cross-check | strategy-blocking | locked-post-spike | Strategy §3.5 — spike validates trait-resolution OR compile-time registry mechanism; Fallback B if both fail |

## Refresh & rotation (operational)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f15 | In-proc vs cross-process RefreshToken handle mixed | tech-spec-material | locked-post-spike | Two-tier coordinator (L1 proc + L2 durable claim) — Tech Spec §6 |
| draft-f16 | Heartbeat cadence / claim TTL / refresh timeout discipline | tech-spec-material | locked-post-spike | CI test + `debug_assert` constraints (not just documentation) per critique-c12 |
| draft-f17 | Mid-refresh crash with rotated refresh_token (IdP invalidates old) | sub-spec | proposed | [`docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`](../superpowers/specs/2026-04-24-credential-refresh-coordination.md) — draft proposal (651 lines, status `proposal`) |
| user-op-refresh | Refresh strategy: proactive / reactive / coordinated / failure handling | tech-spec-material | locked-post-spike | Tech Spec §6 |
| user-op-distributed | Multi-replica refresh lock + cache invalidation broadcast + rotation coordination | sub-spec | pending-sub-spec | Refresh lock — see `draft-f17` (proposed); rotation leader + cache invalidation broadcast — separate sub-spec TBD (`RotationLeaderClaimRepo`) |

## ProviderRegistry (sub-spec cluster)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f18 | Who seeds initial ProviderSpec (cloud / self-hosted / desktop divergence) | sub-spec | pending-sub-spec | ProviderRegistry design spec |
| draft-f19 | ProviderSpec update = breaking for existing credentials | sub-spec | pending-sub-spec | Same spec — versioning + migration path |
| draft-f20 | Microsoft multi-tenant URL template (`.../{tenant}/oauth2/...`) | sub-spec | pending-sub-spec | Same spec — URI template + `template_vars` |
| draft-f21 | Admin API security — operator compromise → SSRF via registry | tech-spec-material | locked-post-spike | Multi-layer defense: admin RBAC + audit + optional signed entries |

## Multi-step / pending state

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f22 | Accumulator state between multi-step flows not in `PendingStore` | sub-spec | pending-sub-spec | Atomic-only for Strategy; extended `PendingStore` with typed accumulator — separate spec when use case lands |
| draft-f23 | `continue_resolve()` signature for step N | tech-spec-material | decided | Current signature handles OAuth2 single-continuation (atomic); extends when f22 sub-spec lands |

## Execution-scoped credentials

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f24 | Ephemeral vs persisted credential namespace collision | tech-spec-material | decided | `ExecutionCredentialRef<C>` typed newtype distinct from `CredentialRef<C>` (per critique-c17 — typed distinction preferred over prefix convention) |
| draft-f25 | Cancellation vs zeroize-on-drop on abort (Drop may skip) | tech-spec-material | locked-post-spike | Explicit `cleanup()` called at teardown including cancel path; documented as "zeroize is best-effort, cleanup is mandatory" |

## Connection-bound resources

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f26 | `on_credential_refresh(&mut self, ...)` on active pool + concurrent queries | tech-spec-material | locked-post-spike | Blue-green pool swap — `Arc<RwLock<PgPool>>` read for queries, write for refresh |
| draft-f27 | Refresh frequency for DB/Kafka — overbuilt abstraction | tech-spec-material | decided | Accept — `on_credential_refresh` default no-op; real use case = AWS IAM DB auth |

## Storage layer

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f28 | SQLite vs Postgres schema parity + desktop NoOp repos | tech-spec-material | locked-post-spike | Migration scripts parity CI-gated; `NoOpClaimRepo` for single-replica |
| draft-f29 | `AuditLayer` fail-closed vs degraded mode audit-writes | tech-spec-material | locked-post-spike | Degraded read-only + fallback sink for audit-writes when audit DB down |
| user-data-schema | Storage schema (tables / indices / FKs / tombstones) | tech-spec-material | locked-post-spike | Tech Spec §13 |

## Scheme / injection details

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| draft-f10 | `Send + Sync + ZeroizeOnDrop` bounds not explicit on `SchemeInjector` | implementation-phase | in-implementation | Add explicit bounds |
| draft-f11 | AWS SigV4 signing over streaming body | tech-spec-material | locked-post-spike | `SigningContext::body_hash: Option<Vec<u8>>` + `UNSIGNED-PAYLOAD` option; caller pre-computes |
| draft-f12 | `inject` default `Err` + `sign` default fallback to inject — confusing | tech-spec-material | decided | Split capability markers: `AcceptsStaticInjection` vs `AcceptsRequestSigning` (mutually exclusive by default) |
| draft-f13 | `InjectError::NotApplicable` outside `Classify` axis | tech-spec-material | decided | New `Capability` axis in `nebula-error` (WrongScheme / WrongInjection / NotSupported) |
| draft-f30 | `pattern_support` not covering custom plugin patterns | tech-spec-material | decided | `PatternSupport::{Static(&[...]), Dynamic(Box<dyn Fn + Send + Sync + 'static>)}` enum |
| draft-f31 | `DeserializeFromProvider` format-coupled | tech-spec-material | decided | Two-stage: `RawProviderOutput { bytes, metadata }` + `TryFrom<&RawProviderOutput>` for Scheme |
| draft-f33 | `CredentialMetadata` static hardcoded — operator customization limits | tech-spec-material | decided | Two-layer: `::defaults()` + `::with_override(MetadataOverrides)` via registry or per-tenant config |
| draft-f37 | `FieldSensitivity::Identifier` vs `Public` distinction not meaningful | implementation-phase | in-implementation | Collapse to `Public` / `Secret`; identifier hint → `FieldUi` metadata |

## Open / ambiguous

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| critique-c9 | `const PROVIDER_ID: &'static str` not meaningful for non-OAuth schemes (AppPassword self-issued) | tech-spec-material | open | Needs Tech Spec decision — `Option<&'static str>` or scheme-conditional trait |
| draft-f34 | WebSocket `/credentials/events` auth + cardinality + rate | sub-spec | pending-sub-spec | UX/realtime sub-spec |
| draft-f35 | Trigger ↔ credential integration (IMAP watcher, webhook HMAC) | sub-spec | pending-sub-spec | Separate trigger-credential design spec (compat sketch #1 in spike NOTES) |
| draft-f36 | Schema migration on encrypted rows (v1→v2 `State`) | sub-spec | pending-sub-spec | Schema migration spec (compat sketch #4 in spike NOTES) |

## Critique meta (process findings)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| critique-c1 | Spike budget 3–5 days under-estimate | process | decided | A1 — trimmed Q5/Q6; 6–9 day realistic budget per Strategy §Spike plan |
| critique-c3 | Proc-macro in/out of scope ambiguity | process | decided | OUT of scope; hand-expand macro output |
| critique-c4 | No loom / no bench / no hypotheses for iteration | process | decided | Criterion baseline + h1/h2/h3 benches required; loom deferred to Tech Spec §7 |
| critique-c5 | No inter-iteration checkpoint | process | decided | Operational trigger — commit + final message + stop-all-tool-calls per Strategy §Spike plan |
| critique-c7 | Q6 signing streaming ill-formed for mock | process | decided | Q6 dropped; signing streaming arises naturally in real HTTP impl |
| critique-c8 | GenericOAuth2 fallback vs Pattern 2 not compat | process | decided | Strategy §2.2 — GenericOAuth2 is Pattern 3 consumer, not impl of service traits |
| critique-c12 | Heartbeat/TTL discipline as documentation insufficient | tech-spec-material | decided | CI test + `debug_assert!` contracts |
| critique-c13 | `PatternSupport::Dynamic(Box<dyn Fn>)` bounds missing | implementation-phase | decided | Default bounds `Send + Sync + 'static` |
| critique-c14 | Binary success/failed — no partial criteria | process | decided | Strategy §Spike plan — partial criteria explicit (≥4 resolved + blocker statement on rest) |
| critique-c15 | `S1` path undefined | process | decided | Label removed everywhere; inline "accept current architecture, finish rollout cleanup only" |
| critique-c17 | `ExecutionCredentialRef` typed distinction vs prefix convention | tech-spec-material | decided | Typed newtype (enforced on type level), not prefix-only (not type-enforceable) |

## Lifecycle (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-lifecycle-creation | Creation strategies: interactive (OAuth2) / programmatic / imported / bootstrapped | tech-spec-material | locked-post-spike | Tech Spec §4 |
| user-lifecycle-update | Update / rotation: user-initiated / provider-initiated / scheduled / emergency | tech-spec-material | locked-post-spike | Tech Spec §4 + rotation orchestration (engine-owned per ADR-0030) |
| user-lifecycle-revocation | Revocation: soft (tombstone + grace) / hard (immediate) / cascade | tech-spec-material | locked-post-spike | Tech Spec §4 |
| user-lifecycle-deletion | Deletion: soft (tombstone + retention) / hard purge / cascading on workflow refs | tech-spec-material | locked-post-spike | Tech Spec §4 |
| user-lifecycle-expiration | TTL / auto-refresh vs mark-expired vs notify / grace period | tech-spec-material | locked-post-spike | Tech Spec §4 |
| user-lifecycle-migration | Schema migration v1→v2 without downtime | sub-spec | pending-sub-spec | Same as draft-f36 |
| user-lifecycle-import-export | Backup / transfer between instances / n8n-compat import | sub-spec | pending-sub-spec | Import/export sub-spec |

## Security (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-sec-encryption-at-rest | Algorithm / AAD / KDF | product-policy | policy-frozen | Preserve §12.5 bit-for-bit (Strategy §1.2 non-goal) |
| user-sec-key-rotation | Master key rotation / re-encryption flow / keyring with key_id | tech-spec-material | locked-post-spike | Tech Spec §5 + `rotate-master-key` walker CLI (implementation-phase) |
| user-sec-access-control | RBAC matrix for C/R/U/D/use | tech-spec-material | locked-post-spike | Tech Spec §5 |
| user-sec-scope-isolation | Tenant × workflow × user boundaries | tech-spec-material | decided | `ScopeLayer` in storage (Strategy §2.4) |
| user-sec-audit | What / where / retention / immutability / fail-closed vs fail-open | tech-spec-material | locked-post-spike | Tech Spec §5 + draft-f29 (degraded read-only) |
| user-sec-redaction | Logs / error messages / debug output / serialized dumps | tech-spec-material | locked-post-spike | Tech Spec §5 |
| user-sec-zeroization | Where plaintext lives, how long, how wipe guaranteed | product-policy | policy-frozen | Preserve per Strategy §1.2 non-goal |
| user-sec-egress | SSRF prevention / allowed endpoints / per-tenant egress policy | tech-spec-material | locked-post-spike | Tech Spec §5 + ProviderRegistry sub-spec |
| user-sec-session-binding | CSRF / PKCE / state param / cookie flags | tech-spec-material | locked-post-spike | Tech Spec §10 (all in `PendingStore` with encryption pipeline) |
| user-sec-compromise-response | Detection (anomaly, failed-auth spikes) / auto-revoke / quarantine | sub-spec | pending-sub-spec | Compromise response runbook sub-spec |

## Operational (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-op-caching | TTL / invalidation / per-replica vs shared / negative caching | tech-spec-material | locked-post-spike | Tech Spec §6 |
| user-op-retry | Retryable (network) vs non-retryable (4xx IdP) / backoff / budget | tech-spec-material | locked-post-spike | Tech Spec §6 |
| user-op-circuit-breaker | Per-credential / per-provider / per-endpoint | tech-spec-material | locked-post-spike | Tech Spec §6 via `nebula-resilience` |
| user-op-concurrency | Thundering herd / single-flight refresh / rate limit to IdP | tech-spec-material | decided | Existing `RefreshCoordinator`; IdP rate limit in Tech Spec |
| user-op-failure-modes | IdP down / network partition / audit down / cache down (per each: fail-open / fail-closed / degraded) | tech-spec-material | locked-post-spike | Tech Spec §6 — failure mode matrix |
| user-op-health-check | Credential-valid probe without side effects | tech-spec-material | decided | Engine background task per-credential cadence (from `CredentialMetadata`, default 1h) |
| user-op-observability | Metrics (cardinality control) / traces (span boundaries) / logs (structured) / events (fan-out) | tech-spec-material | locked-post-spike | Tech Spec §6 — per-layer observability contract |

## Testing (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-test-unit | Pure primitives (PKCE, HMAC, URL builders) — no HTTP | tech-spec-material | locked-post-spike | Tech Spec §7 |
| user-test-integration | Fake storage + mock OAuth2 server (wiremock) — end-to-end flow | tech-spec-material | locked-post-spike | Tech Spec §7 |
| user-test-contract | Real provider sandboxes (Google / GitHub / Slack test accounts) — periodic | tech-spec-material | locked-post-spike | Tech Spec §7 — `#[ignore]` + nightly |
| user-test-security | Fuzz (callback params) / property (crypto invariants) / miri (zeroize paths) | tech-spec-material | locked-post-spike | Tech Spec §7 |
| user-test-concurrency | loom for `RefreshCoordinator` L1+L2; stress for thundering herd | tech-spec-material | locked-post-spike | Tech Spec §7 |
| user-test-failure-injection | Chaos (storage fails / IdP timeouts / network splits) — verify fail-closed holds | tech-spec-material | locked-post-spike | Tech Spec §7 |
| user-test-upgrade | v1→v2 migration correctness / no data loss / rollback | tech-spec-material | locked-post-spike | Tech Spec §7 + user-lifecycle-migration |
| user-test-perf | CodSpeed baselines + regression (hot / cold / refresh paths) | tech-spec-material | locked-post-spike | Tech Spec §7 |
| user-test-determinism | `DeterministicClock` + deterministic PKCE/state generators behind trait | tech-spec-material | locked-post-spike | Tech Spec §7 |
| user-test-fixtures | Generated test credentials without real secrets / CI without leakage | tech-spec-material | locked-post-spike | Tech Spec §7 + pre-commit secret scanner |

## Evolution / interface (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-evo-versioning | Schema version / trait version / wire protocol version | tech-spec-material | locked-post-spike | Tech Spec §8 |
| user-evo-deprecation | Remove old credential types without breaking existing users | tech-spec-material | locked-post-spike | Tech Spec §8 — 2-version deprecation window |
| user-evo-compatibility | Semver policy — what counts as breaking change | tech-spec-material | decided | Any trait method / assoc type / invariant change = major (exemplified by Pattern 1→2 in Strategy §2.2) |
| user-evo-plugin-stability | Stable surface guaranteed for 3rd-party credential impls | tech-spec-material | locked-post-spike | Tech Spec §8 — `AnyCredential`, capability markers, `SchemeInjector` stable; rest internal |
| user-evo-feature-flag | Gradual rollout per new credential type behind cargo feature | tech-spec-material | locked-post-spike | Tech Spec §8 — 3-release-cycle promotion |

## Discovery / UX (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-disc-registration | How credential types register (compile-time vs runtime vs manifest) | tech-spec-material | decided | Explicit `register::<C>()` in plugin init (Strategy §2.1) |
| user-disc-metadata | UI descriptors / icons / help text / documentation links | tech-spec-material | locked-post-spike | Tech Spec §9 — `CredentialMetadata` with override layer (draft-f33) |
| user-disc-validation | Schema (shape) / semantic (test connection) / UX (form hints) | tech-spec-material | locked-post-spike | Tech Spec §9 — three-layer validation |
| user-disc-discovery | Action finds "credentials I can accept" — matching logic | tech-spec-material | locked-post-spike | Tech Spec §9 — capability requirement declaration + service-marker match |
| user-disc-binding | Action declares scope X needed → matches credential instance | tech-spec-material | decided | Strategy §2.3 + §3.3 — compile-time through capability sub-trait |

## Redirect / flow (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-flow-redirect-uri | Fixed per-instance / wildcarded / per-tenant / registration with IdP | tech-spec-material | locked-post-spike | Tech Spec §10 |
| user-flow-state-mgmt | In-flight OAuth2 flow storage / TTL / cleanup | tech-spec-material | decided | `PendingStore` (existing) + GC sweep; TTL 10min, single-use transactional |
| user-flow-multi-step | State machine for N-step credentials (Salesforce JWT) | sub-spec | pending-sub-spec | Same as draft-f22 |
| user-flow-interactive-vs | OAuth2 browser requirement vs headless (CI/CD, desktop, SSH) | tech-spec-material | locked-post-spike | Tech Spec §10 — device code flow where supported; else operator pre-provision |
| user-flow-callback | Success / user-denied / IdP-error / timeout / idempotency | tech-spec-material | locked-post-spike | Tech Spec §10 — per-path handling + idempotent replay protection |
| user-flow-deep-link | Tauri desktop custom URI scheme for browser callback | tech-spec-material | locked-post-spike | Tech Spec §10 + desktop mode §11 |

## Multi-mode deployment (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-mode-desktop | SQLite / OS keychain / no network exposure | tech-spec-material | locked-post-spike | Tech Spec §11 |
| user-mode-selfhosted | Postgres / env-based keys / Vault optional / operator-managed rotation | tech-spec-material | locked-post-spike | Tech Spec §11 |
| user-mode-cloud | Multi-tenant / KMS-managed keys / per-tenant isolation / billing/metering | tech-spec-material | locked-post-spike | Tech Spec §11 |
| user-mode-conditional | What's cloud-only (KMS), desktop-only (OS keychain), shared abstractions | tech-spec-material | locked-post-spike | Tech Spec §11 — feature matrix per mode |

## Integration (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-int-external-secret | Vault / AWS SM / GCP SM / Azure KV — delegation / caching / fallback | tech-spec-material | locked-post-spike | `ExternalProvider` impls in `nebula-storage/src/external_providers/` per Strategy §2.4 |
| user-int-hsm-kms | Envelope encryption / signing via HSM without raw key | tech-spec-material | locked-post-spike | Tech Spec §12 |
| user-int-oidc-sso | Federation / external identity → internal user | product-policy | out-of-scope | Plane A per ADR-0033 — not credential scope |
| user-int-plugin-sandbox | Execution model (in-process / process-isolated / WASM) | product-policy | pending-sub-spec | Separate execution-model ADR (referenced by Strategy §2.1) |

## Data / state (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-data-backup | Encrypted backup preservation / restore / encryption-at-rest preservation | tech-spec-material | locked-post-spike | Tech Spec §13 |
| user-data-dr | Key loss (recoverable?) / storage loss (blast radius) / point-in-time recovery | tech-spec-material | locked-post-spike | Tech Spec §13 |
| user-data-retention | Expired / revoked / audit / pending TTL policies | tech-spec-material | locked-post-spike | Tech Spec §13 — defaults 30d expired, 90d revoked, 1y audit, 10min pending |
| user-data-gdpr | Delete-me / export-me / lawful basis for storage | product-policy | pending-sub-spec | GDPR compliance sub-spec (hard purge with audit stub + encrypted export tarball) |

## Meta (user-list)

| ID | Concern | Label | Status | Resolution |
|---|---|---|---|---|
| user-meta-threat-model | `docs/threat-model/credential.md` / cadence / ownership | sub-spec | pending-sub-spec | Threat model doc — security-lead owned, quarterly review |
| user-meta-compliance | SOC 2 / ISO 27001 / HIPAA mapping | product-policy | pending-sub-spec | Compliance mapping — cloud mode primary, self-hosted/desktop carve-outs |
| user-meta-documentation | ADR index / HLD / runbooks | implementation-phase | in-implementation | Ongoing — each landed piece updates PRODUCT_CANON §15 table |
| user-meta-incident-response | Credential leak / key compromise / IdP outage runbooks | sub-spec | pending-sub-spec | 3 runbook sub-specs |
| user-meta-change-management | How credential API changes pass review / deprecation timeline | tech-spec-material | locked-post-spike | Tech Spec §14 — any `Credential` trait change via ADR + deprecation gate + semver CI |

---

## Maintenance

- **New concern surfaces** → add row with `open` status, triage to one of 6 labels within 2 working days.
- **Sub-spec lands** → flip `pending-sub-spec` or `proposed` → `in-implementation` or `decided` + update Resolution pointer to the sub-spec path.
- **Tech Spec lands** → flip `locked-post-spike` rows to `decided` with Tech Spec § pointer.
- **Strategy frozen rows** stay frozen; supersede requires ADR.
- **Product-policy rows** updated only when the product decision itself changes (via product ADR); independent of engineering cadence.
- **Label / status counts audited** at every register revision — totals table rebuilt when rows are added, removed, or relabeled. Mismatched counts are a register bug.

## Current totals (audited 2026-04-24 — after ADR-0035 landing)

| Label | Count | Notes |
|---|---|---|
| strategy-blocking | 12 | All resolved in Strategy §2/§3 or locked-post-spike |
| tech-spec-material | 82 | Most `locked-post-spike`; unlock with Tech Spec |
| sub-spec | 16 | Each row has a landing-order entry in Strategy §4.3 |
| implementation-phase | 4 | Routine execution tasks |
| product-policy | 7 | Frozen or awaiting product-level decision |
| process | 8 | Findings about the redesign workstream itself |
| **Total** | **129** | Counts audited at each register revision |

Totals rebuilt on every register revision — see maintenance rules below.
