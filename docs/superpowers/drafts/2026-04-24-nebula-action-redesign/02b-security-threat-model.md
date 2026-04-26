# Phase 1 — Security threat model for `nebula-action`

**Date:** 2026-04-24
**Agent:** security-lead
**Scope:** threat-model-only, no fixes proposed (Phase 2+). Reference canon §4.2 safety + §12.5 secrets-and-auth + §12.6 isolation honesty.
**Inputs:** Phase 0 [`01-current-state.md`](./01-current-state.md) (C1–C4, S1–S6, T1–T9).

---

## 1. Threat model scope + methodology

**Scope.** Action execution surface of `nebula-action`:
- Credential resolution (`CredentialContextExt::{credential, credential_typed, credential_by_id}`) and the `CredentialGuard<S>` RAII wrapper.
- Cancellation behavior of `StatelessActionAdapter`, `StatefulActionAdapter`, `WebhookTriggerAdapter`, `PollTriggerAdapter`.
- Output pipeline through `ActionOutput` / `BinaryData` / `DeferredOutput` / `StreamOutput` / `DataReference`.
- Webhook signature-verification primitives (`verify_hmac_sha256*`, `hmac_sha256_compute`, `verify_tag_constant_time`), `SignaturePolicy` + `RequiredPolicy`, `DEFAULT_MAX_BODY_BYTES` / `MAX_HEADER_COUNT`.
- `IsolationLevel` boundary (metadata field → engine dispatch).
- Unsafe-code posture (`#![forbid(unsafe_code)]` vs transitive crypto deps).

**Threat actors in scope.**
1. **Malicious plugin installed by legitimate tenant** — full execution inside action handler; attempts to reach another tenant's credentials, poison outputs, or leak secrets via logs/panics.
2. **Network attacker controlling webhook payload** — depth bombs, size bombs, signature timing attacks, replay.
3. **Supply-chain actor landing one PR** — dead feature flag abuse, small attribute-macro tricks.
4. **Compromised worker process** — what's in memory after a cancelled future, what survives a panic.
5. **Log aggregator with broader read access** — what ends up in `Debug` / error strings / tracing fields.

**Out of scope per prompt.** Cryptographic redesigns (Phase 2+), idiomatic Rust review (rust-senior), architectural coherence (tech-lead), authoring-friction findings (dx-tester).

**Methodology.** Code-read every cited path; verify Phase 0 claims by reading source (not trusting audit summaries); map each attack surface to at least one concrete attacker-actor scenario before assigning severity.

---

## 2. Credential resolution path — leak + collision analysis

**Code read:** `crates/action/src/context.rs:563-689`; `crates/credential/src/secrets/guard.rs` (full); `crates/credential/src/snapshot.rs:79-256`.

### 2.1 `CredentialGuard<S>` construction and Drop

`CredentialGuard<S: Zeroize>` (`guard.rs:34-62`) has the load-bearing security invariants:
- `Deref<Target = S>` — transparent read access.
- `Drop` calls `self.inner.zeroize()` — verified by `drop_zeroizes_inner` test (`guard.rs:143-168`).
- `fmt::Debug` delegates to `nebula_core::guard::debug_redacted` — confirmed "REDACTED" in output.
- **No `Serialize`/`Deserialize` impl** — by construction, cannot accidentally land in `ActionOutput`.
- `Clone` exists for `S: Zeroize + Clone` (`guard.rs:64-71`). Each clone is a new zeroize point. **This is a defense-in-depth gap** — an action that `let c2 = cred.clone()` now has two objects that must both drop cleanly for the plaintext to leave memory.

🟡 **MINOR S-C1** — `CredentialGuard::clone` increases the zeroize-point count without an audit-log trail. No attack scenario makes this exploitable today, but if an action leaks a clone into a long-lived `Arc` (e.g., caching inside a `stateless_ctx_fn` closure capture), the original guard zeroizes on Drop but the clone lives until the closure itself drops. The `must_use` annotation on the struct nudges away from this but does not prevent it.

### 2.2 The type-name-lowercase key heuristic (C3 confirmation)

`context.rs:641-667`:
```rust
let type_name = std::any::type_name::<S>();
let short_name = type_name.rsplit("::").next().unwrap_or(type_name);
let key_str = short_name.to_lowercase();
```

**Confirmed as written in Phase 0.** Collision + shadow analysis:

- **Collision**: `plugin_a::OAuthToken` and `plugin_b::oauth::OAuthToken` both map to key `"oauthtoken"`. Whichever credential the engine registered first under that key is what both plugins resolve.
- **Cross-tenant**: if two independently-authored plugins ship types sharing a short name, and the same workflow composes both actions, credential A written for plugin A is handed to plugin B. Concrete scenario: two integrations each define `struct BearerToken { token: SecretString }` in their own modules. Plugin A (Stripe) expects the Stripe key; plugin B (GitHub) queries `ctx.credential::<BearerToken>()` and receives Stripe's token because of the shared short name. **Both types project to the same `AuthScheme`** (bearer) so `downcast::<CredentialSnapshot>` succeeds and `into_project::<S>()` returns the wrong secret wrapped in the caller's type.
- **Information leak**: `type_name::<S>()` output includes the *full module path* (e.g., `nebula_integrations::stripe::v2::internal::StripeCredV2`) — if the action returns `ActionError::fatal(format!("credential type mismatch for `{type_name}`: {e}"))` (lines 660, 664), the *full path* lands in the error surface that propagates to the engine's structured log. For a closed-source plugin this leaks module structure to anyone with log read access.
- **Downcast-fail path leaks no secret** (good): `snapshot.into_project::<S>()` returns an error; the secret stays inside the boxed snapshot which is then dropped.

