# Credential redesign — exploratory drafts

**Date:** 2026-04-24
**Status:** Exploratory notes. NOT a spec. NOT a plan. NOT an ADR.

Эти файлы — **current thinking** о potential `nebula-credential` redesign. Они содержат acknowledged holes, open questions, и honest admissions о том что paper design не работает в предложенной форме.

## Не используй эти файлы как spec

Любой файл здесь может быть:
- Неверным (мы ещё не поняли)
- Противоречивым с другими файлами (tensions не resolved)
- Целиком отброшенным после prototype spike

Production decisions идут через:
- `docs/adr/` — architecture decisions
- `docs/superpowers/specs/` — implementation specs
- `docs/superpowers/plans/` — phased plans

Эти drafts — источник гипотез для feed в правильный процесс, не subsитут для процесса.

## Файлы

| # | File | Содержание |
|---|---|---|
| 00 | [overview.md](./00-overview.md) | Context, why this draft, tensions, file map, disclaimer |
| 01 | [type-system-draft.md](./01-type-system-draft.md) | Trait shape draft с acknowledged open questions (dyn-safety, ambiguity) |
| 02 | [layer-map.md](./02-layer-map.md) | Cross-crate responsibility map, deny.toml implications |
| 03 | [flows.md](./03-flows.md) | Concrete flows: create / resolve / refresh / rotate / multi-step / revoke |
| 04 | [schemes-catalog.md](./04-schemes-catalog.md) | 15 auth scheme types с injection mechanics |
| 05 | [known-gaps.md](./05-known-gaps.md) | 37 findings triage: blocker / resolvable / new dimension / detail |
| 06 | [prototype-plan.md](./06-prototype-plan.md) | Spike scope, success criteria, dispatch plan |

## Reading order

**Новичок в обсуждении:**
1. `00-overview.md` — понять frame
2. `05-known-gaps.md` — понять что сломано и почему
3. `06-prototype-plan.md` — понять next step

**Обсуждение type system:**
1. `01-type-system-draft.md` — proposed shape + holes
2. `04-schemes-catalog.md` — конкретные примеры применения

**Обсуждение cross-crate arch:**
1. `02-layer-map.md` — кто чем владеет
2. `03-flows.md` — конкретные interactions

## Статус обсуждения

После 3 раундов conversation + 3 specialist agents (security-lead, rust-senior, tech-lead) + detailed user review (37 findings):

**Консенсус:**
- Core trait shape в `nebula-core` + `nebula-schema` + existing `nebula-credential` — solid foundation
- ADR-0028..0033 invariants держатся
- P6-P11 landed work — keep
- Pain points реальны (n8n data-grounded)

**Open / broken:**
- Type system shape ambiguity + dyn-safety (8 findings)
- Pattern 1 vs Pattern 2 default
- Multi-step flow state model
- Multi-replica refresh race с rotated refresh_token
- Trigger integration (new dimension)
- ProviderRegistry seeding + versioning (new dimension)

**Next step:** prototype spike перед writing any spec. См. `06-prototype-plan.md`.

## Related production docs

- [docs/PRODUCT_CANON.md](../../../PRODUCT_CANON.md) §3.5 / §4.5 / §12.5 / §13.2 / §14
- [docs/INTEGRATION_MODEL.md](../../../INTEGRATION_MODEL.md)
- [docs/adr/0028-cross-crate-credential-invariants.md](../../../adr/0028-cross-crate-credential-invariants.md)
- [docs/adr/0029-storage-owns-credential-persistence.md](../../../adr/0029-storage-owns-credential-persistence.md)
- [docs/adr/0030-engine-owns-credential-orchestration.md](../../../adr/0030-engine-owns-credential-orchestration.md)
- [docs/adr/0031-api-owns-oauth-flow.md](../../../adr/0031-api-owns-oauth-flow.md) (candidate for supersede)
- [docs/adr/0032-credential-store-canonical-home.md](../../../adr/0032-credential-store-canonical-home.md)
- [docs/adr/0033-integration-credentials-plane-b.md](../../../adr/0033-integration-credentials-plane-b.md)
- [docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md](../../specs/2026-04-20-credential-architecture-cleanup-design.md) (existing cleanup spec — this draft supplements)
- [docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md](../../plans/2026-04-20-credential-cleanup-p6-p11.md) (P6-P11 landed status)
- [docs/research/n8n-credential-pain-points.md](../../../research/n8n-credential-pain-points.md) (real-world pain data)
- [docs/research/n8n-auth-architecture.md](../../../research/n8n-auth-architecture.md) (peer architecture reference)
