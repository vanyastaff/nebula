# Architecture Specifications — Session 2026-04-15

> **Status:** DRAFT implementation specs
> **Authority:** Subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent document:** [`../2026-04-15-architecture-review-qa.md`](../2026-04-15-architecture-review-qa.md) — one-page summary of all 17 decisions
> **Scope:** Per-decision detailed implementation specifications suitable for handing to developer with zero context

## How to read these specs

Each spec describes **one architectural decision** with enough detail to implement it without re-deriving the design. The parent review document summarises *what was decided*; these specs answer *how to build it*.

## Template

Each spec follows this structure (sections may be omitted when not applicable):

1. **Status + cross-refs** — current state, canon target sections, dependencies on other specs
2. **Problem** — what are we solving and why
3. **Decision** — one-paragraph summary (copied from parent review for context)
4. **Data model** — SQL schemas, Rust types, traits, enums with full signatures
5. **Flows** — sequence descriptions for critical paths (text, not images)
6. **Edge cases** — failure modes, races, security considerations
7. **Configuration surface** — env vars, config fields, defaults, tunability
8. **Testing criteria** — what tests prove this works
9. **Performance targets** — latency / throughput / memory bounds
10. **Module boundaries** — which crate owns what
11. **Migration path** — for changes to existing code
12. **Open questions** — deliberate deferrals

## Index

| # | Spec | Topic | Status |
|---|---|---|---|
| 01 | [Product positioning](01-product-positioning.md) | OSS + cloud, target user, scope boundaries | draft |
| 02 | [Tenancy model](02-tenancy-model.md) | Org → Workspace hierarchy, credential sharing | draft |
| 03 | [Identity & auth](03-identity-auth.md) | `nebula-auth` crate, signup/login, MFA, SSO | draft |
| 04 | [RBAC & sharing](04-rbac-sharing.md) | Roles, transitive permissions, service accounts | draft |
| 05 | [API routing](05-api-routing.md) | Path-based routing, slugs, versioning | draft |
| 06 | [ID format](06-id-format.md) | Prefixed ULID, `domain_key` extension | draft |
| 07 | [Slug contract](07-slug-contract.md) | Rules, reserved words, rename, history | draft |
| 08 | [Cancellation cascade](08-cancellation-cascade.md) | Hierarchical tokens, grace waterfall, escalation | draft |
| 09 | [Retry cascade](09-retry-cascade.md) | Four-layer retry, Classify integration, persistence | draft |
| 10 | [Timeouts & quotas](10-timeouts-quotas.md) | Four-concept taxonomy, enforcement points | draft |
| 11 | [Triggers](11-triggers.md) | Cron, webhook, event, polling, dedup | draft |
| 12 | [Expression language](12-expression-language.md) | CEL-based, sandbox, eval context | draft |
| 13 | [Workflow versioning](13-workflow-versioning.md) | Draft/Published, pinned executions, schema migration | draft |
| 14 | [Stateful actions](14-stateful-actions.md) | Buffer, flush policy, suspend/resume, idempotency | draft |
| 15 | [Delivery semantics](15-delivery-semantics.md) | Four guarantees, marketing language rules | draft |
| 16 | [Storage schema](16-storage-schema.md) | Full SQL definitions, indexes, constraints | draft |
| 17 | [Multi-process coordination](17-multi-process-coordination.md) | Leaderless peers, unified claim query | draft |
| 18 | [Observability stack](18-observability-stack.md) | OTel logs/metrics/traces, audit log, real-time UI | draft |
| 19 | [Error taxonomy](19-error-taxonomy.md) | Per-layer error types, `Classify`, RFC 9457 mapping | draft |
| 20 | [Testing story](20-testing-story.md) | `nebula-testing` 3-tier harness, trigger testing, knife fixture | draft |
| 21 | [Schema crate](21-schema-crate.md) | `nebula-schema` (replaces `nebula-parameter`), Pattern 4, 18 field types, per-type widgets, unified `Rule` | draft |
| 22 | [Credential system v3](22-credential-system.md) | Three-crate split, envelope encryption, external providers, dynamic secrets, OIDC federation, tamper-evident audit, CredentialId newtype | draft |
| 23 | [Cross-crate foundation](23-cross-crate-foundation.md) | `Context` + `HasX` capability traits, `Guard` + `CredentialGuard`/`ResourceGuard`, `Dependencies` + `#[uses_*]` attributes, `ScopeLevel` + `Scope` in `nebula-core` | draft |
| 24 | [`nebula-core` redesign](24-nebula-core-redesign.md) | Cleanup dead code, UUID→ULID migration, `NodeId`→`NodeKey`, `SecretString`→`secrecy`, spec 23 integration, naming convention, 5-PR sequence | draft |
| 25 | [`nebula-resource` redesign](25-nebula-resource-redesign.md) | `ResourceHandle`→`ResourceGuard`, `Ctx`→`ResourceContext`, credential rotation via `ReloadOutcome`, `HasResourcesExt` typed access, 4-PR sequence | draft |
| 26 | [`nebula-credential` redesign](26-nebula-credential-redesign.md) | CredentialAccessor→core, CredentialContext→BaseContext, `SecretString`→`secrecy`, delete retry.rs, `HasCredentialsExt`, three-crate split boundaries, 5-PR sequence | draft |
| 27 | [`nebula-action` redesign](27-nebula-action-redesign.md) | Context→core, ActionContext/TriggerContext structs→traits, `ActionDependencies`→`DeclaresDependencies`, remove anyhow+async_trait+AgentHandler, Rust 1.94 `Pin<Box<dyn Future>>` + `#[diagnostic::on_unimplemented]`, 4-PR sequence | draft |
| 28 | [`nebula-engine` redesign](28-nebula-engine-redesign.md) | Absorb `nebula-runtime`, port-driven routing (delete EdgeCondition), crash recovery (type-aware + idempotency counter), expression resolution by engine, `nebula-eventbus` events, generic TriggerManager, spec 23 contexts, 4-PR sequence | draft |