🔴 **CRITICAL S-C2** — cross-plugin credential shadow. **Attack scenario**: malicious plugin B registers a credential type `pub struct SlackToken;` in `malicious_plugin::SlackToken`. Legitimate plugin A uses the same short name via `official::slack::SlackToken`. Workflow composes both. Depending on registration order, plugin B's `ctx.credential::<SlackToken>()` call returns plugin A's configured credential — full token exfiltration path. Severity elevated over Phase 0's 🔴 C3 rating because the attack requires only *type-name collision*, not code injection. **Exploitable today** unless the engine happens to apply a cross-plugin registration guard that blocks same-short-name keys (verified: no such guard — `resolve_any(CredentialKey::new(&key_str))` reaches whatever store responds to that key first).

🟡 **MINOR S-C3** — module path leaks to error channel at `context.rs:660, 664`. Part of the redesign (either C3 fix lands, method is deprecated, or the error is sanitized before propagation).

### 2.3 `CredentialSnapshot` downcast / projection

`snapshot.rs:247-255` — `Debug` impl renders projected field as `"[REDACTED]"`. `Clone` impl (`snapshot.rs:235-245`) delegates to a `clone_fn` pointer captured at construction — the projected value is cloned preserving type erasure; no secret leaks via the clone path.

`CredentialContextExt::credential_typed` and `credential_by_id` are the spec-aligned paths (explicit key). Safe.

✅ **GOOD** — `CredentialSnapshot::Debug` and `CredentialGuard::Debug` both redact. `CredentialGuard` is explicitly `!Serialize`. Do not remove these guarantees in the redesign.

---

## 3. Cancellation + zeroize drop-order discipline

**Code read:** `webhook.rs:1208-1319`; `poll.rs:1332-1420`; `engine/runtime/runtime.rs:570-583`; `stateless.rs:359-386`; `stateful.rs:546-627`; test `dx_webhook.rs:377-424`.

### 3.1 What the runtime does on cancellation

The engine wraps the handler future in `tokio::select!` with `biased; cancellation → Err(Cancelled)` (verified at `runtime.rs:576-582` for stateful, `webhook.rs:1266-1299` for webhook). On cancel, the `exec_fut` is **dropped** — standard tokio `select!` semantics.

**Drop order inside a cancelled future's locals** follows normal Rust rules: locals in the function body drop in reverse declaration order. `CredentialGuard<S>` declared in the action body will call its `Drop` impl when the future is dropped, invoking `zeroize()`.

**But this is only guaranteed for stack locals held by the awaiting future.** Three cases where zeroize is **not** guaranteed:

1. **Credential moved into a detached `tokio::spawn`** — if an action writes:
   ```rust
   let cred = ctx.credential::<BearerToken>().await?;
   tokio::spawn(async move { /* ... uses cred ... */ });
   return Ok(ActionResult::success(()));
   ```
   Cancellation of the parent future does *not* cancel the spawned task. The token lives until the detached task completes or the runtime shuts down. No zeroize until then. **No lint / no compile-error prevents this.**

2. **Credential moved into an `Arc`** — if an action caches the guard in a shared `Arc<CredentialGuard<_>>` (via `Clone`, since `CredentialGuard: Clone` when `S: Clone`), cancellation drops *one* strong ref; zeroize runs only when the last ref drops.

3. **Credential placed in `ActionOutput`** — guarded against by `!Serialize`. Good.

🟠 **MAJOR S-C4** — no compile-time enforcement against `CredentialGuard` being moved into a `'static` future. **Attack scenario**: malicious plugin spawns a detached task holding a guard, then deliberately panics the main handler future. The runtime cancels/drops the main future; the detached task keeps the credential live in memory. A compromised worker post-cancel then has plaintext in memory the scheduler thinks was zeroized. Fix direction (for Phase 2+): `!Send + !Sync` on guard, or phantom lifetime keying back to the `ActionContext`. Not proposing now per prompt.

### 3.2 Is there a cancellation test for credential zeroize?

Searched `crates/action/tests/` for patterns connecting cancellation to credential zeroize: **no match**. `dx_webhook.rs:377` tests that a cancelled `handle_request` returns cleanly and sends a 503 — that's the operational correctness test, not a drop-order / memory-zeroization test.

🟡 **MINOR S-C5** — no test asserts that `CredentialGuard::Drop` actually fires when the holding future is cancelled. `guard.rs::drop_zeroizes_inner` covers the *explicit drop* case only. Defense-in-depth gap — if the compiler ever re-orders drop semantics across an await point (implausible but non-contracted), the regression would ship silently.

### 3.3 Webhook cancellation shape — positive

`webhook.rs:1261-1298` properly uses `biased; ctx.cancellation().cancelled()` as the *first* branch, sends 503 Service Unavailable, records health error, returns retryable. `InFlightGuard` RAII decrement. Good cancellation discipline.

✅ **GOOD** — webhook cancellation sends 503 on the oneshot channel before returning, so transport does not hang on `RecvError`. Do not lose this in redesign.

