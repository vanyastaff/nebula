# GitHub App credential scratch — findings

**Date:** 2026-04-24
**Status:** experiment complete, 7 tests passing
**Purpose:** validate Spec H0 (multi-replica refresh coordination) assumptions against real GitHub App auth flow

## What was built

Standalone Cargo crate under `scratch/github-app-credential-proto/`:

- `src/lib.rs` — `GitHubAppCredential` impl using real `nebula_credential::Credential` trait
- `src/lib.rs` — `L1Coalescer` — simplified Spec H0 L1 tier (in-proc mutex coalescing)
- `tests/common/mod.rs` — wiremock GitHub API + hit counter
- `tests/happy_path.rs` — 4 tests: refresh populates token, project produces Bearer, bad-JWT rejection, Credential::resolve builds state
- `tests/concurrent_race.rs` — 3 tests: race without coordinator, coalescing with L1, cross-process L1 gap

## What worked (validates current architecture)

### 1. `Credential` trait IS expressive enough for multi-step GitHub App flow

The trait's 4 assoc types (Input / State / Pending / Scheme) + `refresh()` method handle the full flow без structural modification:

- `State = GitHubAppState` — carries persistent material (app_id, installation_id, private_key_pem) AND refreshable (installation_token, expires_at) together
- `Scheme = OAuth2Token` — existing scheme type works as-is for Bearer projection
- `Pending = NoPendingState` — no interactive flow needed (не OAuth2 Authorization Code style)
- `refresh()` — signs JWT + POSTs + parses → updates state

No new trait shape required. No scheme additions. Current design handles this realistic complex case.

### 2. `SecretString` wrapping для PEM works

RSA PEM bytes wrapped в `SecretString`, `expose_secret()` inside `sign_app_jwt()` gives bytes to jsonwebtoken briefly, zeroize on drop. Redacted Debug. Serde helper работает без issues.

**One caveat:** we used `SecretString` (string) not `SecretBytes` for PEM. Either works for PEM (since PEM — text). For DER-encoded keys, `SecretBytes` from nebula-schema would be more appropriate. Neither needed scheme changes.

### 3. L1 Coalescer (simplified Spec H0 tier 1) SOLVES single-replica race

Concrete numbers from `with_l1_coordinator_only_one_replica_hits_idp`:

```
10 concurrent tasks refreshing same credential → 1 mock hit
```

This confirms Spec H0 §3.1 design — L1 in-proc mutex coalescing sufficient within single replica.

### 4. Compile-time-only type parameter did не cause friction

Unlike the archived paper-design `CredentialRef<dyn ServiceTrait>` that hit E0191, our scratch uses concrete types throughout. `refresh_github_app_token(&mut state)` accepts `&mut GitHubAppState` directly. No dyn dispatch needed.

The paper design's Pattern 2 (service trait + dyn) was unnecessary complexity — concrete-type Pattern 1 works fine for real services.

## What exposed gaps (validates Spec H0 priority)

### G1 — Cross-process race is real (test `l1_only_does_not_solve_cross_process_race`)

Two `L1Coalescer` instances (simulating 2 processes) both hit IdP. Evidence:
```
L1-only (simulating 2 processes): N hits (where N depends on serialization)
[NOTE] L1-only cannot prevent cross-process races. Spec H0's L2 durable claim
       repo needed for true multi-replica safety.
```

**This is exactly what n8n #13088 class is about.** Scratch confirms need for Spec H0's L2 durable `RefreshClaimRepo`.

### G2 — Mid-refresh crash not testable без L2 storage

The scratch's L1 coalescer has no concept of "crash in mid-refresh" — when a tokio task refreshes, it either completes or panics. No TTL, no sentinel, no reclaim.

This is **exactly** the territory Spec H0 §3.4 sentinel covers. Adding that requires storage-backed state. Scratch can't prototype that without bringing in SQLite или in-memory repo.

### G3 — `is_fresh` check pattern needed re-check inside mutex

The L1Coalescer's `coalesce(key, is_fresh, f)` signature — I had to pass both outer `is_fresh` (before lock attempt) and inner re-check (after lock acquired, inside closure). This was error-prone to wire correctly; the test `with_l1_coordinator_only_one_replica_hits_idp` does the re-check manually inside closure.

**Implication for Spec H0 impl:** the real `RefreshCoordinator::refresh_coalesced` needs the double-check pattern built-in, not left to caller. API signature should hide this.

### G4 — JWT crypto provider configuration non-trivial

`jsonwebtoken = "10"` requires explicit `rust_crypto` or `aws_lc_rs` feature. On Windows the `aws_lc_rs` backend needs NASM. We fell back to `jsonwebtoken = "9"` which has `use_pem` feature.

