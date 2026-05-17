# ADR-0064: UI form composition — schema vs slot bindings

**Status:** Proposed (2026-05-14)
**Tags:** ui, schema, slot-binding, editor, dx

## Context

Charter F17 / F18: form rendering in workflow editor has two distinct
sources — **schema** (action input fields) and **slot bindings**
(resource/credential pickers). n8n collapses these into one schema
property type and pays in boilerplate, runtime crashes, no
discoverability. Nebula deliberately separates.

Three-layer slot binding architecture:
1. Author declares need (`#[require("key")] field: Handle<T>`).
2. Workflow author binds instance (`slot_bindings: { key:
   "instance_id" }`).
3. Deployment registers instances (`resources.register("instance_id",
   ...)`).

This ADR specifies the **two-panel UI rendering convention** and
formal contract between code-side declarations and editor-side
rendering.

## Decision

### Two-panel rendering

Workflow editor's node-edit modal renders two distinct panels:

```
╔════════════════════════════════════════════════════════════╗
║  ◀  Node Name                                          ✕  ║
╠════════════════════════════════════════════════════════════╣
║                                                            ║
║  ┌─ Action Input ──────────────────────────────────────┐  ║
║  │  (rendered from #[derive(Schema)] on Self::Input)    │  ║
║  │  • field 1                                            │  ║
║  │  • field 2                                            │  ║
║  │  • ...                                                │  ║
║  └──────────────────────────────────────────────────────┘  ║
║                                                            ║
║  ┌─ Bindings ──────────────────────────────────────────┐  ║
║  │  (rendered from #[require(...)] declarations)        │  ║
║  │  • slot 1 (Resource): [picker dropdown]              │  ║
║  │  • slot 2 (Credential): [picker dropdown]            │  ║
║  └──────────────────────────────────────────────────────┘  ║
║                                                            ║
║                                  [ Cancel ] [ Save ]      ║
╚════════════════════════════════════════════════════════════╝
```

**Action Input panel** — generated from `#[derive(Schema)]` on
`Self::Input`. Renders form fields with all `#[field(...)]` attributes
applied (label, widget, conditional visibility, validation).

**Bindings panel** — generated from `#[require(...)]` declarations on
the Action struct. Renders pickers per slot with type-filtered
options. **Schema does NOT contain credential/resource picker fields.**

### Default state per ADR-0065

Default visual rendering is **hidden + Inspector** (not the two-panel
modal shown above). The two-panel modal is the **alternative view** for
authors who explicitly request it. Per-workflow setting persisted.

(See ADR-0065 for full discussion of visible vs hidden modes.)

### Mode field + slot binding two-tier rendering

When credential `Self` includes a Mode field for scheme selection
(API key vs OAuth2 vs Service Account), credential editing UI shows:

```
auth (TelegramCredential):
  Scheme: [API Key ▼]                    ← Mode field (part of credential properties)
  Instance: [main_bot_token ▼]           ← slot binding (instance picker)
```

Instance picker filters options by selected scheme (e.g. only
`API Key` instances shown when scheme = "API Key"). Two-tier rendering
emerges from existing primitives — no new abstraction needed.

### `MetadataSlot` contract

`ActionMetadata` carries a derived list of slots from `#[require(...)]`:

```rust
pub struct MetadataSlot {
    pub key: String,                    // slot key from #[require("key")]
    pub kind: SlotKind,                 // Resource | Credential (inferred)
    pub type_id: TypeId,                // expected type (for picker filtering)
    pub type_name: &'static str,        // for diagnostics
    pub modifier: SlotModifier,         // Required | Optional | Lazy | OptionalLazy
    pub on_failure: OnFailurePolicy,    // FailFast | Degrade | Defer
    pub label: Option<String>,          // optional human-readable, from #[require(label = "...")]
}

pub enum SlotKind { Resource, Credential }
pub enum SlotModifier { Required, Optional, Lazy, OptionalLazy }

impl ActionMetadata {
    pub fn slots(&self) -> &[MetadataSlot];
}
```

Editor reads `ActionMetadata.slots()` to render Bindings panel. Single
source of truth — no separate UI manifest.

### Helpful disabled states

When no resources of required type registered:

```
🔐 auth (StripeCredential):
  ┌─────────────────────────────────────────────────────────┐
  │ — no StripeCredential registered —                  ⓘ   │
  └─────────────────────────────────────────────────────────┘
  ⓘ Add a StripeCredential in your deployment to enable this binding.
     [+ Open Credential Library]
```

Maxim Fateev's pattern from Day 6 evening — never show empty dropdown
without explanation.

### Promote-to-canvas affordance

Each binding row has `[⤴ Promote to canvas]` action. Clicking creates
a canvas-level resource/credential node and supply edge to this
action. See ADR-0065 for full canvas-node visual model.

## Consequences

### Positive

- Schema and slot bindings — orthogonal concerns, never conflated.
  No n8n-style runtime credential type mismatches.
- `MetadataSlot` as documented contract — editors / IDE plugins /
  testing tools all consume same metadata structure.
- Forward-compat with TypedDAG (ADR-0056) — `#[require]` declarations
  unchanged, visual rendering remains correct as workflow becomes
  typed.

### Negative

- Editor implementation cost: two distinct panel renderers vs one
  unified form. Mitigated by `nebula-editor` as separate product (own
  team, own scope).
- Author must understand the distinction (`#[derive(Schema)]` for
  input vs `#[require]` for bindings) — documentation burden.

### Neutral

- Schema's `Field::Mode` variant is reused for both action input
  variants AND credential scheme selection — natural symmetry, single
  widget implementation.

## References

- Conference Day 6 evening (CONFERENCE-NOTES.md) — Jan Oberhauser
  shared n8n scars.
- F17, F18 (charter §3).
- ADR-0065 (visual rendering modes) — defines hidden + Inspector
  default.

## Out of scope

- Editor implementation — `nebula-editor` separate product.
- Visual style / theming — design system concern.
- Internationalization of UI labels — separate concern.
