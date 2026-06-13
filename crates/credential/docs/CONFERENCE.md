# Credential design conference — synthesis (2026-06-12)

A grounded design review of [`DESIGN.md`](DESIGN.md) + [ADR-0092](../../../docs/adr/0092-credential-subsystem-consolidation.md)
by ten seats: six industry architects (each grounded in their real codebase via
DeepWiki) and four adversarial critics (each grounded in `crates/credential/src`).
Raw transcripts: [`research/round1-players-and-critics.md`](research/round1-players-and-critics.md) + [`research/round1-critics-retry.md`](research/round1-critics-retry.md).

The conference graded eight decisions:

- **D1** one crate, 3 bounded contexts, injected ports, no `reqwest`/`sqlx`
- **D2** one `CredentialRuntime` pipeline replaces 4 resolve/refresh entry points
- **D3** policy-as-data + compile-gated capability code; macro derives policy; runtime reads `policy(state)` then the method
- **D4** code-per-protocol, config-per-provider (one `OAuth2Protocol`, providers are data)
- **D5** OAuth Plane Law — Plane A login vs Plane B credential; zero HTTP routes in credential; refresh via injected `RefreshTransport`
- **D6** reactive-only refresh in 1.0 (L1 coalescer + durable `RefreshClaimRepo`); proactive → 1.1
- **D7** values-only persistence; schema from registered types
- **D8** consumer binding to output `Scheme`; slots separate from parameters; unified `#[property]` = Phase-5 sugar

## Verdict tally

| Seat | D1 | D2 | D3 | D4 | D5 | D6 | D7 | D8 |
|------|----|----|----|----|----|----|----|----|
| Temporal | ✅ | ✅ | ✅c | ✅✅ | ✅✅ | ✅ | ✅✅ | ✅ |
| n8n | ✅c | ✅ | ✅ | ✅✅ | ✅c | ✅ | ✅ | ✅ |
| Airflow | ✅ | ⚠️ | ✅ | ✅✅ | ✅ | ⚠️ | ✅ | ⚠️ |
| Dagster/Prefect/Kestra/Restate | rate-limited (not re-run; lower marginal signal) |||||||
| Windmill | ✅ | ✅ | ⚠️ | ✅✅ | ✅ | ⚠️ | ✅ | ⚠️ |
| Vault | ✅ | ✅c | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅c |
| AWS SDK (Rust) | ✅c | ✅ | ✅ | ✅✅ | ✅c | ⚠️ | ✅ | ✅c |
| critic-arch | ❌ | ⚠️ | ❌ | ⚠️ | ⚠️ | ❌ | ✅ | ✅ |
| critic-types | ⚠️ | ❌ | ❌ | ⚠️ | ✅ | ⚠️ | ✅ | ⚠️ |
| critic-sec | ✅c | ❌ | ⚠️ | ✅ | ⚠️ | ⚠️ | ✅ | ✅c |
| critic-dx | ✅ | ⚠️ | ❌ | ❌* | ✅ | ✅ | ✅ | ⚠️ |

`✅✅`=strong agree, `✅`=agree, `✅c`=agree w/ caveat, `⚠️`=caveat, `❌`=disagree.
`*` critic-dx: D4 agree as **goal**, disagree as a statement of present fact (zero
implementation in the branch).

**Consensus:** D4, D5, D7 are the strongest decisions and validated by every
industry seat. D2/D3 are right as goals but **unimplemented and internally
contradictory in the current code** — the critics found this with file:line proof.
D1/D6 carry real, named risks. D8 is sound but has a sharp security caveat.

---

## Finding 1 — D2/D3 "policy-first routing" is vaporware **and** self-contradictory (must fix)

Code-grounded by critic-types and confirmed by Temporal/n8n:

- The `#[credential]` macro synthesizes `fn policy(_state)` that **ignores its
  argument** and always emits `expires_at: None, lease: None`
  (`macros/src/credential_attr.rs:435-447`). "Policy computed from live state" is
  false.