### 3.4 `PollTriggerAdapter` cancellation

`poll.rs:1337-1418` — pre-poll check + cancel-aware sleep. Good.

Note on stateful loop `runtime.rs:617-623`: sleep itself is cancel-aware via `tokio::select!`. Good.

---

## 4. Output sanitization at adapter boundary

**Code read:** `output.rs:510-805`; `stateless.rs:370-386`; `stateful.rs:560-624`; `webhook.rs:1261-1318`.

### 4.1 `BinaryData` — no size limit at adapter

`BinaryData::effective_size` (`output.rs:781-786`) is the authoritative size accessor; comment at `output.rs:759-766` explicitly warns that `size` field can be *falsified* out of sync with inline byte length ("can be bypassed by passing oversize inline bytes with a falsified `size`"). This is a **documented footgun**, not a latent bug — but the adapter itself does **no size enforcement** before wrapping output into `ActionOutput::Binary`.

**Attack scenario**: malicious plugin returns `ActionOutput::Binary(BinaryData { content_type: "image/png", data: BinaryStorage::Inline(vec![0; 1_000_000_000]), size: 1024, metadata: None })`. 1 GB stays live in the engine's process memory (adapter just serializes). Downstream consumers relying on `size` instead of `effective_size()` misjudge memory pressure. The crate carefully documents the footgun; consumers are expected to call `effective_size()`. Whether the engine and downstream nodes actually do is not this crate's concern but is worth auditing in Phase 2.

🟠 **MAJOR S-O1** — `StatelessActionAdapter::execute` does not enforce a per-output size cap. Action output flows through `serde_json::to_value(output)` (line `stateless.rs:381`) which happily serializes arbitrarily large `BinaryStorage::Inline` into a JSON `{"Inline": [...]}` array — billion-byte arrays land as JSON numbers one at a time. Memory + CPU DoS vector. The webhook surface has `DEFAULT_MAX_BODY_BYTES = 1 MiB` for *input*; nothing symmetric on *output*. Mitigates somewhat: `DataReference` + `Stored` are designed paths to push large bytes out-of-band.

### 4.2 `StreamOutput::buffer` — unbounded-by-default

`BufferConfig::capacity: usize` (`output.rs:284-288`) — no enforced upper bound at adapter boundary. Author-specified. An action that returns `StreamOutput { buffer: Some(BufferConfig { capacity: usize::MAX, on_overflow: Overflow::Block }), .. }` lets a producer-side attacker pin unbounded memory. The `Overflow::Block` vs `DropOldest` vs `DropNewest` vs `Error` distinction is meaningful; `Block` with `capacity: usize::MAX` is the worst case.

🟡 **MINOR S-O2** — no per-crate ceiling on `BufferConfig::capacity`. Not exploitable without engine cooperation (consumer must allocate), but defense-in-depth suggests a crate-level MAX_BUFFER_CAP constant validated at adapter boundary.

### 4.3 `DeferredOutput::resolution` — unbounded URL / endpoint fields

`Resolution::Callback { endpoint: String, token: String }` (`output.rs:125-130`) — no length cap, no URL validation, no token-format validation. Untrusted plugin output can emit `endpoint: "http://attacker.internal"` or a token containing control characters that downstream logging fields may not escape.

🟡 **MINOR S-O3** — `Resolution::Callback::endpoint` / `Poll::url` / `SubWorkflow::workflow_id` are all raw `String`s. If downstream nodes log them unredacted (plausible for a `Callback` endpoint), an attacker-controlled plugin places arbitrary text in log aggregator output — log injection vector if structured logging is not strict about field escaping.

### 4.4 The `stateful` adapter's tracing::error!

`stateful.rs:609-615` emits `tracing::error!(action = %..., serialization_error = %ser_err, action_error = %action_err, "...")` when state serialization fails. The `action_error` is `ActionError`'s `Display`. If an action author constructs `ActionError::fatal(format!("API returned {token}"))`, the token flows through `%action_err` to logs. This is a credential-leak path that the error-construction rule at §12.5 (“no secrets in error strings”) is supposed to cover, but **nothing in the adapter validates it**.

🟡 **MINOR S-O4** — `stateful.rs:609` trusts `ActionError::Display` to be secret-free. The contract is stated but not enforced. Redesign cascade could fix by requiring `ActionError::Display` to sanitize, or by requiring typed error variants whose `Display` impl is audited.

---

## 5. Webhook signature verification (HMAC + timing + replay + size limits)

**Code read:** `webhook.rs:78-206, 1329-1789`; `api/src/services/webhook/transport.rs:31-467` (signature enforcement).

### 5.1 Constant-time posture — correct

`webhook.rs:1482, 1508, 1787` — verification delegates to `hmac::Mac::verify_slice` (internally uses `subtle::ConstantTimeEq`) and the bare `verify_tag_constant_time` uses `a.ct_eq(b).into()`. Both are RFC-safe constant-time.

✅ **GOOD** — all four verifiers (`verify_hmac_sha256`, `verify_hmac_sha256_base64`, `verify_hmac_sha256_with_timestamp`, `verify_tag_constant_time`) use constant-time compare. Do not regress.

### 5.2 Timing-invariant decode — correct

`webhook.rs:1465-1489` (hex) and `1496-1515` (base64): on decode failure, substitutes a zero-filled 32-byte expected tag and still runs the full MAC computation, so a decode error takes the same time as a valid-hex/base64 mismatch. Good discipline (H9 referenced in comments).

