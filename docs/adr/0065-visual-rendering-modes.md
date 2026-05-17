# ADR-0065: Visual rendering modes for slot bindings

**Status:** Proposed (2026-05-14)
**Tags:** ui, editor, slot-binding, multi-agent

## Context

Charter F19 / F20: bindings (resource/credential references) admit
multiple visual rendering choices. Author code (`#[require("key")]
field: Handle<T>`) is fixed; editor decides how to draw. Day 7
conference voted **Hybrid**: default hidden + Inspector, opt-in canvas
nodes (Pattern B), multi-agent auto-promotion.

This ADR specifies the modes, the contract between author code and
editor, and the persistence of per-workflow rendering choice.

## Decision

### Three modes

#### Mode 1 — Hidden + Inspector (default)

- Main canvas shows **only actions and data flow edges**.
- Side **Inspector panel** lists all bindings of selected node and
  workflow-wide.
- Inspector supports search by binding key, "show where used"
  navigation, audit log, promote actions.
- **80% of workflow authors (business analysts / operators) see
  clean canvas.**

#### Mode 2 — Canvas nodes (opt-in, Pattern B)

- Resource and credential **rendered as canvas nodes** (visually
  distinct from action nodes — different shape and/or color).
- **Supply edges** from resource/credential nodes to consuming action
  nodes (visually distinct from data-flow edges — dotted vs solid).
- One resource/credential node can supply multiple actions —
  reusability visible.
- **Power users / integration architects / multi-agent designers**
  promote bindings explicitly.

#### Mode 3 — Layered canvas (backlog, post-MVP)

- Two canvas layers: **Logic layer** (actions + data flow) and
  **Infrastructure layer** (resources + credentials + supply edges).
- User toggles between layers.
- Hickey's "simple is unbraided" approach taken to its extreme.
- **Backlog** — implement after Modes 1-2 ship and adoption is
  evaluated.

### Multi-agent auto-promotion (heuristic, default-on)

When **3+ actions/agents reference same resource/credential**, the
shared binding is **automatically promoted** to canvas node (Mode 2
applied to that specific binding).

Threshold tunable per workflow (`auto_promote_threshold: 3` default;
0 = never auto-promote, ∞ = never).

Multi-agent workflows benefit visually — shared tools / vector stores
/ rate limiters drawn once, supply edges to all consuming agents.

### Per-workflow persistence

Workflow file (YAML or typed Rust) carries rendering preference:

```yaml
metadata:
  rendering:
    default_mode: hidden            # hidden | canvas | layered
    auto_promote_threshold: 3
    promoted_bindings:
      - { key: "shared_vector_store", kind: resource, type: PineconeStore }
      - { key: "github_token", kind: credential, type: GitHubCredential }
```

Collaborator opens workflow → sees same rendering. Persistence in
workflow file (not user preference) ensures team consistency.

### Mode selection UI

Workflow editor toolbar:

```
[ View: Hidden ▼ ]   [ Inspector ]   [ Auto-promote: 3 ]
```

User can switch modes per-workflow, see Inspector at any time,
adjust auto-promotion threshold.

### Canvas node visual specification

Resource node:

```
╭─ 📦 prod_db ─╮
│ PostgresPool │
╰──────────────╯
```

Credential node:

```
⬡ 🔐 main_token ⬡
  GitHubCredential
```

(Different shape — square corners for resources, hexagon for
credentials.)

Supply edge: dotted, with label of slot key:

```
  prod_db ╎─ db ─╎─→ Query DB action
```

Data flow edge: solid:

```
  Query DB ──────→ Generate Report
```

### Inspector panel content

```
┌─ Inspector — Workflow "Daily Report" ──────────────────┐
│                                                          │
│  ▼ Bindings                                              │
│  ────────────────                                        │
│                                                          │
│  🔐 main_token (GitHubCredential)                        │
│     Used by: Query Issues, Generate Report               │
│     [Show where used] [⤴ Promote to canvas]              │
│                                                          │
│  📦 prod_db (PostgresPool)                               │
│     Used by: Query DB                                    │
│     [Show where used] [⤴ Promote to canvas]              │
│                                                          │
│  📦 default_metrics (MetricsCollector)                   │
│     Used by: ALL nodes (auto-bound)                      │
│     [Show where used]                                    │
│                                                          │
│  🔍 Search: [____________]                               │
└──────────────────────────────────────────────────────────┘
```

## Consequences

### Positive

- Default protects 80% of workflow authors from infrastructure
  cognitive load (Maxim Fateev's "workflow author shouldn't pick
  infrastructure" principle honored).
- Power users get visual topology when they need it (Mark Payne's
  NiFi-experienced advocacy).
- Multi-agent workflows automatically expose shared topology (Joao
  Moura's CrewAI insight).
- Per-workflow persistence ensures team consistency.
- Author code unchanged across mode choices (F19) — engine produces
  same `slot_bindings` regardless.

### Negative

- Editor implementation cost: three rendering modes × node types.
  Mitigated by phased rollout (Mode 1 first, Mode 2 next, Mode 3
  later).
- Auto-promotion heuristic may surprise users (3+ becomes node
  unexpectedly). Mitigated by tunable threshold + clear UI feedback.

### Neutral

- TypedDAG (ADR-0056) compatibility: typed workflows still admit
  mode choice — generic param replaces UI picker, but visual
  rendering of "this workflow consumes these resources" still
  applies.

## Implementation phases

| Phase | Mode | Target |
|---|---|---|
| 1 | Mode 1 (Hidden + Inspector) — default | `nebula-editor` MVP |
| 2 | Mode 2 (Canvas nodes) — opt-in, manual promote | MVP+1 |
| 3 | Multi-agent auto-promotion heuristic | MVP+1 (with Mode 2) |
| 4 | Mode 3 (Layered canvas) — backlog | post-MVP, adoption-driven |

## References

- Conference Day 7 (CONFERENCE-NOTES.md) — full debate, voting,
  outcome.
- ADR-0064 — UI form composition (two-panel modal); this ADR
  defines visual rendering of bindings beyond the modal.
- F19, F20 (charter §3).

## Out of scope

- Editor implementation language / framework — `nebula-editor` team
  decision.
- Color palette / icon design — design system concern.
- Mobile / touch UI — desktop-first MVP.
