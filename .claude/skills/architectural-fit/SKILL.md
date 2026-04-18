---
name: architectural-fit
description: Use BEFORE writing code for non-trivial Nebula changes — adding a new public type / trait / module / crate; touching execution lifecycle, storage, credentials, integration model, plugin packaging, or layer boundaries; forcing a new concept into an existing abstraction. Walks the canon decision gate plus bounded-context check. Prevents quick-win patches that silently drift canon.
---

# architectural-fit

## When to invoke

Invoke before coding when the change:

- Adds a new public type, trait, module, or crate.
- Touches the integration model (Resource / Credential / Action / Schema / Plugin).
- Touches execution lifecycle, storage CAS, durable outbox, checkpoints, leases, or journal.
- Shifts a layer boundary or cross-cutting dep.
- Force-fits a new concept into an existing abstraction.

Unsure? Invoke anyway.

## Checklist

1. **Canon decision gate.** Walk the six questions in root `CLAUDE.md` §"Decision gate (before proposing an architectural change)". Some are directional (Q1 golden-path: "strengthens" is fine, "diverts" is not); others are yes/no hazards (Q2–Q6: a "yes" names a violation). Any **blocked or hazard** answer → STOP and open an ADR per `docs/PRODUCT_CANON.md §0.2`. Do not restate the six questions — read them from `CLAUDE.md` each time.

2. **Bounded-context mapping.** Name the context the change lands in:
   - **Core** — core / validator / parameter / expression / workflow / execution
   - **Business** — credential / resource / action / plugin
   - **Exec** — engine / runtime / storage / sandbox / sdk / plugin-sdk
   - **API** — api + webhook
   - **Cross-cutting** — log / system / eventbus / telemetry / metrics / config / resilience / error / schema

   No upward deps (enforced by `deny.toml`). If the concept fits two contexts, it is probably two concepts — split.

3. **Concept-promotion severity.** Is this a new semantic concept or a specialization?
   - 🟢 existing abstraction fits as-is
   - 🟡 extend existing abstraction, document in crate README
   - 🟠 new module / trait inside existing crate
   - 🔴 new crate, or L2 invariant change — ADR required in the same PR

4. **Quick-Win trap scan.** Re-read root `CLAUDE.md` §"Quick Win trap catalog". Confirm none of the listed traps applies. When tempted, remember: the rationalization is the trap.

## Output

```
## Architectural fit: <change>

Decision gate:     [all green / blocked at Q# — description]
Bounded context:   <name>
Concept promotion: 🟢 / 🟡 / 🟠 / 🔴 — <1-sentence rationale>
Quick-Win traps:   [none / list]

Next step: [proceed / open ADR and hand off to tech-lead for review]
```

Under 150 words. Do not proceed to code until "next step" is concrete.