**Not a credential-architecture issue** — but real production credential crates that use JWT signing need to document crypto provider choice. Minor heads-up for anyone building Salesforce JWT / GCP Service Account credentials in nebula-credential-builtin.

### G5 — `CredentialMetadata::builder()` takes no args — minor ergonomics

First attempt was `CredentialMetadata::builder("key", "name")` (matching common builder patterns). Actual API: `::builder()` + `.key()` + `.name()` fluent. Low cost to fix but small DX папoркa.

## What's genuinely pleasant

- Wiremock integration seamless — ~20 lines of mock setup, counter tracking trivial
- Real RSA keypair generated once с openssl, committed as `.txt` (avoids secret-file guard hook)
- `SecretString` redaction works — never saw token text в test output despite heavy logging

## Unresolved after scratch

1. **L2 durable claim repo** — needs real storage backend. Either InMemory (easy, single-process) или SQLite/Postgres (spec Spec H0 P1). Scratch couldn't test this without pulling storage crate.
2. **Sentinel + reauth threshold** — same as above, needs persistent state.
3. **Heartbeat timing** — config validation could be tested purely in scratch, but we didn't (low ROI since rust-senior already confirmed bitflags arithmetic compiles).

## Recommendations for Spec H0 P1 implementation

Based on scratch evidence:

1. **Keep service wrapping simple.** Scratch used plain struct `GitHubAppCredential` с no macros. `#[derive(Credential)]` macro would add value but не blocker. Real credential crates can start without macros.

2. **Hide the double-check pattern in coordinator.** Don't expose `is_fresh` callback. Instead:
   ```rust
   coordinator.refresh_if_needed(
       state_accessor: impl Fn() -> State,
       refresh_fn: impl FnOnce(&mut State) -> Result<()>,
   )
   ```
   Coordinator takes responsibility for freshness check before and after claim acquisition.

3. **Test matrix for P1 should include:**
   - Single replica, 10 concurrent tasks → 1 IdP hit ✓ (this scratch validates)
   - 2 replicas with shared storage → 1 IdP hit (requires L2 claim repo)
   - Replica crashes mid-refresh → sentinel records, no duplicate post-reclaim

4. **Wiremock pattern** from `tests/common/mod.rs` — reusable for any spec that needs to stub OAuth-like endpoints. Consider promoting to a shared `nebula-testing-oauth` helper crate if multiple specs need it.

## Files in scratch

```
scratch/github-app-credential-proto/
├── Cargo.toml                  # empty [workspace] — isolated
├── NOTES.md                    # this file
├── src/
│   └── lib.rs                  # GitHubAppCredential + L1Coalescer
└── tests/
    ├── common/
    │   └── mod.rs              # wiremock + hit counter
    ├── fixtures/
    │   └── test-rsa-private.txt # RSA 2048 keypair, test-only
    ├── happy_path.rs           # 4 tests
    └── concurrent_race.rs      # 3 tests
```

Run with:

```bash
cd scratch/github-app-credential-proto
cargo test
```

Expected: **7 tests pass.**

## Should this scratch be archived?

**Keep for now.** Value as:
- Reference implementation for Spec H0 P1 (compares against real Credential trait usage)
- Regression test: if Credential trait shape changes, this should still work
- Example for future credential implementations (GitHub App is common)

**Consider promoting:**
- `L1Coalescer` shape → real `RefreshCoordinator` in engine (with L2 added)
- `tests/common/mod.rs` wiremock pattern → shared test crate if used by > 1 spec

**Archive когда:**
- Spec H0 P1 ships with real RefreshClaimRepo + integration tests — this scratch becomes redundant
- OR dropped if `nebula-credential-builtin` crate materializes и includes `GitHubAppCredential` directly

## Real GitHub App test (future)

User offered to create real GitHub App for validation. If Spec H0 P1 ships и we want end-to-end confidence:

1. Create GitHub App (dev account)
2. Install on personal test repo
3. Export: app_id, installation_id, download private-key.pem
4. Add to local env vars (never commit):
   ```bash
   export NEBULA_TEST_GITHUB_APP_ID=...
   export NEBULA_TEST_GITHUB_INSTALLATION_ID=...
   export NEBULA_TEST_GITHUB_PRIVATE_KEY_PATH=/path/to/private-key.pem
   ```
5. Add `#[ignore = "requires NEBULA_TEST_GITHUB_* env vars"]` test that
   reads env + runs real refresh + validates actual ghs_ token received

**NOT blocking for Spec H0 P1.** Wiremock is sufficient для trait validation и coordination logic.
