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

- identity and metadata contracts: `Action`, `ActionMetadata`
- dependency declarations: `ActionComponents` (`CredentialRef`, `ResourceRef`)
- flow-control model: `ActionResult`, `WaitCondition`, `BreakReason`
- data model: `ActionOutput` (value/binary/reference/deferred/streaming)
- error model: `ActionError` (retryable vs fatal)
- topology model: `InputPort`, `OutputPort`, `SupportPort`, `DynamicPort`
- minimal context abstraction: `Context` (bridge `NodeContext` exists temporarily)

## What does not live here

- workflow scheduling/orchestration
- retry/backoff engine logic
- sandbox executor internals
- resource/credential storage implementations

Those belong to runtime/engine/sandbox/resource/credential crates.

## Docs map

- [ARCHITECTURE.md](./ARCHITECTURE.md): current + target architecture
- [API.md](./API.md): stable API and authoring patterns
- [INTERACTIONS.md](./INTERACTIONS.md): crate-to-crate integration contracts
- [DECISIONS.md](./DECISIONS.md): accepted architectural decisions
- [ROADMAP.md](./ROADMAP.md): phased path to hardened production state
- [PROPOSALS.md](./PROPOSALS.md): candidate breaking and non-breaking extensions

## Archive

Legacy drafts and imported notes are preserved in:
- [`_archive/`](./_archive/)