- The resolver **never calls `policy()`**. It routes on `state.expires_at()` and a
  hardcoded `if C::KEY != OAuth2Credential::KEY { return Ok(None) }`
  (`runtime/resolver.rs:209,536`). `RefreshStrategy` / `CredentialLifecycle::policy`
  have **zero production consumers**.
- Capability lives in **two desyncable mirrors**: the `Capabilities` bitflag (from
  sub-trait membership) and `policy.refresh`. For OAuth2-without-`refresh_token`
  the bitflag says *refreshable* while the intended policy says *ReAcquire* — the
  exact §16 "sub-trait dispatch ignores policy" bug, now elevated into the design.
- Conditional capability is inexpressible: `OAuth2Credential` carries three grant
  types in one type, so `is_refreshable` is type-level binary; a client-credentials
  instance (no refresh token) is still flagged refreshable.

**Adopted correction:** policy must be a genuine `fn(&State) -> Decision` and the
resolver must route on it — **delete the `C::KEY`-string branch**. Either (a) keep
the macro but make policy an *explicit* data declaration whose disagreement with
the implemented method is a **compile error** (`assert_impl!(C: Refreshable)` under
`RefreshStrategy::Refresh*` → E0046; Temporal + AWS proposal), or (b) adopt
critic-types' **sealed `enum ProtocolKind`** with an exhaustive `match` that forces
every category (Static/RefreshPair/Leased/Federated/…) to return a strategy. n8n +
Temporal both ask for an explicit `#[credential(refresh = reacquire_if_no_token)]`
override regardless. Phase-1 DoD must include the **ReAcquire-vs-RefreshToken test**.
→ recorded as DESIGN open question #5.

## Finding 2 — Leased credentials never refresh (category-level silent failure)

critic-types A7 + Vault: the resolver decides `needs_refresh` from
`state.expires_at()` only. A `RefreshStrategy::Lease` secret (Vault/k8s) has
`expires_at: None` (the server tracks TTL), so it **never enters the refresh
window**, and `is_auto_renewable()` is never called. The "Vault-leased secret and a
static PAT look identical, differ only in cadence" claim hides a **silent loss of
renewal** for the whole Leased category.

**Adopted correction (Vault + AWS):** lease/expiry becomes a **first-class**
`CredentialRuntime` concept. Introduce an opaque `LeaseHandle` carried by
`CredentialGuard<Scheme>`; `policy(state)` returns the unified
`Decision{Use|Refresh|ReAcquire|Revoke}` (Vault `framework.CalculateTTL`); the
resolver consults it for **every** strategy, not just inline-expiry. A
`RefreshStrategy::Lease` path that the resolver cannot drive is a Phase-1 blocker.

## Finding 3 — Security hardening the design states as prose, not as types

critic-sec (code-grounded), echoed by Temporal/n8n/AWS:

1. **Confused-deputy is open on the slot path.** `resolve_for_slot` checks
   `binding.fingerprint() == from_scope(scope)` — a value compared with itself — then
   `resolver.resolve(id)` loads by raw id with **no store-level owner gate**. The
   only real barrier (owner-scoped store query) is missing.
2. **`owner_id` collision** if any producer skips the length-prefixed derivation
   (the api manual-enforcement arm is "dead, follow-up deletion").
3. **`RefreshTransport` DNS-rebind is `SHOULD`, not enforced** — a second
   composition root (CLI/test/worker) can inject a permissive transport and bypass
   SSRF. Temporal/n8n/AWS all independently flag this.
4. **`pending_store.get(token)` bypasses 3 of 4 binding dimensions** (replay /
   fixation on device-code polling).
5. **revoke vs in-flight refresh CAS** can resurrect a revoked credential (delete
   then CAS-upsert with no tombstone epoch).
6. **Circuit-breaker serves a stale-but-revoked token** because `record_failure`
   does not distinguish `invalid_grant` (revoked) from a 5xx blip.