## Reading order recommendations

**If you are about to implement foundation layer:** 06 → 07 → 16 → 02 → 04
(IDs → slugs → storage → tenancy → RBAC — these form the data model base)

**If you are working on execution path:** 08 → 09 → 14 → 17 → 11
(Cancel → retry → stateful → coordination → triggers)

**If you are working on API surface:** 05 → 03 → 04 → 10 → 15
(Routing → auth → RBAC → quotas → delivery semantics)

**If you are writing an integration (action author):** 09 → 12 → 14 → 11
(Retry policy → expressions → stateful pattern → trigger types)

## Relationship to canon

These specs are **input for canon updates**, not canon themselves. Target canon sections are listed in each spec's header. A spec being "green" does not mean canon is updated — canon fold-in requires deliberate PR per section.

If a spec and canon disagree, **canon wins** until the spec is promoted through a canon update PR.

## Naming corrections (2026-04-15, post Q18)

After reviewing `nebula-core::ScopeLevel` during Q18, the following naming decisions were made. **Specs 02, 04, 05, 06, 09, 14, 16, 17 use terms that need consolidation**; this list is authoritative until specs are updated inline.

| Concept | Use this term | Prefix | Do NOT use |
|---|---|---|---|
| Tenant collaborative space (mid-level between org and workflow) | **`Workspace` / `WorkspaceId`** | `ws_` | `Project` / `ProjectId` |
| Row in `execution_nodes` representing one attempt | **`AttemptId`** | `att_` | `NodeAttemptId`, `NodeId` |
| Logical reference to workflow graph node (stable across attempts) | **`NodeId`** (kept from `nebula-core`) | string-backed, no ULID prefix | — |
| OS process / Nebula binary instance / K8s pod | **`InstanceId`** | `nbl_` | `NodeId` (was confusing), `WorkerId` |

**Migration note:** `nebula-core` has `ProjectId` and `ScopeLevel::Project` today. These must be renamed to `WorkspaceId` and `ScopeLevel::Workspace` as part of a single refactor PR. Cascade affects every spec that uses tenant terminology.

**Observability note (Q18):** `ScopeLevel` remains the authority for **resource lifecycle** (pool management, cleanup timing). A separate `ObservabilityContext` (spec 18) reuses the **same underlying IDs** as tracing/logging attributes, but is a parallel surface — not derived from `ScopeLevel` automatically.

## Changelog

- **2026-04-15** — initial draft of all 17 specs from Q&A session; Q18 added spec 18 + naming corrections
- **2026-04-15** — added spec 21 (`nebula-schema` crate, replaces `nebula-parameter`) after deep-dive Q&A on parameter system architecture
- **2026-04-15** — added spec 22 (credential system v3) after research into HashiCorp Vault, AWS KMS, GCP/Azure Key Vault, SPIFFE/SPIRE, Infisical, Doppler, and Rust ecosystem (secrecy, zeroize, keyring, vaultrs, kms-aead)
- **2026-04-15** — added spec 23 (cross-crate foundation) after multi-round expert Q&A resolving Context/Guard/Dependencies/Scope across nebula-core, nebula-resource, nebula-credential, nebula-action — prerequisite for per-crate redesign
- **2026-04-15** — added spec 24 (`nebula-core` redesign) after full SRP audit of all 23 specs + grep usage analysis: cleanup dead code (6 files, 15 dead types), UUID→ULID migration via `domain-key` v0.5, `NodeId`→`NodeKey` rename, `SecretString`→`secrecy`, naming convention, 5-PR migration sequence
- **2026-04-15** — added spec 25 (`nebula-resource` redesign) after deep study of 38 source files + 13 plan/doc files + 4 prototypes (Postgres, Google Sheets, Telegram, SSH): credential rotation via ReloadOutcome, HasResourcesExt typed access, ResourceContext replaces Ctx
- **2026-04-15** — added spec 26 (`nebula-credential` redesign): CredentialAccessor unified in core, CredentialResolverRef→HasCredentials, SecretString→secrecy, three-crate split boundaries, 5-PR sequence
- **2026-04-15** — added spec 27 (`nebula-action` redesign): Context name collision resolved, ActionContext/TriggerContext structs→umbrella traits, remove anyhow+async_trait+AgentHandler, Rust 1.94 features, AwaitAction universal suspend/resume, ToolProvider on Resource
- **2026-04-15** — added spec 28 (`nebula-engine` redesign): absorb nebula-runtime, port-driven routing (delete EdgeCondition), type-aware crash recovery, engine-managed idempotency, expression resolution, nebula-eventbus, generic TriggerManager
