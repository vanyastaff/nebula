# Webhook subsystem — deferred breaking / cross-crate changes

**Status:** skeleton / deferred
**Spec:** `docs/plans/2026-04-13-webhook-subsystem-spec.md`
**Predecessors:** `2026-04-13-webhook-implementation-plan.md` (Sessions 1-3 landed)

## Purpose

Park everything from the webhook audit that is either (a) breaking
API change, (b) requires runtime capabilities we don't yet have,
(c) speculative and waiting for a real user, or (d) security /
ergonomic improvements that didn't make the initial three-session
bundle. Each item lives here so it does not rot; promote to a
dated plan file before actually implementing.

This file is a **skeleton**. Do not drive code directly from it.

## Items

### V1 — Typestate `WebhookAction<Inactive>/<Active>`

**Source:** architect review.

**Problem.** Today the adapter uses `RwLock<Option<Arc<State>>>` +
an `AtomicBool` guard to enforce "one active registration at a
time" at runtime. Double-start is caught with a `Fatal` error; a
failed rollback logs a warn and leaks the external hook. Three
audit findings (W1, W2, W5) all stem from state ownership being
runtime-managed instead of type-managed.

**Proposal.** Reshape `WebhookAction` so `on_activate` consumes a
`WebhookAction<Inactive>` and returns a `WebhookAction<Active,
State>`. Failed activation can't produce an `Active` value, so
there's no orphan state to roll back. `handle_request` only
compiles against `Active`, so the "before start" error path
disappears. `stop()` consumes `Active` and produces `Inactive`.

**Breaking.** Yes — trait shape changes. Every `WebhookAction`
impl in the workspace (currently zero, but will grow) must switch
to the new pattern.

**Cost.** ~2 days. Touches the trait, the adapter, examples,
docs. Also requires extending `TriggerHandler` to support
typestate or keeping the dyn adapter as an escape hatch.

**Blocked on:** needing more than one non-trivial `WebhookAction`
in-tree so the redesign is informed by real usage, not
speculation.

### V2 — Event ID deduplication (Idempotency-Key / X-Event-ID)

**Source:** sdk-user review, audit W4.

**Problem.** GitHub sends `X-GitHub-Delivery`, Stripe sends
`idempotency-key`, Slack sends `X-Slack-Request-Id`. Without
dedup, replay of a valid signed request causes double workflow
execution. Action authors have to build their own bounded LRU.

**Proposal.** Optional trait method on `WebhookAction`:

```rust
fn event_id(&self, request: &WebhookRequest) -> Option<String> {
    None
}
```

If `Some(id)` is returned, the adapter tracks the id in a
bounded LRU (size 10_000 by default, 24h TTL) and skips handling
if the id has been seen within that window.

**Breaking.** Additive (default impl returns None) — not
technically breaking, but only useful once `TriggerStateStore`
exists for cross-restart persistence.

**Blocked on:** runtime storage (same as poll F8 / V5). Without
persistence, the LRU resets on every restart and replays within
a restart window sneak through.

### V3 — Orphan external hook reconcile loop

**Source:** audit W2, security-lead H4.

**Problem.** Double-start race path in current adapter: if
`on_activate` partially registered with an external provider and
the subsequent lost-race rollback call to `on_deactivate` fails
(e.g., GitHub API down), the hook stays live on the external
provider pointing at a trigger ID that may later be reused by a
different workflow. Warn log is the only signal.

**Proposal.** Two-part fix:
1. **Idempotent registration.** Action stores `hook_id` in
   persistent state before calling the provider; on startup, if
   state shows a prior `hook_id`, action does a `DELETE` then a
   fresh `CREATE`.
