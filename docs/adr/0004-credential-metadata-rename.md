---
id: 0004
title: credential-metadata-rename
status: accepted
date: 2026-04-17
supersedes: []
superseded_by: []
tags: [credential, naming, canon-alignment]
related: [crates/credential/src/record.rs, crates/credential/src/metadata.rs, docs/PRODUCT_CANON.md, docs/superpowers/specs/2026-04-17-rename-credential-metadata-description.md]
---

# 0004. Credential Metadata→Record, Description→Metadata rename

## Context

Canon §3.5 establishes the integration-catalog pattern as
"`*Metadata` + `ParameterCollection`" — the `*Metadata` struct holds the
catalog-visible identity of an integration (key, name, description, tags).
`ActionMetadata` and `ResourceMetadata` follow this pattern.

`nebula-credential` had the pattern inverted: `CredentialMetadata` was a
*runtime operational-state* struct (created_at, last_accessed, version counter,
expires_at, rotation_policy), and the *catalog-identity* struct was
`CredentialDescription` (separate file, `description.rs`). A reader encountering
`CredentialMetadata` by analogy with `ActionMetadata` would expect catalog data
but get runtime state instead. This is information leakage through naming and
violated canon §3.5 explicitly.

Source spec: `docs/superpowers/specs/2026-04-17-rename-credential-metadata-description.md`.

## Decision

Two atomic renames in `nebula-credential`, no shims:

1. `CredentialMetadata` → `CredentialRecord`
   Runtime operational state. "Record" is DDIA ch.2 terminology for a
   persisted entity row at a moment in time. No collision with `CredentialState`
   (trait) or `nebula_storage::rows::CredentialRow` (storage layer, distinct concern).

2. `CredentialDescription` → `CredentialMetadata`
   Integration catalog type. Now matches `ActionMetadata` / `ResourceMetadata`
   per canon §3.5.

Files renamed accordingly: `metadata.rs` → `record.rs`, `description.rs` → `metadata.rs`.
All internal consumers (`accessor.rs`, `any.rs`, macros, tests) updated in the
same commit. No re-export aliases; old names deleted entirely.

## Consequences

Positive:

- `nebula-credential` now follows the canonical `*Metadata = catalog` pattern;
  readers can reason by analogy across all three integration types.
- `CredentialRecord` is unambiguous: DDIA terminology, no collision risk.
- Zero wire-format break: only Rust type names change; serde field shapes are
  unchanged.

Negative:

- Breaking Rust API change for any external consumers of `CredentialMetadata`
  or `CredentialDescription`. At time of rename, all consumers were in-workspace
  (action, engine, sdk, desktop).
- Tauri desktop bindings may require frontend TypeScript regeneration if
  binding codegen references type names.

Follow-up:

- Evaluate whether `CredentialRecord` (runtime operational state) belongs in
  `nebula-credential` or in `nebula-storage` (backlog item: "Evaluate
  `CredentialRecord` placement — storage concern or credential concern?").
- A shared `IntegrationMetadata` trait across all three integration types is
  deferred pending schema migration spec 21.

## Alternatives considered

- **`CredentialEntry`**: viable but slightly vague; evokes HashMap entry
  rather than a mutable persistent row.
- **`CredentialSnapshot`**: implies point-in-time immutability, but the struct
  is mutated as rotations advance and access timestamps update. Misleading.
- **`CredentialState`**: collides with the existing `CredentialState` trait in
  the same crate.
- **Add `#[deprecated]` aliases**: rejected per canon §14 and user memory
  ("never propose adapters/bridges/shims — replace the wrong thing directly").

## Seam / verification

Seam:
- `crates/credential/src/record.rs` — `CredentialRecord` (was `CredentialMetadata`).
- `crates/credential/src/metadata.rs` — `CredentialMetadata` (was `CredentialDescription`).

`rg "CredentialDescription" crates/ apps/ --glob "*.rs"` returns zero production
hits post-rename.

Landed in commit `51baa36f` (refactor(credential): rename Metadata→Record,
Description→Metadata per canon §3.5).