✅ **GOOD** — timing-invariant decode on both hex and base64. Do not regress.

### 5.3 Replay window — correct but narrow

`verify_hmac_sha256_with_timestamp` (`webhook.rs:1669-1737`):
- Reads `request.received_at()` (set by transport at construction), not wall clock. **Deterministic for persisted event replay**, good.
- Accepts `[received − tolerance, received + 60s]` — forward-skew cap is hardcoded 60s.
- Rejects timestamp parse failure / clock skew → `Invalid`.
- On future > 60s, rejects. On past > tolerance, rejects.

**Observations**:
- `received_secs as i64` (`1717`) — `u64` → `i64` cast of seconds since epoch. Safe until year 2554. Acceptable.
- `tolerance.as_secs() as i64` (`1721`) — if `Duration::from_secs(i64::MAX + 1)` were passed, the cast wraps. Caller-controlled tolerance; a sloppy caller could pass `Duration::MAX` and get a wrap. Not attacker-reachable — tolerance is action-code-controlled, not request-controlled.

🟡 **MINOR S-W1** — `FUTURE_SKEW_SECS: i64 = 60` (`webhook.rs:1726`) is hardcoded. For providers with clock drift > 60s (rare on NTP-synced infra, non-rare in IoT/edge), valid signatures are rejected as `Invalid`. Defense-in-depth vs availability — fail-closed is correct for security, but for a future design this could be configurable with a low ceiling (e.g., up to 300s).

### 5.4 `DEFAULT_MAX_BODY_BYTES` enforcement

`webhook.rs:82-91, 187-207` — 1 MiB default enforced at `WebhookRequest::try_new` / `try_new_with_limits`. Fields are private; the only construction path goes through these. Good.

`MAX_HEADER_COUNT = 256` at `webhook.rs:103, 198-206`. Also enforced. Good.

✅ **GOOD** — size/header limits enforced at the only construction path; no bypass because `WebhookRequest` fields are private. Reminder in redesign: do not expose a public constructor that skips `try_new_with_limits`.

### 5.5 `SignaturePolicy::None` — does not exist

The prompt asks "what if `SignaturePolicy::None` is set by a misconfigured plugin?" — read the enum at `webhook.rs:737-751`:
```rust
pub enum SignaturePolicy {
    Required(RequiredPolicy),
    OptionalAcceptUnsigned,
    Custom(Arc<dyn Fn(&WebhookRequest) -> SignatureOutcome + Send + Sync>),
}
```
There is no `None` variant. `OptionalAcceptUnsigned` is the explicit opt-out. Default is `Required` with empty secret (fail-closed): transport returns 500 until the author sets a real secret (`transport.rs:362-371`).

However: **`Custom(Arc<dyn Fn>)` is the escape hatch.** A plugin returning `SignaturePolicy::custom(|_| SignatureOutcome::Valid)` bypasses all signature enforcement. The `Custom` closure is uninspectable from transport side — transport trusts whatever `SignatureOutcome` it returns.

🟠 **MAJOR S-W2** — `SignaturePolicy::Custom` is an unbounded trust delegation. A malicious plugin author authoring `SignaturePolicy::custom(|_| SignatureOutcome::Valid)` accepts any unsigned request while *declaring it uses a custom verifier*. From the outside (audit surface: registry metadata, plugin.toml) there's no visible difference between a real Stripe `t=…,v1=…` verifier and a sham pass-through. The `OptionalAcceptUnsigned` variant is designed to be the audit-trail surface for "this webhook accepts unsigned" — `Custom` defeats that.

**Attack scenario (supply chain):** PR lands a plugin with `SignaturePolicy::custom(verify_v1)` where `verify_v1` is a one-line closure that reviewers glance over. Weeks later, a malicious PR changes `verify_v1` internals to `|_| SignatureOutcome::Valid`. The `SignaturePolicy` variant is still `Custom` — the static shape (`Debug` output, `SignaturePolicy::Custom(..)` per line 774) gives reviewers no signal. Phase 2+ may want a verifier-source-attestation (closure name / sha) or constrain `Custom` to composed primitives.

### 5.6 `hmac_sha256_compute` has no empty-secret check

`webhook.rs:1751-1764` — pure primitive, explicitly documented as escape hatch. A caller using this with an empty `&[]` secret computes a deterministic MAC. All `verify_*` entry points DO guard against empty secret. A plugin composing `hmac_sha256_compute(&[], body)` is choosing to accept any signature.

🟡 **MINOR S-W3** — `hmac_sha256_compute` escape hatch is correctly documented as "build your own"; empty-secret is the caller's responsibility. Defense-in-depth: a `debug_assert!(!secret.is_empty())` would catch author bugs in test but not in release. Not blocking.

### 5.7 `single_header_value` — correct

`webhook.rs:1429-1441` rejects proxy-chain duplicate headers as `Multiple`, which `verify_*` convert to `SignatureOutcome::Invalid`. Good — closes H3 (proxy-chain slot).

✅ **GOOD** — strict single-valued header check prevents proxy-chain duplicate-header attack.

### 5.8 `Stripe-Signature` custom helper is NOT provided (intentional)

Documented at `webhook.rs:41-48` — Stripe/Slack authors must compose from primitives. Good — avoids offering a half-correct helper that drops tolerance or clock-source discipline.

