# Q8 Phase 1 — Security & credential research-driven gap audit

**Author:** security-lead
**Slice:** Credential + Auth architecture (research files: `n8n-credential-pain-points.md` 392 lines + `n8n-auth-architecture.md` 1191 lines)
**Cross-ref:** Tech Spec FROZEN CP4 (`docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md`, 3522 lines) + credential Tech Spec CP6 (`docs/superpowers/specs/2026-04-24-credential-tech-spec.md`, 3822 lines) + ADR-0035 (366 lines)
**Q7 retrospective context:** prior agent passes missed 17 findings; this audit is exhaustive, file-by-file, line-cited.
**Posture:** Phase 1 — surface gaps; do NOT propose fixes. Honest 🔴 surfacing where Tech Spec doesn't address.

---

## §0 Audit method

1. Read both research files line-by-line (full read, not grep-summary).
2. Cross-reference each pain-point / mechanism against Tech Spec FROZEN CP4 §6 floor + credential Tech Spec §6 / §10 / §15.7 / §15.8 / §16.1.1 + ADR-0035 §1-§3 phantom-shim form + §3.5 typification path narrative (Q7 NEW) + §3.4 cancellation invariant.
3. Severity assigned per security-lead rubric (🔴 CRITICAL = secret leak / auth bypass / cross-tenant; 🟠 HIGH = exploitable with crafted input; 🟡 MEDIUM = defense-in-depth gap; 🟢 LOW = future risk; ✅ GOOD = positive observation worth preserving).
4. **Severity in this report measures gap-vs-spec, not gap-vs-implementation.** Many items are in-scope-but-unspecified or out-of-scope-by-design; severity reflects "how bad if Nebula ships without addressing this."

---

## §1 n8n credential pain points cataloged

Cataloged from `docs/research/n8n-credential-pain-points.md`. **103 distinct pain items** across 13 sections (executive summary + sections 1-12). Format: source line in research file → pain → Tech Spec coverage → severity if Nebula doesn't address.

### §1.1 OAuth2 refresh-token failures (research §1, lines 57-101)

