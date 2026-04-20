---
id: 0022
title: webhook-signature-policy
status: accepted
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [security, webhook, action, trigger, auth, hmac]
related:
  - docs/audit/2026-04-19-codebase-quality-audit.md
  - docs/adr/0020-library-first-gtm.md
  - docs/adr/0005-trigger-health-trait.md
  - crates/action/src/webhook.rs
  - crates/api/src/webhook/transport.rs
linear: []
---

# 0022. Webhook signature policy — `Required` by default

## Context

`nebula-action` ships four constant-time HMAC primitives in
[`crates/action/src/webhook.rs:972+`](../../crates/action/src/webhook.rs#L972):
`verify_hmac_sha256` (GitHub), `verify_hmac_sha256_base64`
(Shopify / Square), `verify_hmac_sha256_with_timestamp` (Stripe / Slack
replay window), and the `hmac_sha256_compute` / `verify_tag_constant_time`
pair for bespoke schemes. They are careful — timing-invariant decode,
strict single-valued header lookup, fail-closed on empty secret — and
already audited by `security-lead`.

The primitives are correct. The **trait surface is wrong**.

Today `WebhookAction::handle_request` has no opinion on signatures: an
author who forgets to call the verify primitive ships a webhook that
trusts every POST that happens to hit its `(uuid, nonce)` URL. The audit
([`docs/audit/2026-04-19-codebase-quality-audit.md`](../audit/2026-04-19-codebase-quality-audit.md))
flagged this as a HIGH-priority `security-lead` finding under the
"security-lead — secret/auth surface" subsection:

> `WebhookTrigger` lacks a `signature_policy()` contract — primitives
> exist in `crates/action/src/webhook.rs:972+` (constant-time tag
> compare), but enforcement is opt-in. Authors who forget the verify
> call ship unsigned webhooks behind discoverable URLs. Need `Required`
> default at the trait level.

The guard-rails section on the same document adds:

> **Webhook signature `Required` by default** before the URL space is
> advertised in any deployment doc.

That is a pre-condition for [ADR-0020](0020-library-first-gtm.md) —
once `apps/server` exists, webhook URLs start leaking into
third-party provider dashboards (GitHub repo settings, Slack app
manifests, Stripe endpoint lists). By then the URL space is public
and an "oh, we'll turn it on later" migration is impossible without
breaking every existing integration.

The mechanical transport already exists — see
[`crates/api/src/webhook/transport.rs:257`](../../crates/api/src/webhook/transport.rs#L257)
(`webhook_handler`). The audit named that file as the place that
"trusts every (uuid, nonce)-matching POST" today. The enforcement hook
is a single insertion between request construction and dispatch.

## Decision

1. **Add `SignaturePolicy` to `WebhookAction`.** The enum lives in
   [`crates/action/src/webhook.rs`](../../crates/action/src/webhook.rs)
   and is returned by a new trait method on `WebhookAction`:

   ```rust
   pub trait WebhookAction: Action + Send + Sync + 'static {
       // ... existing items ...
       fn signature_policy(&self) -> SignaturePolicy {
           SignaturePolicy::default()   // == Required(RequiredPolicy::default())
       }
   }
   ```

   ```rust
   pub enum SignaturePolicy {
       /// Require HMAC-SHA256 via the configured header and scheme.
       /// Default: `X-Nebula-Signature`, Sha256Hex, empty secret
       /// (fail-closed — transport returns 500 until an author supplies a secret).
       Required(RequiredPolicy),
       /// Explicit opt-out. Accepts any request, signed or not.
       /// Use only for public-by-design webhooks or local testing;
       /// the override itself is the audit trail.
       OptionalAcceptUnsigned,
       /// Escape hatch for schemes the standard verifiers do not cover
       /// (Stripe-style timestamped HMAC, bespoke canonicalisation).
       Custom(Arc<dyn Fn(&WebhookRequest) -> SignatureOutcome + Send + Sync>),
   }
   ```

   `RequiredPolicy` carries three fields — `secret: Arc<[u8]>`, `header:
   HeaderName`, `scheme: SignatureScheme` — with a builder-style API
   (`with_secret`, `with_header`, `with_scheme`). `SignatureScheme` is
   `Sha256Hex | Sha256Base64`; the hex variant accepts both bare and
   `sha256=`-prefixed forms (GitHub convention).

2. **`Required` is the default, with an empty secret that fails closed.**
   An action that does not override `signature_policy()` inherits
   `SignaturePolicy::Required(RequiredPolicy::default())` — an empty
   secret. The transport treats an empty secret the same as a missing
   credential: it returns **500** with a clear problem+json
   `detail` and does NOT call `handle_event`. This preserves the
   "Authors who forget the verify call" class of bug as an observable
   server failure rather than silent acceptance.

3. **`OptionalAcceptUnsigned` is the escape valve for legitimate
   unsigned webhooks.** Public-by-design webhooks (Slack's URL
   verification handshake, public repo webhook for an OSS tool) and
   local testing both exist. The author writes:

   ```rust
   fn signature_policy(&self) -> SignaturePolicy {
       // Public webhook — this endpoint accepts anonymous POSTs by design.
       SignaturePolicy::OptionalAcceptUnsigned
   }
   ```

   The override in source, plus the doc-comment justification, is the
   audit trail. Reviewers look for it; no other mechanism is required.

4. **`Custom` covers everything else.** Stripe's `t=…,v1=…` with a
   timestamp window, Shopify's base64 with a derived payload, and
   future schemes we have not seen all fit through the same
   `Arc<dyn Fn(&WebhookRequest) -> SignatureOutcome + Send + Sync>`
   shape. The custom verifier calls the existing
   `verify_hmac_sha256_with_timestamp` / `hmac_sha256_compute` /
   `verify_tag_constant_time` primitives — the trait shape carries no
   new crypto.

5. **Transport enforcement lives in
   [`crates/api/src/webhook/transport.rs`](../../crates/api/src/webhook/transport.rs).**
   The `TriggerHandler` trait grows a default `fn
   signature_policy(&self) -> Option<&SignaturePolicy> { None }`.
   `WebhookTriggerAdapter` stores the policy at construction (reads it
   once from the wrapped action) and overrides the method to return
   `Some`. The transport consults it between `WebhookRequest::try_new`
   (step 5 in the existing handler) and oneshot setup (step 6):

   | Policy / outcome                               | HTTP status                                        |
   |---|---|
   | `OptionalAcceptUnsigned`                       | pass through to `handle_event`                     |
   | `Required` with empty secret                   | **500** `application/problem+json`                 |
   | `Required`, signature mismatch / missing / invalid | **401** `application/problem+json`             |
   | `Custom` verifier returns `Valid`              | pass through to `handle_event`                     |
   | `Custom` verifier returns `Missing` / `Invalid`| **401** `application/problem+json`                 |

   The 401 / 500 responses reuse the existing `ProblemDetails` shape
   from [`crates/api/src/errors.rs`](../../crates/api/src/errors.rs);
   they are **not** `WebhookHttpResponse`. An action that has a
   `Required` policy is never given the chance to observe an unsigned
   request — there is no in-handler code path to forget.

6. **Canonical default: `X-Nebula-Signature: sha256=<hex>`.** An author
   who sets `SignaturePolicy::Required(RequiredPolicy::default()
   .with_secret(&self.secret))` gets a working signed webhook without
   touching header names or schemes. Providers bring their own
   conventions (`X-Hub-Signature-256` for GitHub, `X-Shopify-Hmac-SHA256`
   for Shopify); those authors override the header explicitly via
   `.with_header(...)`.

7. **Metric:
   [`NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`](../../crates/metrics/src/naming.rs)
   with a low-cardinality `reason` label.** Three reason values,
   all static strings so test and call sites can compare without
   stringifying twice — `missing` (header absent), `invalid` (present
   but mismatched / unparsable), `missing_secret` (`Required` policy
   with empty secret at activation / verification time). The reason
   set is intentionally small so no per-trigger label is needed; the
   counter rolls up across the deployment and any non-zero value is
   an operational signal worth a dashboard alert.

## Consequences

**Positive**

- The "forgot to call `verify_hmac_sha256`" class of bug is eliminated
  at the trait level. An author writing a webhook from scratch starts
  in the fail-closed state and has to deliberately opt out.
- Existing authors who already call the verify primitive inside
  `handle_request` continue to work unchanged — the transport refuses
  the unsigned POST before dispatch. Their in-handler verify becomes
  defence-in-depth.
- Transport-level enforcement means the signature-failure metric
  rolls up cleanly across all webhook triggers; operators see one
  counter, not N action-specific ones.
- Fits into the existing `ProblemDetails` shape
  ([`crates/api/src/errors.rs`](../../crates/api/src/errors.rs)), so
  the 401 / 500 responses look identical to every other API error to
  a provider's dashboard tooling.
- `Custom` keeps the trait surface small while accommodating
  Stripe / Slack schemes — we do not need to grow the enum every time
  a provider ships a new signature shape.

**Negative**

- `WebhookAction` grows a new trait method. Pre-v1 — acceptable per
  `docs/STYLE.md` — but it is a deliberate public-surface addition
  that later consumers have to know about. Default impl covers the
  "never touched it" case; only authors who override need to read the
  docs.
- The `Arc<dyn Fn>` in `Custom` cannot derive `Debug`; we hand-
  implement `fmt::Debug` with a placeholder (`"SignaturePolicy::Custom(..)"`)
  so nothing leaks and the variant is still visible in logs.
- Secret material now lives on `RequiredPolicy` inside the adapter
  (as `Arc<[u8]>`), not only inside the action's `&self`. That is the
  price of lifting verification out of the handler. The adapter
  never logs it; `Debug` on `RequiredPolicy` redacts to
  `"RequiredPolicy { secret: [<redacted len=N>], header: ..., scheme: ... }"`.

**Neutral**

- Replay protection (timestamp window + nonce store) is **out of
  scope**. The trait shape supports it via `Custom`, and
  `verify_hmac_sha256_with_timestamp` is already on the menu for
  per-action use. A generic replay store (nonce cache, window policy)
  is a separate ADR.
- This ADR does not introduce a deploy-time toggle. Signature policy
  is per-trigger, declared in the action source. A global toggle
  would defeat the audit-trail intent — it only takes one env var flip
  to silently weaken every webhook in a fleet.

## Alternatives considered

- **Keep enforcement opt-in; document the verify-call requirement in
  `WebhookAction` docs more loudly.** Reject. Docs do not prevent
  bugs; the audit found the gap precisely because the primitives are
  already documented and still not universally called. Enforcement
  has to be on the dispatch path, not on the author's memory.
- **Fold signature verification into `WebhookAction::handle_request`
  via a blanket impl wrapper.** Reject. The wrapper would need to
  know the secret at trait-impl time, which is the same trait-method
  addition in a less honest shape. Also: a blanket impl hides the
  enforcement from reviewers; a dedicated `signature_policy()` method
  surfaces it where it belongs.
- **Global per-deployment toggle (`NEBULA_WEBHOOK_REQUIRE_SIG=1`).**
  Reject. The audit's whole point is that signature decisions are
  per-trigger — a public-by-design webhook and a signed GitHub
  webhook co-exist in the same deployment. A global toggle gets
  flipped off "just for this test" and forgotten.
- **Strongly-typed variants per provider (`GitHubPolicy`,
  `StripePolicy`, `ShopifyPolicy`).** Reject. The combinatorial
  explosion is large (every major SaaS has its own scheme) and
  ages badly (providers rotate conventions — Stripe moved from v0 to
  v1, Slack from v0 to v0 with signed timestamp). `Custom` + the
  existing primitives give the same coverage without trapping us in
  a taxonomy.
- **Put `signature_policy` on `TriggerAction`, not `WebhookAction`.**
  Reject. Poll / cron / queue triggers have no HTTP surface; a
  required method on the broader trait would be `None`-returning
  boilerplate in every non-webhook adapter. The method lives where
  its semantics apply.
- **Use `SecretString` instead of `Arc<[u8]>` for the stored secret.**
  Defer. `Arc<[u8]>` keeps the policy `Clone` cheap and avoids a
  `nebula-credential` dep from `nebula-action` (layer boundary —
  action is Business, credential is Business, but action must not
  pull credential's `SecretString` in for a single field in a pre-v1
  crate). The adapter redacts in `Debug` and the bytes never travel
  out of transport-layer calls. If credential material grows a
  first-class `SecretBytes` shape that avoids the layer pull, revisit.

## Follow-ups

- Replay store ADR (nonce cache + timestamp window policy). Stripe
  and Slack already need it; the trait shape is ready via `Custom`,
  the infrastructure is not.
- When the `KeyProvider` ADR lands (TBD at audit time), the
  `RequiredPolicy` secret source becomes a natural candidate for
  migration — rather than the action owning the bytes, the policy
  holds a handle that resolves against the engine-owned credential
  store. That is the shape this ADR wants to allow without forcing.
- `docs/audit/2026-04-19-codebase-quality-audit.md` "Open ADRs needed"
  table: flip the webhook-signature row from TBD to this ADR ID in
  the same PR that lands the implementation.
- `docs/MATURITY.md` `nebula-action` row: tighten the webhook-sig
  note from `partial (webhook sig covered; …)` to a wording that
  reflects default-Required enforcement.