---

## 6. JSON depth / deserialization attack surface

**Code read:** `stateless.rs:370-386`; `stateful.rs:561-582`; `webhook.rs:299-349, 1367-1413`.

### 6.1 `StatelessActionAdapter::execute` — no depth cap (C4 confirmation)

`stateless.rs:370`: `let typed_input: A::Input = serde_json::from_value(input).map_err(...)`. **Confirmed no depth cap.** Input originates upstream (engine handed-down `serde_json::Value`), so the question is: **can untrusted JSON reach this adapter without passing through a limit-applying intermediary?**

Tracing upstream:
- Engine → `handler.execute(input, context)` at `runtime.rs:434` with input typed as `serde_json::Value`.
- Engine receives input from several sources: prior-node output (action-controlled), workflow input (user-controlled at API boundary), trigger event (webhook path applies its own 1 MiB body cap but forwards raw JSON upward to downstream actions).

**Concrete path for untrusted deep JSON to reach a stateless action**: webhook trigger action uses `req.body_json_bounded(64)` (good) and emits the parsed value as `TriggerEventOutcome::Emit(payload)`. Payload flows through engine → connected downstream `StatelessAction::Input`. But wait: once `body_json_bounded` has produced a `Value`, the `Value` itself carries the already-reduced structure; downstream stateless adapter's `from_value(input)` operates on already-parsed `Value` (serde_json::Value is tree-shaped, depth is already bounded by the upstream `body_json_bounded`).

**BUT** — this protection chain holds only if *all* trigger/input sources pre-apply depth bounds. A poll trigger action that does `serde_json::from_slice(response_body)` without depth checking is a hole. An API endpoint that accepts user-submitted workflow-input JSON without depth check is a hole.

🔴 **CRITICAL S-J1** — stack overflow via deep JSON is exploitable if *any* upstream path reaches `StatelessActionAdapter::execute` without a depth cap. **Attack scenario**: tenant POSTs a manual workflow execution with input body `{"a":{"a":{...100000 deep...}}}`. API deserializes to `serde_json::Value` (depth-unbounded because `serde_json` has no default depth cap). Engine passes the value to the first stateless action. `from_value` attempts to deserialize into the typed `Input` struct and recurses at each level — stack overflows the 2 MiB tokio worker stack. Worker dies. Per `engine.rs` cascade behavior, dying a worker propagates to adjacent work.

Phase 0 rates this 🔴 C4; I confirm the rating. Fix (Phase 2+) is to either (a) apply `serde_stacker` or an equivalent at adapter boundary, or (b) push the depth check to the `serde_json::Value` construction site (API handler).

### 6.2 `StatefulActionAdapter` — same hole + state also unbounded

`stateful.rs:561-582` — both `input.clone()` and `state.clone()` pass through `serde_json::from_value` without depth cap. State is less adversarial (persisted; presumably trusted) but checkpoint corruption by a compromised storage backend would then land user-controlled deep-JSON at adapter boundary.

🟠 **MAJOR S-J2** — stateful adapter's state deserialization is also depth-unbounded. Depth-bomb state could be landed by: (a) a storage compromise, (b) a prior adapter iteration that produced a deeply-nested state (author bug). Secondary impact; flagged because the fix is symmetric to S-J1 and both should be addressed together.

### 6.3 `webhook.rs` depth-scan implementation

`check_json_depth` (`webhook.rs:1378-1413`) is a pre-scan byte loop — single pass, no recursion, handles string escapes. **Looks correct.** Tracks `{`/`[` opens, ignores depth inside JSON strings, treats `\` as an escape toggle. One edge case I want to double-check: does a string with odd escapes desync the state?

Tracing: `escape` flag is only set inside `in_string`. If `in_string = true` and byte is `\\`, next byte is skipped (any char inside escape context). If the string ends with `"` (not escaped), `in_string = false`. This handles `"\\\\"` correctly. The comment at `webhook.rs:1374-1377` explicitly disclaims full JSON validation — it's a conservative upper-bound depth scan. **Safe.**

✅ **GOOD** — `body_json_bounded` provides a real depth guard that does not require parsing the whole document into `Value` first. Keep this pattern and consider extending it to `StatelessActionAdapter`.

---

## 7. Type-name-as-key information leak + collision vectors

Covered in §2.2 (S-C2, S-C3). Additional notes specific to §15 of canon:

- `std::any::type_name::<S>()` is **not stable** across Rust compiler versions by spec; debug-only in intent. Any protocol keying off its output has an implicit coupling to compiler minutiae. `rsplit("::").next()` is the current stabilization of that, but the keyed heuristic is brittle — a future Rust release that adds generics disambiguation to `type_name` output could shift all keys silently.
- `short_name.to_lowercase()` uses Unicode default case folding. A type `ıToken` (with Turkish dotless i) lowercases to `ıtoken` (not `itoken`). If multiple plugins happen to use locale-dependent names, collisions are locale-sensitive. **Theoretical**, not seen in practice, but exemplifies why type-name → string → key is fragile.

🟡 **MINOR S-C6** — locale-dependent `.to_lowercase()` on type names. Depending on compiler version and locale, same source name can produce different keys. Redesign Option A (`CredentialRef<C>` with const `C::KEY`) eliminates this entirely.

---

## 8. Feature flag discipline (retry-scheduler on-but-unwired)

