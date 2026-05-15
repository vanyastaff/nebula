# Architecture Decision Records

This directory holds the worktree-local Architecture Decision Records (ADRs) accepted during the M6 resource finalization + dependency redesign cascade (2026-04-29).

The repository's full ADR archive lives at the parent project's `docs/adr/` (e.g. `C:/Users/vanya/RustroverProjects/docs/adr/0001..0041`). This worktree-local set captures the foundation decisions for the M6 cascade and the §M11 dependency-redesign milestone.

## Index

| # | Title | Status | Tags |
|---|---|---|---|
| [0042](./0042-node-binding-mechanism.md) | Node → ResourceId / CredentialId binding mechanism | accepted (2026-04-29) | action, resource, credential, workflow, binding, slot, m6, m11 |
| [0043](./0043-dependency-declaration-dx.md) | Dependency declaration DX (slot binding + Variant A trait + FromWorkflowNode) | accepted (2026-04-29) | action, resource, credential, schema, macro, slot, m11 |
| [0044](./0044-supersede-0036-resource-credential-singular.md) | Supersede ADR-0036 — Resource::Credential singular → slot fields | accepted (2026-04-29) | resource, credential, slot-binding, m11, supersession |
| [0045](./0045-eventtrigger-scope-deferral.md) | EventTrigger DX-wrapper deferral (candidate ROADMAP §M6.4) | accepted (2026-04-29) | trigger, dx, deferral, m6, m11, roadmap |
| [0046](./0046-metrics-telemetry-boundary.md) | Merge `nebula-telemetry` into `nebula-metrics` — single observability crate | accepted (2026-05-06) | metrics, telemetry, observability, boundary, m9 |
| [0047](./0047-openapi-31-generator.md) | OpenAPI 3.1 spec generation — adopt `utoipa` for `nebula-api` | accepted (2026-05-06) | api, openapi, drift-detection, layer-boundary, m3 |
| [0048](./0048-idempotency-store-backend.md) | Idempotency-Store Backend — Hybrid (in-memory + PG-backed) | accepted (2026-05-07) | api, idempotency, storage, m3 |
| [0050](./0050-m3-5-w3c-trace-context-propagation.md) | M3.5 W3C Trace Context propagation (HTTP → control queue → engine) | accepted (2026-05-11) | observability, tracing, api, engine, m3, m9 |
| [0052](./0052-credential-runtime-crate.md) | Credential management runtime crate (`nebula-credential-runtime`) | accepted (2026-05-15) | credential, runtime, layer-boundary, breaking, m11 |

## Supersession

| Superseded ADR | Supersedes | Note |
|---|---|---|
| 0036 (`resource-credential-adoption-auth-retirement`, external `C:/Users/vanya/RustroverProjects/docs/adr/0036-*.md`) | [0044](./0044-supersede-0036-resource-credential-singular.md) | Singular `Resource::Credential` associated type → typed credential slot fields via `#[credential(key = …)]`. Per-slot rotation hook replaces the singular `on_credential_refresh` signature. |
| Canon `[L1-§3.10]` (in `crates/telemetry/README.md`) | [0046](./0046-metrics-telemetry-boundary.md) | The "primitives below, naming/policy/export above" cross-crate invariant is replaced by intra-crate module discipline (`mod` boundaries + `pub`/`pub(crate)`) in the merged `nebula-metrics`. Implementation deferred to follow-up `/aif-plan` iteration. |
| `0030` facade slice (`engine-owns-credential-orchestration`, external `C:/Users/vanya/RustroverProjects/docs/adr/0030-*.md`) | [0052](./0052-credential-runtime-crate.md) | Management facade ownership moves to `nebula-credential-runtime` (Exec). ADR-0030's low-level mechanism (resolver/RefreshCoordinator/claim-repo) stays in `nebula-engine`. ADR-0041/0051 untouched. |

## Related plan

- `.ai-factory/plans/m6-resource-finalization-integration-audit.md` — full M6 closure plan (Phases 0–11).
- `.ai-factory/ROADMAP.md` §M6 + §M11 — milestone closure.
