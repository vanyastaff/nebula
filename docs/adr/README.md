# Architecture Decision Records (in-repo)

Accepted ADRs for the **M6 resource finalization** and **M11 dependency redesign**
cascade and later work. Numbering starts at **0042** in this directory.

**Agents:** Read [`docs/README.md`](../README.md) for the full documentation map.

## Historical ADRs (0001–0041)

Index only: [`HISTORICAL.md`](./HISTORICAL.md). Full text at `docs/adr/NNNN-*.md`
(excluded from agent index — see `.cursorignore`). Do not bulk-read all 41 files.

## Index (0042–0066)

| # | Title | Status |
|---|-------|--------|
| [0042](./0042-node-binding-mechanism.md) | Node → ResourceId / CredentialId binding mechanism | accepted |
| [0043](./0043-dependency-declaration-dx.md) | Dependency declaration DX (slots + `FromWorkflowNode`) | accepted |
| [0044](./0044-supersede-0036-resource-credential-singular.md) | Supersede ADR-0036 — slot credentials on resources | accepted |
| [0045](./0045-eventtrigger-scope-deferral.md) | EventTrigger DX-wrapper deferral | accepted |
| [0046](./0046-metrics-telemetry-boundary.md) | Merge telemetry into `nebula-metrics` | accepted |
| [0047](./0047-openapi-31-generator.md) | OpenAPI 3.1 generation (`utoipa`) | accepted |
| [0048](./0048-idempotency-store-backend.md) | Idempotency store — hybrid L1 + PG | accepted |
| [0049](./0049-webhook-handler-convergence.md) | Webhook handler convergence | accepted |
| [0050](./0050-m3-5-w3c-trace-context-propagation.md) | W3C Trace Context propagation | accepted |
| [0051](./0051-external-provider-redesign.md) | External provider redesign | accepted |
| [0052](./0052-schema-validator-condition-seam.md) | Validator condition evaluation seam | accepted |
| [0052-action](./0052-action-surface-hybrid.md) | Action surface hybrid | accepted |
| [0053](./0053-two-struct-dx-consolidation.md) | Two-struct DX consolidation | accepted |
| [0054](./0054-typed-capability-system.md) | Typed capability system | accepted |
| [0055](./0055-nebula-sdk-facade.md) | `nebula-sdk` re-export façade | accepted |
| [0056](./0056-type-safe-dag.md) | Type-safe DAG | accepted |
| [0057](./0057-ai-agent-sdk.md) | AI agent SDK (`nebula-agent` direction) | proposed |
| [0058](./0058-schema-field-vocabulary.md) | Schema field vocabulary | accepted |
| [0059](./0059-cross-foundation-dependency-graph.md) | Cross-foundation dependency graph | accepted |
| [0060](./0060-symmetric-foundation-api.md) | Symmetric foundation API | accepted |
| [0061](./0061-nebula-schema-core-ratification.md) | Schema core ratification | accepted |
| [0062](./0062-nebula-schema-stdlib-newtype-zoo.md) | Schema stdlib newtype zoo | accepted |
| [0063](./0063-json-schema-2020-12-interop.md) | JSON Schema 2020-12 interop | accepted |
| [0064](./0064-ui-form-composition.md) | UI form composition | accepted |
| [0065](./0065-visual-rendering-modes.md) | Visual rendering modes | accepted |
| [0066](./0066-layered-retry.md) | Layered retry (action-internal vs node-level) | accepted |

## Supersession

| Superseded | Supersedes | Note |
|------------|------------|------|
| [0036](./0036-resource-credential-adoption-auth-retirement.md) | [0044](./0044-supersede-0036-resource-credential-singular.md) | Singular `Resource::Credential` → typed credential slot fields. |
| Canon `[L1-§3.10]` (`crates/telemetry/README.md`) | [0046](./0046-metrics-telemetry-boundary.md) | Telemetry merged into `nebula-metrics`. |

## Related plans

- `docs/plans/2026-05-17-002-refactor-doc-consolidation-plan.md` — doc stack hygiene (this repo).
- `docs/plans/2026-05-17-001-feat-integrator-flagship-platform-plan.md` — implementation (blocked on doc gate).
- `.ai-factory/plans/m6-resource-finalization-integration-audit.md` — M6 closure context.