**Code read:** `crates/action/Cargo.toml:14-20`; `crates/engine/src/engine.rs:1877-1910`.

Phase 0 T2 rated 🟠 but asks: is the on-but-unwired state exploitable?

**Engine behavior verified at `engine.rs:1889-1910`**: the engine detects `Retry` variants via the always-compiled `ActionResult::is_retry()` predicate (not cfg-gated). When a handler returns `Retry`, engine converts it to `ActionError::retryable("Action retry is not supported by the engine")` and processes it through the normal failure-routing pipeline — classify → recovery → route → checkpoint → emit.

**Action code cannot "silently succeed" when returning `Retry`.** The engine always treats it as a failure. So the concern "action returns `Retry`, engine drops it, action assumes retry happened, re-enters cleanup prematurely" does **not** materialize.

**However**, the feature-unification wrinkle: `cargo build --all-features` at `ci.yml:109` un-hides `ActionResult::Retry` in the `nebula-action` crate that engine compiles against. Engine does NOT need its own `unstable-retry-scheduler` feature — it uses `is_retry()`. So the engine correctly handles the variant regardless of the caller's feature selection. ✅ on variant handling.

🟡 **MINOR S-F1** — the dead feature flag is a supply-chain footgun, not a runtime one. A PR landing `unstable-retry-scheduler = ["some-new-unsafe-dep"]` would slip in a dependency that CI immediately turns on via `--all-features`. Phase 0 T2 already scopes this to devops; my security assessment: defense-in-depth, devops owns the remediation.

✅ **GOOD** — engine handles `Retry` variant via runtime predicate (`is_retry()`), independent of feature gating. This closes the "silent drop" attack vector that the prompt hypothesized.

---

## 9. Isolation level boundaries

**Code read:** `metadata.rs:9-24, 109-138`; `engine/runtime/runtime.rs:413-446, 480-500`; `sandbox/src/lib.rs:1-56`; canon §12.6 at `docs/PRODUCT_CANON.md:393-398`.

### 9.1 `IsolationLevel::None` default is a silent escalation risk

`metadata.rs:15-16` — `#[default] None`. `ActionMetadata::new` at `metadata.rs:135` constructs with `isolation_level: IsolationLevel::None`. **An action that does not call `.with_isolation_level(Isolated)` runs in-process with no isolation.**

The `#[non_exhaustive]` + `#[serde(rename_all = "snake_case")]` pair means: a future added variant (e.g., `Isolated` → `IsolatedStrict`) would ser/de correctly across versions, BUT **`serde` deserializing a config file with `isolation_level: "isolated_v2"` that does not exist in the current compiler would fail to construct** — however JSON default for missing field is `None`. If an ops team configured stricter isolation in a YAML/JSON config that targeted a future variant and the running binary didn't know it, the fall-through to `None` is a silent escalation.

Verify: `metadata.rs:98-118` has `#[serde(flatten)]` on `base` but the `isolation_level` field has no `#[serde(default)]` attribute. If the field is missing from the JSON blob, `serde_json::from_str` fails. So upgrade-safe only in one direction (old config without the field fails to deserialize into new struct). Acceptable.

🟡 **MINOR S-I1** — `IsolationLevel::None` as default is canon-honest (per §12.6 "correctness boundary, not security boundary") but a config-drift vector. A redesign that added persistence of metadata to disk could deserialize a missing `isolation_level` field with a default-derived `None`, silently dropping a previously-Isolated action to in-process. Today `serde(flatten)` + missing `#[serde(default)]` reject it, so the hazard is latent not active.

### 9.2 Engine enforcement — correct

`runtime.rs:433-445`:
```rust
match metadata.isolation_level {
    IsolationLevel::None => Ok(handler.execute(input, context).await?),
    IsolationLevel::CapabilityGated | IsolationLevel::Isolated => {
        let sandboxed = SandboxedContext::new(context);
        Ok(self.sandbox.execute(sandboxed, metadata, input).await?)
    },
    _ => Err(RuntimeError::Internal(format!(
        "unknown isolation level for action '{}' — refusing to dispatch",
        metadata.base.key.as_str()
    ))),
}
```
The `_ =>` arm fails closed on unknown variants. ✅

`runtime.rs:484-495`: stateful + non-None isolation → error (sandbox slice 1d pending). **Sandboxed stateful actions are blocked at runtime.** Canon-honest per §4.5.

### 9.3 What does `CapabilityGated` actually guarantee?

`sandbox/src/lib.rs:8-12` is explicit: "**This is not a security boundary against malicious native code.**" Canon §12.6 is explicit: "correctness and least privilege for accidental misuse, not a security boundary against malicious native code."

So `CapabilityGated` today = `SandboxedContext::new(context)` wrapping + dispatch through `InProcessSandbox` (per sandbox README phase 0 comment). Capabilities per `capabilities.rs:13-71` are declared but **unenforced** (discovery wiring TODO documented at sandbox README).

🟠 **MAJOR S-I2** — `IsolationLevel::CapabilityGated` is a documented-as-false capability today (canon §12.6 + sandbox README). The action crate exposes the variant via public metadata; authors set it on their actions; engine routes to `SandboxRunner` — but the sandbox does not yet enforce capability allowlists. The variant's existence is consistent with §4.5 ("public surface exists iff engine honors it end-to-end") **only if** the doc-layer clearly states "capability enforcement TODO". It does (canon §12.6; sandbox README). But a plugin author seeing `CapabilityGated` in the enum and setting it on their action may reasonably assume some enforcement. Defense-in-depth for redesign: either hide the variant until enforcement lands, or emit a WARN log at engine dispatch when an action declares capabilities that are not yet enforced.

