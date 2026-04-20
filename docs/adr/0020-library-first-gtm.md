---
id: 0020
title: library-first-gtm
status: proposed
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [gtm, strategy, apps, composition-root, canon-1, canon-2, canon-12]
related:
  - docs/PRODUCT_CANON.md#1-one-line-definition
  - docs/PRODUCT_CANON.md#2-position
  - docs/PRODUCT_CANON.md#35-integration-model-one-pattern-five-concepts
  - docs/PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities
  - docs/PRODUCT_CANON.md#121-layering-and-dependencies
  - docs/PRODUCT_CANON.md#123-local-path
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/audit/2026-04-19-codebase-quality-audit.md
  - docs/adr/0008-execution-control-queue-consumer.md
  - docs/adr/0013-compile-time-modes.md
  - apps/README.md
  - deny.toml
linear: []
---

# 0020. Library-first GTM + `apps/server` as thin composition root

## Context

[ADR-0008](./0008-execution-control-queue-consumer.md) §Follow-up names an
`apps/server` single production composition root as "tracked separately; this
ADR only names the need." `deny.toml:62-66` (wrapper list for
`nebula-engine`) already carves a slot for that future binary by allowing
`nebula-api` to pull the engine in a dev-only knife test "until a dedicated
`apps/server` composition root exists." That deferred decision left two
unanswered strategic questions: *should* Nebula build an `apps/server`, and
if yes, *what shape* — a thin composition root, or a product-shaped stack
with its own auth / key custody / storage surfaces.