7. **Plugin `test`/`acquire`/`project` have no egress allowlist** — SSRF hardening
   exists only on the OAuth2 token-url branch; a malicious plugin can exfiltrate
   plaintext `State` from `test`/`project`.

**Adopted correction (the cross-cutting fix, from critic-sec + critic-arch):** make
owner isolation a **type**, not a discipline. Introduce a privately-constructed
`OwnerScopedKey` (length-prefixed owner_id + id) and **remove `get(&str)` from the
store port**, so the resolver *cannot express* an unscoped load. `ValidatedCredentialBinding`
carries the `OwnerScopedKey`, not a string + tautological fingerprint. Plus:
connect-layer SSRF resolver as **MUST** (type the seam so it cannot carry
`OAuth2State`); `pending.get` honors the full 4-D binding; revoke writes a tombstone
epoch the refresh-CAS consults; circuit-breaker only serves stale on **transient**
(not terminal) failures; a plugin egress policy on `test`/`acquire`.
→ DESIGN §9 + §10 + §11 hardening; abuse-cases become worked scenarios.

## Finding 4 — D4/D5/D7 are right; tighten the wording, don't relax the seam

- **D4 (universal strong-agree):** Vault `framework.Backend`+`MountEntry`, n8n
  `extends ['oAuth2Api']`, AWS `ImdsProvider`+config, Airflow `provider.yaml`
  `conn-fields`, Windmill `OAuthConfig` data — every seat ships provider-as-data.
  Honesty note: **none of it exists in the branch yet** (`OAuth2ProviderConfig` /
  `provider()` = zero grep hits; `oauth2.rs` is still a 1481-LoC monolith). DESIGN
  must read as *spec*, present tense removed. And the `<500 LoC` oauth2 target needs
  **code-per-grant** modules (authz-code / client-credentials / device-code are
  different control flow) — i.e. *code-per-protocol-per-grant, config-per-provider*.
  Add **client-credentials as a first-class family** (Windmill).
- **D5 (universal agree):** the narrow `RefreshTransport` seam (bare `url`+form →
  capped bytes; SSRF + bounded-read + `OAuth2State` mutation stay inside credential)
  must be a **type invariant** — Temporal: "the seam type must physically be unable
  to carry `OAuth2State`/keys." n8n: split the §15 table — n8n's refresh lives in
  `OauthService`, **not** `CredentialsHelper`; map `CredentialsHelper → CredentialRuntime`
  (orchestration) **and** `OauthService → RefreshTransport + OAuth2 state-logic`.
- **D7 (universal agree):** Temporal `DataBlob`+`EncodingType`, Windmill
  `ResourceType.schema`, Vault `FieldSchema` — all store values, schema from type.
  Close the honest gap with a **typed error + trace span on validate-fail** (DoD).

## Finding 5 — D1/D6 risks the owner already accepted, plus one new lever

- **D1 compile-firewall loss** (critic-arch, AWS-SDK, Temporal, n8n): a touch to the
  contract recompiles ~6k+ LoC of hot runtime during the exact 5-phase contract
  rewrite that touches it most. AWS keeps contract (`aws-credential-types`) and
  runtime in separate crates *for this reason*. Recommendation surfaced as DESIGN
  open question: a **minimal `nebula-credential-core` / runtime split** restores the
  one firewall hit most often — but this *reverses ADR-0092's owner-directed
  single-crate choice*, so it needs explicit owner sign-off, not a silent flip.