### 9.4 Drop from `Isolated` to `None` via config drift?

Searched: no path in `nebula-action` that mutates `metadata.isolation_level` after construction. Builder-only (`with_isolation_level`). Safe.

Cross-cut: a plugin.toml config that downgrades an installed plugin's isolation level from `Isolated` to `None` is in `nebula-sandbox`'s / `nebula-plugin`'s scope, not action's. Not re-flagging.

---

## 10. Unsafe code discipline (action + transitive deps)

**Code read:** `action/src/lib.rs:34` (`#![forbid(unsafe_code)]`); `action/macros/src/lib.rs:5` (same); `sandbox/src/lib.rs:1` (`#![deny(unsafe_code)]`); `Cargo.toml:50-54` (crypto deps).

### 10.1 Action crate — clean

Confirmed: **zero `unsafe` blocks** in `crates/action/src/` or `crates/action/macros/src/`. Only occurrences of the word "unsafe" are the `#![forbid(unsafe_code)]` attributes themselves. ✅

### 10.2 Transitive deps

Action depends on `hmac = "0.13"`, `sha2 = "0.11"`, `subtle = "2.6"`, `zeroize = "1.8.2"`, `base64`, `hex`. All of these are RustCrypto-maintained (`hmac`/`sha2`/`subtle`) or widely-used (`base64`/`hex`). RustCrypto crates ship small targeted `unsafe` for SIMD / assembly paths (e.g., `sha2` AVX2 backends). This is standard crypto-crate posture and accepted.

`zeroize = "1.8.2"` uses `core::ptr::write_volatile` with `unsafe` in its core — that IS the point of `zeroize`, and it's extensively audited.

🟡 **MINOR S-U1** — workspace `deny.toml` does not include a `bans.deny` entry for `openssl` in the *action* layer specifically (only at workspace-wide ban at `deny.toml:49`). Currently adequate because no openssl pulls in through action's tree, but the lack of per-crate ban makes drift possible. Out of action-scope; dx-tester + devops should track.

✅ **GOOD** — `#![forbid(unsafe_code)]` on both action crate and its macros crate. Transitive crypto unsafe is bounded to well-audited RustCrypto crates. No audit concerns at this layer.

---

## 11. Top-N security findings, severity-ranked

Consolidated. Severity assignments reflect *exploitability today in current code*, not after Phase 2 redesign.