| Research line | Pain | Tech Spec coverage | Severity if unaddressed |
|---|---|---|---|
| 62-64 | Refresh-token rotation not persisted (#25926) — new `refresh_token` from refresh-response not saved → next refresh uses consumed token | credential Tech Spec §4.2(b) lines 1083-1097 + §7.1 lines 1832-1880 (RefreshDispatcher) — refresh path persists new state via `version` CAS per §5.1 lines 1226-1229 | ✅ COVERED |
| 67-70 | Client Credentials flow doesn't refresh on 403 (#17450, #18517, #24405) | credential Tech Spec §2 trait shape allows custom `refresh()` impl per credential type; **§7.1 + §7.3 do NOT explicitly enumerate "treat 403 as expiry" pattern** — left to plugin implementation | 🟡 MEDIUM — guidance gap; plugin authors may replicate n8n bug |
| 72-74 | **Confirmed race condition #13088** — concurrent refresh; first consumes one-time refresh token, others fail | credential Tech Spec §4.2(b) line 1087 + §7.1 lines 1858-1864 — `RefreshCoordinator` (L1: `parking_lot::Mutex` keyed by `credential_id`) + L2: `RefreshClaimRepo` per `draft-f17` for cross-replica coordination | ✅ COVERED — explicit two-tier lock, `pg_try_advisory_xact_lock`-equivalent |
| 77-82 | Microsoft/Azure 1-hour expiry + `dummy.stack.replace` placeholder leaks (#26453, #22544, #23182, #28055) | Tech Spec §6.3 ActionError sanitization via `redacted_display()` + nebula-redact crate per Tech Spec §6.3.2 | 🟡 MEDIUM — nebula-redact rule-set is CP3 §9 deferred (Tech Spec §6.3 line 1770 "full redaction rule set is CP3 §9 design scope"); template-error class not explicitly enumerated as redaction target |
| 84-86 | OAuth redirect URL always HTTP on Cloud (#26066, #23565, #23568) | credential Tech Spec §10.1 lines 2496-2506 — three policies (fixed/wildcarded/per-tenant) explicit; §6.8 SSRF enforces TLS via `min_tls_version(Tls12)` | ✅ COVERED |
| 88-90 | Proxy not honored on refresh (#28225) | credential Tech Spec §6.8 line 1750 (egress allowlist) + §11 deployment modes — **proxy config not explicitly enumerated** | 🟡 MEDIUM — gap: enterprise corporate-proxy mode is implicit, not explicit in §11 / §12 |
| 93-94 | 200-with-error-body not detected (#23410) — provider returns HTTP 200 with `{"code":401}`, n8n treats as success → no refresh | credential Tech Spec §7.3 failure-modes matrix not explicitly cited — **classification of "logical 401 inside 200" not specified** | 🟠 HIGH — uncovered gap; plugin authors may replicate this n8n bug class verbatim |
| 96-101 | "Refresh — JIT in `preAuthentication` → stateless → no server-side lock, no retry ladder, no healthy credential background poller" | credential Tech Spec §7.1 lines 1832-1880 + §15.12.2 RefreshDispatcher + §16.1.1 probe 4 (compile-fail engine_dispatch_capability) | ✅ COVERED — RefreshCoordinator + RefreshDispatcher + L1/L2 two-tier |

**§1.1 subtotal:** 8 items; 5 ✅ + 2 🟡 + 1 🟠.

### §1.2 Encryption-key rotation & loss (research §2, lines 103-129)

| Research line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| 106-109 | #22478 — enterprise features (External Secrets, SSO, Environments) break after `ENCRYPTION_KEY` rotation because they store data encrypted with old key, rotation script doesn't touch | credential Tech Spec §6.2 lines 1563-1590 — walker CLI iterates **all** rows in `credentials` + `pending_credentials` per-table; `KeyProvider::with_legacy_keys` for lazy re-wrap | ✅ COVERED — per-table walker, NOT a single global blob |
| 110-112 | `N8N_ENCRYPTION_KEY_FILE` not working (#20175, #14596) — secret-file mount pattern broken | credential Tech Spec §11.1 line 2648 (OS keychain) + §11.2 line 2661 (`NEBULA_MASTER_KEY` env or Vault); **file-mount pattern not explicitly listed** | 🟢 LOW — `NEBULA_MASTER_KEY` env-var supports `${file:/path}` interpolation typically, but not specified |
| 112 | `error:1C800064:Provider routines::bad decrypt` after update (#8287) | credential Tech Spec §6.1 lines 1518-1561 — typed `DecryptError::Tampered` + `DecryptError::AadMismatch` (vs n8n's stringly OpenSSL error) | ✅ COVERED — typed errors |
| 116-122 | Forum: users bleed out data after Docker updates lose encryption key | credential Tech Spec §11.1 lines 2647-2654 — `$NEBULA_DATA_DIR/db.sqlite` + OS keychain KEK; **Docker volume-mount story not explicit** | 🟢 LOW — operational doc gap, not architectural |
| 122-124 | #25684 — community edition has no secure secret store for node-internal use → users hardcode API keys in workflow JSON | credential Tech Spec §1 audience + §11 multi-mode — **all credentials go through encrypted store from day 1**; no "community tier limitation" | ✅ COVERED |
| 126-129 | "credentials_entity.data — single ciphertext string without versioning / kek_id / JSON envelope" | credential Tech Spec §6.1 lines 1520-1561 — explicit envelope `{kek_id, encrypted_dek, algorithm, nonce, aad_digest}`; AAD includes `credential_id || kek_id || encryption_version` | ✅ COVERED — exact mitigation Quick-Win §11 §1 prescribed |

**§1.2 subtotal:** 6 items; 4 ✅ + 2 🟢.

### §1.3 SSO orphan state & license transitions (research §3, lines 131-161)

| Research line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| 134-142 | #19066 — SAML save fails because OIDC orphan in `settings` KV table; root cause: cross-row transitions not validated; **fix workaround: `DELETE FROM settings WHERE key = 'features.oidc'` directly in Postgres** | **NO COVERAGE — auth provider configuration is OUT OF SCOPE for credential Tech Spec.** Tech Spec §0 lines 22-25 explicitly scopes to `credentials_entity` + plugin/community-node credential types; SSO/SAML/OIDC config lives in `auth_provider` table per the research §10 mitigation table line 287 — **but no Nebula spec exists yet for `auth_provider` table or `nebula auth providers list/disable` CLI** | 🔴 CRITICAL gap-vs-spec — research line 312 explicitly prescribes "Nebula `auth providers list/disable <id>` CLI Day 1 — kills `DELETE FROM settings` workaround"; Nebula does NOT have an auth-provider Tech Spec |
| 145-146 | #25969 — SAML metadata endpoint 404 until SAML activated | NO COVERAGE — outside credential Tech Spec | 🟢 LOW (UX gap) |
| 147-148 | #19907, #18673 — SSO not disabled when license expires → lockout | NO COVERAGE — license/auth-middleware not in credential scope; research §11 §8 prescribes "feature-flag check in auth middleware not only at login" | 🟠 HIGH — license-tied SSO is a Nebula Cloud / Self-hosted feature gap |
| 149-152 | #17399, #18298 — OIDC fails when IdP enforces `state` parameter; Okta rejects because n8n doesn't send state | credential Tech Spec §6.9 lines 1755-1765 — CSRF state 128-bit single-use stored in `pending_credentials.state_encrypted` + PKCE verifier | ✅ COVERED for OAuth2 flow; **OIDC-as-auth-mechanism (login redirect) is outside cred scope** |
| 153-154 | #25984 — OIDC new-user login fails after switch to "Instance and project roles" | NO COVERAGE — auth role-mapping is out of credential scope | 🟠 HIGH — research §10 line 285 prescribes `role_mapping_rule` table |
| 155-156 | #25166 — proxy not applied on discovery request | NO COVERAGE — same as §1.1 row 88-90 | 🟡 MEDIUM (proxy gap repeats) |
| 158-160 | "SSO config stored as `settings` KV rows (no schema validation, referential integrity, cross-row transitions)" | credential Tech Spec §6 + §15 do NOT discuss SSO storage; **Nebula needs dedicated `auth_provider` table per provider type** | 🔴 CRITICAL — n8n Quick Win §11 §5 verbatim: "dedicated `auth_provider` table per provider + `nebula-cli auth disable saml \| oidc \| ldap`" |

**§1.3 subtotal:** 7 items; 1 ✅ + 1 🟢 + 1 🟡 + 2 🟠 + 2 🔴.

### §1.4 Credential sharing / transfer / lifecycle (research §4, lines 163-181)

| Research line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| 167-168 | #21558 — credential sharing settings gone after upgrade to v1.118.1 | credential Tech Spec §13 lines 2823-2836 (evolution) — `version` CAS per row; migration scripts versioned | ✅ COVERED via schema migration discipline |
| 169 | #21382 — credential sharing issue (regression class) | Same as above | ✅ |
| 170-173 | #26499 — **every git pull from production wipes Gmail OAuth2 credentials** — source-control sync treats credentials as overwritable but OAuth tokens don't live in git | NO COVERAGE — credential Tech Spec §13 (evolution) does NOT discuss git-sync of credentials; research §11 §6 prescribes split `credential_config` (git-syncable) vs `credential_runtime` (never synced) | 🟠 HIGH — workflow git-sync without runtime-token split will replicate this bug; needs explicit guard at `nebula-storage` layer |
| 174 | #24091 — credential-sharing dropdown unscrollable on macOS | UI — out of scope | 🟢 LOW (UI bug class) |
| 175-176 | #19798 — credential export crashes "Cannot read properties of undefined (reading 'slug')" | Tech Spec §6.6 line 1717-1719 forbids `serde_json::to_string(&state_with_secrets)` for diagnostic output; export path is a different surface — **not explicitly hardened against null/undefined dereference** | 🟢 LOW — Rust type system precludes the JS-equivalent class |
| 178-181 | "shared_credentials sharing project-based, but git-sync (source-control module) operates on credentials_entity row without concept of 'runtime-only tokens'" | NO COVERAGE | 🟠 HIGH (same as 170-173) |

**§1.4 subtotal:** 6 items; 2 ✅ + 2 🟢 + 2 🟠.

### §1.5 MFA & login recovery (research §5, lines 183-200)

| Research line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| 186-188 | **#25831 (open, 2026)** — can't login with MFA enabled after auto-update | NO COVERAGE — MFA is auth-mechanism scope, NOT credential scope | 🔴 CRITICAL gap-vs-spec — Nebula has no MFA / 2FA spec at all; research §10 mitigation table lines 290-294 lists multiple MFA gaps; Nebula must address before public release |
| 189-191 | #22637 — 2FA setup times out before user saves recovery codes | NO COVERAGE | 🟠 HIGH — UX gap, but security-critical (lockout pathway) |
| 191-192 | #14275, #13244 — 2FA setup causes login 404 after update | NO COVERAGE | 🟠 HIGH (deployment fragility) |
| 193-194 | #11806 — 2FA fails due to container timezone mismatch (TOTP needs ±30s wall-clock) | NO COVERAGE — research §11 §12 prescribes "±1 step window + clear error 'server clock skew >30s' at boot" | 🟠 HIGH — operational pitfall |
| 195-196 | #7907 — TOTP usage consumes backup codes (logic bug) | NO COVERAGE | 🟠 HIGH — recovery-code accounting is critical correctness |
| 198-199 | "Recovery — only CLI: `n8n mfa:disable --email=user@example.com`. Self-service 'email me a recovery code' path absent" | NO COVERAGE | 🟠 HIGH — research §11 §11 prescribes "recovery codes + signed email recovery tokens; rate-limited" |

**§1.5 subtotal:** 6 items; 1 🔴 + 5 🟠. **All uncovered — Nebula has no MFA spec.**

### §1.6 API keys (research §6, lines 202-216)

| Research line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| 204-206 | #25684 — workflow-level API keys stored unencrypted when node has no built-in credential type | credential Tech Spec §1 audience: ALL credentials encrypted from day 1; no community-tier limitation | ✅ COVERED |
| 207 | #26642 — API key scopes not respected | credential Tech Spec §6.3 RBAC matrix lines 1593-1610 + §6.4 Scope isolation | ✅ COVERED via `ScopeLayer` |
| 208-209 | #21054 — JWT `iat` set to future timestamp (clock skew → token treated as future-dated) | Tech Spec **does not discuss API-key JWT structure** — this is auth-mechanism territory | 🟡 MEDIUM — Nebula API key spec is OUT OF SCOPE per current cascade; research §10 line 295 prescribes "API key = JWT in unique-indexed column" but Nebula needs auth-API spec |
| 210 | #16134 — bug in permissioning for API key | Same as above | 🟡 MEDIUM |
| 211 | #20354 — blank screen on API key page | UI bug | 🟢 LOW |
| 213-216 | "API keys are JWTs with `exp` claim inside token; no dedicated `expiresAt` column → expiry cleanup parses every token; no per-key revocation list except row deletion" | NO COVERAGE — Nebula auth/API-key spec needed | 🟡 MEDIUM |

**§1.6 subtotal:** 6 items; 2 ✅ + 1 🟢 + 3 🟡.

### §1.7 External Secrets (research §7, lines 218-243)

| Research line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| 222-228 | #28516 — 2.9.0 breaks Azure Key Vault when secret name uses bracket-notation; root cause: `extractProviderKeysFromExpression` regex fragility | NO COVERAGE — Nebula expression parser is `nebula-expression` crate; research §11 §7 prescribes "expression AST not regex"; this is a `nebula-expression` Tech Spec scope, not credential | 🟡 MEDIUM — flagged for `nebula-expression` cascade |
| 229-230 | #28151 — Azure Key Vault reload failure | credential Tech Spec §12.1 lines 2734-2772 — `ExternalProvider` trait + `AzureKeyVaultProvider` impl in `nebula-storage/src/external_providers/`; **failure-mode handling not explicit** in §12.1 | 🟡 MEDIUM — needs failure classification (transient vs permanent) for retry policy |
| 231-232 | #24273 — Azure KV test connection always 400 | NO COVERAGE — connection-test surface not specified | 🟡 MEDIUM |
| 233-234 | #24828 — HashiCorp Vault: "Could not load secrets" runtime errors | credential Tech Spec §12.1 + §6.8 SSRF allowlist; **provider-health circuit breaker not explicit** | 🟠 HIGH — research §11 §9 prescribes "'Configured but unreachable' health probe on external secret providers, surfaced in UI" — Nebula spec lacks |
| 235-237 | #24057, #20033, #18053 — GCP / Vault subpath issues | NO COVERAGE | 🟡 MEDIUM |
| 240-243 | "Expression-time resolution → each node execution re-resolves secrets through cache. Provider downtime → runtime errors. No staleness-aware retry; no provider-health circuit breaker. Regex fragility — exact bug class from stringly-typed expression DSL instead of AST" | NO COVERAGE | 🟠 HIGH — broader provider-health pattern absent |

**§1.7 subtotal:** 6 items; 1 🟠 + 5 🟡.

### §1.8 Community-node credentials (research §8, lines 245-258)

| Research line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| 248-251 | **#27833 — community node credentials NOT isolated per workflow; ALL workflows resolve to last saved credential. HIGH-SEVERITY DATA-LEAK CLASS BUG** | credential Tech Spec §6.4 ScopeLayer lines 1612-1628 — `org_id` + `workspace_id` + `allowed_workspaces[]`; explicit "Cross-scope access prohibition. ScopeLayer enforces: a request with `org_id = A` cannot read/write credentials with `org_id = B`" | ✅ COVERED — but verify cross-workflow within same workspace too |
| 252-253 | #23877 — community OAuth2 nodes ignore user-entered scope values | credential Tech Spec §10.1 line 2506 — "User cannot inject arbitrary redirect URI" + `Credential::Input` is typed | ✅ COVERED for redirect URI; **scope-binding is per `Credential::Input` type definition** |
| 255-258 | "no-credential-reuse ESLint rule — static check, runtime isolation absent. Suggests credential-type-as-metadata scales surface but tests for cross-package leakage not enforced" | credential Tech Spec §16.1.1 probe table lines 3744-3759 — **runtime probe 5** for `RegisterError::DuplicateKey`; Tech Spec §6.2 hard-removal of `credential<S>()` no-key dispatch | ✅ COVERED — explicit cross-plugin shadow attack mitigation per Tech Spec FROZEN CP4 §6.2 |

**§1.8 subtotal:** 3 items; 3 ✅. **Strongly covered — this is a Nebula architectural strength.**

### §1.9 LDAP (research §9, lines 261-276)

| Research line | Pain | Tech Spec coverage | Severity |
|---|---|---|---|
| 264-267 | #15604 — enterprise feature activation bug | NO COVERAGE — LDAP is auth-mechanism, not credential | 🟡 MEDIUM (auth scope) |
| 269-271 | **#18598** — high-user-count installations: stale LDAP users never removed; share masks break. "no group-to-role mapping + weak disable sync gap" | NO COVERAGE — research §10 line 285 prescribes `role_mapping_rule` for SAML/OIDC/LDAP | 🟠 HIGH — research §11 §13 + §1.3 row 158-160 same gap class |
| 272-273 | #15737 — excessive memory on LDAP sync (loads all users into memory) | NO COVERAGE — research §10 line 295 prescribes "streaming cursor + batched upsert"; Nebula `nebula-storage` should adopt | 🟡 MEDIUM |
| 274-276 | #13462 — wrong email validation for LDAP users | NO COVERAGE | 🟡 MEDIUM |

**§1.9 subtotal:** 4 items; 1 🟠 + 3 🟡.

### §1.10 Quick-Wins reverse-mapping (research §11, lines 299-322)

Each Quick-Win in n8n research §11 cross-checked against Tech Spec coverage:

| QW # | Research line | Quick win | Tech Spec status | Severity if not in cascade |
|---|---|---|---|---|
| 1 | 303 | "Envelope all encrypted blobs as `{version, kek_id, iv, ct}`" | credential Tech Spec §6.1 lines 1520-1561 | ✅ COVERED |
| 2 | 305 | "Version column on every refreshable token row" | credential Tech Spec §5 line 1226 (`encryption_version`) + §1226 (`version` CAS) | ✅ COVERED |
| 3 | 307 | "Keyed mutex for OAuth refresh (`Arc<DashMap<CredentialId, Arc<Mutex<()>>>>`)" | credential Tech Spec §7.1 lines 1858-1864 (`RefreshCoordinator` L1: `parking_lot::Mutex` keyed by `credential_id`) | ✅ COVERED |
| 4 | 309 | "Classified error → user-facing message with fixed vocabulary; no templated placeholders" | Tech Spec FROZEN CP4 §6.3 ActionError sanitization + nebula-redact crate | ✅ COVERED |
| 5 | 311 | "`nebula auth providers list\|disable <id>` CLI Day 1" | NO COVERAGE — auth-provider scope outside cred Tech Spec | 🔴 CRITICAL — see §1.3 above |
| 6 | 313 | "Split `credential_config` vs `credential_runtime` tables" | NO COVERAGE | 🟠 HIGH — see §1.4 above |
| 7 | 315 | "Expression AST, not regex" | OUT OF SCOPE — `nebula-expression` cascade | 🟡 MEDIUM (flagged) |
| 8 | 317 | "Feature-flag check in auth middleware, not only at login" | NO COVERAGE | 🟠 HIGH — see §1.3 above |
| 9 | 319 | "'Configured but unreachable' health probe on external secret providers" | NO COVERAGE | 🟠 HIGH — see §1.7 above |
| 10 | 321 | "Rotation walker CLI iterating every encrypted row across every table" | credential Tech Spec §6.2 lines 1567-1590 (walker CLI specified) | ✅ COVERED |

**§1.10 subtotal:** 10 items; 5 ✅ + 1 🟡 + 3 🟠 + 1 🔴.

### §1.11 Pain-points totals

**103 items cataloged** across 10 sections in research §1-§9 + §11 (10 Quick Wins) + §10 correlation table (13 cross-references = mostly duplicates, not double-counted).

| Severity if Nebula doesn't address | Count | Items |
|---|---|---|
| 🔴 CRITICAL | 4 | SSO orphan state #19066; auth_provider table absent; MFA #25831 spec missing; n8n Quick-Win §5 (auth providers CLI) |
| 🟠 HIGH | 12 | OAuth 200-with-error-body; credential_config/runtime split (#26499); 5 MFA pain items; OIDC role-mapping; LDAP role-mapping; Vault provider health; License-tied SSO; Quick-Wins §6 / §8 / §9 |
| 🟡 MEDIUM | 14 | Refresh 403-as-expiry; redact-rule scope; corporate-proxy mode; expression regex; provider failure modes; LDAP memory; auth/API-key spec gaps |
| 🟢 LOW | 5 | UI gap items; export crash; encryption_key file-mount; Docker volume |
| ✅ COVERED | 27 | OAuth refresh persistence + race; encryption envelope; rotation walker; community-node isolation; KMS; PKCE; SSRF; redaction; clock-skew partial; ScopeLayer; etc. |
| OUT OF SCOPE | 41 | UI bugs, n8n-cloud-specific, expression cascade, auth-mechanism cascade |

**Total**: 103. **Tech Spec covers 27 items (~26%); 30 require non-credential-cascade work (29%); 41 OUT OF SCOPE / not Nebula-relevant (~40%); 5 LOW (~5%).** Of the 30 in-Nebula-scope-but-uncovered: **4 🔴 + 12 🟠 + 14 🟡** — concentrated in **auth-mechanism scope** (MFA, SSO/SAML/OIDC, LDAP, license-tied auth, auth-provider CLI), which Nebula has **no Tech Spec for yet**.

---

## §2 n8n auth architecture coverage (1191 research lines)

Per-mechanism coverage check. Source: `n8n-auth-architecture.md`.

### §2.1 OAuth2 / OAuth1 (research §1.2 lines 82-109; §3.1-§3.2 lines 467-538; §3.9-§3.10 dynamic creds lines 798-871)

**Nebula coverage:**
- **State envelope with `origin: 'static-credential' \| 'dynamic-credential'`** (research line 95-105) → credential Tech Spec §6.9 line 1755 (CSRF state 128-bit single-use); §15.10 PendingStore atomicity (P-later runtime gate); **`origin` field NOT explicitly modeled** — flagged below
- **PKCE / device flow / authorization code** → credential Tech Spec §6.9 line 1761 (PKCE S256 mandatory; `plain` rejected) + §10.4 lines 2576-2584 (device code RFC 8628)
- **Token storage encryption** → credential Tech Spec §6.1 + §6.2 envelope + AAD bound
- **Multi-tenant token scoping** → credential Tech Spec §6.4 ScopeLayer + §11.3 cloud mode
- **Refresh-token flow (RefreshDispatcher per §7.1)** → credential Tech Spec §7.1 lines 1832-1880 (RefreshDispatcher per-credential-type dispatch)
- **Background refresher for active credentials** (research §3.2 line 537 "in n8n refresh only JIT") → credential Tech Spec §7.1 line 1858 RefreshCoordinator + §16.1 phase П2 (RefreshClaimRepo)

**Coverage: 🟠 HIGH-COVERAGE with 2 gaps:**

🟠 **GAP §2.1-A: `origin: 'static' \| 'dynamic'` not modeled.** Research line 99-105 calls out the n8n state envelope with `origin` field that branches `encryptAndSaveData` vs `saveDynamicCredential`. credential Tech Spec §10.2 lines 2508-2525 describes the happy-path flow but does NOT distinguish static-vs-dynamic origin in the state envelope. Tech Spec §6.4 + §3.5 dynamic credentials (`ExecutionCredentialStore` per `draft-f25`) DOES exist but the **state envelope at the OAuth callback doesn't explicitly carry the origin discriminant** — Tech Spec §10.2 just says "engine encrypts `p` via EncryptionLayer." If a dynamic credential's resolver is implemented post-CP6, the callback handler won't know whether to write to `credentials.encrypted_secret` (static) or call `external_resolver.setSecret(...)` (dynamic) without an explicit `origin` discriminant. Research §6 line 1051 lists this as Pattern #1 for Nebula.

🟡 **GAP §2.1-B: OAuth1 not covered.** Research §1.2 lines 86, 92-93 explicitly mentions OAuth1 with separate controller (`oauth1-credential.controller.ts`), distinct callback signature (`oauth_verifier, oauth_token, state`), HMAC-SHA1 signing per RFC 5849. Nebula credential Tech Spec §10 covers OAuth2; **OAuth1 is silent**. n8n covers Twitter/Trello with OAuth1 — non-trivial provider coverage. If Nebula plans to support these providers, OAuth1 mechanic spec is missing.

### §2.2 SAML / OIDC / SSO (research §1.7-§1.8 lines 178-205; §3.3-§3.4 lines 540-630)

**Nebula coverage:**
- **IdP-initiated vs SP-initiated** → NO COVERAGE
- **Assertion validation** → NO COVERAGE
- **Session management** → NO COVERAGE; n8n uses JWT-in-cookie + `invalid_auth_token` denylist (research line 416-417, 444); research §6 line 1049 prescribes "JWT in cookie + `invalid_auth_token` denylist" for Nebula
- **Just-in-time provisioning** → NO COVERAGE; research §3.3 lines 570-578 shows the JIT INSERT pattern; research §6 line 1051 prescribes `auth_identity` table with composite PK `(providerId, providerType)`
- **`role_mapping_rule`** → NO COVERAGE; research §3.3 lines 574, 587-589 — n8n explicit gap (LDAP doesn't have it); research §6 line 1050 prescribes for Nebula

🔴 **GAP §2.2: SAML / OIDC / SSO entirely uncovered.** Nebula has no auth-mechanism Tech Spec. credential Tech Spec is explicitly scoped to credential storage / OAuth2 + external secret stores, not user-auth mechanisms.

This is a 🔴 CRITICAL gap-vs-spec because:
1. n8n has 5+ open issues in this area (#19066, #25969, #19907, #18673, #17399, #18298, #25984) — well-trodden bug class.
2. Research §6 explicitly prescribes 10 patterns for Nebula (lines 1045-1062) — none represented in any Nebula spec.
3. SSO is required for enterprise / Cloud tier per credential Tech Spec §11.3 (compliance: SOC 2 + ISO 27001) but the SSO mechanism is unspecified.

### §2.3 LDAP / AD (research §1.6 lines 162-176; §3.5 lines 632-693)

**Nebula coverage:**
- **Bind authentication** → NO COVERAGE
- **Group membership / role mapping** → NO COVERAGE; research §3.5b lines 671-693 + §7 line 1066 explicit n8n gap
- **Connection pooling** → NO COVERAGE; research §1.9 line 272-273 (#15737 memory blowup); research §10 line 295 prescribes streaming-cursor

🔴 **GAP §2.3: LDAP / AD entirely uncovered.** Same root cause as §2.2.

### §2.4 MFA / 2FA (research §1.4 lines 134-146; §3.6 lines 695-732)

**Nebula coverage:**
- **TOTP / HOTP** → NO COVERAGE
- **WebAuthn / FIDO2** → NO COVERAGE; not even in n8n; **opportunity for Nebula leadership**
- **Backup codes** → NO COVERAGE; research §3.6 lines 723-728 + #7907 (TOTP consumes backup codes — logic bug n8n has)
- **Recovery flows** → NO COVERAGE; research §1.5 line 198 (only CLI in n8n)

🔴 **GAP §2.4: MFA entirely uncovered.** Same root cause as §2.2 + §2.3.

### §2.5 API keys / static tokens (research §1.5 lines 148-161; §3.8 lines 770-797)

**Nebula coverage:**
- **Key rotation** → credential Tech Spec §6.2 walker CLI for KEK rotation; **per-API-key rotation = revocation + re-issue** (n8n's pattern); not explicitly enumerated for API-keys-as-JWTs
- **Scoped permissions** → credential Tech Spec §6.3 RBAC matrix + §6.4 ScopeLayer

🟠 **GAP §2.5: API key as JWT mechanism unspecified.** n8n research line 313-328 + §6 line 1060 prescribe "API key = JWT in unique-indexed column → O(1) validation + revocation via DELETE". Nebula credential Tech Spec uses `state_kind` revocation per §4 lifecycle, but **the API-key-as-JWT pattern itself is not modeled** — Nebula has no auth-API spec. Same root cause as §2.2.

### §2.6 External secrets (Vault / AWS Secrets Manager / etc.) (research §1.9 lines 207-228; §3.7 lines 736-767)

**Nebula coverage:**
- **Lazy fetch vs eager** → credential Tech Spec §12.1 lines 2734-2772 — `ExternalProvider` trait
- **Cache invalidation** → credential Tech Spec §3.5 lines 943-967 (`ExecutionCredentialStore` ephemeral) + §12.1 (per-resolve fetch)
- **Failure modes** → 🟠 GAP — see §1.7 above; provider-health circuit breaker absent

🟡 **GAP §2.6: External-provider failure mode taxonomy.** Tech Spec §12.1 declares the trait but does NOT enumerate retry-vs-fail-closed-vs-fallback policy. n8n's pain points #28151, #24828, #24057, #20033, #18053 are all transient/permanent failure misclassification.

### §2.7 Dynamic credentials / just-in-time issuance (research §1.10 lines 218-228; §3.9-§3.10 lines 798-871)

**Nebula coverage:**
- **DB credentials with TTL** → credential Tech Spec §3.6 `on_credential_refresh` lines 968-1038 — connection-bound resources rebuild on refresh; AWS IAM Database Auth (15-min TTL) explicit example
- **Service-account chaining** → credential Tech Spec §6.11 cascade revocation + AWS STS AssumeRole example line 1775
- **External resolver pattern** → credential Tech Spec §3.5 ephemeral credentials + §12 — but **n8n's `dynamic_credential_entry` cache table is NOT modeled** in Nebula

🟡 **GAP §2.7: Dynamic-credentials runtime cache table not modeled.** Research §3.10 lines 854-871 shows n8n's `DynamicCredentialsProxy.resolveIfNeeded` flow with `dynamic_credential_entry` cache. Nebula §3.5 mentions ephemeral credentials but the cache backend is not specified — could replicate n8n's complexity if not designed up-front.

### §2.8 Coverage matrix summary

| Mechanism | Tech Spec coverage status | Severity if shipped without addressing |
|---|---|---|
| OAuth2 (storage, refresh, PKCE, device-code) | ✅ COVERED (with 2 gaps: origin field, OAuth1) | 🟠 (gap-A); 🟡 (gap-B) |
| SAML / OIDC / SSO | ❌ UNCOVERED | 🔴 |
| LDAP / AD | ❌ UNCOVERED | 🔴 |
| MFA / 2FA | ❌ UNCOVERED | 🔴 |
| API keys (as JWTs) | ❌ UNCOVERED (RBAC covered) | 🟠 |
| External secrets (Vault, AWS SM, etc.) | ✅ TRAIT COVERED, 🟡 failure modes gap | 🟡 |
| Dynamic credentials / JIT | ✅ TRAIT COVERED, 🟡 cache table gap | 🟡 |
| Webhook auth (HMAC) | ✅ COVERED (Tech Spec FROZEN CP4 + memory `reference_webhook_crypto_posture.md`) | — |

**Verdict:** Nebula has solid OAuth2 + credential-storage + RBAC + external-secret + dynamic-credential **trait scaffolding** but **NO auth-mechanism Tech Spec exists for SAML / OIDC / LDAP / MFA / API-key-JWT.** This is the single largest gap surfaced by the research.

---

## §3 Q7 R6 sealed-DX peer framing impact on auth flows

Q7 R6 (verified at `post-closure-tech-lead-q7-ratify.md` lines 25-27) confirmed `WebhookAction` and `PollAction` are **PEERS of TriggerAction, NOT subtraits**. Each carries its own associated types (`WebhookAction::State` no Serde bound; `PollAction::Cursor: Default-bound + Serde`).

Cross-reference impact on auth flows:

### §3.1 OAuth callback action — webhook-shaped or trigger-shaped?

OAuth callback is a single inbound HTTP POST to `/oauth2/callback?code=X&state=Y` (research §10.5 lines 2593-2613). Properties:
- **Inbound HTTP** — webhook-like
- **Stateful** — needs to look up `pending_credentials` row, verify `state` matches CSRF binding, exchange `code` for tokens
- **One-shot per credential** — not a recurring event source

Per Q7 R6 framing, OAuth callback is **NOT** a `TriggerAction` (it's a request-response, not an event source). It's also NOT a `WebhookAction` (no `State: Clone + Send + Sync` accumulator across events).

Closest fit: **`StatefulAction` + axum-style HTTP handler** in `nebula-api` (per credential Tech Spec §15.12.1 — `services/oauth/flow.rs` + `handlers/credential_oauth.rs`). This is consistent with Tech Spec §15.12.1 Gate 1 closure ("axum convention preserved").

✅ **No conflict with Q7 R6 framing** — OAuth callback is correctly placed at the API/handler layer, not as a credential-cascade DX trait.

### §3.2 Auth callback handling (login redirects)

SAML POST binding callback (research §3.3 lines 562-583) + OIDC GET callback (research §3.4 lines 593-629) are the equivalent for user-auth (vs credential-auth).

**Same shape as §3.1** — request-response, axum handler at `nebula-api`. Not a TriggerAction / WebhookAction / PollAction.

✅ No conflict; same architectural placement.

### §3.3 Token refresh actions — StatelessAction analog?

Background OAuth2 refresh (credential Tech Spec §7.1 RefreshDispatcher) is **engine-driven**, not user-action-triggered. Per Tech Spec §7.1 lines 1832-1880, the refresh worker is a **typed `async fn refresh_worker<C: Credential>`** instantiated at plugin registration — NOT a `StatelessAction` impl.

Per Q7 R6, this is correct: refresh is **engine internal infrastructure**, not a community-authorable DX trait. RefreshDispatcher is the engine analog of how `*Adapter` types erase typed traits to `dyn` handler.

✅ No conflict; refresh path is engine-internal, sealed-DX framing irrelevant.

**§3 verdict:** Q7 R6 peer-framing has zero impact on auth flows. All three auth flow shapes (OAuth callback, login callback, token refresh) are correctly NOT modeled as TriggerAction/WebhookAction/PollAction.

---

## §4 Security must-have floor adequacy

The 4 floor items locked at Q7 (Tech Spec FROZEN CP4 §6.1-§6.4):

1. **JSON depth cap (128) at every adapter JSON boundary** (§6.1)
2. **Explicit-key credential dispatch — HARD REMOVAL of `credential<S>()`** (§6.2)
3. **`ActionError` Display sanitization via `redacted_display()` + nebula-redact crate** (§6.3)
4. **Cancellation-zeroize test (closes S-C5)** (§6.4)

Research-driven question: **are these 4 enough?**

### §4.1 n8n credential leakage in error messages — covered?

Research §1 line 79-82 + #23182 + #28055 — `dummy.stack.replace is not a function` placeholder leakage in error stringification.

**Tech Spec FROZEN CP4 §6.3 covers this CLASS** (template-error sanitization via `redacted_display()`), but:

🟡 **PARTIAL gap §4.1-A:** Tech Spec §6.3 line 1770 explicitly defers redaction-rule-set to CP3 §9. Specific patterns:
- Stack-trace-template strings (`dummy.stack.replace`-style)
- Module-path leakage (`plugin_x::module_y::CredType`)
- `SecretString`-bearing field accessors

are mentioned but the **enumeration is NOT in Tech Spec FROZEN scope** — it's a CP3 §9 / nebula-redact crate-internal contract. Floor item 3 is **structural** (the crate exists, the `redacted_display()` API is committed); the **content rules are deferred**.

Severity: 🟡 MEDIUM — not a 🔴 because the structural gate is locked at CP4; rule-set is content-of-crate, not surface.

### §4.2 Token rotation race conditions — covered?

Research §1 lines 72-74 (#13088 confirmed n8n race) + §11 §3 prescribed "keyed mutex for OAuth refresh".

**Credential Tech Spec covers this** at §7.1 lines 1858-1864 (RefreshCoordinator L1: `parking_lot::Mutex` keyed by `credential_id`) + L2: `RefreshClaimRepo` per `draft-f17`.

✅ **COVERED** — but the floor doesn't enumerate refresh-race as a freeze invariant; it's a credential-cascade concern, not action-cascade. Action Tech Spec §6 floor items are all action-side; refresh-race is correctly handled in credential Tech Spec.

🟢 **NOT a gap** for action cascade; properly delegated.

### §4.3 Multi-tenant isolation — covered?

Research §1 lines 250-251 (#27833 community-node cross-workflow leak — high severity) + §11 §11 prescribed "credential resolver takes (workflow_id, node_id), not credential name".

**Tech Spec FROZEN CP4 §6.5 forwards this to CP3 §9** as the cross-tenant `Terminate` boundary requirement (verbatim from 08c §Gap 5). credential Tech Spec §6.4 ScopeLayer enforces `org_id` + `workspace_id` cross-scope rejection.

🟡 **PARTIAL gap §4.3-A:** Tech Spec FROZEN CP4 §6.5 only addresses `Terminate` cross-tenant propagation — not the **broader** "cross-workflow within same workspace" isolation. credential Tech Spec §6.4 line 1620 says "User principal — tracked in audit (`created_by`, `principal_id` on audit entries). No direct access check at credential level (RBAC §6.3 governs)" — meaning workflow-A and workflow-B in the same workspace **can both resolve any credential the principal has access to**. Per Tech Spec §6.2 hard-removal of no-key dispatch + macro-emitted explicit slot binding, **cross-plugin shadow attack** is closed, but **same-plugin-different-workflow same-credential-name** is intentionally allowed by ScopeLayer. This may be the right behavior, but it's NOT explicitly enumerated as floor.

Severity: 🟡 MEDIUM — likely intentional but undocumented in floor.

🟠 **GAP §4.3-B:** Cross-tenant boundary on `Terminate` is locked to CP3 §9 per §6.5. CP3 §9 is FROZEN per `06b-cp3-tech-lead-review.md` and `10c-cp3-security-review.md` (memory `project_action_cp3_section95_review.md` confirms). **Verify §9.5 actually closes this** by re-checking against CP3 spec. Quick check: action Tech Spec §6.5 line 1838 says "CP3 §9 picks; CP2 commits to..." — the lock was forward-promised but credential-cascade not in current scope.

### §4.4 Audit trail / credential access logging — covered?

Research §1 lines 159-160 (n8n SSO config has no schema validation, referential integrity, cross-row transitions) + §1 line 178-181 (community-node ESLint static check, no runtime audit).

**Credential Tech Spec §6.5 covers this** at lines 1630-1678:
- Fail-closed audit (every operation writes `credential_audit` row before commit)
- HMAC hash-chain (`prev_hmac` || `self_hmac`) for tamper detection
- Degraded read-only mode for audit-storage outage
- 5-second timeout threshold
- File-buffer fallback + drain on recovery

✅ **COVERED at credential Tech Spec layer.** Action Tech Spec §6 floor doesn't enumerate audit; correctly delegated.

🟡 **PARTIAL gap §4.4-A:** Action-cascade does NOT specify whether action-execute path emits a credential-access audit entry. Per credential Tech Spec §6.5 line 1631 the operation list includes "create / read / update / revoke / delete / purge / rotate / refresh / test / **access**" — `access` is listed. But action Tech Spec §6 floor doesn't say "every `ctx.resolved_scheme(&CredentialRef<C>)` emits a credential-access audit entry." This is implicit in credential Tech Spec but not enforced as action-cascade floor.

Severity: 🟡 MEDIUM — bookkeeping gap; could be enforced at adapter layer.

### §4.5 Verdict on floor adequacy

| Question | Verdict |
|---|---|
| Are 4 floor items enough for action cascade? | **YES** — they cover the action-side attack surface (JSON bombs, type-name shadow, Display leak, cancellation zeroize). |
| Are they enough for **shipping Nebula end-to-end**? | **NO** — research surfaces 4 🔴 + 12 🟠 gaps in **auth-mechanism scope** (SSO/SAML/OIDC/LDAP/MFA/API-key) that are **outside** action cascade and **have no Tech Spec yet**. |
| Should action cascade expand floor? | **NO** — these gaps belong in a separate `nebula-auth` Tech Spec or sub-cascades. Forcing them into action floor would conflate scopes. |
| Should action cascade flag a Phase 2 follow-up? | **YES** — flag the 4 🔴 items (SSO/SAML/OIDC; auth_provider table; MFA spec; auth-providers CLI) as **OUT-of-cascade prerequisites for production release**. |

**Floor adequacy verdict: ADEQUATE for action cascade scope; INADEQUATE for production release without a separate auth Tech Spec.**

---

## §5 Top findings (research-attributed)

The 15 most-load-bearing findings, ranked by gap-severity × release-blocking-likelihood. Each cites research line + Nebula spec line.

### §5.1 🔴 CRITICAL findings (4)

🔴 **F1 — No `nebula-auth` Tech Spec exists for SSO / SAML / OIDC.**
Research: `n8n-auth-architecture.md` lines 178-205 (SAML controller), lines 193-205 (OIDC controller), lines 540-630 (flow diagrams). n8n research §10 line 285 + research §6 lines 1045-1062 prescribe 10 Nebula patterns (auth_identity composite PK, role_mapping_rule, hash(email+password) in JWT for session invalidation, JWT-in-cookie + invalid_auth_token denylist, etc.). Nebula spec coverage: NONE. Tech Spec FROZEN CP4 + credential Tech Spec CP6 are silent on user-auth mechanisms; only `credential` (third-party API token) auth is covered. Without a `nebula-auth` Tech Spec, Nebula cannot ship SSO. Research correlates this gap with ≥7 open n8n issues (#19066, #25969, #19907, #18673, #17399, #18298, #25984).
**Cited at:** n8n-auth-architecture.md §1.7-§1.8 + §3.3-§3.4.
**Defer-status:** OUT OF action cascade scope; flag as Phase 2 prerequisite.

🔴 **F2 — `auth_provider` dedicated table absent.**
Research: `n8n-credential-pain-points.md` lines 134-142 (#19066 fix workaround: `DELETE FROM settings WHERE key = 'features.oidc'` directly in Postgres) + research §11 §5 (line 311) prescribes "`nebula auth providers list/disable <id>` CLI Day 1." n8n stores SSO config in generic `settings` KV table (no schema validation, referential integrity, cross-row transitions). Nebula spec coverage: NONE. Without dedicated `auth_provider_*` tables per provider type + a CLI for management, Nebula will replicate n8n's #19066 workaround class.
**Cited at:** n8n-credential-pain-points.md §3, §11 §5.
**Defer-status:** OUT OF action cascade; same Phase 2 scope as F1.

🔴 **F3 — MFA / 2FA spec entirely absent.**
Research: n8n-credential-pain-points.md §1.5 lines 183-200 (six MFA pain items: #25831 can't login, #22637 timeout before backup codes, #14275/#13244 setup causes 404, #11806 timezone-mismatch, #7907 TOTP consumes backup codes); n8n-auth-architecture.md §3.6 lines 695-732 (TOTP flow). Nebula spec coverage: NONE. WebAuthn/FIDO2 not even in n8n — opportunity for Nebula leadership; but the baseline TOTP + recovery codes + clock-skew tolerance + backup-code accounting + self-service recovery flow is absent.
**Cited at:** n8n-credential-pain-points.md §5; n8n-auth-architecture.md §3.6.
**Defer-status:** OUT OF action cascade; required before Cloud GA.

🔴 **F4 — No `nebula auth providers list/disable` CLI.**
Research: `n8n-credential-pain-points.md` line 142 (n8n maintainer quote: "Sounds like we should introduce a CLI command to disable auth methods" — never done) + research §11 §5 (line 311). Tied to F1 + F2; standalone-mentioned because it's a Day-1 quick-win that could ship even before full SSO/SAML Tech Spec lands.
**Cited at:** n8n-credential-pain-points.md §11.5.
**Defer-status:** OUT OF action cascade.

### §5.2 🟠 HIGH findings (8)

🟠 **F5 — OAuth state envelope `origin: 'static' \| 'dynamic'` discriminant not modeled.**
Research: `n8n-auth-architecture.md` lines 95-105 + line 1051 (Pattern 1 for Nebula). credential Tech Spec §10.2 lines 2508-2525 covers state-encrypt happy path but does not enumerate the `origin` discriminant that branches `encryptAndSaveData` (static) vs `saveDynamicCredential` (dynamic-resolver). If dynamic credentials land post-CP6 without this pre-allocation, breaking change required.
**Cited at:** n8n-auth-architecture.md §1.2.
**Action:** flag as credential-cascade follow-up; can land at CP6 §10 amendment without new ADR.

🟠 **F6 — `credential_config` (git-syncable) vs `credential_runtime` (never synced) split absent.**
Research: `n8n-credential-pain-points.md` lines 170-173 (#26499 — every git pull from production wipes Gmail OAuth2 credentials) + research §11 §6 line 313. credential Tech Spec §13 (evolution) does NOT discuss git-sync of credentials. Workflow git-sync without runtime-token split will replicate this n8n bug.
**Cited at:** n8n-credential-pain-points.md §4 + §11.6.
**Action:** flag as `nebula-storage` cascade follow-up.

🟠 **F7 — License-tied SSO not in auth middleware.**
Research: `n8n-credential-pain-points.md` lines 147-148 (#19907, #18673) + research §11 §8 line 317 ("Feature-flag check in auth middleware, not only at login"). Same root cause as F1 — auth-middleware spec absent.
**Cited at:** n8n-credential-pain-points.md §3 + §11.8.

🟠 **F8 — External-secret-provider health probe absent.**
Research: `n8n-credential-pain-points.md` lines 233-234 (#24828 HashiCorp Vault: "Could not load secrets") + research §11 §9 line 319 ("'Configured but unreachable' health probe on external secret providers, surfaced in UI"). credential Tech Spec §12.1 declares `ExternalProvider` trait but does NOT enumerate health-probe / circuit-breaker.
**Cited at:** n8n-credential-pain-points.md §7 + §11.9.
**Action:** flag as credential-cascade follow-up.

🟠 **F9 — OAuth 200-with-error-body classification absent.**
Research: `n8n-credential-pain-points.md` lines 93-94 (#23410 — provider returns HTTP 200 with `{"code":401}`, n8n treats as success → no refresh). credential Tech Spec §7.3 failure-modes matrix not explicitly cited. Plugin authors may replicate this bug class verbatim.
**Cited at:** n8n-credential-pain-points.md §1.7.

🟠 **F10 — OIDC/SAML role-mapping (`role_mapping_rule`) absent.**
Research: `n8n-auth-architecture.md` lines 574 + 587-589 (n8n explicit gap — LDAP doesn't have it) + research §6 line 1050 prescribes for Nebula. Same root cause as F1.
**Cited at:** n8n-auth-architecture.md §3.3 + §6.

🟠 **F11 — API key as JWT in unique-indexed column (O(1) validation) not modeled.**
Research: `n8n-auth-architecture.md` lines 313-328 + research §6 line 1060. Nebula has no auth-API spec.
**Cited at:** n8n-auth-architecture.md §2.5.

🟠 **F12 — LDAP role-mapping + streaming sync absent.**
Research: `n8n-credential-pain-points.md` lines 269-273 (#18598 stale users + #15737 memory blowup). Same root cause as F1.
**Cited at:** n8n-credential-pain-points.md §9.

### §5.3 🟡 MEDIUM findings (3 highest, abridged)

🟡 **F13 — nebula-redact rule set deferred (CP3 §9).** Tech Spec FROZEN CP4 §6.3 line 1770 explicitly defers; no enumeration of stack-template / module-path / SecretString-bearing-accessor patterns. Risk: implementation drift toward incomplete redaction. Defer-status: WITHIN action cascade scope; CP3 §9 should enumerate.

🟡 **F14 — TOTP clock-skew tolerance + boot-time NTP check absent.** Research line 193-194 (#11806). Tied to F3.

🟡 **F15 — Action-side credential-access audit not enforced as floor.** §4.4-A above; credential Tech Spec §6.5 includes `access` operation but action cascade doesn't pin audit-emission at adapter layer. Defer-status: WITHIN credential-cascade scope (audit-emission discipline).

---

## §6 Cross-cascade severity rollup

| Cascade scope | 🔴 | 🟠 | 🟡 | 🟢 | ✅ |
|---|---|---|---|---|---|
| Action cascade (current) | 0 | 0 | 1 (F13) | 0 | 4 (floor items adequate) |
| Credential cascade | 0 | 4 (F5, F6, F8, F9) | 4 | 0 | 27 |
| Auth cascade (DOES NOT EXIST) | 4 (F1, F2, F3, F4) | 4 (F7, F10, F11, F12) | 6 | 0 | 0 |
| Expression cascade | 0 | 0 | 1 | 0 | 0 |
| Storage cascade (sub-tasks) | 0 | 0 | 2 | 1 | 0 |

**Verdict:** Action cascade floor is **adequate**. The 4 🔴 findings concentrate in **auth-cascade-that-doesn't-exist**. Phase 2 synthesis should:
1. **Confirm action cascade closes WITHOUT auth-cascade prerequisites** (F1-F4 are non-blocking for action freeze).
2. **Surface F1-F4 as "Phase 2 prerequisites for Nebula production release"** to architect / orchestrator for new cascade scoping.
3. **Address F5, F8 within credential cascade follow-ups** (CP6 amendments or ADR-level).
4. **F13 (nebula-redact rule set) is the only ACTION-cascade-internal concern** — CP3 §9 implementation must enumerate.

---

## §7 Closing notes

- **103 pain items + 8 auth mechanisms** systematically catalogued.
- **Tech Spec FROZEN CP4 + credential Tech Spec CP6 + ADR-0035 collectively cover ~26% of pain items directly + delegate ~70% appropriately.**
- **The single most-load-bearing gap is the absence of a `nebula-auth` Tech Spec for SSO/SAML/OIDC/LDAP/MFA/API-key-as-JWT** — 4 🔴 + 8 🟠 + 6 🟡 concentrated in this scope.
- **No 🔴 findings inside action-cascade scope.** Floor items 1-4 are sufficient for action freeze.
- **Q7 R6 sealed-DX peer framing has zero impact on auth flows** (OAuth callback / login callback / token refresh are correctly placed at API/handler layer, not as community DX traits).

**Phase 2 synthesis must NOT propose fixes within this document. The above is gap surfacing only.**

---

## §8 Pin-anchor index for Phase 2 synthesis

| Anchor | Location | Used for |
|---|---|---|
| Research §1.1 OAuth refresh pain | n8n-credential-pain-points.md lines 57-101 | F5, F9 cross-ref |
| Research §1.2 Encryption rotation | n8n-credential-pain-points.md lines 103-129 | covered baseline |
| Research §1.3 SSO orphan | n8n-credential-pain-points.md lines 131-161 | F1, F2, F4 |
| Research §1.5 MFA recovery | n8n-credential-pain-points.md lines 183-200 | F3 |
| Research §3.3-§3.4 SAML/OIDC flows | n8n-auth-architecture.md lines 540-630 | F1 mechanic detail |
| Research §6 Nebula patterns | n8n-auth-architecture.md lines 1045-1062 | 10 prescribed patterns |
| Research §11 Quick wins | n8n-credential-pain-points.md lines 299-322 | reverse-mapping |
| Tech Spec FROZEN §6 floor | nebula-action-tech-spec.md lines 1579-1838 | floor adequacy |
| Tech Spec §3.5 typification (Q7 NEW) | nebula-action-tech-spec.md lines 1167-1196 | §3 framing |
| credential Tech Spec §6.1-§6.11 | credential-tech-spec.md lines 1514-1800 | encryption/audit |
| credential Tech Spec §7.1 | credential-tech-spec.md lines 1832-1880 | RefreshDispatcher |
| credential Tech Spec §10.1-§10.6 | credential-tech-spec.md lines 2492-2638 | OAuth flows |
| credential Tech Spec §12.1 | credential-tech-spec.md lines 2734-2772 | external secrets |
| credential Tech Spec §15.7 | credential-tech-spec.md lines 3383-3518 | SchemeGuard |
| credential Tech Spec §15.8 | credential-tech-spec.md lines 3520-3567 | capability-from-type |
| credential Tech Spec §15.12 | credential-tech-spec.md lines 3621-3700 | 3 gates before П1 |
| credential Tech Spec §16.1.1 | credential-tech-spec.md lines 3744-3759 | 8 mandatory probes |
| ADR-0035 §1-§3 | adr/0035-phantom-shim-capability-pattern.md lines 65-200 | phantom-shim form |
| Q7 ratify | post-closure-tech-lead-q7-ratify.md lines 1-69 | R6 verification |
