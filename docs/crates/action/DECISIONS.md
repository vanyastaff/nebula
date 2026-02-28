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

## D006: Core-vs-DX separation

Status: Accepted

Core contracts remain in `nebula-action`; convenience trait families/macros should move to optional DX layer to keep core stable.

## D007: Versioned metadata compatibility

Status: Accepted

`ActionMetadata.version` is treated as interface contract version, not implementation version. Breaking port/schema changes require version bump.

## D008: Sandbox-aware error signaling

Status: Accepted

Capability denials are represented directly (`SandboxViolation`) to keep policy failures observable and machine-actionable.