| # | Severity | ID | Location | Finding | Attack actor |
|---|---|---|---|---|---|
| 1 | 🔴 CRITICAL | **S-C2** | `context.rs:641-667` | Type-name-lowercase credential key enables cross-plugin shadow attack — malicious plugin B with same short-name credential type resolves legitimate plugin A's secret | Malicious plugin installed by tenant |
| 2 | 🔴 CRITICAL | **S-J1** | `stateless.rs:370` | `serde_json::from_value` depth-unbounded; attacker-controlled deep JSON reaching a stateless action (via workflow input or non-webhook trigger) overflows the 2 MiB worker stack | Network attacker → API boundary; compromised upstream storage |
| 3 | 🟠 MAJOR | **S-W2** | `webhook.rs:737-798`, `transport.rs:460-470` | `SignaturePolicy::Custom(Arc<dyn Fn>)` is unbounded trust delegation — a closure of `|_| SignatureOutcome::Valid` accepts any unsigned request while appearing (in metadata/debug) to use a custom verifier | Supply-chain actor with single-PR access |
| 4 | 🟠 MAJOR | **S-C4** | `guard.rs:64-71`; missing test coverage | `CredentialGuard` moved into detached `tokio::spawn` or shared `Arc` outlives parent future's cancellation; zeroize is deferred indefinitely. No compile-time enforcement | Compromised worker process |
| 5 | 🟠 MAJOR | **S-J2** | `stateful.rs:561-582` | Stateful adapter's `state.clone()` + `from_value` is depth-unbounded symmetrically; state from compromised storage can land a depth bomb | Compromised storage backend |
| 6 | 🟠 MAJOR | **S-O1** | `stateless.rs:381`; `output.rs:749-770` | No per-output size cap at adapter boundary; `BinaryData::Inline(vec![0; 1_000_000_000])` flows through `serde_json::to_value` and downstream | Malicious plugin |
| 7 | 🟠 MAJOR | **S-I2** | `metadata.rs:17-18`; canon §12.6 | `IsolationLevel::CapabilityGated` variant is public but enforcement is TODO per sandbox README — documented false-capability; authors may over-trust | Malicious plugin |
| 8 | 🟡 MINOR | **S-C1** | `guard.rs:64-71` | `CredentialGuard::Clone` silently creates additional zeroize points without audit trail | Supply-chain / compromised worker |
| 9 | 🟡 MINOR | **S-C3** | `context.rs:660, 664` | Full `type_name::<S>()` module path leaks to error messages | Log aggregator |
| 10 | 🟡 MINOR | **S-C5** | (no test) | No test asserts zeroize fires when holding future is cancelled | Regression vector |
| 11 | 🟡 MINOR | **S-O2** | `output.rs:284-288` | `BufferConfig::capacity` uncapped at crate level | Malicious plugin (needs consumer cooperation) |
| 12 | 🟡 MINOR | **S-O3** | `output.rs:125-130` | `Resolution::Callback::endpoint` / `token` raw strings — log injection vector | Malicious plugin → log aggregator |
| 13 | 🟡 MINOR | **S-O4** | `stateful.rs:609` | `tracing::error!(action_error = %e)` trusts `ActionError::Display` to be secret-free — not enforced | Plugin author bug → log aggregator |
| 14 | 🟡 MINOR | **S-W1** | `webhook.rs:1726` | `FUTURE_SKEW_SECS = 60` hardcoded; no config knob | Availability vs security trade-off |
| 15 | 🟡 MINOR | **S-W3** | `webhook.rs:1751-1764` | `hmac_sha256_compute` has no `debug_assert!` empty-secret guard (doc-only) | Plugin author bug |
| 16 | 🟡 MINOR | **S-C6** | `context.rs:645` | `to_lowercase()` is locale-dependent for non-ASCII type names | Theoretical cross-locale collision |
| 17 | 🟡 MINOR | **S-F1** | `action/Cargo.toml:14-20` | Dead `unstable-retry-scheduler` feature flag is a supply-chain surface | Supply-chain actor (devops-scope fix) |
| 18 | 🟡 MINOR | **S-I1** | `metadata.rs:15, 135` | `IsolationLevel::None` default + no `#[serde(default)]` on field means future-variant config-drift surface is latent | Config drift |
| 19 | 🟡 MINOR | **S-U1** | `deny.toml:49` | No per-crate `openssl` ban specifically for action layer (workspace-wide only) | Supply-chain drift |
| — | ✅ GOOD | — | `webhook.rs:1482, 1508, 1787` | All HMAC verifiers use `subtle::ConstantTimeEq` / `Mac::verify_slice` — correct constant-time compare |  |
| — | ✅ GOOD | — | `webhook.rs:1465-1515` | Timing-invariant decode on both hex and base64 paths — decode failure takes same time as mismatch |  |
| — | ✅ GOOD | — | `webhook.rs:82-91, 178-207` | Body-size + header-count limits enforced at `WebhookRequest::try_new` construction path; fields private |  |
| — | ✅ GOOD | — | `webhook.rs:1429-1441` | `single_header_value` rejects proxy-chain duplicate-header attack |  |
| — | ✅ GOOD | — | `guard.rs:34-62` | `CredentialGuard::Drop` → `zeroize`; `!Serialize`; redacted `Debug`. Test verifies Drop actually fires |  |
| — | ✅ GOOD | — | `snapshot.rs:247-255` | `CredentialSnapshot::Debug` redacts projected field as `"[REDACTED]"` |  |
| — | ✅ GOOD | — | `engine.rs:1877-1910` | `ActionResult::Retry` → `is_retry()` predicate (always-on) → synthetic `retryable` error. Engine never silently drops; feature flag's on-but-unwired state is not exploitable at runtime |  |
| — | ✅ GOOD | — | `webhook.rs:1378-1413` | `check_json_depth` pre-scan is a correct single-pass byte loop — handles string escapes, saturating depth counter |  |
| — | ✅ GOOD | — | `action/src/lib.rs:34`, `action/macros/src/lib.rs:5` | `#![forbid(unsafe_code)]` at crate root on both action and its proc-macro sibling |  |

---

## Summary (≤200 words)

Two exploitable-today findings gate the redesign: **S-C2** (type-name-lowercase credential key enables cross-plugin shadow attack — literally allows plugin B to resolve plugin A's token if their credential types share a short name) and **S-J1** (Phase 0's C4 confirmed — depth-unbounded `serde_json::from_value` at `StatelessActionAdapter`; attacker-deep JSON reaching a stateless action via workflow input or non-webhook trigger stack-overflows the worker). Both are 🔴 CRITICAL.

Beyond that, the webhook primitives are solid: constant-time compare, timing-invariant decode, 1 MiB body cap, 256 header cap, fail-closed `Required` default, proxy-chain header-duplication rejection. The single structural gap is **S-W2** — `SignaturePolicy::Custom(Arc<dyn Fn>)` is an unbounded trust delegation that defeats the `OptionalAcceptUnsigned`-as-audit-trail design intent; a pass-through closure is indistinguishable from a real verifier at audit surfaces.

`CredentialGuard` Drop+zeroize is correct in the common case but is structurally defeasible via `tokio::spawn` + `Arc` clone (**S-C4**). No test covers the cancelled-future drop path.

`IsolationLevel::CapabilityGated` is a documented-false-capability today (§4.5 / §12.6) — acceptable per canon but worth surfacing in the redesign.

## Top 3 findings

1. 🔴 **CRITICAL S-C2** — `crates/action/src/context.rs:641-667` — Type-name-lowercase credential key shadow attack across plugins.
2. 🔴 **CRITICAL S-J1** — `crates/action/src/stateless.rs:370` — `serde_json::from_value` depth-unbounded at adapter boundary.
3. 🟠 **MAJOR S-W2** — `crates/action/src/webhook.rs:737-798` — `SignaturePolicy::Custom(Arc<dyn Fn>)` defeats the audit-trail design of `OptionalAcceptUnsigned`.

*End of Phase 1 security threat model.*
