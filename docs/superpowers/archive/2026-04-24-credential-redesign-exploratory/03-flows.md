# Credential flows

**Статус:** draft flow-диаграммы для основных сценариев. Part of exploratory notes, not spec.

## Flow 1: Credential creation — OAuth2 interactive

Shows API → engine → storage + ProviderRegistry.

```
User ── POST /api/v1/orgs/:org/workspaces/:ws/credentials ──► nebula-api
         { type: "SlackOAuth2", config: {...} }
           │
           │ AuthContext extractor: tenant_ctx, user_id, RBAC check
           │ (credential:create scope)
           ▼
         handlers::credentials::create
           │
           ├─► nebula-schema::ValidSchema::validate(config)
           │   └─► ValidValues (proof token)
           │
           ▼
         engine::CredentialService::create(type_id, config, tenant_ctx)
           │
           ├─► CredentialRegistry::lookup(type_id) → ProviderId "slack"
           │
           ├─► storage::ProviderRegistryRepo::get("slack") ──► ProviderSpec
           │   (validates: scopes ⊂ provider.allowed_scopes,
           │                redirect_uri ∈ provider.allowed_redirects)
           │
           ├─► Credential::resolve(values, ctx)
           │   └─► для OAuth2 AuthCode grant:
           │       returns ResolveResult::Interactive(OAuth2Pending { ... })
           │
           ├─► storage::CredentialStore::put_pending(pending_state, ttl=10min)
           │   └─► EncryptionLayer: PKCE verifier + CSRF token encrypted at rest
           │
           ├─► eventbus::publish(CredentialEvent::Created{id, type, tenant})
           │
           ▼
         returns 201 { credential_id, status: "pending_authorization", authorize_url }
```

## Flow 2: OAuth2 authorize → callback → ready

Shows HMAC state, callback validation, token exchange (engine-owned HTTP per revised layer map).

```
User (browser) ──► opens authorize_url ──► IdP (Slack)
                                             │
                                             │ user authorizes
                                             │
                                             ▼
IdP redirects user ──► GET /api/v1/credentials/:id/oauth2/callback?code=...&state=...
                              │
                              ▼ nebula-api::oauth_callback_controller
                              │
                              ├─► verify session cookie (CSRF binding)
                              │
                              ├─► engine::oauth2::flow::complete_callback(
                              │       credential_id, code, state, session_id, tenant_ctx
                              │   )
                              │     │
                              │     ├─► verify state HMAC constant-time
                              │     │    (fail BEFORE pending lookup — no side channel)
                              │     │
                              │     ├─► storage::PendingStore::get_then_delete(id)
                              │     │   └─► single-use transactional; session_id match
                              │     │
                              │     ├─► build token exchange body (Zeroizing<Vec<u8>>)
                              │     │
                              │     ├─► [engine's reqwest client]
                              │     │   POST {provider.token_endpoint}  ← from ProviderRegistry
                              │     │     ├─ TLS only, redirect cap 5
                              │     │     ├─ timeout 30s, body cap 1 MiB
                              │     │     ├─ verify endpoint matches registry (literal)
                              │     │     └─ zeroize partial response on error
                              │     │
                              │     ├─► parse response → OAuth2State (Zeroizing<OAuth2State>)
                              │     │   (ZeroizeSecretVisitor for deserialize — finding #8)
                              │     │
                              │     ├─► storage::CredentialStore::put(state)
                              │     │   └─► EncryptionLayer (AAD=credential_id, kek_id)
                              │     │   └─► AuditLayer::write (fail-closed)
                              │     │   └─► CacheLayer::invalidate(id)  (finding #4 fix)
                              │     │
                              │     ├─► eventbus::publish(CredentialEvent::Resolved)
                              │     │
                              │     ▼
                              │   returns Ok
                              │
                              ▼
                            302 redirect to UI: /credentials/:id?status=ok
```