The 2026-04-19 codebase quality audit (four agents: tech-lead, rust-senior,
dx-tester, security-lead — see
[`docs/audit/2026-04-19-codebase-quality-audit.md`](../audit/2026-04-19-codebase-quality-audit.md))
answered the first question unanimously: **library-first**. It also framed
the second question: a product-shaped `apps/server` would fork the security
budget (rust-senior: "worst-of-both is what we have today"; security-lead:
"infinite surface — TLS / RBAC / multi-tenant / KMS — needs a 12-18mo
full-time security engineer we don't have") without strengthening the
`nebula-sdk` authored façade that is already the headline surface per canon
§1, §2, and §3.5.

The structural state at audit time made the call urgent:

- `apps/web/` was a one-README placeholder with no build target.
- `apps/desktop/` was an unfinished Tauri scaffold with ambiguous release
  intent.
- `apps/server/` did not exist but was referenced from `deny.toml` and from
  ADR-0008's follow-up text.
- `nebula-sdk` — the canon-§2 primary-audience surface — was broken at its
  first-contact macro (`simple_action!` → nonexistent `ProcessAction` trait).

PR [#493](https://github.com/vanyastafford/nebula/pull/493) ("audit:
library-first verdict + P0 newcomer-blocker cleanup") fixed the SDK P0s and
added `examples/hello_action.rs`. PR
[#497](https://github.com/vanyastafford/nebula/pull/497) ("audit-p1:
structural pruning + re-apply #493 review feedback") deleted `apps/web/`,
pinned `apps/desktop/` as `Status: reference shell, not a release artefact`,
and landed the other P1 structural items. Both PRs executed the audit's fix
queue without locking in the strategic direction as a citable invariant —
that is the job of this ADR.

[ADR-0013](./0013-compile-time-modes.md) already established the
build-system contract the thin `apps/server` must compose: the
`mode-self-hosted` and `mode-cloud` cargo features on a top-level binary
crate, with a `build.rs` mutual-exclusivity gate. ADR-0013 §Decision rule 1
explicitly anticipates "any server binary introduced later — `crates/api`
stays a library and is composed from those binaries." This ADR does not
change that contract; it records the strategic constraint on what the
binary is *allowed to do* beyond composing the existing library surface.

## Decision

### 1. Library-first is the primary GTM

Nebula's go-to-market surface is **`nebula-sdk` and the integration model
described in canon §3.5**. The primary audience is Rust developers
embedding Nebula into their own binaries or writing integrations against
the typed `Action` / `Credential` / `Resource` / `Trigger` surface. This
matches canon §2 ("primary audience: developers writing integrations") and
§1 ("Rust-native, self-hosted, owned by you"). SaaS / hosted UI is
**downstream of this**, not parallel — the hosted shape is "Nebula on
infra" per canon §5, not a different product.

Per-agent audit verdicts that produced this call (one line each, sourced
from [`docs/audit/2026-04-19-codebase-quality-audit.md`](../audit/2026-04-19-codebase-quality-audit.md)
§"Strategic verdict"):

- **tech-lead** — audience signals (canon §3.5 integration model uniformity)
  already point library; a product-first track would fork the budget.
- **rust-senior** — library-first is cheaper long-term (single SemVer gate
  on `nebula-sdk` vs 25 crates under a product surface); the current
  worst-of-both combination is what this ADR retires.
- **security-lead** — library surface is ~12 invariants that fit in one
  head; product surface is infinite (TLS / RBAC / multi-tenant / KMS) and
  needs a 12-18-month full-time security engineer Nebula does not have.
- **dx-tester** — library layer is close to correct (`TestRuntime`,
  `stateless_fn` work); the remaining gaps (broken macro, missing
  `hello_action`) were single-PR fixes (landed via #493 / #497). The
  product-first build-gap would have been months of work.

### 2. `apps/server` is allowed, narrowly — thin composition root only

A future `apps/server` **may ship**, constrained to a **thin composition
root** over the existing library primitives. Concretely, `apps/server` is
**wiring**, not a new layer:

- **No forked auth stack.** `apps/server` consumes the same
  `nebula-credential` / `nebula-api` auth primitives that any embedder
  consumes. It does not define a new auth scheme or a second identity
  provider boundary.
- **No forked key custody path.** Encryption-at-rest key material is
  loaded via the `KeyProvider` seam that must exist in
  [`crates/credential/src/layer/encryption.rs`](../../crates/credential/src/layer/encryption.rs)
  before any `apps/server` PR is merged (see §3 pre-conditions). No
  env-only, no bespoke key-loading path specific to `apps/server`.
- **No forked storage backend interface.** `apps/server` selects between
  SQLite and Postgres via the existing ADR-0013 `mode-self-hosted` /
  `mode-cloud` feature gates; the storage boundary is the existing
  `ExecutionRepo` / `ControlQueueRepo` surface. No parallel storage port
  defined inside `apps/server`.
- **No relaxation of `deny.toml` layer enforcement** (canon §12.1) "to
  make `apps/server` simpler." The binary composes downward through the
  same `nebula-api` → `nebula-engine` → `nebula-storage` path the dev-only
  knife test already exercises; adding `apps/server` to the `nebula-api`
  wrapper list in `deny.toml:62-66` is the expected mechanical change, not
  a structural one. Any proposal to relax the upper bound to a second
  top-layer crate requires a new ADR superseding this one.

This is the ADR-0008 follow-up slot: the `apps/server` the follow-up named
is **this** thin-composition-root shape, not a product-shaped stack.

### 3. Pre-conditions gate before any `apps/server` PR merges

Three guard rails from the audit (`docs/audit/2026-04-19-codebase-quality-audit.md`
§"Guard rails") are pre-conditions for the first `apps/server` PR — not
nice-to-haves, not follow-ups. Once `apps/server` composes auth + key
custody + REST in one binary, whichever shape lands first freezes as de
facto API for operators writing systemd units, configs, and runbooks. The
audit called this explicitly: "once `apps/server` ships with env-only key
loading, that becomes de facto API forever."

Pre-conditions:

1. **`KeyProvider` seam exists in
   [`crates/credential/src/layer/encryption.rs`](../../crates/credential/src/layer/encryption.rs).**
   The current surface accepts `Arc<EncryptionKey>` directly
   (`encryption.rs:62` at audit time) with no provider trait between the
   composition root and the key material. `apps/server` requires a
   `KeyProvider` trait + `EnvKeyProvider` impl landed under a separate
   ADR (audit P0 #6), so the future file / KMS / HSM providers plug in
   without rewriting `apps/server`'s wiring.
2. **`WebhookTrigger::signature_policy()` defaults to `Required`.**
   `crates/action/src/webhook.rs` has the constant-time tag compare
   primitive (audit-noted at `:972+`), but enforcement is opt-in. Authors
   who forget the verify call ship unsigned webhooks behind discoverable
   URLs. `apps/server` advertises webhook URLs in deployment docs the
   moment it ships — the default must flip to `Required` first.
3. **REST `DefaultBodyLimit` (1 MiB) wired in
   [`crates/api/src/app.rs`](../../crates/api/src/app.rs).** Webhook
   transport caps itself, but `/workflows` and `/credentials` POST
   handlers do not. 1 MiB is the audit-recommended default; finer tuning
   can come later. This gate is about not shipping the unlimited-body
   surface as the `apps/server` default.

Each pre-condition has its own ADR slot in the audit's "Open ADRs needed"
table; this ADR does not subsume them, it names them as the gate.

### 4. `apps/web` stays closed until `apps/server` ships

PR [#497](https://github.com/vanyastafford/nebula/pull/497) already deleted
the placeholder; this ADR makes the policy normative: **no `apps/web/`
directory reappears until `apps/server` is merged**, and then only as the
canonical SaaS frontend tied to it. A web UI without a server to talk to is
a placeholder that drifts — the audit found one such placeholder already.
The canonical-SaaS-frontend framing means `apps/web` is not a "web SDK" or
"embeddable widget" — it is the official operator UI for the `mode-cloud`
(and optionally `mode-self-hosted`) deployment shapes described in
ADR-0013.

### 5. `apps/desktop` is a reference shell, not a release artefact

PR [#497](https://github.com/vanyastafford/nebula/pull/497) pinned
`apps/desktop/README.md` with `# Status: reference shell, not a release
artefact`. This ADR makes that normative: `apps/desktop` exists to
demonstrate the `mode-desktop` composition over the library primitives (and
to give the TUI / embedded-action developer a runnable reference), not as a
shipped end-user product. Promoting `apps/desktop` to a release artefact
requires a new ADR — otherwise the release surface stays `nebula-sdk` +
(eventually) `apps/server`, not the Tauri shell.

## Consequences

**Positive**

- The strategic question ADR-0008 deferred is closed. Future PRs that
  touch `apps/server` cite this ADR as the constraint rather than
  re-opening the question.
- `nebula-sdk`'s primary-audience status is explicit and citable —
  contributors evaluating "should we add X public surface?" have a
  canon-grounded answer (X must strengthen the integration author path;
  everything else is downstream).
- Security budget is bounded. The ~12 invariants the library surface
  requires (key custody, credential zeroization, webhook signature,
  plugin sandbox boundary, auth scheme, body limits, etc.) fit one
  head. A product-first track would balloon this into a surface that
  cannot fit one head — the audit's load-bearing rationale.
- ADR-0013's `mode-*` feature contract gains its first concrete consumer
  pattern: `apps/server` composes the existing library primitives under
  `mode-self-hosted` / `mode-cloud` without redefining any layer.

**Negative / accepted costs**

- The deferred "should we build `apps/server`?" question stays deferred
  until the three pre-conditions land. This is deliberate — the audit's
  guard-rails framing makes shipping the gate before the binary a
  non-negotiable ordering.
- `apps/server` staying on the backlog means `simple_server.rs`
  (`crates/api/examples/simple_server.rs`) keeps its `// DEMO ONLY`
  marker (ADR-0008 §4) longer. Acceptable: ADR-0008 already landed the
  consumer skeleton, so the demo's role as "this is not the real
  composition root" is structurally explicit.
- Operators looking for a "standard Nebula daemon" have only `apps/cli`
  + `--tui` until `apps/server` lands. This matches canon §5 ("local
  storage truth" — one SQLite path today) and is already documented
  there.

**Neutral**

- `deny.toml:62-66` wrapper list continues to allow `nebula-api` to pull
  `nebula-engine` as a dev-only knife-test path. When `apps/server`
  ships, the expected change is **adding** `nebula-server` (or whatever
  the binary crate is named) to that wrapper list — not relaxing the
  deny rule. That edit is mechanical and follows from this ADR; no new
  ADR needed for it.

## Alternatives considered

### A. Product-first GTM (ship `apps/server` as a parallel track with its own auth / key / storage stack)

Rejected by all four audit agents. Creates 25-crate SemVer drag per
rust-senior, infinite-surface security debt per security-lead, and does
not strengthen the `nebula-sdk` integration-author surface canon §3.5
already identifies as the differentiator. The worst-of-both state
described in the audit's executive summary is the current cost of having
*not* made this call; continuing to defer it is a vote for worst-of-both
by default.

### B. Hybrid — library-first now, product-shaped `apps/server` later with its own auth / key / storage stack

Rejected. The "later" shape is precisely what this ADR forbids: a parallel
stack with its own surfaces. The narrowly-allowed hybrid is **this ADR's
Decision §2**: `apps/server` as a thin composition root. Anything broader
re-introduces the forked-budget problem the audit retired.

### C. Defer the strategic question pending more data

Rejected. The audit explicitly framed its role as "picking a direction
now lets us prune the other and stop spending budget on both." Deferring
further keeps the worst-of-both posture: placeholders in `apps/`,
ambiguous audience signals in README and SDK, and a `deny.toml` slot for
a binary whose shape nobody has committed to. The audit settled this; the
ADR records the settlement.

### D. Relax `deny.toml` layer enforcement (canon §12.1) "to make `apps/server` simpler"

Rejected. Canon §12.1 layering is load-bearing for the "library is 25
composable crates, not a god binary" framing (canon §5 row 1). An
`apps/server` that can't compose through `nebula-api` without relaxing
the one-way-layer rule is evidence the binary is doing too much, not
evidence the rule is wrong. This is explicitly a non-option; any future
proposal to move it requires a superseding ADR.

## Follow-ups

This ADR is a strategic constraint, not an implementation plan. The
downstream chips live in their own ADRs (several already listed in the
audit's "Open ADRs needed" table):

- **KeyProvider trait + `EnvKeyProvider`** — security-lead, audit P0 #6.
  Blocks any `apps/server` PR per §3 pre-condition 1.
- **WebhookTrigger signature policy (Required default)** — security-lead,
  audit "Open ADRs needed". Blocks per §3 pre-condition 2.
- **REST `DefaultBodyLimit` (1 MiB)** — security-lead, audit "Guard rails"
  #2. Blocks per §3 pre-condition 3.
- **Crate publication policy (`publish = true` ≥3 consumers OR ADR)** —
  rust-senior, audit "Open ADRs needed". Orthogonal to this ADR but
  reinforces the library-first surface discipline.
- **`apps/server` binary crate — config story, deployment artefacts,
  observability stack** — downstream of the three gates above, each in
  its own follow-up ADR. This ADR deliberately does **not** decide those.

Non-goals for this ADR (restating for future readers):

- Does not decide `apps/server` implementation details (config shape,
  deployment targets, TLS termination, observability stack).
- Does not re-open the strategic question — future ADRs may supersede
  this one, but the default disposition is "library-first" until a new
  ADR explicitly changes it.
- Does not reopen ADR-0013. The `mode-*` feature contract is the surface
  `apps/server` composes; this ADR cites it, not supersedes it.

## Seam / verification

This ADR is a strategic invariant, so the "seam" is the canon / audit /
`deny.toml` consistency rather than a runtime check:

- Inbound link from [`docs/PRODUCT_CANON.md`](../PRODUCT_CANON.md) §2
  Position — so the library-first framing is reachable from the normative
  core's audience line.
- Inbound link from
  [`docs/audit/2026-04-19-codebase-quality-audit.md`](../audit/2026-04-19-codebase-quality-audit.md)
  §"Open ADRs needed" — the "Library-first GTM + apps/server as thin
  composition root" row cites this ADR id.
- `apps/README.md` already describes the future `apps/server` as "the
  production composition root for the `mode-self-hosted` deployment shape
  (ADR-0013) in a future `apps/server` chip" — consistent with this ADR;
  no edit required.
- `deny.toml:62-66` wrapper list comment already anticipates "a dedicated
  `apps/server` composition root is still tracked as a separate
  follow-up" — consistent; updated mechanically when the binary lands.

Related ADRs:

- [ADR-0008](./0008-execution-control-queue-consumer.md) — named the
  `apps/server` follow-up slot that motivated this ADR.
- [ADR-0013](./0013-compile-time-modes.md) — build-system contract the
  thin `apps/server` composes; not superseded, cited.