- **D6 reactive-only is already half-violated:** the lease scheduler
  (`runtime/lease/scheduler.rs`) renews at N% TTL — proactive by definition.
  **Adopted (Temporal + AWS):** define a `durable-timer` port **now** (even unused)
  so 1.1 proactive is an *added impl*, not a coordinator relocation back to engine
  (this discharges ADR-0092's "Open risk" cheaply). Add AWS `buffer_time` + `jitter`
  + `load_timeout` to the L1 coalescer in 1.0 — proactive-buffer inside the reactive
  cache removes the latency-spike/thundering-herd without a scheduler.

## Finding 6 — DX: stop shipping vaporware as worked examples

critic-dx (code-grounded):

- `#[property]` and `#[action(unified)]` **do not exist**; the §7 worked examples do
  not compile. DESIGN must label all Phase-5 unified-authoring as *not-yet-built*.
- Capability is inferred by **string method-name matching** (`refresh`/`revoke`/…) —
  fragile; rename a method and capability silently changes. Prefer a `#[refresh]`
  **annotation on the method** (name-free) or the typed-lifecycle field.
- The **dual macro path** (`#[derive(Credential)]` flag-based + `#[credential]`
  infer-based) is two opposite philosophies on one word — **delete the derive now**
  (DESIGN §16 already wants this; do it, don't defer).
- **T3 (manual impl)** requires five hand-written `IsX` consts that can **silently
  desync** from the trait impls (`capability_report.rs:51-57` admits the silent
  downgrade) — the very self-attestation §15.8 claimed to close.
- `metadata()` is duplicated in every builtin (`api_key.rs`, `bearer_token.rs`) even
  though the macro can synthesize it — fix the exemplars so agents stop copying the
  `.expect(...)` panic sites.

---

## Concrete adoptions (folded into DESIGN.md)

1. Resolver reads `policy(&State)`; delete the `C::KEY` string branch (Finding 1).
2. `LeaseHandle` + unified `Decision{Use|Refresh|ReAcquire|Revoke}`; lease is
   first-class; `is_auto_renewable` actually consulted (Finding 2).
3. `OwnerScopedKey` newtype; remove store `get(&str)`; binding carries the scoped
   key (Finding 3).
4. `RefreshTransport` seam typed so it cannot carry `OAuth2State`; DNS-rebind
   connect-layer resolver = MUST (Findings 3, 4).
5. revoke tombstone epoch consulted by refresh-CAS; circuit-breaker gates on
   transient-only; plugin egress policy on `test`/`acquire` (Finding 3).
6. code-per-protocol-**per-grant**, config-per-provider; client-credentials a
   first-class family (Finding 4).
7. `durable-timer` port defined in 1.0; L1 gets `buffer_time`+`jitter`+`load_timeout`;
   cache partition keyed `owner × scheme × provider-fingerprint` (Findings 2, 5).
8. observable `refresh_error` / `last_refresh_at` in State, projected to catalog
   (Windmill); typed error + span on validate-fail (Temporal, D7).
9. RefreshClaimRepo secondary index credential→owner for mass-revoke
   (`RevokeByToken`/`RevokePrefix`) + recovery-on-startup of pending claims (Vault).
10. Fix DESIGN §9 Ports (decorators stay in `nebula-storage`, ADR-0092 step-3
    reverted) and §15 n8n table (Helper vs OauthService split); mark §7 unified
    authoring as not-yet-built (n8n, critic-dx).

## Strategic forks — all decided (planёрка 2026-06-12, see "Планёрка" below)

- **F1 — crate split. RESOLVED 2026-06-12: single crate stays** (owner: AI-discoverability
  beats the firewall). The conference's firewall objection is accepted; its
  boundary-erosion risk is mitigated in-crate by a dependency-direction architecture
  test (`service → runtime → contract`, never reverse) — see DESIGN §21 "F1 enforcement".
- **F2 — capability model. DECIDED: open trait wins** (the sealed `enum ProtocolKind` is
  dead — it forecloses F3). Grounding: n8n has no sealed category enum (`ICredentialType`
  = one open trait; capability = presence of optional methods; OAuth2 = one `oAuth2Api`
  base + provider data; refresh-vs-reauth is data-driven at runtime in `refreshOrFetchToken`
  = exactly a state-derived decision), Vault `CalculateTTL` the same shape. The planёрka
  then sharpened it: liveness moves **off the author's return value** onto a
  framework-clocked lease seam (the deadline-field-makes-it-unrepresentable claim is false).
  **Owner ruled (2026-06-12): no "valid forever" category** — even a no-freshness-signal
  static credential (a plain API key) carries a framework-imposed mandatory re-validation
  floor (`decide_refresh` returns NeedsRefresh past the floor even with `expires_at:None`).
  Resolver always calls a total `decide_refresh(&State, now)`; the `C::KEY` branch is
  deleted; compile-gate keeps capability↔strategy honest (E0046).
- **F3 — open-world plugins. DECIDED: sealed family enum + per-protocol marker type**, no
  `Custom(Box<dyn>)` / `Opaque` escape hatch. Two axes: the protocol *family* is a sealed
  enum (runtime mechanics, egress, `decide_refresh`); the *binding* identity is a zero-cost
  marker type (`Slot<S: Scheme>`, nominally checked at compile time). A plugin author adds a
  marker + declares an existing family; an unsound family choice is rejected at registration.

---

## Real-issue grounding (n8n / Windmill / Airflow GitHub, 2026-06-12)

The conference's "грабли" were then checked against **real reported bugs**, because the
owner's goal is to fix the credential pain other engines actually have. Findings (full
matrix in [DESIGN.md](DESIGN.md) §22):

- **Rotated `refresh_token` not persisted** → 1-hour-then-reconnect loop — n8n
  [#30345](https://github.com/n8n-io/n8n/issues/30345), [#25926](https://github.com/n8n-io/n8n/issues/25926); Windmill Revolut #4582. The single most common OAuth bug. → DESIGN §10 rule 13 (atomic whole-state CAS incl. rotated token).
- **Concurrent refresh storm burns one-time tokens / garbles data** — n8n
  [#12742](https://github.com/n8n-io/n8n/issues/12742). → L1 single-flight per `(owner,credential_id)` + durable claim CAS (D6).
- **Stale in-memory credential after refresh (DB-only update)** — n8n
  [#1695](https://github.com/n8n-io/n8n/issues/1695); Windmill [#1732](https://github.com/windmill-labs/windmill/issues/1732). → §10 rule 14 (ArcSwap hot-swap + `CredentialRef` re-resolve).
- **Refresh only on HTTP 401; `expires_in`/403 ignored** — n8n
  [#17450](https://github.com/n8n-io/n8n/issues/17450), [#18517](https://github.com/n8n-io/n8n/issues/18517). → §10 rule 16 (trigger on stored expiry; "expired" signal is provider data).
- **Bad provider response corrupts stored credential** — n8n
  [#12742](https://github.com/n8n-io/n8n/issues/12742). → §10 rule 15 (validate before CAS; atomic).
- **Confused-deputy / secret exfiltration via a resource token + SSRF** — Windmill
  patched this in commit [#9428](https://github.com/windmill-labs/windmill/commit/8053266) (2026-06-03): resolve the token through the caller's permissioned path. Independently matches critic-sec's confused-deputy + plugin-egress findings. → §10 rules 8/9/12 (`OwnerScopedKey`, narrow transport, egress policy).
- **Secrets-backend latency/outage on the hot path** — Airflow mitigates with
  `SecretCache` (TTL) + context-aware backend chains + isolation (workers never read the metastore directly). → §10 rule 17 (`ExternalProvider`: ordered chain + timeout + cache + fail-closed).

These real issues are now **worked-scenario regression contracts** (§22): each must
become a passing test before its capability is called done.

---

## Round 2 — differentiation + scalability (2026-06-12)

Second round: all 10 industry seats + a scale-critic and a moat-skeptic, each
reacting to the *current* DESIGN (incl. §22) and grounded in the actual code.
Raw transcript: [`research/round2-differentiation-scalability.md`](research/round2-differentiation-scalability.md). Folded into DESIGN
§23 (differentiation/moat) and §24 (scalability walls).

### Differentiation verdict (moat-skeptic, sharpest)

Of six apparent differentiators, **4 are renamed industry patterns** (active
lifecycle = parity w/ n8n/Windmill; code-per-protocol = n8n `extends`; reactive-only
= behind n8n; Plane Law = internal hygiene), **1 is a self-tax** (single crate), and
**only `CredentialGuard<Scheme>` + `OwnerScopedKey` + typed slot-binding through the
DAG is a real structural moat** — compile-time owner-isolation + scheme-typing that
no competitor can express (they all do it by runtime RBAC/ACL/discipline). Adopted:
make the typed-integration-spine the explicit moat thesis (§23), catch up on OAuth
mechanics silently as parity/quality (not the pitch), and close the confused-deputy
**first** (the moat is a hole today). Two honest product gaps surfaced: hierarchical
inherited scope (Kestra) and a shipped external-SM backend (Kestra EE/Airflow).

**Cohort correction (DESIGN §23).** The "parity" half of the verdict is scoped to
n8n/Windmill — the JS/Python incumbents, the right bar for OAuth *mechanics*. Against
the field Nebula actually competes with on substrate — Rust-native workflow engines — a
survey of ~27 such projects shows the opposite: **0 ship a credential subsystem of
comparable depth; 0 encrypt at rest; 0 use zeroize/secrecy; 0 do OAuth refresh; 0 have
typed credential kinds; multi-tenant ownership appears only as a gap** (the one peer
with a SQL credential vault has no owner column → any user reads any credential). So the
differentiation pitch is "most Rust workflow engines have no credential layer; the few
that try miss owner-isolation," and that real peer's missing owner column is field
evidence that owner-isolation is the hard part — reinforcing confused-deputy-first.

**Two more items folded into §23/§17 from the read-back.** (a) Reference-not-copy mode
(store a vault pointer, not the secret → a DB dump can't steal tokens) as both an
adoption path and a security differentiator; (b) **credential read/material-access
auditing** (today only refresh/rotation/revoke are audited) as a SOC 2 prerequisite and
a §17 Phase-1 fail-closed-audit DoD item. Plus §10 gained two seam rules from the same
read-back: provider-returned strings are untrusted (rule 18 — `error_uri`/`error_description`
log-injection + token echo) and success is content- not status-determined (rule 19 —
200-with-error-body), each a new §22 regression row (M, K).

### Scalability — three walls, named by every seat, code-grounded

1. **L1 single-flight serializes a burst on a hot shared credential**; constants are
   inconsistent (`coordinator.rs`: backoff cap 5s < refresh_timeout 8s) → contender
   `ContentionExhausted` if IdP > 5s. Fix: blocking watch/notify on the claim row +
   dynamic backoff cap + proactive buffer+jitter off the hot path.
2. **RefreshClaimRepo CAS hot-row contention at Z replicas + O(N·M) fan-out**; the
   per-replica `semaphore=32` (`l1.rs`) protects the replica, not the provider →
   8000-cred/12-replica midnight expiry = 384 concurrent against one app = 429-storm.
   Fix: shard claim key by `owner×scheme` (Vault fairshare), batched owner-epoch
   tombstone, **per-provider concurrency bucket (new §22 row J)**.
3. **In-process registry P types → cold-start/binary + cross-replica config
   divergence + blast radius**. Fix: code-per-protocol in binary (~10), but
   **config-per-provider as durable store data with pub/sub invalidation**.

Plus cross-cutting invariants → §24 + §17 DoD: sharded coalescer map (not single
Mutex — AWS lesson), one cache/owner key derivation (AWS S3Express), batch
`get_many` (no N+1), pure cached `policy` (no write on cache-hit), ExternalProvider
numeric budget + single-flight + env-fail-open / terminal-only-fail-closed (Airflow),
staggered rotation fan-out (HikariCP-#1836 class).

**Framing:** all three walls are the cost of pulling credential coordination *inside*
a single-node engine (the DX/security moat) where Temporal/Restate/Vault push it out
to sharded logs/partitions/job-managers. Right trade for 1.0 self-hosted; the first
multi-replica install hits Walls 1+2, so the mitigations are 1.0 constraints.

## Планёрка — decision round (2026-06-12)

A third meeting forced the six coupled open decisions to a verdict
(proponent → adversary → chair). Unlike rounds 1-2 (design correctness, then
differentiation/scalability), this round was **decision-forcing**, and several
adversaries **read the as-built source** — finding the branch has already routed past
some assumed bugs (`OAuth2Credential::refresh` returns a typed `ReauthRequired` not a
hard error; OAuth2 `policy` is hand-written and state-reading) and one *new* latent
defect (`ensure_local_source` absent from `resolve_for_slot`). Folded into DESIGN
§19.1 (verdict table) + §17 Phase-1 spec deltas.

| Decision | Verdict | Core move |
|----------|---------|-----------|
| **F2** capability model | **Decided (owner)** | Open trait wins (sealed enum kills F3). The "non-`Option` deadline makes leased-never-refresh unrepresentable" claim is **false** — same type for `MAX` and a real value. Fix: liveness is framework-clocked (constructor ceiling so `Duration::MAX` is unconstructible; resolver returns a lease handle, never `&Secret`); `Decision` is time-free `{Usable,NeedsRefresh,NeedsReacquire,Dead}`. **Owner ruled: no "valid forever" — even a static API key carries a mandatory re-validation floor** (`decide_refresh` returns NeedsRefresh past the floor even with `expires_at:None`). |
| **Q2** OAuth2 KEY | **Decide A** | One `oauth2` KEY + provider-data. Attack failed on the KEY axis; surfaced a separate **grant-discriminant gap** (`client_credentials` mis-routed to interactive reauth) — a Phase-1 rider, not a KEY change. |
| **F3** open-world plugins | **Decide A + marker types** | Two axes: sealed **family enum** (runtime mechanics, egress, `decide_refresh`) + per-protocol **marker type** (`Slot<S: Scheme>`, nominal binding). No `Opaque`/`Box<dyn>`. Stripe→Twilio bind = compile error; wrong-family marker caught at registration. |
| **Q8** durable-timer port | **Decide C-minus** | Ship `decide_refresh(state,now) -> {Fresh,RefreshNow}` + extract `on_access`; **no** `RefreshAt(Instant)` / `DurableTimer` / no-producer arch-test — the 1.1 trigger is a sharded sweeper, not a per-credential `Instant`; extending the internal enum in 1.1 is non-breaking. |
| **Kestra** hierarchy | **Decided (owner): retroactive required** | Flat in 1.0, but **keep the read-key seam**. Owner ruled a new child MUST auto-inherit parent secrets provisioned before it existed, no re-provision ("else a pile of tickets") — this **overrides** reference-at-write (it can't do retroactive). 1.1 = **read-time inherited resolution via an authority-vouched `InheritedScopeKey`** (proven ancestor chain, privately constructed; resolver still cannot express an unscoped/unvouched load — not a free tree-walk). |
| **Q9/Q10** lifecycle scope | **Decide (split)** | Q9 = binding-validation rejects tombstoned creds (typed `CredentialTombstoned`, **no `references()` port** — that inverts F1). Q10 1.0 = the **resolver-source-aware correctness fix only** (move `ensure_local_source` into the resolver tail; `External`→`Unsupported`); real unleased/leased providers are 1.1. |

**Both owner-reserved calls are now decided (2026-06-12).** F2: no "valid forever" — even
a static API key carries a mandatory framework re-validation floor. Kestra: retroactive
auto-inheritance is required, overriding reference-at-write; 1.1 does read-time inherited
resolution via an authority-vouched `InheritedScopeKey` that keeps the moat's no-unvouched-load
invariant. **All six decisions are closed; nothing in the credential design now waits on the owner.**

Raw transcript: workflow `credential-planerka-decisions` (run `wf_f18853d3-8ce`).
