# ADR-0055: `nebula-sdk` facade specification

**Status:** Proposed (2026-05-14)
**Tags:** sdk, facade, distribution, plugins

## Context

Charter F4: *"Single facade — `nebula-sdk`. Plugin authors and engine
integrators depend on exactly one Nebula crate."*

Current state: workspace contains 30+ crates organized in layered
architecture (per `deny.toml`). Plugin authors and integrators
historically import from multiple crates directly (`nebula-action`,
`nebula-engine`, `nebula-credential`, etc.) — fragile against internal
refactoring, harder to onboard.

Pattern precedent: `tokio` (umbrella + features), `serde` (re-exports
`serde_derive`), `axum` (re-exports `axum-core` + `axum-macros`).

## Decision

`nebula-sdk` becomes the **canonical entry point** for both audiences:

- **Plugin author** writes actions. Imports `nebula-sdk = "1"` plus
  third-party crates (reqwest, sqlx, anything from crates.io).
- **Engine integrator** builds deployment binary. Imports
  `nebula-sdk = "1"` with feature flags + plugin crates.

Internal crates (`nebula-action`, `nebula-engine`, `nebula-credential`,
`nebula-resource`, `nebula-workflow`, `nebula-storage`,
`nebula-execution`, `nebula-schema`, `nebula-validator`,
`nebula-expression`) are **implementation details** re-exported through
the facade.

### Facade content (top-level re-exports)

```rust
// === Authoring (plugin author surface) ===
pub use nebula_action::{
    Action, StatelessAction, StatefulAction, TriggerAction, ResourceAction,
    ControlAction, WebhookAction, PollAction, PaginatedAction, BatchAction,
    StatelessOutcome, StatefulOutcome, ControlOutcome, TriggerEventOutcome,
    OutputEnvelope, OutputMeta, ActionMetadata, ActionError,
    ActionContext, TriggerContext,
    action,                                 // attribute macro
};
pub use nebula_action::derive::Action;      // derive macro

pub use nebula_credential::{Credential, CredentialHandle, Refreshable};
pub use nebula_resource::{Resource, ResourceHandle};
pub use nebula_schema::{
    HasSchema, ValidSchema, ValidValues, ResolvedValues,
    Field, FieldKey, FieldValue, FieldValues,
    field_key,
};
pub use nebula_schema::stdlib::*;           // Email, Url, Cron, etc. (F13)
pub use nebula_validator::{Validator, ValidatorRegistry, Predicate, Rule};
pub use nebula_expression::{Expression, ExpressionAst, ExpressionEngine};

// === Symmetric API (ADR-0060) ===
pub use nebula_action::{Acquirable, Resolvable, Handle, require};

// === Integrator surface ===
pub use nebula_engine::{WorkflowEngine, EngineBuilder, EngineConfig, ActionRegistry};
pub use nebula_storage::Storage;            // backends behind features

// === Common ===
pub use nebula_error::{NebulaError, ErrorKind};

// === Prelude ===
pub mod prelude {
    pub use crate::{
        Action, StatelessAction, ActionContext, ActionError, ActionMetadata,
        StatelessOutcome, OutputEnvelope, action, require,
        Handle, HasSchema,
    };
    pub use serde::{Deserialize, Serialize};
}
```

### Feature flags

```toml
[features]
default = ["webhook", "poll", "stateful", "stateless", "control", "credentials", "resources", "schema-stdlib"]

# Action shapes — opt-in
stateless    = []                                       # always available
stateful     = ["nebula-action/stateful"]
trigger      = ["nebula-action/trigger"]
webhook      = ["nebula-action/webhook"]
poll         = ["nebula-action/poll"]
control      = ["nebula-action/control"]
resource-action = ["nebula-action/resource-action"]

# Subsystems
credentials  = ["dep:nebula-credential"]
resources    = ["dep:nebula-resource"]
schema-stdlib = ["nebula-schema/stdlib"]                # Email/Url/Cron newtypes

# Storage backends
storage-postgres = ["nebula-storage/postgres"]
storage-sqlite   = ["nebula-storage/sqlite"]
storage-memory   = ["nebula-storage/memory"]            # tests / dev

# Sandbox tier (per F8 security model)
sandbox-inproc   = []                                   # default, always available
sandbox-process  = ["nebula-engine/sandbox-process"]

# Observability
metrics = ["nebula-engine/metrics"]
tracing = ["nebula-engine/tracing"]
```

### Versioning policy

- `nebula-sdk = "1.x"` — single major version per multi-year cycle.
- Internal crates may bump major freely; `nebula-sdk = "1.x"` pins
  them to a tested compatible set (Hyrum's law mitigation).
- Public re-exports treated as `#[stable(since = "1.0")]`. Removal =
  major bump.
- `prelude` module — extra-stable. Changes only at major bumps.
- Internal crates accessible via `nebula_sdk::__internal::*` with
  `#[doc(hidden)]` for rare escape hatches.

### Plugin distribution

Plugin = ordinary cargo crate exposing
`pub fn register_into(registry: &mut nebula_sdk::ActionRegistry)`.

```toml
# my-stripe-plugin/Cargo.toml
[dependencies]
nebula-sdk = { version = "1.0", features = ["stateless", "credentials", "webhook"] }
async-stripe = "0.40"
```

No special plugin packaging. No private registry. No dynamic loading.
No ABI compatibility concerns.

## Consequences

### Positive

- Single onboarding entry point: `cargo add nebula-sdk` + 4-line
  Hello World.
- Internal crate refactoring invisible to plugin authors.
- Versioning discipline at facade level — Hyrum's law mitigation.
- Standard Rust ecosystem patterns (`tokio` / `serde` / `axum`
  precedent).

### Negative

- Re-export maintenance: each new public type in internal crates
  requires `pub use` in `nebula-sdk` (mitigated by review checklist
  + grep-based CI lint).
- Feature flag explosion if not disciplined — list above is the
  agreed initial set; new features require ADR amendment.

### Neutral

- Plugin authors who genuinely need internal access can `use
  nebula_action::factory::*` directly — `#[doc(hidden)]` + opt-in
  acknowledgement.

## References

- Conference Day 3 evening (CONFERENCE-NOTES.md) — facade decision.
- ADR-0052 — referenced this facade pattern in §Decision.
- `tokio`, `serde`, `axum` umbrella crate precedents.

## Out of scope

- Internal crate publication policy (whether `nebula-action` etc. go
  to crates.io or remain workspace-only) — separate operational
  decision.
- Documentation site (rustdoc landing page) — `nebula-sdk` docs are
  the canonical docs; internal crates documented but not
  promoted.
