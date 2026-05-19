# Architecture Decision Records (in-repo)

Accepted ADRs for the **M6 resource finalization** and **M11 dependency redesign**
cascade and later work. Numbering starts at **0042** in this directory.

**Agents:** Read [docs/README.md](../README.md) first. Use **thematic groups** (below) before opening individual files.


---

## Thematic index (agents start here)

| Theme | Primary ADRs | Typical question |
|-------|----------------|------------------|
| **M6 integration binding** | **0081** (absorbs ADR-0042–0045, 0051, 0066–0067) | Slots, resources, rotation |
| **Schema & validation** | **0080** (absorbs ADR-0052, 0058–0064) | Forms, JSON Schema |
| **Storage (spec-16)** | 0072 | Port, adapter, tenancy |
| **Action surface & retry** | 0069, 0053, 0068 | Action traits, retry |
| **SDK & capabilities** | 0054, 0055 | nebula-sdk |
| **Workflow graph** | 0056 | DAG validation |
| **Visual canvas** | 0065 | Supply edges |
| **API & webhooks** | **0082** (absorbs ADR-0047–0049) | OpenAPI, webhooks |
| **Observability** | 0046, 0050 | Metrics, traces |
| **AI (deferred)** | 0057 proposed | STRATEGY.md |

## Historical ADRs (0001–0041)

Index only: [`HISTORICAL.md`](./HISTORICAL.md) (title + status per id). Full
decision text is **git-history-only** — recover with
`git log -- docs/adr/<file>` then `git show <rev>:docs/adr/<file>`, or the external archive named in
[`../ARCHIVE.md`](../ARCHIVE.md). The per-file bodies are no longer in the
working tree (evicted 2026-05-18). Do not expect `docs/adr/00NN-*.md` to exist.

## Contract ADRs (0080–0082)

| # | Title | Status |
|---|-------|--------|
| [0080](./0080-schema-validation-platform.md) | Schema & validation platform | accepted |
| [0081](./0081-m6-resource-credential-integration.md) | M6 resource & credential integration | accepted |
| [0082](./0082-api-webhooks-idempotency.md) | API edge — OpenAPI, idempotency, webhooks | accepted |

## Index (live standalone, 0046–0072)

Stubs 0042–0067 that Wave B folded into the contract ADRs were evicted
2026-05-18 (full text in git history; supersession recorded below). Only
live standalone decisions remain as individual files:

| # | Title | Status |
|---|-------|--------|
| [0046](./0046-metrics-telemetry-boundary.md) | Merge telemetry into `nebula-metrics` | accepted |
| [0050](./0050-m3-5-w3c-trace-context-propagation.md) | W3C Trace Context propagation | accepted |
| [0053](./0053-two-struct-dx-consolidation.md) | Two-struct DX consolidation | accepted |
| [0054](./0054-typed-capability-system.md) | Typed capability system | accepted |
| [0055](./0055-nebula-sdk-facade.md) | `nebula-sdk` re-export façade | accepted |
| [0056](./0056-type-safe-dag.md) | Type-safe DAG | accepted |
| [0057](./0057-ai-agent-sdk.md) | AI agent SDK (`nebula-agent` direction) | proposed |
| [0065](./0065-visual-rendering-modes.md) | Visual rendering modes | accepted |
| [0068](./0068-layered-retry.md) | Layered retry (action-internal vs node-level) | accepted |
| [0069](./0069-action-surface-hybrid.md) | Action surface hybrid (suffix — shares cascade era with 0052) | accepted |
| [0072](./0072-nebula-storage-spec16-port-adapter-tenancy.md) | `nebula-storage` spec-16 port / adapter / tenancy | accepted |

## Supersession (audit trail — text-only; superseded bodies in git history)

| Superseded | Supersedes / consolidated by | Note |
|------------|------------------------------|------|
| ADR-0036 → ADR-0044 | ADR-0081 | Singular `Resource::Credential` → typed credential slot fields; M6 chain consolidated into contract 0081. |
| Canon `[L1-§3.10]` (`crates/telemetry/README.md`) | ADR-0046 | Telemetry merged into `nebula-metrics`. |
| ADR-0044 (hook signature + slot-field/migration shape) → ADR-0067 | ADR-0081 | `on_credential_refresh` → `&self` + `SlotCell`; rotation stays engine-owned (ADR-0030). |
| ADR-0030 facade slice → ADR-0066 | ADR-0081 | Management facade → `nebula-credential-runtime`; engine keeps resolver/refresh mechanism. |
| "Sprint E — adopt spec-16 row model" deferral | ADR-0072 | Spec-16 is the shipped port + adapter + tenancy architecture for storage. |

## Supersession (Wave B contract merges)

| Contract | Absorbs (evicted stubs) |
|----------|-------------------------|
| [0080](./0080-schema-validation-platform.md) | ADR-0052, 0058–0064 |
| [0081](./0081-m6-resource-credential-integration.md) | ADR-0042–0045, 0051, 0066–0067 |
| [0082](./0082-api-webhooks-idempotency.md) | ADR-0047–0049 |

## Related plans

- `docs/plans/2026-05-18-003-refactor-docs-adr-eviction-plan.md` — this eviction (stubs + 0001–0041 out of tree).
- `docs/plans/2026-05-18-001-refactor-docs-stack-contract-consolidation-plan.md` — Wave A/B (agent router + contract ADRs).
- `docs/plans/2026-05-18-002-refactor-code-doc-citation-cleanup-plan.md` — Wave C (Rust comments).
- `docs/plans/2026-05-17-001-feat-integrator-flagship-platform-plan.md` — implementation (doc gate cleared).
