---
id: 0030
title: engine-owns-credential-orchestration
status: accepted
date: 2026-04-20
supersedes: []
superseded_by: []
tags: [credential, engine, rotation, refresh, canon-13.2, orchestration, reqwest]
related:
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0031-api-owns-oauth-flow.md
  - docs/adr/0016-engine-cancel-registry.md
  - docs/adr/0017-control-queue-reclaim-policy.md
  - docs/adr/0023-keyprovider-trait.md
  - docs/adr/0025-sandbox-broker-rpc-surface.md
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/PRODUCT_CANON.md#132-rotation-refresh-seam
  - docs/STYLE.md#6-secret-handling
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
linear: []
---

# 0030. `nebula-engine` owns credential orchestration

## Context

[ADR-0028](./0028-cross-crate-credential-invariants.md) establishes the
umbrella of cross-crate credential invariants. [ADR-0029](./0029-storage-owns-credential-persistence.md)
hands persistence to `nebula-storage`. This ADR codifies the second
migration: **runtime orchestration moves from `nebula-credential` to
`nebula-engine/src/credential/`**.

Today `nebula-credential` owns four kinds of orchestration that belong
elsewhere:

1. **Rotation orchestration.** `rotation/scheduler.rs`,
   `grace_period.rs`, `blue_green.rs`, `transaction.rs` — when to rotate,
   how long the grace window is, how to flip state transactionally. These
   are execution-lifecycle concerns; they belong adjacent to
   `engine/control_consumer.rs` (which already owns cancel / resume /
   restart orchestration per
   [ADR-0008](./0008-execution-control-queue-consumer.md) /
   [ADR-0016](./0016-engine-cancel-registry.md) /
   [ADR-0017](./0017-control-queue-reclaim-policy.md)).
2. **Resolver + executor + registry.** `resolver.rs`, `executor.rs`,
   `registry.rs` — type-erased dispatch and the hot-path credential
   resolve during workflow execution. Engine already has
   `credential_accessor.rs` and `resolver.rs`; the credential-side
   files are redundant wrappers.
3. **Token refresh HTTP.** The refresh fragment inside
   `credentials/oauth2/flow.rs` (nested under `oauth2/` since P2)
   performs an HTTP round-trip to an OAuth2 token endpoint when a
   projected access token nears expiry.
   The call happens during resolve, on the engine's hot path — it is
   engine work, not contract work.

The seam that must **not** move is `RefreshCoordinator`
(`crates/credential/src/refresh.rs`). It is a thundering-herd
prevention primitive (like `tokio::sync::Semaphore`), not a pluggable
policy — see §3 below.

The canon context that binds this migration:

- [§13.2 — Rotation / refresh seam](../PRODUCT_CANON.md#132-rotation-refresh-seam)
  — the seam is defined in credential; the invariant enforcer is
  engine. ADR-0028 invariant 2.
- [§12.5 — Secrets and auth](../PRODUCT_CANON.md#125-secrets-and-auth)
  — engine never materialises plaintext in a hot loop outside a
  zeroize container. ADR-0028 invariants 1 and 7.
- [STYLE.md §6 — Secret handling](../STYLE.md#6-secret-handling) —
  log-redaction helper. Non-negotiable for token refresh (see §4).

## Decision

### 1. Runtime orchestration moves to `nebula-engine`

The following types move from `nebula-credential/src/` to
`nebula-engine/src/credential/`:

| From `nebula-credential/src/` | To `nebula-engine/src/credential/` |
|---|---|
| `rotation/scheduler.rs` | `rotation/scheduler.rs` |
| `rotation/grace_period.rs` | `rotation/grace_period.rs` |
| `rotation/blue_green.rs` | `rotation/blue_green.rs` |
| `rotation/transaction.rs` | `rotation/transaction.rs` |
| `resolver.rs` + `executor.rs` | `resolver.rs` (merged with existing `credential_accessor.rs` and `engine/resolver.rs`) |
| `registry.rs` | `registry.rs` |
| `credentials/oauth2/flow.rs` refresh fragment | `rotation/token_refresh.rs` (new file, reqwest client — see §4) |

The target shape under `crates/engine/src/credential/` (spec §3):

```
crates/engine/src/credential/
├── mod.rs
├── resolver.rs           # merged: existing credential_accessor + credential/executor + credential/resolver
├── registry.rs           # type-erased dispatch
└── rotation/
    ├── mod.rs
    ├── scheduler.rs
    ├── grace_period.rs
    ├── blue_green.rs
    ├── transaction.rs
    └── token_refresh.rs  # HTTP token refresh (reqwest) during resolve
```

Contract types **stay** in `nebula-credential/src/rotation/`:
`policy.rs`, `state.rs`, `validation.rs`, `error.rs`, `events.rs`
(data types). Engine consumes these via sibling dep.

### 2. Resolver + executor + registry merge

The four existing resolver-shaped files across two crates
(`credential::resolver.rs`, `credential::executor.rs`,
`credential::registry.rs`, `engine::credential_accessor.rs`,
`engine::resolver.rs`) collapse into **two files** in
`engine/src/credential/`:

- `resolver.rs` — hot-path credential resolve, consuming the
  `CredentialStore` from `nebula-storage` (per ADR-0029). Merges
  `credential::resolver`, `credential::executor`, and the existing
  `engine::credential_accessor.rs`. The pre-migration split of
  "resolver" vs "executor" vs "accessor" was three names for the same
  role; post-migration one type does the work.
- `registry.rs` — type-erased dispatch against `Arc<dyn AnyCredential>`.
  Symmetric with the Y-model for actions (plugin registry dispatches
  actions; engine credential registry dispatches credentials).

The merge is **lossless**: every public call site against the old
shape maps 1:1 to a call against the new shape.

### 3. `RefreshCoordinator` design note — concrete, not trait

**`RefreshCoordinator` stays in `nebula-credential/src/refresh.rs` as
a concrete primitive (not a trait).** Engine uses it via its concrete
API surface. This ADR explicitly declares **no extension seam desired**
for `RefreshCoordinator`.

Rationale:

- Thundering-herd prevention is a property of the data structure, not
  a pluggable policy. The analogous primitive is
  `tokio::sync::Semaphore` — not a trait, not because "semaphore is
  simple," but because a swappable semaphore impl would invite broken
  impls that violate the contract (a semaphore that drops permits is
  not a semaphore).
- `RefreshCoordinator` uses a `parking_lot::Mutex<LruCache<String,
  Arc<CircuitBreaker>>>` at `refresh.rs:51` to bound memory under
  adversarial input (credential-id fanout) and coordinate per-key
  refresh coalescing. A trait-ified version would accept any impl,
  including ones that skip the LRU bound or the per-key coalescing.
  Both failures are silent (no type error) and severe (memory DoS or
  thundering herd on the upstream token endpoint).
- No production consumer has asked for an alternative coordination
  strategy. The one-good-impl-only posture matches the existing
  `nebula-resilience` retry primitives, `tokio`'s sync primitives,
  and the design spec §4 principle that "a seam you cannot honestly
  enforce is a §11.6 false capability."

If a future need arises for an alternative coordination strategy
(e.g., distributed refresh coalescing across multiple nebula-engine
replicas via a storage-backed lock), a new ADR opens. That ADR does
not relax `RefreshCoordinator`; it **supersedes** this §3 decision and
introduces a new primitive.

### 4. Token refresh logging — redaction mandatory

`crates/engine/src/credential/rotation/token_refresh.rs` performs an
HTTP POST to the OAuth2 token endpoint with the refresh grant and
receives a new access token (and optionally a new refresh token).
The file **MUST NOT** log access tokens, refresh tokens, bearer
values, or response body — **ever, at any tracing level, including
DEBUG and TRACE.**

Specific rules:

- All HTTP responses pass through a redaction filter before any
  `tracing::` call touches them. The redaction filter is the same
  helper used by STYLE.md §6 log-redaction tests.
- Tracing spans carry only metadata: duration, HTTP status code,
  credential_id, and the token-endpoint host (not full URL with
  query). Never the request body. Never the response body. Never
  request or response headers except `Content-Type` and
  `Content-Length`.
- `reqwest::Error` values and any intermediate response buffers are
  wrapped in `Zeroizing<>` (or scrubbed manually in a `Drop` impl)
  so that a panic mid-call does not leave plaintext in an error
  chain printed by the async runtime.
- Partial / truncated responses → fail-closed with zeroization of all
  buffers. Same posture as ADR-0031 §4.

CI gate: **one redaction test per token_refresh code path** in
`crates/engine/tests/credential_refresh_redaction.rs`. Each test
injects a secret-bearing response, invokes the code path, and greps
all emitted tracing spans / audit events / metrics labels / error
strings for substring. Any hit = CI fail. A new code path (e.g., an
additional retry-on-server-error path, a forked token-request shape
for a new IdP flavor) adds a row to the test.

### 5. `reqwest` becomes a base dep of `nebula-engine`

`reqwest` is already in the workspace `[workspace.dependencies]`
table (used today by `nebula-credential` for OAuth flow, scheduled to
move; potentially by `nebula-sandbox` for network broker verb). Adding
it to `nebula-engine/Cargo.toml` as a base dep is a layer-consistent
move: engine is the execution layer and already performs outbound
work (the control-queue consumer dispatches to actions that reach out,
but the engine itself was not the outbound HTTP owner until now).

This is consistent with the [ADR-0025](./0025-sandbox-broker-rpc-surface.md)
broker pattern — engine is the host-side owner for out-of-process
verbs. Token refresh is exactly that shape: engine mediates the
outbound call on behalf of the credential resolve.

`reqwest` configuration inherits from ADR-0031 §4 where applicable
(TLS only, bounded timeout, bounded response size). Token-refresh
specific tuning is declared in `token_refresh.rs` mod docs.

### 6. Canon §12.5 interaction on the hot path

Engine never materialises plaintext credential material in the hot
loop outside a zeroize container. Concrete rules:

- All reads go through `CredentialStore` (from `nebula-storage` per
  ADR-0029); the returned `StoredCredential` is already encrypted-
  at-rest semantics.
- Projected auth material (the output of `Credential::project()`)
  lives in a zeroize container for the lifetime of the action
  invocation. Scope exit scrubs it per STYLE.md §6.
- Token refresh writes the new state back through the store; the
  encrypt-at-rest path goes through `EncryptionLayer` per ADR-0029.
- The resolver does not log projected material, even on `ERROR` path.
  Errors reference the credential id and operation verb, never the
  decrypted bytes.

### 7. Rotation orchestration semantics

Rotation orchestration (scheduler, grace period, blue/green,
transaction) moves as-is from credential to engine. **No semantic
change** in this ADR — the move is physical. If the scheduler's
backoff, the grace period duration, or the blue/green swap invariants
require tuning, a separate ADR opens against the post-migration
location.

Adjacency: rotation orchestration lives in `engine/src/credential/
rotation/`; execution control-queue lives in
`engine/src/control_consumer.rs`. The two interact via the same
engine-internal error taxonomy (`engine::Error`) and the same
storage-backed durability
([ADR-0008 §5](./0008-execution-control-queue-consumer.md)). A future
ADR may unify rotation dispatch with control-queue dispatch if the
patterns converge; out of scope here.

## Consequences

**Positive.**

- `nebula-credential` becomes a pure contract crate (trait + DTOs +
  §12.5 primitives + contract-level rotation data types + refresh
  primitive). `Engine integration` MATURITY column flips from
  `partial` to `stable` (per ADR-0028 invariant 5, after P11 lands).
- Rotation orchestration lives next to execution orchestration. A
  reviewer tracing a `Cancel` through `control_consumer.rs` follows
  the same shape as tracing a `Rotate` through `rotation/scheduler.rs`.
- Token refresh is on the engine's hot path, which is where the
  redaction gate belongs. No surprise tracing leak from a credential-
  side fragment that escapes the engine's span context.
- Deleting `credential::executor.rs` + `credential::resolver.rs` +
  `engine::credential_accessor.rs` + `engine::resolver.rs` and merging
  to two files is a net-simpler engine surface.
- `RefreshCoordinator` stays concrete and close to the data
  structures — the seam per §13.2 is `Credential::refresh()` (the
  trait method), not a trait over the coordinator. This matches the
  canon seam shape.

**Negative / accepted costs.**

- `reqwest` becomes an engine base dep. Increases engine's compile-
  time cost and binary size (reqwest + rustls + tokio-util). Accepted:
  engine is the outbound HTTP host-side owner, consistent with ADR-
  0025. Duplicating reqwest usage across three crates (credential
  today, engine for refresh, api for OAuth exchange) was the cost we
  wanted to eliminate.
- Engine takes on a larger surface post-merge (resolver.rs and
  registry.rs pull in behavior that lived in credential). Accepted:
  the merge reduces **total** code (four files → two files) and
  eliminates a layer of thin wrapping.
- Rotation orchestration tests that lived in `credential/tests/units/`
  move to `engine/tests/`. Same invariants, different location.
- The `token_refresh.rs` redaction CI gate is an ongoing test-
  maintenance cost. One redaction test per code path is the quantified
  ongoing load. Accepted — this is the §4 non-negotiable.

**Neutral.**

- Contract-level rotation types (`policy.rs`, `state.rs`,
  `validation.rs`, `error.rs`, `events.rs`) stay in credential and
  are consumed by engine via sibling dep. No behavioral change to
  those files in this phase.
- `RefreshCoordinator` stays in credential (§3). Engine uses its
  concrete API; no trait.
- The scheduler's existing `tokio_util::sync::CancellationToken`
  dependency moves with the file — `tokio-util` becomes an engine dep
  and leaves credential's base deps (spec §9).

## Alternatives considered

### A. Leave rotation orchestration in `nebula-credential`

**Rejected.** Preserves the existing SRP violation. The crate's
MATURITY `partial / Engine integration` row captures this — rotation
orchestration in credential is already the load-bearing complaint.
Also misaligned with `nebula-action` (thin contract) and the planned
`nebula-resource` symmetric rework.

### B. New `nebula-credential-rotation` crate

**Rejected** for the same reason as Model B+ in ADR-0028 §A: adds a
fifth workspace member for a single orchestration concern when
`nebula-engine` already owns execution-lifecycle orchestration. The
adjacency argument (rotation next to control-queue) is load-bearing —
a separate crate loses that adjacency.

### C. Trait-ify `RefreshCoordinator`

**Rejected.** See §3 rationale above. Trait-ification invites broken
impls, and the use case for swapping the coordinator does not exist
in any production workflow. The §13.2 seam is the `Credential::refresh()`
method (trait-level), not the coordinator (primitive-level).

### D. Put `token_refresh.rs` in `nebula-credential` behind a feature

**Rejected.** Feature-gating the HTTP fragment keeps the dep on
`reqwest` in credential, preserving today's problem. The refresh-
during-resolve path is engine hot-path work; moving it cleanly to
engine removes reqwest from credential entirely (the goal per spec
§9) and eliminates the feature-flag bitrot risk.

### E. Merge resolver / executor / registry into a single `orchestration.rs` file

**Rejected.** Two files (resolver + registry) is the natural split:
`resolver.rs` owns the resolve-for-this-action path (hot path);
`registry.rs` owns type-erased dispatch setup (construction-time).
A single-file merge conflates two lifetimes (hot path vs construction)
and loses the documentation locality that's useful for new contributors.

### F. Put token refresh in `nebula-api` alongside OAuth callback exchange

**Rejected.** Token refresh happens during **workflow execution**
(engine hot path), not during **API request handling** (api hot
path). A credential whose access token expires mid-execution must be
refreshed before the action's next outbound call; the refresh is
not user-initiated. n8n makes this exact split
(`packages/core/execution-engine/utils` for refresh vs
`packages/cli/oauth/` for callback); we follow the same reasoning.

## Seam / verification

Files that carry the invariants after the migration (spec §3):

- `crates/engine/src/credential/mod.rs` — module root.
- `crates/engine/src/credential/resolver.rs` — merged resolver +
  executor + credential_accessor. §6 redaction on error paths.
- `crates/engine/src/credential/registry.rs` — type-erased dispatch
  registry.
- `crates/engine/src/credential/rotation/mod.rs` — rotation module
  root.
- `crates/engine/src/credential/rotation/scheduler.rs` — rotation
  scheduler.
- `crates/engine/src/credential/rotation/grace_period.rs` — grace
  window semantics.
- `crates/engine/src/credential/rotation/blue_green.rs` — state
  swap.
- `crates/engine/src/credential/rotation/transaction.rs` —
  transactional flip.
- `crates/engine/src/credential/rotation/token_refresh.rs` — reqwest
  client; §4 redaction rules apply.
- `crates/credential/src/refresh.rs` — `RefreshCoordinator` stays
  here (§3). Consumed by engine via concrete API.

Test coverage:

- `crates/engine/tests/credential_refresh_redaction.rs` — §4 CI gate.
  Row per token-refresh code path. ADR-0028 invariants 1 and 7.
- `crates/engine/tests/credential_rotation_scheduler.rs` — scheduler
  trigger + grace-period + blue/green flow migrated from
  `crates/credential/tests/units/rotation_*`.
- `crates/credential/tests/units/thundering_herd_tests.rs` — stays
  in credential (tests the coordinator primitive directly). §3
  guarantees that the primitive is not trait-ified; this test pins
  the bound LRU + per-key coalescing invariants.
- `crates/credential/tests/units/resolve_snapshot_tests.rs` — moves
  to engine alongside the resolver merge.

CI signals:

- **Layer direction in `deny.toml`**: `nebula-engine` may depend on
  `nebula-storage` + `nebula-credential`; reverse direction forbidden.
  Rule lands in P8 (same PR as the move).
- **MSRV 1.95**: all new files respect MSRV per
  [ADR-0019](./0019-msrv-1.95.md).
- **Redaction CI**: as §4 defines. No token substring in any output.

## Follow-ups

- **Phase P8 implementation PR** (spec §12) — physical move of
  `rotation/*`, `resolver.rs`, `executor.rs`, `registry.rs` into
  `engine/src/credential/`.
- **Phase P9 implementation PR** — physical move of token-refresh
  fragment from `credentials/oauth2/flow.rs` into
  `engine/src/credential/rotation/token_refresh.rs`. `reqwest`
  becomes engine base dep in this PR.
- **[ADR-0031](./0031-api-owns-oauth-flow.md)** — downstream sibling;
  api owns OAuth callback exchange (separate from engine-side refresh
  per §F).
- **Distributed refresh coalescing ADR** — if a future requirement
  needs cross-replica coordination, opens against §3.
- **Rotation dispatch unification with control-queue ADR** — if the
  two orchestration shapes converge, opens against §7.
- **MATURITY flip** — `nebula-credential` `Engine integration`
  `partial` → `stable` lands in P11 (after this and ADR-0031's impl
  lands). ADR-0028 invariant 5.
