# Known gaps — 37 findings triage

**Статус:** honest. Every finding from user review (2026-04-24) mapped to severity + resolution path.

Finding nums mapping back to user's detailed audit message. NONE ignored. Some marked "OPEN" — those need prototype или separate spec.

## Severity legend

- **🔴 BROKEN** — design не работает в предложенной форме; требует rethink перед spec
- **🟡 RESOLVABLE** — ясен direction, но требует explicit design (не ad hoc)
- **🔵 NEW DIMENSION** — concern не рассмотрен в моём дизайне, нужен separate analysis
- **🟢 DETAIL** — fixable в implementation phase

## Type system (findings #1-4, #32)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 1 | `ctx.credential::<C>()` ambiguity при двух слотах одного типа | 🔴 | Dual API: `ctx.credential_at(&self.slack)` default (explicit field ref); `ctx.credential::<C>()` только для uniquely-bound slots (opt-in through derive attribute) | Open — design decision pending prototype |
| 2 | `ctx.credential::<dyn AcceptsBearer>()` конфликт с сигнатурой (AcceptsBearer — не Credential) | 🔴 | Разделить two APIs: `ctx.credential::<C: Credential>()` для credential type; `ctx.scheme::<S: SchemeInjector>()` для capability-only binding. Признать что это **два concept'а** с похожим именем. | Direction clear; syntax TBD |
| 3 | `dyn Credential` / `dyn BitbucketCredential` не dyn-safe (4 assoc types) | 🔴 | Keep existing `AnyCredential` object-safe supertrait pattern. Service traits (BitbucketCredential) projects unique `type Scheme = ConcreteBearerType` (not `Scheme: SomeBound`). Prototype needed to confirm projections работают for specific case | Open — prototype required |
| 4 | `CredentialRef<C>` runtime shape — PhantomData only | 🟡 | Runtime shape = `CredentialKey + PhantomData<fn() -> C>`. Type parameter enforces compile-time; runtime resolve ищет в CredentialRegistry TypeId → supported capabilities. Registry — new infra (populated при credential registration). | Known, needs design |
| 32 | `CredentialGuard<C::Scheme>` projection через dyn service trait | 🔴 | For `C = dyn SlackCredential`, `C::Scheme` requires `type Scheme = ConcreteType` in trait decl, not `Scheme: Bound`. Prototype must validate this form works. | Open — prototype required |

**Aggregate:** type system требует prototype spike до writing spec. Four concerns interlock; paper design can't resolve.