**Где живёт HMAC secret (finding #16):**
- `ApiConfig::oauth_state_hmac_secret` (32+ bytes)
- Provided via `KeyProvider` для multi-replica sharing
- Rotation: versioned prefix в state (`v2:base64url(hmac_sha256(...))`), grace period during rotation

## Flow 3: Action execution using credential (hot path)

The common case — action needs credential, resolves through engine, uses через resource.

```
Engine starts action execution
  │
  │ builds ActionContext { bindings: {slack: CredentialKey("..."), ...}, ... }
  ▼
Action::execute:
  
  let auth = ctx.credential_at(&self.slack).await?;
         │
         ▼ ActionContext::credential_at
         │
         ▼ ScopedCredentialAccessor::resolve(key, scope)
         │
         ├─► ScopeLayer: verify scope.workflow_id matches current (finding #1 — cross-plugin leakage)
         │
         ├─► CacheLayer: check hit
         │   └─► if hit + not near expiry → return
         │
         ├─► storage::CredentialStore::get(key)
         │   └─► EncryptionLayer::decrypt (AAD verified)
         │   └─► returns StoredCredential<State>
         │
         ├─► check state.expires_at
         │   └─► if near expiry → initiate refresh (Flow 4)
         │
         ├─► Credential::project(&state) → Scheme (e.g. OAuth2Token)
         │   └─► wrap в CredentialGuard<Scheme>
         │
         ▼
       returns CredentialGuard<OAuth2Token>
  
  ─────────────────────────────────────────────────
  // Now auth can be used. Pattern A — per-request:
  
  let client = ctx.resources.acquire::<SlackHttpClient>().await?;
  let resp = client
      .authenticated(&auth)                  // builder wraps &auth
      .post("/api/chat.postMessage")
      .json(&payload)
      .send().await?;
  //     ^^^ inside .send(): 
  //     1. Start request build
  //     2. auth.expose(|scheme| scheme.inject(req))
  //        or scheme.sign(req, sig_ctx) for SIGNING schemes
  //     3. tokio::spawn HTTP future
  //     4. await response
  //     5. auth drops at .await point — zeroize CredentialGuard
  
  // Plaintext token lives ~μs during step 2. No other heap copy.
```

**Cache invalidation sequence (finding #4):**

After rotation or refresh writes new state:
```
engine::rotation::transaction::commit_swap
  │
  ├─► storage::CredentialStore::put(new_state)
  │
  ├─► (NEW) CacheInvalidationChannel::broadcast(credential_id)
  │   │
  │   ├─► L1 cache in this replica — remove entry
  │   └─► via eventbus (other replicas subscribe for their local caches)
  │
  ▼
next resolve hits fresh state
```

## Flow 4: Token refresh — with multi-replica coordination

Addresses finding #5 (multi-process race) and finding #17 (mid-refresh crash losing rotated refresh_token).

```
Action resolve discovers state near expiry:

ctx.credential_at(&self.slack) [near expiry path]
  │
  ▼ engine::RefreshCoordinator::refresh_coalesced(credential_id, do_refresh)
  │
  ├─► [L1] in-proc LRU mutex on credential_id
  │   │
  │   ├─► lock() ← coalesces within this replica
  │   │
  │   ├─► re-check state (maybe another coroutine refreshed)
  │   │   └─► if now fresh → return (coalesced)
  │   │
  │   ├─► [L2] storage::RefreshClaimRepo::try_claim(
  │   │       credential_id,
  │   │       holder = self.replica_id,
  │   │       ttl = 30s
  │   │   )
  │   │   │
  │   │   ├─► claim acquired:
  │   │   │    │
  │   │   │    ├─► spawn heartbeat task (every 10s — TTL/3 per finding #16)
  │   │   │    │
  │   │   │    ├─► do_refresh.await
  │   │   │    │    │
  │   │   │    │    ├─► fetch current state (re-read — другая replica могла refresh до claim)
  │   │   │    │    ├─► build refresh request (Zeroizing<Vec<u8>>)
  │   │   │    │    ├─► [engine's reqwest client]
  │   │   │    │    │   POST {provider.token_endpoint} grant_type=refresh_token
  │   │   │    │    │
  │   │   │    │    ├─► IdP response (may include NEW refresh_token — rotation)
  │   │   │    │    │
  │   │   │    │    ├─► parse response → new OAuth2State
  │   │   │    │    │
  │   │   │    │    ├─► storage::CredentialStore::put(new_state)  [atomic]
  │   │   │    │    │   └─► EncryptionLayer + AuditLayer + CacheLayer invalidation
  │   │   │    │    │
  │   │   │    │    └─► returns Ok
  │   │   │    │
  │   │   │    ├─► heartbeat task abort
  │   │   │    └─► storage::RefreshClaimRepo::release(claim)
  │   │   │
  │   │   └─► claim denied (other replica refreshing):
  │   │        │
  │   │        ├─► wait 200ms
  │   │        ├─► re-check state
  │   │        │   └─► if now fresh → return (они успели)
  │   │        └─► else loop (retry try_claim)
  │   │
  │   ├─► unlock L1
  │
  ▼
resolve continues с fresh state
```

**Open question (finding #17) — mid-refresh crash с refresh_token rotation:**

Scenario:
```
Replica A: acquires claim
Replica A: sends refresh POST with old_refresh_token
IdP: response arrives — new_refresh_token, new_access_token (old invalidated)
Replica A: CRASHES before writing response to storage
Replica B: reclaims expired claim, sees stale state
Replica B: sends refresh POST with old_refresh_token (still has it)
IdP: rejects — "token already consumed"
Result: credential permanently broken, needs manual reauth
```

**Partial mitigations (no final answer):**

1. **Pre-write sentinel.** Before POST, write `refresh_in_flight = true` flag to storage. On crash + reclaim, detect sentinel → mark credential as `ReauthRequired` vs trying again. Surface to UI — user сейчас видит notification раньше vs discovering через next resolve failure.

2. **Two-phase commit.** Client-side: save response to temporary storage before final commit. Requires IdP idempotency (не все support).

3. **Capture response на network layer.** Nebula-level HTTP proxy captures token endpoint response before it reaches engine. If engine crashes, proxy writes response. Complex.

4. **Accept the loss.** Document ass "rare edge case": crash-during-refresh window ~100-500ms. Rate: < 1 per 100K refreshes (в practical terms). User notification + manual reauth path. Simpler.

**Предложение (не final):** combine 1 + 4. Sentinel detection surfaces ReauthRequired quickly; accept rare loss as known limitation; document in operator runbook.

## Flow 5: Credential rotation (scheduled or emergency)

Per ADR-0030 rotation orchestration. Adds multi-replica leader election.

```
Rotation scheduler startup:
  │
  ├─► each replica attempts: storage::RotationLeaderClaimRepo::try_claim(
  │       scope = "tenant/{tenant_id}",  // or "global"
  │       holder = replica_id,
  │       ttl = 60s
  │   )
  │
  ├─► only 1 claim succeeds; others become followers
  │
  ├─► leader spawns heartbeat (every 20s)
  │
  ├─► leader runs scheduler loop:
  │   │
  │   ├─► for each credential with rotation policy:
  │   │   │
  │   │   ├─► check policy (time-based, usage-based, event-triggered)
  │   │   │
  │   │   └─► if due:
  │   │       │
  │   │       ├─► acquire rotation transaction lock (row-level CAS)
  │   │       │
  │   │       ├─► Credential::refresh(current_state, ctx)
  │   │       │   // for OAuth2 — refresh_token flow
  │   │       │   // for API key — error (not refreshable) → emergency path
  │   │       │   // for AWS STS — chain another assumeRole call
  │   │       │
  │   │       ├─► blue_green swap:
  │   │       │   ├─► write new state с new_key_id
  │   │       │   ├─► mark old state as "legacy" с grace window
  │   │       │   ├─► in-flight resolves получают old state до grace end
  │   │       │   └─► after grace → delete old
  │   │       │
  │   │       ├─► transaction commit:
  │   │       │   ├─► storage::CredentialStore::put(new_state)
  │   │       │   ├─► AuditLayer::write("rotated", {from_key_id, to_key_id})
  │   │       │   └─► CacheInvalidation broadcast
  │   │       │
  │   │       └─► eventbus::publish(CredentialEvent::Rotated)
  │   │
  │   └─► sleep until next check
  │
  ├─► на exit или crash — RotationLeaderClaimRepo::release (или let TTL expire)
  │
  └─► другая replica acquires leader claim
```

## Flow 6: Multi-step credential (Salesforce JWT bearer)

**Open:** as discussed, multi-step state accumulation открытый вопрос.

Desired flow shape (не implementation-ready):

```
Step 1: create credential — user provides private_key + client_id + username + audience
  │
  ▼ engine::CredentialService::create
  │
  ├─► validate config schema
  │
  ├─► Credential::resolve(values, ctx)
  │   │
  │   └─► NOT interactive, but multi-step (Capabilities::MULTI_STEP)
  │       │
  │       ├─► step 1: sign JWT assertion (pure)
  │       ├─► step 2: POST /oauth2/token with assertion → access_token + refresh_token
  │       │   (engine's reqwest client)
  │       └─► step 3: store as OAuth2-like state (access_token, expires_at)
  │
  ▼
stored state = { access_token, expires_at, private_key (kept for refresh) }

Refresh flow (когда near expiry):
  │
  ▼ Credential::refresh(&mut state, ctx)
  │
  ├─► re-sign JWT assertion (pure)
  ├─► POST token endpoint → new access_token
  └─► update state.access_token, state.expires_at
```

**Multi-step persistence (если crash mid-flow):**

Option considered:
```
PendingStore::upsert_step({
    credential_id,
    step_number: N,
    accumulator: JSON_value_of_prior_step_outputs,
    expires_at,
})
```

After crash, next attempt reads accumulator, skips prior completed steps. Idempotency key per step prevents duplicate side effects.

**Open (#22 — finding):** for which credential types нужно persistent multi-step vs in-memory atomic. Salesforce JWT — atomic (sign + exchange before return). Session login (API cookie) — possibly atomic too. Если N-step с external side effects (send OTP email → verify code) — persistence needed. Rare в credential domain, common в auth domain (Plane A — out of scope).

**Предложение (не final):** start with **atomic-only** multi-step — все steps happen within single `Credential::resolve()` call. Persistent multi-step deferred until real use case arises (DYNAMIC credential family possibly).

## Flow 7: Revocation

```
User ── DELETE /api/v1/credentials/:id ──► nebula-api
           │
           ▼ handlers::credentials::revoke
           │
           ├─► RBAC check (credential:delete scope)
           │
           ├─► engine::CredentialService::revoke(id, tenant_ctx)
           │   │
           │   ├─► storage::CredentialStore::get(id) → state
           │   │
           │   ├─► Credential::revoke(state, ctx)
           │   │   └─► if IdP supports revocation:
           │   │        POST {provider.revocation_endpoint} с tokens
           │   │        (best-effort — log failure, continue)
           │   │
           │   ├─► storage::CredentialStore::delete(id)  [atomic]
           │   │   ├─► EncryptionLayer no-op (just delete)
           │   │   ├─► AuditLayer::write("revoked", {...})
           │   │   └─► CacheInvalidation broadcast
           │   │
           │   ├─► eventbus::publish(CredentialEvent::Revoked)
           │   │
           │   └─► notify dependent resources (refresh lifecycle)
           │       └─► для active pools, connections — graceful close
           │
           ▼
         204 No Content
```

**In-flight actions (finding #18):** any action currently holding `CredentialGuard` continues using current material до drop. Next resolve получает `CredentialError::Revoked` — action может decide continue partial vs fail.

**Cascade revocation:** credentials с `Capabilities::COMPOSED` (SSH-tunneled DB) — revoke trigger dependent revoke? Open question. Metadata flag `CascadeRevoke = true/false` per credential type.

## Summary — what each flow validates

| Flow | Addresses | Status |
|---|---|---|
| 1 Creation | Happy path OAuth2 AuthCode start | Рабочее design, implementation exists |
| 2 Callback | HMAC state, token exchange, cache invalidation | Ok на paper, нужен cache invalidation impl |
| 3 Execute | Hot path with per-request injection | Core design работает, depends on SchemeInjector shape |
| 4 Refresh | Multi-replica coordination + crash recovery | **Open mid-refresh race** |
| 5 Rotation | Leader-elected scheduler + blue/green | **Open RotationLeaderClaim needed** |
| 6 Multi-step | Salesforce JWT atomic happy path | **Open persistent multi-step** |
| 7 Revoke | Soft revoke + cascade + in-flight handling | Design ok, needs revoke semantics ADR |
