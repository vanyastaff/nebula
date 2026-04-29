---
id: 0045
title: eventtrigger-scope-deferral
status: accepted
date: 2026-04-29
supersedes: []
superseded_by: []
tags: [trigger, dx, deferral, m6, m11, roadmap]
related:
  - .ai-factory/plans/m6-resource-finalization-integration-audit.md
  - docs/adr/0043-dependency-declaration-dx.md
  - .ai-factory/ROADMAP.md
---

# 0045. EventTrigger DX-wrapper deferral (candidate ROADMAP §M6.4)

## Context

`crates/resource/plans/06-action-integration.md:222-260` proposed an
`EventTrigger` DX trait wrapping `TriggerAction` to give plugin authors a
`fn on_event(...)`-only API surface — the engine generates the lifecycle
machinery (re-acquire on error, exponential backoff, listen loop with
cancellation):

```rust
pub trait EventTrigger: Action {
    type Source: Resource;
    type Event: Serialize + DeserializeOwned;

    async fn on_event(
        &self,
        source: &<Self::Source as Resource>::Lease,
        ctx: &TriggerContext,
    ) -> Result<Option<Self::Event>>;

    async fn on_error(...) -> ErrorAction { ErrorAction::Reconnect }
}
```

The wrapper is purely DX sugar over `TriggerAction` + `nebula-engine::daemon`
event-source primitives (per ADR-0037). It is **not** required for any M6
exit criterion — Phase 10.2 (Telegram multi-workflow runnable example) can
reach its goals using raw `nebula_engine::daemon::EventSource` +
`TriggerAction` directly.

The user's headline scenario for M6 is "one Telegram bot resource backing
ten workflow trigger nodes" — a *shared-resource* test, not an
*EventTrigger DX* test. Implementing EventTrigger inside the
`m6-resource-finalization-integration-audit.md` cascade would inflate Phase
3 scope by an estimated additional ~3-5 agent-days of macro + engine
plumbing for capability that is verification-orthogonal to the M6 exit
criteria.

## Decision

**Defer the `EventTrigger` DX wrapper to a candidate ROADMAP §M6.4
follow-up milestone.** Inside the M6 + dependency-redesign cascade
(`m6-resource-finalization-integration-audit.md`):

1. **No `EventTrigger` trait, no engine-generated trigger lifecycle.**
2. **Phase 10.2 (Telegram multi-workflow example)** uses raw
   `nebula_engine::daemon::EventSource` + a thin `TriggerAction` impl that
   subscribes to the daemon's broadcast channel and fans out to triggered
   workflows. Verbose but ships M6.
3. **ROADMAP gets a §M6.4 candidate placeholder** for the future EventTrigger
   work, scheduled after M6 closes. Exit criteria for §M6.4 will be drafted
   from the lessons learned in the M6 Telegram example.

The deferral is explicit per `feedback_active_dev_mode.md` ("before saying
'defer X', confirm the follow-up has a home") — §M6.4 is the home.

## Consequences

### Positive

- **M6 cascade stays bounded.** Phase 3 (Action redesign) ships the trait
  family + macros without absorbing trigger-DX scope creep.
- **Telegram example demonstrates the actual M6 invariant** — shared-resource
  pattern across N workflows — without depending on a not-yet-shipped
  `EventTrigger` DX layer.
- **§M6.4 specification written from real-use experience** rather than
  speculative pre-design. The Telegram example exercises the underlying
  primitives; whatever DX gaps surface inform the §M6.4 scope.
- **Plugin authors get the deferral explicit** in the action README, with a
  pointer at the raw `EventSource` + `TriggerAction` path until §M6.4 lands.

### Negative

- **Trigger authoring DX is verbose during the deferral window** — every
  trigger plugin author hand-rolls the lifecycle loop the EventTrigger
  wrapper would have generated. Mitigation: provide a copy-pasteable
  template in the Telegram example.
- **Two future migrations possible** for trigger plugins authored in the
  deferral window: (i) M6 raw-loop pattern → (ii) §M6.4 EventTrigger when
  it ships. Acceptable per `feedback_hard_breaking_changes.md` for an alpha
  surface.

### Follow-up work

- ROADMAP §M6.4 candidate (after this plan closes) — `EventTrigger` DX
  wrapper. Scope draft from Phase 10.2 raw-loop experience.
- Telegram example provides a raw-loop template that future EventTrigger
  authors can use as the reference shape to wrap.

## Alternatives considered

### Alternative A — Implement EventTrigger now (option (A) of v4 design dialogue)

Rejected: ~3-5 agent-days of additional macro + engine work for capability
that is orthogonal to M6 exit criteria. Inflates cascade beyond bounded
budget. Per `feedback_brainstorming_pace.md` (cap scope when structure
agreed), defer the orthogonal addition.

### Alternative B — Re-time the Telegram example to wait for EventTrigger

Rejected: M6.3 (`resource-prototypes` → `examples/`) is the user's headline
verification of the shared-resource pattern. Delaying it stalls the M6
exit, which is the cascade's primary goal.

### Alternative C — Skip Telegram example, ship M6 without it

Rejected: the user's request explicitly framed Telegram-bot multi-workflow
as the canonical illustration of the shared-resource pattern. Without a
runnable example, the pattern stays conceptual.

## Seam / verification

- **Phase 10.2 example.** `examples/telegram-multi-workflow/src/main.rs`
  uses raw `nebula_engine::daemon::EventSource` + `TriggerAction`.
  Documentation comment cross-links to this ADR.
- **ROADMAP §M6.4 placeholder.** Phase 11.3 of
  `m6-resource-finalization-integration-audit.md` appends a §M6.4 entry to
  `.ai-factory/ROADMAP.md` referencing this ADR for context.
- **No code lock.** This ADR locks scope (deferral), not implementation —
  no test gate.