2. **Reconcile job.** Runtime-owned background task that
   periodically lists registrations across providers (via the
   action's new `list_registrations` method) and cleans up hooks
   that don't correspond to any active trigger.

**Breaking.** Adds a new optional method to `WebhookAction`.
Also requires runtime scheduling infrastructure.

**Blocked on:** state persistence + runtime scheduled jobs.

### V4 — Additional crypto primitives

**Source:** audit 30-case matrix, security-lead L2.

**Ed25519** (Discord interactions, SendGrid events): pulls in
`ed25519-dalek`, ~20 LOC to wire. Ship in core when the first
Discord user files an issue.

**HMAC-SHA1** (Intercom): trivial — `Hmac<Sha1>` instead of
`Hmac<Sha256>`. Ship in a `legacy` feature flag or a per-provider
crate. SHA1 in the core primitive set invites misuse on new
integrations.

**RSA-SHA1** (AWS SNS SignatureVersion=2): requires fetching and
caching the signing cert from the URL in the message + RSA verify.
Per-provider crate, not core.

**Blocked on:** first real user for each scheme. Until then,
these additions are speculative and risk bit-rot.

### V5 — Subscription renewal scheduler

**Source:** audit 30-case matrix (MS Graph, Google Calendar /
Drive push, Zoom).

**Problem.** MS Graph change notifications expire every 3 days.
Google Calendar push channels expire every 7 days (configurable).
Zoom subscriptions require periodic re-verification. Without a
scheduler, the webhook silently stops receiving events when the
subscription expires upstream.

**Proposal.** New optional trait method
`fn renewal_interval() -> Option<Duration>` and a companion
`async fn renew(&self, state: &State, ctx: &TriggerContext)
-> Result<(), ActionError>`. Runtime-scheduled background job
calls `renew` on the interval.

**Breaking.** Additive on the trait, but requires runtime
scheduled job infrastructure (same blocker as V3 reconcile).

**Blocked on:** runtime scheduled jobs.

### V6 — TriggerState test/prod UUID split

**Source:** salvaged idea from deleted `crates/webhook/` crate,
architect Q4.

**Problem.** Production and test webhook traffic currently share
the same routing map entries. Test environment requests reach
production triggers and vice versa.

**Proposal.** `WebhookTransport::activate` takes an
`Environment { Test, Production }` argument. Each environment
has its own routing map (or uses separate nonces). `TriggerContext`
grows an `env: Environment` field.

**Architect note:** environment separation belongs in the
`ActionRegistry` metadata layer, not in the transport. Transport
only routes bytes; identity is an engine concept. When the engine
grows environment awareness, this falls out naturally.

**Blocked on:** engine-level environment concept.

### V7 — Outbound webhook delivery

**Source:** deleted `crates/webhook/` had a `WebhookDeliverer`
(Nebula-as-sender). Nothing in our audit recommended keeping it.

**Proposal.** When Nebula needs to POST webhooks to external
systems, that's a `nebula-action::http_sender` action type
composed with `nebula-resilience` for retries and backoff. NOT a
webhook-specific crate.

**Out of scope** until a real use case appears. Do not salvage
from the deleted crate.

### V8 — `SecretString` in HMAC helpers

**Source:** security-lead M1.

**Problem.** Current helpers take `secret: &[u8]`. The `nebula-credential`
crate has a `SecretString`/`Zeroizing` pattern that guarantees
secrets are zeroed on drop. Consistency with credential discipline
matters.

**Proposal.** Change HMAC helper signatures to `secret: &SecretString`
(or similar). Low exploit value today because the secret lives
longer in the credential store anyway; the value is consistency.

**Breaking.** Yes — all helper signatures change.

**Cost.** ~1 day. Touches ~6 call sites, easy migration.

### V9 — Stripe/Slack parser helpers

**Source:** sdk-user review top-3 missing helpers.

**Problem.** `verify_hmac_sha256_with_timestamp` serves both
schemes via a generic `canonicalize_fn`, but the **header
parsing** is still up to the action author: Stripe's
`Stripe-Signature: t=<ts>,v1=<hex>,v1=<hex>` needs comma-split +
per-key parse + multi-`v1` handling. Every Stripe plugin will
reimplement this.

**Proposal.** Dedicated helpers:
- `parse_stripe_signature(header: &str) -> Option<StripeSig>` with
  `StripeSig { timestamp: i64, v1_signatures: Vec<String> }`.
- `verify_stripe(request, secret, tolerance) -> SignatureOutcome`
  that composes parse + verify.
- Same for Slack: `verify_slack_v0(request, secret, tolerance)`.

**Breaking.** Additive — pure new functions.

**Cost.** ~half a day each. Ship when we have a Stripe or Slack
example in-tree.

### V10 — `body_json` default depth cap

**Source:** security-lead H2 upgrade path.

**Problem.** Today plain `body_json` has no depth cap (just a
docstring warning). `body_json_bounded(max_depth)` is opt-in.
Secure-by-default would flip this.

**Proposal.** Make `body_json::<T>()` call
`body_json_bounded::<T>(64)` internally. Keep
`body_json_bounded` as explicit knob for providers that need a
different limit.

**Breaking.** Yes, behavioural: previously-accepted deeply-nested
input now fails. In practice this only affects hostile inputs
(>64 levels), which are exactly the case we want to reject.

### V11 — Runtime integration

**Source:** gap not in the audit but revealed by Session 2
implementation.

**Problem.** `nebula-runtime` does not yet call
`WebhookTransport::activate` when a workflow with a webhook
trigger starts. The transport and the adapter are both ready,
but the wiring between them lives in runtime.

**Proposal.** `nebula_runtime::ActionRegistry` (or a
`TriggerManager` sibling) gains a `Option<WebhookTransport>`
dependency. When a workflow deploys, runtime iterates over its
webhook triggers, calls `transport.activate(handler,
ctx_template)`, stores the `ActivationHandle`, and calls
`adapter.start(&handle.ctx)`. On workflow undeploy, runtime
calls `adapter.stop(&ctx)` and then
`transport.deactivate(&handle)`.

**Blocked on:** runtime maturity. Runtime is still alpha (blocked
on credential DI + Postgres storage in `active-work.md`).

**Cost.** ~1 day once runtime is ready. Until then, webhook
triggers are registered in `ActionRegistry` but not fired.

## Prioritisation order when picking this up

1. **V11 (runtime integration)** — highest leverage, unlocks
   end-to-end webhook flow. Do as soon as runtime lifts blockers.
2. **V9 (Stripe/Slack parser helpers)** — ship when we have a
   first example in-tree. Half-day each.
3. **V8 (SecretString)** — discipline + consistency, do in a
   quiet week.
4. **V10 (body_json depth default)** — breaking but low-risk,
   batch with next trait-shape touch.
5. **V2 (event_id dedup)** — blocked on state storage.
6. **V3 (reconcile loop)** — blocked on runtime scheduling.
7. **V5 (subscription renewal)** — same blocker as V3.
8. **V1 (typestate)** — waits for real usage signal.
9. **V4 (extra primitives)** — per-user basis.
10. **V6 (env split)** — engine-level concept needed first.
11. **V7 (outbound)** — do not revisit until concrete need.

## Review cadence

Revisit this file:
- Before cutting a v1 release of `nebula-action` or `nebula-api`
- Every time a new real-world webhook integration lands (it may
  promote one of the V-items)
- Every 3 months: items older than 3 months get promoted to a
  plan file or deleted.

## Deferred items that are NOT in this file

- **Ed25519 / RSA / SHA1 / more HMAC variants** — tracked in V4.
- **Subscription renewal** — tracked in V5.
- **Test/prod env separation** — tracked in V6.
- **Outbound delivery** — tracked in V7.
- **Persistence (state storage)** — blocked generically on runtime;
  not a webhook-specific item.
- **IP whitelist helpers** — wait for first user (Telegram,
  DataDog). Not listed above because the need is speculative.
- **JWT path auth** (Salesforce-style) — wait for first user.
- **mTLS support** — wait for first enterprise user.
