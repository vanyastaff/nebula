# nebula-action

Contract-first action system for Nebula.

This crate is the canonical boundary between action authors and engine/runtime implementation.
It must stay stable, explicit, and backward-compatible enough for long-lived workflow ecosystems.

## Production intent

`nebula-action` is considered production-ready when:
- action contracts are stable and versioned
- flow-control semantics are deterministic and fully documented
- integration contracts with sibling crates are explicit
- sandbox and capability boundaries are enforceable
- migration path for action trait evolution is defined

## What lives in this crate

- **action** — `Action` trait (metadata, components); execution is in `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`, etc. (see lib.rs re-exports).
- **metadata** — `ActionMetadata` (key, name, description, version, inputs, outputs, parameters: ParameterCollection); re-exports `InterfaceVersion` from core.
- **components** — `ActionComponents` (credentials: Vec&lt;CredentialRef&gt;, resources: Vec&lt;ResourceRef&gt;).
- **port** — `InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`; `FlowKind`, `ConnectionFilter`, `PortKey`.
- **result** — `ActionResult&lt;T&gt;`: Success, Skip, Continue, Break, Branch, Route, MultiOutput, Wait, Retry; `WaitCondition`, `BreakReason`, `BranchKey`.
- **output** — `ActionOutput&lt;T&gt;`, BinaryData, DataReference, DeferredOutput, StreamOutput, OutputMeta, etc.
- **error** — `ActionError`: Retryable, Fatal, Validation, SandboxViolation, Cancelled, DataLimitExceeded.
- **context** — `Context`, `ActionContext`, `TriggerContext` (+ capability modules). Re-exports: `ParameterCollection`, `ParameterDef` from nebula-parameter.

## What does not live here

- workflow scheduling/orchestration
- retry/backoff engine logic
- sandbox executor internals
- resource/credential storage implementations

Those belong to runtime/engine/sandbox/resource/credential crates.

## Docs map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md): current + target architecture
- [API.md](./API.md): stable API and authoring patterns
- [INTERACTIONS.md](./INTERACTIONS.md): crate-to-crate integration contracts
- [DECISIONS.md](./DECISIONS.md): accepted architectural decisions
- [ROADMAP.md](./ROADMAP.md): phased path to hardened production state
- [PROPOSALS.md](./PROPOSALS.md): candidate breaking and non-breaking extensions
- [SECURITY.md](./SECURITY.md): threat model and controls
- [RELIABILITY.md](./RELIABILITY.md): SLO and failure handling model
- [TEST_STRATEGY.md](./TEST_STRATEGY.md): contract and integration test plan
- [COMPATIBILITY.md](./COMPATIBILITY.md): schema-stable types and contract tests
- [MIGRATION.md](./MIGRATION.md): compatibility and rollout/rollback guide

## Archive

Legacy drafts and imported notes are preserved in:
- [`_archive/`](./_archive/)
