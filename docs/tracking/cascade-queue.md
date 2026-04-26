---
name: Nebula cascade queue
description: Committed implementation cascade slots — named owner + scheduled date + queue position per slot, per Strategy §6.6 silent-degradation guard.
status: active
last-reviewed: 2026-04-26
related:
  - docs/superpowers/specs/2026-04-24-action-redesign-strategy.md
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
  - docs/adr/0038-action-trait-shape.md
  - docs/adr/0039-action-macro-emission.md
  - docs/adr/0040-controlaction-seal-canon-revision.md
---

# Nebula cascade queue

This file tracks **committed implementation cascade slots** with the three required fields per [`Strategy §6.6`](../superpowers/specs/2026-04-24-action-redesign-strategy.md) line 416-426: **named owner**, **scheduled date** (absolute, not "post-X-cascade"), **queue position** (relative to other queued cascades).

Slots without all three fields are **not committed** — they are intent placeholders. Path-gating decisions (e.g., action redesign Strategy §6.6 path (c) viability) require all three fields populated.

## Cascade slot table

| Slot # | Cascade name | Trait shape (architect-recommended) | Owner | Scheduled date | Queue position | Trigger condition / source |
|---|---|---|---|---|---|---|
| 1 | **Credential CP6 implementation** | `CredentialRef<C>` / `SlotBinding` / `SchemeGuard<'a, C>` / `SchemeFactory` / `RefreshDispatcher` per credential Tech Spec CP6 | TBD | TBD | TBD | Required if action redesign user picks path (b) or (c) per [`action Strategy §6.6`](../superpowers/specs/2026-04-24-action-redesign-strategy.md) line 416-426; see [`action Tech Spec §16.5`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) cascade-final precondition |
| 2 | **Cluster-mode coordination** | TriggerAction cluster-mode hooks (`IdempotencyKey`, `on_leader_acquire` / `on_leader_release`, `dedup_window` metadata) per [`action Tech Spec §2.2.3`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) cluster-mode trailing prose; engine-side coordination implementation | TBD | TBD | TBD (queued behind slot 1 per [`action Strategy §6.6`](../superpowers/specs/2026-04-24-action-redesign-strategy.md) line 426) | After action redesign implementation lands; engine-cascade scope per [`action Tech Spec §1.2`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) N4 |
| 3 | **ScheduleAction cascade** | Sealed-DX peer of TriggerAction (`ScheduleAction`) — no canon revision per [`ADR-0040 §2`](../adr/0040-controlaction-seal-canon-revision.md) Webhook/Poll precedent — + open `Schedule` runtime trait + 3 blessed impls (`CronSchedule`, `IntervalSchedule`, `OneShotSchedule`). Per [`action Tech Spec §15.12`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) Q8 Phase 2.5 deeper analysis | TBD | TBD | TBD | After action redesign implementation lands |
| 4 | **EventAction cascade** (renamed from QueueAction) | Sealed-DX peer of TriggerAction (`EventAction`) — event-source family, unified shape covering Kafka / RabbitMQ / SQS / NATS. No canon revision per ADR-0040 §2 precedent. Per [`action Tech Spec §15.12`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) Q8 user naming | TBD | TBD | TBD (after slot 3 OR parallel) | After action redesign implementation lands |
| 5 | **AgentAction + ActionTool cascade** | NEW primary trait family (AI ≠ trigger / event / data); likely canon §3.5 revision per ADR-0040 §2 enumeration discipline. AI use case priority. Per [`action Tech Spec §15.12`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) Q8 user naming | TBD | TBD | TBD | After action redesign implementation lands; AI use case priority |
| 6 | **StreamAction + StreamStage cascade** | NEW primary trait family (output streaming + composable pipeline stages); likely canon §3.5 revision per ADR-0040 §2. Per [`action Tech Spec §15.12`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) Q8 user naming | TBD | TBD | TBD | After action redesign implementation lands |
| 7 | **TransactionAction cascade** | Shape TBD — sealed-DX over `StatefulAction` for compensation patterns OR new primary trait family. Per [`action Tech Spec §15.12`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) Q8 user naming | TBD | TBD | TBD | After action redesign implementation lands |
| 8 | **`nebula-auth` Tech Spec cascade** | NEW crate — SSO / SAML / OIDC / LDAP / MFA. Architect-recommended scope per [`action Tech Spec §15.12`](../superpowers/specs/2026-04-24-nebula-action-tech-spec.md) Q8 Part D outside-action-cascade findings (security-lead identified 3 🔴 outside cascade scope). Orchestrator commits | TBD | TBD | TBD | After action redesign implementation lands; security-lead surfaced 3 🔴 gaps outside action cascade scope per Q8 Phase 2 |

## Slot governance

- **Three-field commit discipline.** Slot is "committed" only when **owner**, **scheduled date** (absolute date — `YYYY-MM-DD`, not "post-action-cascade"), and **queue position** are all populated. Until then, the slot is an **intent placeholder** — path-gating decisions that depend on the slot must treat it as uncommitted.
- **Trigger conditions** record what unblocks the cascade work (e.g., "after action redesign implementation lands"). They are not commitments — they are sequencing notes.
- **Trait shape** column carries the **architect-recommended shape** at slot commit time. Future cascade owners may revise the shape (an ADR records the revision); the recorded shape is the starting point, not a binding contract.
- **Source citation** column points to the document that committed the slot — use `grep` to verify the back-reference exists.

## Adding a new slot

When a new cascade slot lands:

1. Append a row to the table. Set Owner / Scheduled date / Queue position to `TBD` until committed.
2. The slot **must** carry a `Trigger condition / source` back-reference to the document that named the slot — Strategy §6.6, Tech Spec §15.X, or equivalent.
3. If the slot is **path-gating** for an active cascade (analogous to credential CP6 slot 1 per Strategy §6.6), document the gate explicitly in the source document.
4. Trait shape recommendations should be `grep`-able to a specific Tech Spec / ADR section that explains the rationale.

## Slot fulfillment

When a slot's three fields populate (owner + scheduled date + queue position commit), the slot becomes **active** for path-gating decisions. The implementation cascade then runs to its own ratification gates per [`AGENT_PROTOCOL.md`](../AGENT_PROTOCOL.md) cascade discipline.

When a cascade implementation lands and ratifies, the row stays in this table with status updated to `LANDED` (or moved to a `## Landed` section). Removing the row loses the historical record of what was promised.

