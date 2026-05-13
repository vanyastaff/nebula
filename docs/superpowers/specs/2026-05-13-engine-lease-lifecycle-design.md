# Engine-side LeasedProvider consumption — Phase D of ADR-0051

**Status:** Drafted 2026-05-13. Closes the ADR-0051 follow-up cascade.

## Context

Phases A–C of the ADR-0051 follow-up landed:

- **Phase A** (#664) — `ProviderCacheLayer` in `nebula-storage`.
- **Phase B** (#665) — `LeasedProvider: ExternalProvider` sub-trait with
  `handles_lease` / `renew` / `revoke`, plus `LeaseHandle::provider`
  attribution routed through `ExternalProviderChain` and `ProviderCacheLayer`.
- **Phase C** (#666) — `nebula-credential-vault` crate as the first concrete
  `LeasedProvider` (KV v2 + dynamic-secret reads with `lease_duration`).

What's missing: the engine never **calls** `renew` or `revoke`. A Vault
dynamic lease carries a TTL but nothing in the runtime proactively refreshes
it, and rotating a credential record does not revoke the upstream grant.
Survey across `crates/engine/src/credential/` confirms `ExternalProvider` /
`LeasedProvider` have zero call sites in the engine today — Phase D both
provides the mechanism and wires the rotation hook so future credential→
provider bridging only has to register leases.

## Decision

A self-contained `LeaseLifecycle` subsystem in `crates/engine/src/credential/lease/`
sibling to `refresh` / `rotation` / `registry` / `resolver`. Public API is
the registration entry point; everything else (renewal scheduling, retries,
event emission, metrics) is private to the module.

### Module layout

```
crates/engine/src/credential/lease/
├── mod.rs        # Public surface: LeaseLifecycle, LeaseLifecycleError, LeaseToken
├── registry.rs   # In-process registry of tracked leases keyed by LeaseToken
├── scheduler.rs  # Background tokio task driving renewals + revocations
└── policy.rs     # RenewalPolicy (ratio, backoff schedule, max retries)
```

`LeaseEvent` lands in `crates/credential/src/provider/event.rs` (re-exported
through `provider::mod`) — domain type lives next to `LeaseHandle`, just like
`CredentialEvent` lives in `nebula-credential` and is published by the
engine. Published on a dedicated `EventBus<LeaseEvent>` (not shoehorned into
`CredentialEvent`) because lease events carry richer payload (lease id,
provider name, TTL remaining) and a sizeable subset of leases will be
unattributed to a `CredentialId`.

### Decisions on the five open points

1. **Scheduler vs inline (decided: dedicated scheduler).** A single
   `tokio::spawn` per engine owns a `BinaryHeap<(Reverse<Instant>,
   LeaseToken)>` priority queue plus a `HashMap<LeaseToken, LeaseEntry>`.
   Driven by `tokio::select!` over `sleep_until(next_renew)` / command
   `mpsc` / `CancellationToken::cancelled()`. Inline renewal would either
   require periodic polling (same cost, worse coupling) or risk silent
   expiry during idle workflows that hold a lease for hours. One task per
   engine — not per credential.

2. **Renewal policy.** Renew at `issued_at + ttl * 0.7` (Vault Agent
   recommendation, also AWS STS guidance). On `ProviderError::Unavailable`
   or `Backend` apply bounded backoff `[1s, 2s, 4s, 8s, 16s]` with max 5
   retries before dropping the lease and emitting `LeaseEvent::Expired { ..,
   reason: RenewalFailed }`. On `ProviderError::NotFound` or `AccessDenied`
   drop immediately with `Expired { reason: NotFoundUpstream }` — Vault has
   already invalidated the lease, retries are useless.

3. **Revoke-on-rotate hook.** `LeaseLifecycle::revoke_for_credential(id)`
   scans the registry for entries whose `credential_id == Some(id)` and
   revokes each through `LeasedProvider::revoke`. Wiring is opt-in: the
   composition root passes an `Arc<LeaseLifecycle>` to the rotation
   orchestrator, which calls the method post-commit. The rotation
   pipeline's local `RotationTransaction` is untouched — no breaking
   signature change. Leases unattributed to a `CredentialId` (orphan
   resolutions) are unaffected.

4. **Failure semantics.**
   - `renew` failures are observable but never propagate up: log
     `tracing::warn`, increment `nebula.credential.dynamic_lease_renew_failed_total`,
     retry per backoff, emit `LeaseEvent::RenewalFailed` per attempt,
     drop + `Expired` after the budget is exhausted.
   - `revoke` failures during rotation: `tracing::warn` + metric +
     `LeaseEvent::RevocationFailed`, **do not block rotation** — local
     state has already committed; upstream cleanup is best-effort. A
     downstream audit subscriber can flag unrevoked leases via the
     event stream.
   - Public surface returns typed `LeaseLifecycleError` (thiserror).
     Only three variants need to escape the lifecycle — internal failures
     are absorbed by the backoff / event-publishing path and do not
     surface to callers:
     - `ResolutionMissingLease` — `track` called with an empty
       `ProviderResolution.lease`.
     - `Revoke { reason }` — provider returned an error from `revoke`.
       The lease is still removed from the registry.
     - `Shutdown` — scheduler task no longer accepting commands.
   - **No `unwrap` / `expect` / `panic!`** anywhere on the lifecycle path.

5. **Observability.**
   - Metric constants added to `CredentialMetrics`:
     - `DYNAMIC_LEASE_RENEWED_TOTAL` — counter, labels: `outcome`, `provider`.
     - `DYNAMIC_LEASE_REVOKED_TOTAL` — counter, labels: `outcome`, `provider`.
     - `DYNAMIC_LEASE_RENEW_FAILED_TOTAL` — counter, labels: `reason`,
       `provider`.
     - `DYNAMIC_LEASE_ACTIVE` — gauge of currently-tracked leases.
   - Spans: `lease.renew`, `lease.revoke`, `lease.expired` — each with
     `provider`, `lease_id`, and (when present) `credential_id`.
   - Events on `EventBus<LeaseEvent>`:
     - `LeaseRenewed { credential_id: Option<CredentialId>, lease_id, provider, new_ttl }`.
     - `LeaseRevoked { credential_id, lease_id, provider }`.
     - `LeaseRenewalFailed { credential_id, lease_id, provider, reason }`.
     - `LeaseRevocationFailed { credential_id, lease_id, provider, reason }`.
     - `LeaseExpired { credential_id, lease_id, provider, reason }`
       where `reason` is one of `RenewalFailed` (budget exhausted) /
       `NotFoundUpstream` (gone upstream) / `NonRenewable` (renewed
       with zero TTL) / `Shutdown` (lifecycle cancelled).
   - Every new state transition (track → renew-scheduled, scheduled →
     renewing, renewing → re-scheduled / dropped) emits exactly one of the
     above (DoD per AGENTS.md).

### Public API surface

```rust
pub struct LeaseLifecycle { /* Arc-shared handle to the scheduler task */ }

pub struct LeaseToken { /* opaque registry key */ }

impl LeaseLifecycle {
    pub fn spawn(
        config: LeaseLifecycleConfig,
        event_bus: Option<Arc<EventBus<LeaseEvent>>>,
        metrics: Option<Arc<dyn MetricsEmitter>>,
        shutdown: CancellationToken,
    ) -> Self;

    pub fn track(
        &self,
        provider: Arc<dyn LeasedProvider>,
        resolution: ProviderResolution,
        credential_id: Option<CredentialId>,
    ) -> Result<LeaseToken, LeaseLifecycleError>;

    pub async fn revoke(&self, token: LeaseToken) -> Result<(), LeaseLifecycleError>;

    pub async fn revoke_for_credential(&self, id: CredentialId) -> usize;

    pub fn active_lease_count(&self) -> usize;
}
```

`LeaseLifecycleConfig` carries `RenewalPolicy { ratio: f32, backoff:
Vec<Duration>, max_retries: u32 }` with a Vault-style default plus a
`provider_call_timeout: Duration` (default 30s) that bounds each
`renew` / `revoke` call so a hung backend cannot monopolise the
scheduler task. On timeout the call is mapped to
`ProviderError::Unavailable` and the standard backoff schedule
absorbs it.

### Non-goals (for Phase D)

- **Wiring `ExternalProvider::resolve` into `CredentialResolver`.** That's a
  separate redesign — the resolver currently operates on `CredentialStore`,
  not on provider-resolved secrets. Phase D delivers the lifecycle
  mechanism so the future bridge only has to call `LeaseLifecycle::track`.
- **Persisting tracked leases across engine restarts.** The registry is
  in-memory; on engine restart the lease is left to expire upstream at its
  natural TTL. ADR-0051 explicitly rejected pre-staged fallback secrets;
  the symmetric pattern here is "no silent persistence of lease tokens"
  either.
- **Cross-replica lease coordination.** Renewal is best run by exactly one
  replica per lease, but Phase D ships single-replica semantics. The
  follow-up (separate ADR) would gate renewal through the existing
  `RefreshCoordinator` L2 claim repo.

## Tests

- **Unit (in `crates/engine/src/credential/lease/`)**:
  - Scheduler picks renewal time at 70% of TTL.
  - Successful renew reschedules at 70% of refreshed TTL.
  - `Unavailable` triggers backoff; recovery before max retries succeeds.
  - Backoff exhaustion drops the lease and emits `Expired`.
  - `NotFound` drops immediately without retries.
  - `revoke` removes from registry, emits `LeaseRevoked`.
  - `revoke_for_credential` revokes only matching entries.
  - Shutdown via `CancellationToken` is graceful; outstanding leases drop
    with `Shutdown` reason.

- **Integration**:
  - Mock `LeasedProvider` (reuse the `LeasedMock` pattern from
    `crates/credential/src/provider/leased.rs`) — exercise the full
    track → auto-renew → revoke path with a manually-advanced
    `tokio::time::pause()` clock.

## Verification gate

```
cargo check  --workspace --all-features
cargo test   -p nebula-engine --all-features
cargo test   -p nebula-credential --all-features
cargo clippy --workspace --all-targets -q -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cargo deny check
```

Then conventional commit (`feat(engine): …`), `gh pr create` referencing
ADR-0051 and PRs #664 / #665 / #666. ADR-0051 grows its third `## Update`
block on landing.
