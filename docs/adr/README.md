# Architecture Decision Records (in-repo)

Accepted ADRs for the **M6 resource finalization** and **M11 dependency redesign**
cascade and later work. Numbering starts at **0042** in this directory.

**Agents:** Read [docs/README.md](../README.md) first. Use **thematic groups** (below) before opening individual files.


---

## Thematic index (agents start here)

| Theme | Primary ADRs | Typical question |
|-------|----------------|------------------|
| **M6 integration binding** | **0081** (+ stubs 0042-0045, 0051, 0066-0067) | Slots, resources, rotation |
| **Schema & validation** | **0080** (+ stubs 0052, 0058-0064) | Forms, JSON Schema |
| **Storage (spec-16)** | 0072 | Port, adapter, tenancy |
| **Action surface & retry** | 0069, 0053, 0068 | Action traits, retry |
| **SDK & capabilities** | 0054, 0055 | nebula-sdk |
| **Workflow graph** | 0056 | DAG validation |
| **Visual canvas** | 0065 | Supply edges |
| **API & webhooks** | **0082** (+ stubs 0047-0049) | OpenAPI, webhooks |
| **Observability** | 0046, 0050 | Metrics, traces |
| **AI (deferred)** | 0057 proposed | STRATEGY.md |
| **Agent harness** | **0083** | Intent / structural-budget / honesty gate |

## Historical ADRs (0001–0041)

Index only: [`HISTORICAL.md`](./HISTORICAL.md). Full text at `docs/adr/NNNN-*.md`
(excluded from agent index — see `.cursorignore`). Do not bulk-read all 41 files.

## Contract ADRs (0080–0082)

| # | Title | Status |
|---|-------|--------|
| [0080](./0080-schema-validation-platform.md) | Schema & validation platform | accepted |
| [0081](./0081-m6-resource-credential-integration.md) | M6 resource & credential integration | accepted |
| [0082](./0082-api-webhooks-idempotency.md) | API edge — OpenAPI, idempotency, webhooks | accepted |

## Index (0042–0072, stubs + standalone)

| # | Title | Status |
|---|-------|--------|
| [0042](./0042-node-binding-mechanism.md) | Node → ResourceId / CredentialId binding mechanism | superseded → contract |
| [0043](./0043-dependency-declaration-dx.md) | Dependency declaration DX (slots + `FromWorkflowNode`) | superseded → contract |
| [0044](./0044-supersede-0036-resource-credential-singular.md) | Supersede ADR-0036 — slot credentials on resources | superseded → contract |
| [0045](./0045-eventtrigger-scope-deferral.md) | EventTrigger DX-wrapper deferral | superseded → contract |
| [0046](./0046-metrics-telemetry-boundary.md) | Merge telemetry into `nebula-metrics` | accepted |
| [0047](./0047-openapi-31-generator.md) | OpenAPI 3.1 generation (`utoipa`) | superseded → contract |
| [0048](./0048-idempotency-store-backend.md) | Idempotency store — hybrid L1 + PG | superseded → contract |
| [0049](./0049-webhook-handler-convergence.md) | Webhook handler convergence | superseded → contract |
| [0050](./0050-m3-5-w3c-trace-context-propagation.md) | W3C Trace Context propagation | accepted |
| [0051](./0051-external-provider-redesign.md) | External provider redesign | superseded → contract |
| [0052](./0052-schema-validator-condition-seam.md) | Validator condition evaluation seam | superseded → contract |
| [0069](./0069-action-surface-hybrid.md) | Action surface hybrid (**suffix** — shares cascade era with 0052) | accepted |
| [0053](./0053-two-struct-dx-consolidation.md) | Two-struct DX consolidation | accepted |
| [0054](./0054-typed-capability-system.md) | Typed capability system | accepted |
| [0055](./0055-nebula-sdk-facade.md) | `nebula-sdk` re-export façade | accepted |
| [0056](./0056-type-safe-dag.md) | Type-safe DAG | accepted |
| [0057](./0057-ai-agent-sdk.md) | AI agent SDK (`nebula-agent` direction) | proposed |
| [0058](./0058-schema-field-vocabulary.md) | Schema field vocabulary | superseded → contract |
| [0059](./0059-cross-foundation-dependency-graph.md) | Cross-foundation dependency graph | superseded → contract |
| [0060](./0060-symmetric-foundation-api.md) | Symmetric foundation API | superseded → contract |
| [0061](./0061-nebula-schema-core-ratification.md) | Schema core ratification | superseded → contract |
| [0062](./0062-nebula-schema-stdlib-newtype-zoo.md) | Schema stdlib newtype zoo | superseded → contract |
| [0063](./0063-json-schema-2020-12-interop.md) | JSON Schema 2020-12 interop | superseded → contract |
| [0064](./0064-ui-form-composition.md) | UI form composition | superseded → contract |
| [0065](./0065-visual-rendering-modes.md) | Visual rendering modes | accepted |
| [0066](./0066-credential-runtime-crate.md) | Credential management runtime crate (`nebula-credential-runtime`) | superseded → contract |
| [0067](./0067-engine-owned-rotation-fanout-self-refresh-hook.md) | Engine-owned per-slot rotation fan-out + `&self` refresh hook | superseded → contract |
| [0068](./0068-layered-retry.md) | Layered retry (action-internal vs node-level) | accepted |
| [0072](./0072-nebula-storage-spec16-port-adapter-tenancy.md) | `nebula-storage` spec-16 port / adapter / tenancy | accepted |

## Supersession

| Superseded | Supersedes | Note |
|------------|------------|------|
| [0036](./0036-resource-credential-adoption-auth-retirement.md) | [0044](./0044-supersede-0036-resource-credential-singular.md) | Singular `Resource::Credential` → typed credential slot fields. |
| Canon `[L1-§3.10]` (`crates/telemetry/README.md`) | [0046](./0046-metrics-telemetry-boundary.md) | Telemetry merged into `nebula-metrics`. |
| 0044 — hook signature + slot-field/migration shape only | [0067](./0067-engine-owned-rotation-fanout-self-refresh-hook.md) | `on_credential_refresh` → `&self` + `SlotCell`; rotation stays engine-owned (ADR-0030). |
| ADR-0030 facade slice | [0066](./0066-credential-runtime-crate.md) | Management facade → `nebula-credential-runtime`; engine keeps resolver/refresh mechanism. |
| "Sprint E — adopt spec-16 row model" deferral | [0072](./0072-nebula-storage-spec16-port-adapter-tenancy.md) | Spec-16 is no longer deferred: it is the shipped port + adapter + tenancy architecture for storage. |

## Supersession (Wave B contract merges)

| Contract | Absorbs (stubs) |
|----------|-----------------|
| [0080](./0080-schema-validation-platform.md) | 0052, 0058-0064 |
| [0081](./0081-m6-resource-credential-integration.md) | 0042-0045, 0051, 0066-0067 |
| [0082](./0082-api-webhooks-idempotency.md) | 0047-0049 |

## Related plans

- `docs/plans/2026-05-18-001-refactor-docs-stack-contract-consolidation-plan.md` — active (Wave B).
- `docs/plans/2026-05-18-002-refactor-code-doc-citation-cleanup-plan.md` — Wave C (Rust comments).
- `docs/plans/2026-05-17-001-feat-integrator-flagship-platform-plan.md` — implementation (doc gate cleared).