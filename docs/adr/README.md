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

## Supersession

| Superseded ADR | Supersedes | Note |
|---|---|---|
| 0036 (`resource-credential-adoption-auth-retirement`, external `C:/Users/vanya/RustroverProjects/docs/adr/0036-*.md`) | [0044](./0044-supersede-0036-resource-credential-singular.md) | Singular `Resource::Credential` associated type → typed credential slot fields via `#[credential(key = …)]`. Per-slot rotation hook replaces the singular `on_credential_refresh` signature. |

## Related plan

- `.ai-factory/plans/m6-resource-finalization-integration-audit.md` — full M6 closure plan (Phases 0–11).
- `.ai-factory/ROADMAP.md` §M6 + §M11 — milestone closure.