## Pattern default (findings #5-7)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 5 | Pattern 2 (service trait для multi-auth) должен быть DEFAULT, not Pattern 1 | 🔴 | Revise entire documentation + macro defaults. Pattern 2 default; Pattern 1 — opt-in для single-auth services; Pattern 3 (generic `dyn AcceptsBearer`) — для service-agnostic utilities | Acknowledged in 01-type-system-draft |
| 6 | Wrapper type (SlackOAuth2Credential) vs ProviderRegistry duplicate info | 🟡 | Service trait declares `const PROVIDER_ID: &'static str`. Activation validates `ProviderRegistry::get(Self::PROVIDER_ID)` exists. Scopes/defaults — registry wins runtime; wrapper provides UI-level defaults (display-only). If operator removes "slack" from registry — existing credentials fail loudly. | Direction clear; concrete spec needed |
| 7 | Generic OAuth2Api fallback (n8n's catch-all) не вписывается в "per-service types" | 🟡 | Add `GenericOAuth2Credential` как catch-all type. ProviderId specified via credential config (user-provided). Service trait model для known services; generic для everything else. Three categories: known-service concrete + generic-runtime-configured + service-trait-multi-auth | Direction clear |

## Sealed vs plugin (findings #8-9)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 8 | `Credential: sealed::Sealed` запрещает plugin impls; цель — 400+ plugin credentials; contradiction | 🔴 | Revise sealing approach. Options: (a) remove sealing, enforce invariants via contract tests + CI. (b) two-tier sealing: `Credential` sealed, but subtrait `PluginCredential: Credential` opens via macro `#[plugin_credential]` emitting sealed-compatible boilerplate. (c) replace sealing с `#[non_exhaustive]` patterns. | Decision needed — not rushed |
| 9 | Capability markers в `nebula-credential` tied resource writers к credential crate even if they're credential-agnostic | 🟡 | Accept. Capability markers (AcceptsBearer, AcceptsSigning) — cross-cutting abstraction same crate as scheme injection trait. Resource writers depend on credential crate = expected for credential-aware resources. Alternative (move markers to core) leaks injection concerns to core. | Accept trade-off |

## SchemeInjector specifics (findings #10-14)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 10 | `Send + Sync + ZeroizeOnDrop` bounds not shown | 🟢 | Add explicit bounds: `trait SchemeInjector: nebula_core::AuthScheme + Send + Sync + ZeroizeOnDrop`. AuthScheme already `Send + Sync`, inherits — `ZeroizeOnDrop` explicit | Trivial fix |
| 11 | AWS SigV4 signing over streaming body | 🟡 | `SigningContext::body_hash: Option<Vec<u8>>` — pre-computed by caller. Для streaming: UNSIGNED-PAYLOAD option. Resource decides buffered vs streaming при construction. Signer impl accepts pre-computed hash or UNSIGNED sentinel. Document explicitly which signers require buffered bodies | Direction clear; needs careful doc |
| 12 | `inject` default Err + `sign` default fallback to inject — концептуально путает | 🟡 | Split capability markers: `AcceptsStaticInjection` (implements inject with default sign fallback) vs `AcceptsRequestSigning` (implements sign, inject returns NotApplicable). Mutually exclusive by default. Custom scheme может implement both. | Marker split resolves |
| 13 | `InjectError::NotApplicable` вне nebula-error Classify axis | 🟡 | Add new axis `Capability` to `Classify` trait. Errors: `Capability(WrongScheme / WrongInjection / NotSupported)`. Not retryable, not transient — programming error. Separately from runtime errors. | New axis в nebula-error |
| 14 | Multi-credential resource (mTLS + Bearer в одном HTTP client) | 🔴 | `Resource::Auth = DualAuth<A, B>` variant. Resource declares multiple auth requirements; engine resolves all; passes tuple. Prototype needed — variadic arity compile issues. | Open — prototype validation |

## RefreshCoordinator multi-process (findings #15-17)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 15 | `RefreshToken` newtype — in-process handle vs cross-process claim id mixed | 🟡 | Split: `InProcRefreshToken(u64)` for L1 coordinator; `DurableClaimToken(String)` for L2 storage-backed claim. Two-tier coordinator composes both; trait signature in core abstracts the detail. | Design clear |
| 16 | Heartbeat cadence + claim TTL + refresh duration discipline | 🟡 | Document: TTL = 30s, heartbeat = 10s (TTL/3), refresh timeout = 25s (< TTL - 1 heartbeat cycle). Mismatch → claim expires mid-refresh → race. CI test для heartbeat consistency. | Direction clear — needs parameters doc |
| 17 | Mid-refresh crash с rotated refresh_token (IdP invalidates old) | 🔴 | **OPEN**. Partial mitigations: pre-write sentinel (`refresh_in_flight=true`) + detect on reclaim → mark `ReauthRequired` instead of retry. Accept rare loss (<1/100K) as known limitation. Document runbook. | Open — needs separate spec |

## ProviderRegistry углы (findings #18-21)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 18 | Who seeds initial ProviderSpec entries | 🔵 | Three-mode strategy: (a) cloud → Anthropic-curated registry, updated out-of-band; (b) self-hosted → opinionated defaults bundled в engine release, operator may override via admin CLI; (c) desktop → bundled, non-editable. Separate "registry update" release cadence. | New dimension — separate spec |
| 19 | ProviderSpec update = breaking для existing credentials | 🔵 | ProviderSpec versioning. `version: u32`. Existing credentials carry `provider_spec_version: u32` in state. On resolve, check match; if provider.version > state.provider_spec_version → mark "ProviderUpdated" flag, surface to UI. Operator migrates via CLI (update credential's provider binding). | New dimension — needs design |
| 20 | Microsoft multi-tenant endpoints (`.../{tenant}/oauth2/...`) | 🔵 | URI template: `token_endpoint: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"` с `template_vars: {"tenant": TemplateVarSpec { validation: UUID_OR_COMMON, required: true }}`. User credential provides `tenant` binding; validation at activation. Registry stores template, not literal URL. | New dimension — parameterized URLs needed |
| 21 | Admin API security — operator compromise → SSRF via registry | 🟡 | Multi-layer defense: (a) admin API RBAC (strictest permission); (b) provider spec hash recorded в audit (any registry change tracked); (c) optional signed registry entries (future); (d) separate operator role for registry admin vs general operator; (e) cloud mode restricts registry mutability entirely (Anthropic-managed). For self-hosted: accepted trade-off — operator is trusted. | Direction clear |

## Multi-step flows (findings #22-23)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 22 | Accumulated state between multi-step flows не в PendingStore | 🔴 | **OPEN**. Options: (a) extend PendingStore с accumulator field (JSON). (b) separate MultiStepStore repo. (c) atomic-only multi-step (all steps in single `resolve()`, no persistence). Predлагаемый start — atomic-only; defer persistent multi-step до use case. | Open — narrow to atomic start |
| 23 | `continue_resolve()` signature для step N unclear | 🔴 | Tied to #22. If atomic-only, `continue_resolve` нужен только для OAuth2 callback (one-step continuation). If persistent multi-step added later, signature evolves. Current signature (for OAuth2 callback) works; expand when needed. | Deferred on #22 |

## Execution-scoped credentials (findings #24-25)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 24 | Ephemeral vs persisted credential namespace collision | 🟡 | `ExecutionCredentialStore` uses separate CredentialKey prefix `exec:{execution_id}:{name}`. Or `ExecutionCredentialRef<C>` newtype distinct from `CredentialRef<C>`. Type system distinguishes. Engine's resolver dispatches by ref type. | Design clear |
| 25 | Cancellation vs zeroize-on-drop — abort may skip Drops | 🟡 | `ExecutionCredentialStore::cleanup()` explicit method called at execution teardown (even on cancel). Engine's executor ensures call даже at abort path (via tokio::select + guaranteed finally block). Document as "zeroize-on-drop is best-effort; cleanup() is mandatory". Some plaintext may live до GC в abort case; surface as known limitation. | Direction clear — needs careful impl |

## Resource connection-bound variant (findings #26-27)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 26 | `on_credential_refresh(&mut self, ...)` on active pool + concurrent queries | 🟡 | Internal impl — `Arc<RwLock<PgPool>>`. Queries take read lock; refresh takes write lock + rebuild pool internally. Existing queries continue using old pool (inside read lock); new queries after refresh use new pool. Blue-green swap. Trait surface hides locking detail. | Direction clear — doc pattern |
| 27 | Refresh frequency для DB/Kafka — overbuilt abstraction | 🟢 | Accept. `on_credential_refresh` is optional trait method (default no-op). Used rarely (AWS IAM DB auth is real case). Cost — one unused method per Resource. Minor. | Accept |

## Storage layer deltas (findings #28-29)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 28 | SQLite vs Postgres schema parity + desktop mode NoOp repos | 🟡 | Migration scripts for each new table (`refresh_claims`, `rotation_leader_claims`, `provider_registry`) in both dialects, CI gate parity. Desktop mode — single replica, NoOpClaimRepo returns "immediate success" — trait impl dispatch. Documented pattern. | Direction clear |
| 29 | AuditLayer fail-closed vs degraded mode audit-writes | 🟡 | DegradedReadOnly mode — resolve for existing credentials continues; audit writes go to fallback sink (local file buffer, replayed when audit DB recovers). New credential ops blocked. Fallback sink — best-effort, bounded. Documented pattern. | Design clear |

## New dimensions (finding #35)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 35 | Trigger ↔ credential not addressed | 🔵 | Trigger trait analogue к Resource: `type Auth: SchemeInjector`. Trigger lifecycle: webhook signature verification uses `HmacSigningScheme` in verify mode; IMAP trigger — connection-bound credential. Separate analysis needed for activation flow, reconnect on refresh, leader election for trigger activation. | Open — separate spike |
| 34 | WebSocket /credentials/events — auth/cardinality/rate | 🔵 | Scoped per-user WebSocket: subscribe only к own credentials events. Tenant admin → tenant-wide. Rate limit per-connection. Reconnect → delivery of missed events with bounded buffer. Event retention — 1 hour recent buffer. | Open — UX spec needed |

## Details (findings #30-37 minus already covered)

| # | Finding | Severity | Direction | Status |
|---|---|---|---|---|
| 30 | `pattern_support` не покрывает Custom plugin patterns | 🟡 | Replace `&'static [AuthPattern]` с enum `PatternSupport { Static(&'static [...]), Dynamic(Box<dyn Fn(&AuthPattern) -> bool>) }`. Runtime extensibility для plugins с Custom variants. | Direction clear |
| 31 | `DeserializeFromProvider` format-coupled | 🟡 | Two-stage: provider returns `RawProviderOutput { bytes, metadata: HashMap }`. Each Scheme `S: TryFrom<&RawProviderOutput>`. Decoupled from provider format. Provider impl maps its native format to `RawProviderOutput`. | Design clear |
| 33 | `CredentialMetadata` statically hardcoded in per-service types — operator customization limits | 🟡 | Two-layer: `CredentialMetadata::defaults()` hardcoded; `CredentialMetadata::with_override(MetadataOverrides)` from ProviderRegistry or per-tenant config. Icon/description/help can be overridden. Operator customization via registry admin API. | Design clear |
| 34 | WebSocket events (already covered above in #35 row) | — | — | — |
| 36 | `credential_config` vs `credential_runtime` split + schema migration на encrypted rows | 🔵 | Schema migration for runtime (encrypted State shape v1→v2) — complex. Approach: versioned State struct (`#[credential_state(version=2, migrate_from=v1)]`). Lazy migration on resolve: decrypt v1 → migrate to v2 → re-encrypt. Or bulk migration CLI. Document migration discipline. | New dimension — migration design |
| 37 | `FieldSensitivity::Identifier` vs `Public` — distinction not meaningful | 🟢 | Collapse to two: `Public` / `Secret`. Identifier concern — UI display hint, belongs in `FieldUi` metadata. | Simplification accept |

## Rollup

- 🔴 **BROKEN (8 findings):** #1, #2, #3, #5, #8, #14, #17, #22, #32
- 🟡 **RESOLVABLE with explicit design (16 findings):** #4, #6, #7, #9, #11, #12, #13, #15, #16, #21, #24, #25, #26, #28, #29, #30, #31, #33 — all have clear direction, need documentation
- 🔵 **NEW DIMENSION (5 findings):** #18, #19, #20, #34, #35, #36 — require separate analysis/spec
- 🟢 **DETAIL (4 findings):** #10, #27, #37 — trivial

## Next actions implied

1. **Prototype spike** (см. `06-prototype-plan.md`) — validates 🔴 findings #1, #2, #3, #32 (type system shape), #14 (multi-credential resource)
2. **Separate spec on mid-refresh race** (#17) — 🔴, storage + engine concerns, needs explicit decision
3. **Separate spec on multi-step flows** (#22, #23) — 🔴, narrow to atomic-only start
4. **Sealed + plugin decision** (#8) — 🔴, design decision, may come from prototype
5. **Pattern 2 as default** (#5) — 🔴, documentation + macro default change
6. **Trigger integration spike** (#35) — 🔵, separate analysis
7. **Provider registry design** (#18-20) — 🔵, separate spec
8. **Schema migration on encrypted rows** (#36) — 🔵, separate design
