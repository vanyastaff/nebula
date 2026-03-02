# Decisions

## D001: Contract-first crate boundary

Status: Accepted

`nebula-action` keeps only contracts/types. Runtime orchestration lives outside this crate.

## D002: Explicit control-plane via `ActionResult`

Status: Accepted

Execution intent is encoded as enum variants rather than side effects or flags.

## D003: Data-plane flexibility via `ActionOutput`

Status: Accepted

Output supports value, binary, reference, deferred, and streaming forms as first-class variants.

## D004: Retry semantics in error type

Status: Accepted

`ActionError::Retryable` vs `ActionError::Fatal` gives engine clear policy hooks.

## D005: Typed dependency references

Status: Accepted

Dependencies are declared with `CredentialRef`/`ResourceRef` to improve static safety over plain string ids.

## D006: Core and DX in one crate

Status: Accepted

Core contracts and DX conveniences (trait families, macros, authoring helpers) all live in `nebula-action`. Optional `dx` or `authoring` submodule may group DX traits to keep the protocol surface clear; no separate crate.

## D007: Versioned metadata compatibility

Status: Accepted

`ActionMetadata.version` is treated as interface contract version, not implementation version. Breaking port/schema changes require version bump.

## D008: Sandbox-aware error signaling

Status: Accepted

Capability denials are represented directly (`SandboxViolation`) to keep policy failures observable and machine-actionable.

## D009: Execution trait names (core)

Status: Accepted

Core execution traits use the following names (see ARCHITECTURE, API, P001):

| Doc name | Current code name | Semantics |
|----------|-------------------|-----------|
| **StatelessAction** | `ProcessAction` | Pure function: `execute(input, &ctx) → ActionResult<Output>`; no state between calls. |
| **StatefulAction** | `StatefulAction` | Persistent state: `execute(input, &mut state, &ctx)`; `Continue`/`Break`. |
| **TriggerAction** | `TriggerAction` | Workflow starter: `start(&ctx)` / `stop(&ctx)`; lives outside execution graph. |
| **ResourceAction** | *(not yet in code)* | Graph-level DI: `configure(&ctx)`, `cleanup(instance, &ctx)`; engine runs before downstream. |

DX traits (same crate): **InteractiveAction**, **TransactionalAction**, **WebhookAction**, **PollAction** — stay as-is; they extend StatefulAction or TriggerAction.

**Migration:** Rename `ProcessAction` → `StatelessAction` when aligning code with Phase 2; add `ResourceAction` when engine supports graph-level DI. Deprecate old name for one cycle if needed.
