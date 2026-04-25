---
name: nebula-resource tech spec (implementation-ready design)
status: CP2 ratified — pending CP3 dispatch
date: 2026-04-25
authors: [architect (subagent dispatch)]
scope: nebula-resource — single-crate redesign; 5 in-tree consumers migrate atomically
cascade_phase: Phase 6 CP1
strategy: docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md (FROZEN CP3)
adrs:
  - docs/adr/0036-resource-credential-adoption-auth-retirement.md (proposed-pending-CP1)
  - docs/adr/0037-daemon-eventsource-engine-fold.md (proposed-pending-CP1)
spike: docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/spike/
credential_cross_ref:
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §3.6 (lines 928-996)
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §Credential::revoke (line 228)
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §4.3 revocation modes (lines 1062-1068)
related:
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md
  - docs/PRODUCT_CANON.md §3.5 §4.5 §12.5 §12.7
---

# nebula-resource Tech Spec (implementation-ready design)

## §0 — Status, scope, freeze policy

### §0.1 Status

**Checkpoint 1 draft** — sections §0, §1, §2, §3 complete; §4-§16 follow in CP2 (§4-§8), CP3 (§9-§13), and CP4 (§14-§16) per the cadence locked in [`03-scope-decision.md` §6, line 169](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md).

CP1 is the foundation: trait contract + runtime model with full Rust signatures. CP1 ratification by tech-lead flips both [ADR-0036](../adr/0036-resource-credential-adoption-auth-retirement.md) and [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md) from `proposed` to `accepted` per their respective acceptance gates. Until CP1 ratifies, both ADRs remain `proposed`.

This Tech Spec is **implementation-normative**. Where the [Strategy Document](2026-04-24-nebula-resource-redesign-strategy.md) §4 establishes binding decisions and the ADRs record the architectural rationale, this Tech Spec provides the compile-able Rust shapes, runtime invariants, storage paths, and migration mechanics that engineers consume directly.

### §0.2 Scope (cascade gate-pass record)

This Tech Spec carries the gate-pass record for the nebula-resource redesign cascade. Phase order:

- **Phase 1** — pain enumeration. 28 findings, 6 🔴-class. [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md).
- **Phase 2** — scope decision. Co-decision body (architect + tech-lead + security-lead) locked Option B in round 1 of max-3. [`03-scope-decision.md`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md).
- **Phase 3** — Strategy document (3 checkpoints, frozen 2026-04-24). [Strategy](2026-04-24-nebula-resource-redesign-strategy.md).
- **Phase 4** — Phase 4 spike. PASSED iter-1; all 7 exit criteria met; iter-2 deferred. [`spike/NOTES.md`](../drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md).
- **Phase 5** — ADR authoring. ADR-0036 (primary trait reshape), ADR-0037 (Daemon/EventSource extraction).
- **Phase 6** — this Tech Spec.

Strategy §4 decisions are the binding contract. CP1 §2-§3 elaborate Strategy §4.1 (trait reshape), §4.2 (revocation), §4.3 (rotation dispatch) into compile-able Rust. CP1 does not re-litigate Strategy decisions; the freeze policy in [Strategy §0](2026-04-24-nebula-resource-redesign-strategy.md) governs.

### §0.3 Freeze policy (per-CP)

**Each checkpoint freezes after review**. CP1 freezes after spec-auditor + rust-senior + tech-lead sign-off. After freeze:

- **Bug fixes** (typo, broken intra-doc link, clarification that does not change semantics) land via "docs(spec)" PR. Reviewer: spec-auditor.
- **Semantic amendments** (Rust signature changes, invariant shifts, decision reversal) require a co-review cycle — architect drafts amendment rationale, tech-lead ratifies, plus relevant specialty reviewer (rust-senior for trait, security-lead for revocation).
- **Strategy §4 supersession** — if a CP discovers Strategy §4 ambiguity, the Tech Spec records the extension with explicit "extends Strategy §X.Y" annotation. If the extension changes Strategy semantics, the Strategy amendment cycle from [Strategy §0](2026-04-24-nebula-resource-redesign-strategy.md) gates first — Tech Spec cannot supersede Strategy in-place.
- **ADR amendments** — if a CP discovers an ADR-0036 / ADR-0037 decision needs adjustment, the ADR's "amended in place on" section records the delta (per ADR-0035 amended-in-place pattern). Material structural changes require new ADR.

Strategy authority supersedes Tech Spec on conflict; Tech Spec wins over sub-spec and implementation plans. ADRs are point-in-time records of the rationale at acceptance; future code may diverge if the ADR is amended or superseded.

### §0.4 Reading order

§0 → §1 (goals + non-goals) → §2 (trait contract) → §3 (runtime model) → §4 (lifecycle, CP2) → §5 (implementation specifics, CP2) → §6 (operational/observability, CP2) → §7 (testing strategy, CP2) → §8 (storage/state, CP2) → §9-§13 (interface, CP3) → §14-§16 (meta + open items + handoff, CP4).

CP1 readers: load Strategy §4, ADR-0036 §Decision, ADR-0037 §Decision, [credential Tech Spec §3.6 lines 928-996](2026-04-24-credential-tech-spec.md), and [`spike/NOTES.md`](../drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md) before reading §2-§3 of this document. Citations are dense; load order matters.

## §1 — Goals and non-goals

### §1.1 Primary goals

- **Replace `Resource::Auth` with `Resource::Credential`.** Adopt [credential Tech Spec §3.6 lines 935-955](2026-04-24-credential-tech-spec.md) shape verbatim per [Strategy §4.1](2026-04-24-nebula-resource-redesign-strategy.md). `type Credential: Credential` becomes the trait-level credential binding; `<Self::Credential as Credential>::Scheme` flows into `create` and the new rotation hooks. `type Auth: AuthScheme` is removed (no shim, no alias) per `feedback_no_shims.md` discipline encoded in [Strategy §2.4](2026-04-24-nebula-resource-redesign-strategy.md).
- **Eliminate the silent revocation drop.** Resolve [Phase 1 finding 🔴-1](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md): `Manager::on_credential_refreshed` at [`crates/resource/src/manager.rs:1360`](../../../crates/resource/src/manager.rs) and `on_credential_revoked` at [`:1386`](../../../crates/resource/src/manager.rs) terminate in `todo!()` over an empty reverse-index. The reverse-index write path lands atomically with the dispatchers in this spec.
- **Per-resource rotation hooks with isolation.** New trait methods `on_credential_refresh` + `on_credential_revoke` with default no-op bodies. Manager dispatches via parallel `join_all` with per-resource timeout enforcement per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) + [security amendment B-1](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md). One slow / failing resource cannot block siblings.
- **Atomic landing.** Trait reshape, reverse-index write path, dispatcher implementation, observability (trace + counter + event), 5-consumer migration, doc rewrite, Daemon/EventSource extraction land in one PR wave per [Strategy §4.8](2026-04-24-nebula-resource-redesign-strategy.md).

### §1.2 Secondary goals

- **Daemon and EventSource extraction.** Per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md), Daemon and EventSource topologies leave `nebula-resource` and fold into the engine layer. The `TopologyRuntime<R>` enum at [`crates/resource/src/runtime/managed.rs:35`](../../../crates/resource/src/runtime/managed.rs) shrinks 7 → 5 variants. Pool / Resident / Service / Transport / Exclusive remain. Engine-side landing site (module layout, primitive naming, `EventSource → TriggerAction` adapter signature) is CP3 §13 deliverable.
- **`manager.rs` file-split.** [Strategy §4.5](2026-04-24-nebula-resource-redesign-strategy.md) keeps the `Manager` type monolithic and splits the 2101-line file into submodules. CP2 §5.4 finalises seven cut points: `mod.rs`, `options.rs`, `registration.rs`, `gate.rs`, `execute.rs`, `rotation.rs`, `shutdown.rs` — extending Strategy §4.5's five-submodule proposal with `registration.rs` (concrete site for the 🔴-1 reverse-index write fix per §3.1) and `shutdown.rs` (concrete site for the 🔴-4 `set_phase_all_failed` fix per §5.5). The two additions are concrete cuts that emerged when laying out the lifecycle, not a Strategy supersession; the Strategy §4.5 proposal did not foreclose extension. Public API does not change.
- **Drain-abort fix bundled.** Wire `ManagedResource::set_failed()` into the `DrainTimeoutPolicy::Abort` path per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md). Resolves [Phase 1 finding 🔴-4](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md). Lands atomically with the file-split PR.
- **Documentation rebuild.** Per [Strategy §4.7](2026-04-24-nebula-resource-redesign-strategy.md), all `crates/resource/docs/*` rewritten against the new shape. CP3 §13 enumerates per-file change list.

### §1.3 Non-goals (deferred per Strategy §5)

- **`AuthenticatedResource: Resource` sub-trait.** Explicitly rejected at [Phase 2](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) and [Strategy §2.4](2026-04-24-nebula-resource-redesign-strategy.md). `type Credential = NoCredential;` is the idiomatic opt-out.
- **`Runtime`/`Lease` collapse.** [Phase 1 finding 🟠-11](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) — 9/9 test resources set `Lease = Runtime`. Deferred per [Strategy §5.3](2026-04-24-nebula-resource-redesign-strategy.md).
- **`AcquireOptions::intent/.tags` wiring.** Engine integration ticket #391 drives the shape. Deferred per [Strategy §5.2](2026-04-24-nebula-resource-redesign-strategy.md); CP2 §5 picks interim posture (`#[doc(hidden)]` vs `#[deprecated]`).
- **Service/Transport merge.** [Phase 1 🟡-22](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md). `feedback_boundary_erosion.md` applies; do not merge without strong evidence.
- **`FuturesUnordered` fan-out cap.** [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) explicitly defers; current N (resources sharing one credential) is small. Future optimization if operational signal demands it.
- **L2 cross-replica rotation coordination.** Belongs to credential cascade per `draft-f17`. Resource-side `on_credential_refresh` is a consumer of L1-coordinated rotations.

### §1.4 Success criteria

CP1 ratification requires:

- All five Phase 4 spike-surfaced open questions resolved with rationale (§2.5).
- Full Rust signature for the `Resource` trait in §2.1 — every method, every associated type, every default body.
- `NoCredential` location decided and the concrete `Credential` impl signed off (§2.2).
- The five topology sub-traits (Pool / Resident / Service / Transport / Exclusive) elaborated with their inherited `type Credential` and any topology-specific shape touched by the rotation contract (§2.4).
- Manager runtime model in §3 with the reverse-index write path (Phase 1 🔴-1 resolved), parallel dispatcher, per-resource timeout configuration, and failure aggregation, all with concrete signatures.
- Phase 1 🔴-1 (silent revocation drop) and 🔴-4 (drain-abort phase corruption) explicitly resolved with file:line cross-references (§3.6).
- Every claim in §2-§3 traceable to: spike file, current code, Strategy §, ADR §, or credential Tech Spec line. No invention.

CP2 / CP3 / CP4 success criteria are recorded in their own checkpoint scope when those drafts open.

## §2 — Trait contract

Full Rust signatures for the reshaped `Resource` trait, the `NoCredential` opt-out, the rotation hooks with their default bodies and invariants, the five topology sub-traits, and the concrete resolutions of all five Phase 4 spike open questions. Section §2 is the binding compile-time contract; §3 is the runtime obligation that consumes it.

### §2.1 Resource trait (full Rust signature)

The reshaped `Resource` trait. Replaces the current trait at [`crates/resource/src/resource.rs:220-298`](../../../crates/resource/src/resource.rs). Imports per spike `resource.rs:17-19` (production version uses `nebula_credential::Credential` + `nebula_credential::CredentialId`).

```rust
use std::future::Future;

use nebula_core::ResourceKey;
use nebula_credential::{Credential, CredentialId, NoCredential};

use crate::context::ResourceContext;

/// Core resource trait — 5 associated types + 6 lifecycle methods.
///
/// Uses return-position `impl Future` (RPITIT) — no `async_trait`, no
/// `Box<dyn Future>` allocation on the hot path. Default-no-op rotation
/// hooks use `async fn` shorthand on the trait body to keep the default
/// readable; impl sites SHOULD use `async fn` per spike NOTES.md clippy
/// finding (`manual_async_fn` discourages `fn ... -> impl Future`
/// matching the trait declaration verbatim — see §2.1.1 for the
/// idiomatic impl form).
///
/// # Associated types
///
/// | Type        | Purpose                                                     |
/// |-------------|-------------------------------------------------------------|
/// | `Config`    | Operational config (no secrets) — `HasSchema` super-trait   |
/// | `Runtime`   | The live resource handle (connection, client, etc.)         |
/// | `Lease`     | What callers hold while using the resource                  |
/// | `Error`     | Resource-specific error type                                |
/// | `Credential`| What the engine binds — opt out via `= NoCredential`        |
///
/// # Lifecycle (post-redesign)
///
/// ```text
/// create(scheme) → Runtime
///   ↓
/// check()  → Ok | Err
///   ↓
/// on_credential_refresh(new_scheme) → Ok | Err   (default no-op)
/// on_credential_revoke(credential_id) → Ok | Err (default no-op)
///   ↓
/// shutdown() → graceful wind-down
///   ↓
/// destroy()  → final cleanup (consumes Runtime)
/// ```
pub trait Resource: Send + Sync + 'static {
    /// Operational configuration type (no secrets).
    type Config: ResourceConfig;
    /// The live resource handle.
    type Runtime: Send + Sync + 'static;
    /// What callers hold during use.
    type Lease: Send + Sync + 'static;
    /// Resource-specific error type.
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;
    /// What this resource binds to.
    ///
    /// Use [`NoCredential`] to opt out (`type Credential = NoCredential;`).
    /// Otherwise pick a real `Credential` impl. The projected scheme
    /// `<Self::Credential as Credential>::Scheme` is what `create` and
    /// `on_credential_refresh` receive from the credential resolver.
    ///
    /// Replaces `type Auth: AuthScheme` per Strategy §4.1 + ADR-0036.
    type Credential: Credential;

    /// Returns the unique key identifying this resource type.
    fn key() -> ResourceKey;

    /// Creates a new runtime instance from config and resolved scheme.
    ///
    /// `scheme` is borrowed from the credential resolver — implementations
    /// MUST NOT clone the scheme onto the runtime per Strategy §4.3
    /// hot-path invariant (each clone is another zeroize obligation).
    /// Pull what is needed (token, connection string), let the borrow end.
    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    /// Health-checks an existing runtime.
    ///
    /// Default: always succeeds. Connection-bound resources should
    /// override with a fast read-only probe (e.g., `SELECT 1`).
    fn check(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Rotation hook — called when the engine detects credential refresh.
    ///
    /// Default: no-op. Connection-bound resources (Postgres pool, Kafka
    /// producer) override with the **blue-green pool swap** pattern from
    /// credential Tech Spec §3.6 lines 961-993: `Arc<RwLock<Pool>>` +
    /// write-lock swap inside the impl. `Manager` does NOT orchestrate
    /// the swap.
    ///
    /// # Invariant — per-resource isolation
    ///
    /// Manager dispatches via parallel `join_all` with per-resource
    /// timeout enforcement (§3.2). One resource's slow / failed refresh
    /// MUST NOT block sibling dispatches. This hook is bounded by the
    /// per-resource budget configured at register time (§3.3).
    ///
    /// # Error semantics
    ///
    /// Returning `Err(e)` causes Manager to emit
    /// `ResourceEvent::CredentialRefreshed { outcome: Failed(...) }`
    /// and a per-resource error tracing span. Sibling dispatches are
    /// unaffected (Strategy §4.3, security amendment B-1).
    fn on_credential_refresh(
        &self,
        new_scheme: &<Self::Credential as Credential>::Scheme,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = new_scheme;
        async { Ok(()) }
    }

    /// Revocation hook — called when the engine signals credential revocation.
    ///
    /// Default: no-op. Override invariant per Strategy §4.2:
    /// **post-invocation, the resource emits no further authenticated
    /// traffic on the revoked credential.**
    ///
    /// The mechanism (destroy pool / mark tainted / wait-for-drain /
    /// reject new acquires) is impl-defined. Connection-bound resources
    /// SHOULD destroy or taint pool instances synchronously inside the
    /// hook; resources holding short-lived per-request schemes MAY rely
    /// on the next `create` call rejecting (the new scheme will not be
    /// available — `<Self::Credential as Credential>::Scheme` cannot
    /// be projected from a `revoked` `state_kind` per credential Tech
    /// Spec §4.3 lines 1062-1068).
    ///
    /// # Default-body rationale
    ///
    /// The default body returns `Ok(())` — it does NOT enforce the
    /// invariant for the implementer. The invariant is a contractual
    /// obligation on overriding implementations. Manager has no way to
    /// verify the invariant at runtime; it relies on the impl honoring
    /// it. CP2 §6 (security) records the audit/test gates that catch
    /// invariant violations.
    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = credential_id;
        async { Ok(()) }
    }

    /// Gracefully winds down a runtime.
    fn shutdown(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Final cleanup — consumes the runtime.
    fn destroy(
        &self,
        runtime: Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = runtime;
        async { Ok(()) }
    }

    /// Returns the schema for this resource's configuration.
    fn schema() -> nebula_schema::ValidSchema
    where
        Self: Sized,
    {
        <Self::Config as nebula_schema::HasSchema>::schema()
    }

    /// Returns metadata for UI and diagnostics.
    fn metadata() -> ResourceMetadata
    where
        Self: Sized,
    {
        ResourceMetadata::for_resource::<Self>(
            Self::key(),
            Self::key().to_string(),
            String::new(),
        )
    }
}
```

#### §2.1.1 Idiomatic impl form

Trait declarations use `impl Future<...> + Send`. Impl sites SHOULD use `async fn` shorthand per spike NOTES.md clippy finding — clippy's `manual_async_fn` (1.95 default `clippy::all`) prefers the desugared form. Both forms are interchangeable from the trait perspective; only the impl side has the lint preference.

```rust
impl Resource for PostgresPool {
    // ... associated types ...

    async fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> Result<Self::Runtime, Self::Error> { /* ... */ }

    async fn on_credential_refresh(
        &self,
        new_scheme: &<Self::Credential as Credential>::Scheme,
    ) -> Result<(), Self::Error> {
        let new_pool = build_pool_from_scheme(new_scheme).await?;
        let mut guard = self.inner.write().await;
        *guard = new_pool;
        Ok(())
    }
}
```

The blue-green swap example mirrors [credential Tech Spec §3.6 lines 981-993](2026-04-24-credential-tech-spec.md). Old connections drain naturally as their RAII guards drop; new queries use the new pool (read lock acquires against the new inner after swap).

### §2.2 NoCredential type (location + impl)

`NoCredential` lives in **`nebula-credential`** (resolution to spike Q1; full rationale in §2.5). It IS a `Credential` impl, so structurally it belongs next to the trait. Resource-side imports it as `nebula_credential::NoCredential`.

Concrete implementation, adapted from spike `no_credential.rs:36-95`:

```rust
// Inside crates/credential/src/no_credential.rs (NEW FILE).

use serde::{Deserialize, Serialize};

use crate::{
    AuthPattern, AuthScheme, Credential, CredentialContext, CredentialError,
    CredentialMetadata, CredentialState, NoPendingState, ResolveResult,
    credential_key,
};

/// Zero-sized scheme that asserts "no authentication".
///
/// Pattern is [`AuthPattern::NoAuth`]. Mirrors the existing `()` impl
/// on `AuthScheme` but as a real named type so it composes with
/// [`Credential`]'s `type Scheme: AuthScheme` bound.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct NoScheme;

impl AuthScheme for NoScheme {
    fn pattern() -> AuthPattern {
        AuthPattern::NoAuth
    }
}

impl CredentialState for NoScheme {
    const KIND: &'static str = "no_scheme";
    const VERSION: u32 = 1;
}

/// `Credential` opt-out marker.
///
/// `type Credential = NoCredential;` declares that a `Resource` does
/// not bind to a credential. The reshaped `Manager` (see
/// [`nebula_resource::Manager`]) special-cases this at the type level:
/// registering an `R` whose `Credential = NoCredential` does NOT
/// populate the credential reverse index, and `on_credential_refreshed`
/// / `on_credential_revoked` will never reach this resource.
pub struct NoCredential;

impl Credential for NoCredential {
    type Input = ();
    type Scheme = NoScheme;
    type State = NoScheme;
    type Pending = NoPendingState;

    const KEY: &'static str = "no_credential";

    fn metadata() -> CredentialMetadata
    where
        Self: Sized,
    {
        CredentialMetadata::builder()
            .key(credential_key!("no_credential"))
            .name("No credential")
            .description("Opt-out marker for resources without an authenticated binding.")
            .schema(<() as nebula_schema::HasSchema>::schema())
            .pattern(AuthPattern::NoAuth)
            .build()
            .expect("static metadata fields are all set above")
    }

    fn project(_state: &NoScheme) -> NoScheme {
        NoScheme
    }

    async fn resolve(
        _values: &nebula_schema::FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<NoScheme, NoPendingState>, CredentialError> {
        // Returning `Complete(NoScheme)` keeps the trait obligations
        // honest. In practice the dispatcher never reaches this method
        // for a `NoCredential`-typed resource — Manager skips the
        // reverse-index write at register time (see §3.1).
        Ok(ResolveResult::Complete(NoScheme))
    }
}
```

**Re-export at the resource crate root.** `crates/resource/src/lib.rs` re-exports for ergonomics:

```rust
pub use nebula_credential::{NoCredential, NoScheme};
```

Consumers writing `type Credential = NoCredential;` import from either crate; the canonical home is `nebula-credential`.

**Registration safety.** Registering `NoCredential` against a credential store is a nonsense operation but doesn't crash — `metadata()` returns a structurally-honest tombstone. Manager's `register` emits `tracing::warn!` when called with `type Credential = NoCredential` paired with `Some(real_credential_id)` and discards the id without populating the reverse-index per §3.1.

### §2.3 on_credential_refresh + on_credential_revoke (signatures + default bodies + invariants)

Both signatures and default bodies are inlined in §2.1. This subsection records the **invariants** each method must honor.

**`on_credential_refresh(new_scheme)`:**

- **Borrow invariant.** `new_scheme: &<Self::Credential as Credential>::Scheme` — borrowed, not owned. Per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) hot-path invariant, no `Scheme::clone()` on the dispatcher path; each clone is another zeroize obligation per [`PRODUCT_CANON.md §12.5`](../PRODUCT_CANON.md). Resource impls pull what they need from the borrowed scheme inside the await window.
- **Per-resource isolation invariant.** Manager (§3.2) bounds each invocation by a per-resource timeout; sibling dispatches run in parallel via `join_all`. A slow or failed refresh on this resource MUST NOT block siblings. The default no-op body satisfies this trivially.
- **Idempotency expectation.** Manager MAY retry under specific recovery flows (CP2 §6 finalizes); impls SHOULD treat repeated `on_credential_refresh(same_scheme)` as a no-op after the first successful call. Default body satisfies; pool-swap impls are naturally idempotent because the second swap re-publishes the same pool.

**`on_credential_revoke(credential_id)`:**

- **Post-invocation invariant.** Per [Strategy §4.2](2026-04-24-nebula-resource-redesign-strategy.md): "post-invocation, the resource emits no further authenticated traffic on the revoked credential." This is a contractual obligation on overriding impls; Manager cannot verify at runtime. CP2 §6 specifies the audit gate and test invariant for this.
- **Mechanism is impl-defined.** Strategy commits to the *invariant*, not the *implementation*. Candidates per [Strategy §4.2](2026-04-24-nebula-resource-redesign-strategy.md): destroy pool instances; mark instances tainted with `RAII-on-release` cleanup; wait-for-drain; reject new acquires. Default body returns `Ok(())` and does not enforce the invariant — opt-out only by impls that genuinely emit no authenticated traffic (e.g., `NoCredential`-typed resources, unauthenticated caches).
- **Argument is `&CredentialId`, not `&Scheme`.** Revocation has no new scheme to swap in; the credential id is sufficient signal. Resource impls that need to look up internal per-credential state (e.g., a multi-tenant pool keyed by credential) do so via the id.
- **Failure semantics.** Manager emits `ResourceEvent::CredentialRevoked { outcome: Failed(...) }` per failed dispatch; Strategy §4.2 also requires `HealthChanged { healthy: false }` per security amendment B-2. CP2 §7 finalizes the event cardinality.

**Why two methods, not one combined `on_credential_event(EventKind)`.** Per [Strategy §4.2](2026-04-24-nebula-resource-redesign-strategy.md) rationale: "two semantically distinct events deserve two methods." Refresh has a new scheme; revoke does not. Dual-semantics on one method would force every implementer to branch on "is this a refresh or a revoke?" via the scheme reference, which is awkward and error-prone.

### §2.4 Topology sub-traits (Pool/Resident/Service/Transport/Exclusive)

Five topology sub-traits remain in `nebula-resource` post-extraction. Daemon and EventSource leave the crate per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md). Each sub-trait extends `Resource` and inherits the new `type Credential` + rotation hooks; topology-specific methods are unchanged from current shape at [`crates/resource/src/topology/{pooled,resident,service,transport,exclusive}.rs`](../../../crates/resource/src/topology/).

The trait declarations themselves do not change shape — only the parent `Resource` trait reshape propagates through. Sub-traits stay in their existing files post-`manager.rs` file-split.

```rust
// crates/resource/src/topology/pooled.rs — UNCHANGED structure.
pub trait Pooled: Resource {
    fn is_broken(&self, _runtime: &Self::Runtime) -> BrokenCheck {
        BrokenCheck::Healthy
    }

    fn recycle(
        &self,
        _runtime: &Self::Runtime,
        _metrics: &InstanceMetrics,
    ) -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send {
        async { Ok(RecycleDecision::Keep) }
    }

    fn prepare(
        &self,
        _runtime: &Self::Runtime,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

// crates/resource/src/topology/resident.rs
pub trait Resident: Resource where Self::Lease: Clone {
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool { true }
    // ... (unchanged from current)
}

// crates/resource/src/topology/service.rs
pub trait Service: Resource {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    fn acquire_token(
        &self,
        runtime: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    fn release_token(
        &self,
        _runtime: &Self::Runtime,
        _token: Self::Lease,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

// crates/resource/src/topology/transport.rs
pub trait Transport: Resource {
    fn open_session(
        &self,
        transport: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
    // ... + close_session etc. (unchanged from current)
}

// crates/resource/src/topology/exclusive.rs
pub trait Exclusive: Resource {
    fn reset(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}
```

**`TopologyRuntime<R>` enum shrink.** Per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md), the enum at [`crates/resource/src/runtime/managed.rs:35`](../../../crates/resource/src/runtime/managed.rs) loses its `Daemon` and `EventSource` variants. Engine-side landing site (module layout, primitive naming, EventSource→TriggerAction adapter signature) is **CP3 §13 deliverable**, not CP1 — CP1 records the enum shrink as a contract, not the engine-side shape.

Spike validated cross-topology composition end-to-end: `MockPostgresPool: Pooled` (credential-bearing) and `MockKafkaTransport: Transport` (credential-bearing) bound to the same `CredentialId` both received the rotation hook in `parallel_dispatch_crosses_topology_variants` test ([`spike/.../resource-shape-test/src/lib.rs:583-607`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)). The dispatcher's type erasure works across topology variants.

### §2.5 Open question resolutions from spike

Five open questions surfaced in [`spike/NOTES.md` lines 155-202](../drafts/2026-04-24-nebula-resource-redesign/spike/NOTES.md). CP1 resolves each with rationale; no question deferred to CP2-CP4.

#### Q1 — `NoCredential` location: **`nebula-credential` crate**

`NoCredential` IS a `Credential` impl. Structurally it belongs next to the trait it satisfies. Three reasons:

- **Layering honesty.** Spike NOTES.md flags this as "structurally honest" — the type's semantics are credential-side, not resource-side. Placing it in `nebula-credential` mirrors how `NoPendingState` already lives there.
- **Reusability beyond resource.** Any future crate consuming `Credential` (e.g., `nebula-action` for `CredentialRef<dyn ...>` defaulting) can import `NoCredential` without taking a transitive dep on `nebula-resource`.
- **No coupling penalty.** `nebula-resource` already depends on `nebula-credential` ([`Cargo.toml`](../../../crates/resource/Cargo.toml)); `use nebula_credential::NoCredential` is a one-line import. `nebula-resource::lib.rs` re-exports for ergonomic `use nebula_resource::NoCredential` per §2.2.

**Trade-off accepted:** `nebula-credential` gains a tombstone-shaped type whose practical use case is solely resource-side opt-out. Mitigated by the type also serving `CredentialRef<dyn ...>` defaulting in actions (future cascade per [Strategy §6.5](2026-04-24-nebula-resource-redesign-strategy.md)).

#### Q2 — `TypeId` vs sealed-trait marker for opt-out detection: **`TypeId`**

Manager's `register::<R>` decides whether to populate the reverse-index by checking `TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>()` (spike `manager.rs:227`). Three reasons to keep this over a sealed-trait marker:

- **One-line check, called once per registration.** Not in the hot path. Production-relevant overhead is zero.
- **Sealed-trait marker forces NoCredential to live with the marker.** A sealed `IsCredentialBearing` (negated for `NoCredential`) trait would have to live in `nebula-credential` (because `NoCredential` does), constraining `nebula-resource::Manager` to consume yet another trait re-export. `TypeId` is host-crate-agnostic.
- **Static dispatch affordance is not load-bearing.** Sealed-trait would let the compiler optimize the branch, but the branch fires once per `register` call (not in `acquire`, not on rotation). Optimization benefit is unmeasurable at this cadence.

**Compile-fail probe added in §2.5 Q5:** the negative case (a non-`Credential` type bound) is caught by the `type Credential: Credential` bound at impl time — verified by spike compile-fail probe `_credential_bound_enforced_must_fail` ([`spike/.../resource-shape-test/src/compile_fail.rs:117-150`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).

#### Q3 — `Box::pin` per dispatch overhead: **acknowledge in observability; no extra metric**

Type erasure requires `Box<dyn ResourceDispatcher>`; trampoline returns `Pin<Box<dyn Future<...> + Send>>` because RPITIT in trait objects is not stable on 1.95 (spike NOTES.md "this almost didn't work" #1). Cost is **once per refresh dispatch per registered resource**, not per acquire.

- **Observability.** `nebula_resource.credential_rotation_dispatch_latency_seconds` histogram per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md) covers the `Box::pin` cost alongside hook execution time. No separate metric — the allocation is statistically dominated by hook execution.
- **Soft cap of 32 concurrent hooks** per credential (tech-lead Phase 2 number) becomes the default `ManagerConfig::credential_rotation_concurrency`. Tunable. N ≤ 32 runs unbounded `join_all`; N > 32 fan-out via `FuturesUnordered` deferred per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md).
- **Trait docs note the cost transparently.** CP4 §15 records "future-cleanup-when-stable" once RPITIT in trait objects stabilizes.

#### Q4 — Per-resource timeout config surface: **per-Manager default + per-Resource override via `RegisterOptions`**

Both. Default lives on `ManagerConfig::credential_rotation_timeout` (uniform default; defaults to 30 seconds — value chosen to accommodate slow blue-green pool builds while still bounding misbehaving impls). Per-resource override lives on `RegisterOptions::credential_rotation_timeout: Option<Duration>`, which when `Some(d)` overrides the Manager default for that registration.

Three reasons:

- **`RegisterOptions` already carries per-resource concerns.** [`crates/resource/src/manager.rs:220-227`](../../../crates/resource/src/manager.rs) already has `scope`, `resilience`, `recovery_gate` on `RegisterOptions`. Adding `credential_rotation_timeout` matches the existing per-resource configuration surface.
- **Operational uniformity AND precision.** Operators set a sensible default at `ManagerConfig` time; override per-resource only when a specific resource has non-uniform requirements (e.g., a remote pool with high handshake latency).
- **Spike validated the parameter shape.** Spike's `Manager::with_timeout(Duration)` ([`spike/.../resource-shape/src/manager.rs:200-205`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape/src/manager.rs)) is the per-Manager default; production extends it to per-resource via `RegisterOptions`. No structural change from spike.

Full signature in §3.3.

#### Q5 — Compile-fail probe production gaps: **trait probes carry forward; runtime gaps go in §3 invariants**

Spike's three compile-fail probes belong to the trait contract; spike NOTES.md flagged two production-relevant gaps that are runtime invariants, not trait probes:

- **Double-registration of same `ResourceKey`.** Manager-runtime invariant, not trait contract. Current code at [`crates/resource/src/manager.rs:329`](../../../crates/resource/src/manager.rs) silently replaces; semantics preserved unchanged (CP2 §5 may revisit). Runtime test added in CP2 §8.
- **NoCredential resource bound to `Some(real_id)`.** Runtime invariant per §3.1: emit `tracing::warn!`, drop the id, register the resource without reverse-index entry. Rationale: configuration mistake, not structural — warn, do not reject. Warn-emission test in CP2 §8.

**CP1 commits four trait probes:**

1. Wrong-signature `on_credential_refresh` override rejected — `_wrong_refresh_signature_must_fail` ([`spike/.../compile_fail.rs:11-95`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)) carries forward.
2. `NoScheme` cannot pretend to be `SecretToken` — `_no_credential_scheme_is_inert_must_fail` ([`spike/.../compile_fail.rs:96-115`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)) carries forward.
3. Non-`Credential` type rejected — `_credential_bound_enforced_must_fail` ([`spike/.../compile_fail.rs:117-150`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)) carries forward.
4. **NEW** for production: wrong-signature `on_credential_revoke` override rejected — symmetric to probe 1.

CP2 §8 elaborates the full test plan.

## §3 — Runtime model

Manager-side runtime contract that consumes the §2 trait shape. Replaces the broken dispatchers at [`crates/resource/src/manager.rs:1360-1401`](../../../crates/resource/src/manager.rs) (the `todo!()` panic over an empty reverse-index — Phase 1 finding 🔴-1). Spike validated the shape with 6 integration tests ([`spike/.../resource-shape-test/src/lib.rs:431-647`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)).

### §3.1 Manager registration with reverse-index write path

`Manager::register` and the five `register_*` topology helpers (`register_pooled`, `register_resident`, `register_service`, `register_transport`, `register_exclusive`) all funnel through the same internal write path, which inspects `R::Credential` to decide reverse-index population.

```rust
// Inside crates/resource/src/manager/registration.rs (post-split per Strategy §4.5).

use std::any::TypeId;
use std::sync::Arc;

use nebula_credential::{Credential, CredentialId, NoCredential};
use crate::{
    manager::Manager,
    resource::Resource,
    runtime::ManagedResource,
};

impl Manager {
    /// Internal funnel — every `register*` path ends here.
    ///
    /// Reverse-index write contract:
    /// - If `R::Credential = NoCredential` → reverse-index NOT written;
    ///   `credential_id` (if `Some`) is logged via `tracing::warn!` and
    ///   discarded (configuration mistake, not error — see Q5).
    /// - Else if `credential_id == None` → return
    ///   `Err(Error::missing_credential_id(R::key()))` — credential-bearing
    ///   resources MUST register against a specific stored credential.
    /// - Else → wrap `R` in a typed dispatcher (§3.2) and append to
    ///   `credential_resources[id]`.
    ///
    /// This method is the explicit write path Phase 1 found missing
    /// (`manager.rs:262, 370` — the field exists but no code wrote to it).
    fn register_inner<R: Resource>(
        &self,
        managed: Arc<ManagedResource<R>>,
        credential_id: Option<CredentialId>,
        options: &RegisterOptions,
    ) -> Result<(), Error> {
        let opted_out = TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>();

        match (opted_out, credential_id) {
            (true, Some(_id)) => {
                tracing::warn!(
                    resource_key = %R::key(),
                    "register: NoCredential resource bound to a credential id; \
                     the id is ignored — this is a configuration mistake but \
                     not a registration error",
                );
                // Fall through — no reverse-index write.
            }
            (true, None) => {
                // OK — NoCredential resources do not register against any id.
            }
            (false, None) => {
                return Err(Error::missing_credential_id(R::key()));
            }
            (false, Some(id)) => {
                let dispatcher: Arc<dyn ResourceDispatcher> = Arc::new(
                    TypedDispatcher {
                        managed: Arc::clone(&managed),
                        timeout_override: options.credential_rotation_timeout,
                    },
                );
                self.credential_resources
                    .entry(id)
                    .or_default()
                    .push(dispatcher);
            }
        }
        // Existing registry write at manager.rs:374 unchanged.
        Ok(())
    }
}
```

**Field type change.** [`crates/resource/src/manager.rs:262`](../../../crates/resource/src/manager.rs) currently declares `credential_resources: dashmap::DashMap<CredentialId, Vec<ResourceKey>>` (resource keys). This Tech Spec changes it to `dashmap::DashMap<CredentialId, Vec<Arc<dyn ResourceDispatcher>>>` — the dispatcher trampoline (§3.2) replaces the resource key. Bare resource keys cannot drive type-erased dispatch; the dispatcher carries the type information and the `Arc<R>` reference.

**Atomicity invariant.** Reverse-index write is part of the registration transaction. If `register_inner` returns `Err`, the registry write at [`manager.rs:374`](../../../crates/resource/src/manager.rs) does not happen (caller short-circuits before reaching it). If `register_inner` returns `Ok`, both writes complete before the `register_*` method returns. There is no observable interim state where the registry has the resource but the reverse-index does not.

**NEW error constructors** required on [`crates/resource/src/error.rs`](../../../crates/resource/src/error.rs) (called out explicitly so the implementer does not infer them silently):

- `Error::missing_credential_id(key: ResourceKey) -> Self` — used by `register_inner` (§3.1 line 641) when a credential-bearing resource (`type Credential != NoCredential`) registers with `credential_id == None`. Carries the resource key for diagnostics; surfaces a registration-time configuration error.
- `Error::scheme_type_mismatch::<R: Resource>() -> Self` — used by `TypedDispatcher::dispatch_refresh` (§3.2 line 743) when the runtime scheme `&(dyn Any + Send + Sync)` cannot downcast to `<R::Credential as Credential>::Scheme`. Generic constructor (parameterized by the registered resource type) so the error message can name the expected scheme type. Surfaces a dispatcher-wiring bug — should never fire in well-formed code, but surfaces clearly if it does.

Both are CP1 contract additions; the error variants land in `crates/resource/src/error.rs` atomically with the §3.1 / §3.2 implementations per [Strategy §4.8](2026-04-24-nebula-resource-redesign-strategy.md). CP3 §13 enumerates the test coverage for each.

### §3.2 Rotation dispatcher (parallel join_all + per-resource isolation)

`Manager::on_credential_refreshed(id, scheme)` and `on_credential_revoked(id)` parallelize across all resources registered against `id` via `futures::future::join_all`. Each per-resource future runs in its own `tokio::time::timeout` bubble per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md). Spike NOTES.md confirmed isolation under both latency (3s sleep, 250ms budget) and errors (one failing, siblings Ok).

**Dispatcher trampoline trait** (replaces the reverse-index `Vec<ResourceKey>` value type per §3.1):

```rust
// Inside crates/resource/src/manager/rotation.rs (NEW).

use std::any::{Any, TypeId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use nebula_credential::{Credential, CredentialId};

/// Type-erased dispatcher for a single registered resource.
///
/// `dyn ResourceDispatcher` is the value type of the credential reverse-
/// index. RPITIT in trait objects is not stable on 1.95, so the trampoline
/// methods return `Pin<Box<dyn Future<...> + Send>>` — a `Box::pin`
/// allocation per dispatch (per-rotation, NOT per-acquire). See §2.5 Q3.
pub(crate) trait ResourceDispatcher: Send + Sync + 'static {
    /// Resource key (`R::key()`) — populates the per-resource event payload + tracing span field.
    fn resource_key(&self) -> ResourceKey;

    /// `TypeId` of the projected scheme. Used to fail-fast on dispatcher wiring mistakes.
    fn scheme_type_id(&self) -> TypeId;

    /// Per-resource timeout override captured at register time (§3.3).
    fn registered_timeout_override(&self) -> Option<Duration>;

    /// Calls `R::on_credential_refresh(scheme)` after downcasting. `&(dyn Any + Send + Sync)` (not
    /// `&dyn Any`) because the future must be `Send` for `join_all` on a multi-thread runtime
    /// (spike NOTES.md "needed an iteration").
    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;

    /// Calls `R::on_credential_revoke(credential_id)`.
    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
}

/// Concrete dispatcher built from a typed `ManagedResource<R>`. Holds the
/// per-resource timeout override captured from `RegisterOptions` at
/// register time (§3.3).
struct TypedDispatcher<R: Resource> {
    managed: Arc<ManagedResource<R>>,
    timeout_override: Option<Duration>,
}

impl<R: Resource> ResourceDispatcher for TypedDispatcher<R> {
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn scheme_type_id(&self) -> TypeId {
        TypeId::of::<<R::Credential as Credential>::Scheme>()
    }

    fn registered_timeout_override(&self) -> Option<Duration> {
        self.timeout_override
    }

    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let scheme: &<R::Credential as Credential>::Scheme = scheme
                .downcast_ref()
                .ok_or_else(|| Error::scheme_type_mismatch::<R>())?;
            self.managed
                .resource
                .on_credential_refresh(scheme)
                .await
                .map_err(Into::into)
        })
    }

    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            self.managed
                .resource
                .on_credential_revoke(credential_id)
                .await
                .map_err(Into::into)
        })
    }
}
```

**Refresh dispatcher signature** (replaces the `todo!()` at [`manager.rs:1378`](../../../crates/resource/src/manager.rs)). Resolves Phase 1 finding 🔴-1.

```rust
impl Manager {
    pub async fn on_credential_refreshed(
        &self,
        credential_id: &CredentialId,
        scheme: &(dyn Any + Send + Sync),
    ) -> Result<Vec<(ResourceKey, RefreshOutcome)>, Error> {
        let dispatchers = self
            .credential_resources
            .get(credential_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();

        if dispatchers.is_empty() {
            return Ok(Vec::new());
        }

        let span = tracing::info_span!(
            "resource.credential_refresh",
            credential_id = %credential_id,
            resources_affected = dispatchers.len(),
        );

        // SAFETY (lifetime story for the dispatch trampoline):
        //   Each per-resource future captures its own `Arc<dyn ResourceDispatcher>`
        //   via the `Arc::clone(d)` move below; the inner `dispatch_refresh(scheme)`
        //   reborrows `&self` from that owned `Arc` for `'a`, and the same `&'a scheme`
        //   reference is shared by every per-resource future. `scheme` outlives
        //   `join_all` because `on_credential_refreshed` holds it for the entire
        //   scope. NO clone of `Scheme` per Strategy §4.3 hot-path invariant.
        //   Spike NOTES.md flagged this as the "iteration that needed Send-bound
        //   on `&dyn Any`" — `&(dyn Any + Send + Sync)` is load-bearing for the
        //   `Send` future on the multi-thread runtime.
        let futures = dispatchers.iter().map(|d| {
            let d = Arc::clone(d);
            let timeout = self.timeout_for(&d);
            let key = d.resource_key();
            async move {
                let outcome = match tokio::time::timeout(timeout, d.dispatch_refresh(scheme)).await {
                    Ok(Ok(())) => RefreshOutcome::Ok,
                    Ok(Err(e)) => RefreshOutcome::Failed(e),
                    Err(_) => RefreshOutcome::TimedOut { budget: timeout },
                };
                self.emit_refresh_event(&key, &outcome);
                (key, outcome)
            }
        });

        let results = span.in_scope(|| join_all(futures)).await;
        self.config.metrics.record_rotation_attempts(&results);
        Ok(results)
    }
}
```

`on_credential_revoked` is structurally symmetric — replace `dispatch_refresh(scheme)` with `dispatch_revoke(credential_id)`; emit `ResourceEvent::CredentialRevoked` per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md); same parallel `join_all`, per-resource timeout, isolation invariant.

**Hot-path borrow invariant.** `scheme: &(dyn Any + Send + Sync)` — borrowed across the entire dispatch. Each per-resource future borrows the same `&Scheme` for the duration; no `Scheme::clone()` per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) constraint 7.

### §3.3 Per-resource timeout enforcement

Per Q4 resolution, two layers: per-Manager default + per-resource override.

```rust
// Inside crates/resource/src/manager/options.rs (post-split).

/// Manager-wide rotation defaults. Applied to every registration
/// unless overridden by `RegisterOptions::credential_rotation_timeout`.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    // ... existing fields ...

    /// Default per-resource credential rotation timeout. Default 30s.
    /// Override per resource via `RegisterOptions::credential_rotation_timeout`.
    pub credential_rotation_timeout: Duration,

    /// Default per-credential rotation concurrency cap. Default 32.
    /// See §3.4 for rationale.
    pub credential_rotation_concurrency: usize,

    // ... existing fields ...
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            // ...
            credential_rotation_timeout: Duration::from_secs(30),
            credential_rotation_concurrency: 32,
            // ...
        }
    }
}

/// Per-resource registration options.
#[derive(Debug, Clone)]
pub struct RegisterOptions {
    pub scope: ScopeLevel,
    pub resilience: Option<AcquireResilience>,
    pub recovery_gate: Option<Arc<RecoveryGate>>,

    /// Per-resource override for credential rotation timeout. `None`
    /// uses `ManagerConfig::credential_rotation_timeout`.
    pub credential_rotation_timeout: Option<Duration>,
}

impl Default for RegisterOptions {
    fn default() -> Self {
        Self {
            scope: ScopeLevel::Global,
            resilience: None,
            recovery_gate: None,
            credential_rotation_timeout: None,
        }
    }
}

impl Manager {
    /// Resolves the timeout for a registered dispatcher. Per-resource
    /// override wins; falls back to Manager default.
    fn timeout_for(&self, dispatcher: &Arc<dyn ResourceDispatcher>) -> Duration {
        dispatcher
            .registered_timeout_override()
            .unwrap_or(self.config.credential_rotation_timeout)
    }
}
```

`TypedDispatcher::timeout_override` (declared in §3.2) is populated from `options.credential_rotation_timeout` at `register_inner` time per §3.1. Per-resource override wins; falls back to `ManagerConfig::credential_rotation_timeout`.

### §3.4 Concurrency cap (default + tunable)

Soft cap of 32 concurrent hooks per credential, default. Tech-lead Phase 2 mention of "~32" carries forward as the production default. Configurable via `ManagerConfig::credential_rotation_concurrency` (§3.3).

**Implementation.** CP1 commits to the `join_all` shape from spike — for N ≤ 32, `join_all` runs N parallel hooks unbounded. For N > 32, a `FuturesUnordered` cap fan-out lands when (and only when) operational signal demands it per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md). CP1 does NOT implement the FuturesUnordered cap; it records the cap value as a future-cleanup hook.

**Why 32:** 5 in-tree consumers × ~3 resources each = ~15 expected; doubling for headroom is 32. Conservative; production load testing in [Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md) post-merge soak validates.

**Observability cost surface.** The `Box::pin` allocation per dispatch (Q3) compounds with N. At N=32, that's 32 box allocations per rotation event. Histogram `nebula_resource.credential_rotation_dispatch_latency_seconds` tracks per-resource latency; `nebula_resource.credential_rotation_attempts` counter labels by `outcome` per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md). Operators monitoring p99 latency at high N will see the box-allocation overhead.

### §3.5 Failure semantics + per-resource outcome aggregation

`RefreshOutcome` and `RevokeOutcome` enums encode per-resource results:

```rust
// Inside crates/resource/src/manager/rotation.rs.

/// Per-resource refresh dispatch result.
#[derive(Debug, Clone)]
pub enum RefreshOutcome {
    /// Hook completed within budget.
    Ok,
    /// Hook returned `Err(...)`.
    Failed(crate::Error),
    /// Hook exceeded its per-resource timeout (the `budget` value).
    TimedOut { budget: Duration },
}

/// Per-resource revocation dispatch result.
#[derive(Debug, Clone)]
pub enum RevokeOutcome {
    Ok,
    Failed(crate::Error),
    TimedOut { budget: Duration },
}

/// Aggregate outcome for one rotation event (refresh OR revoke). Carried in
/// `ResourceEvent::CredentialRefreshed` / `ResourceEvent::CredentialRevoked`
/// payloads (§Event broadcast contract below). One-glance summary for
/// operators reading the event stream; per-resource detail lives in tracing
/// spans, not the event payload.
#[derive(Debug, Clone)]
pub struct RotationOutcome {
    /// Count of dispatchers that returned `RefreshOutcome::Ok` / `RevokeOutcome::Ok`.
    pub ok: usize,
    /// Count of dispatchers that returned `*Outcome::Failed(_)`.
    pub failed: usize,
    /// Count of dispatchers that returned `*Outcome::TimedOut { … }`.
    pub timed_out: usize,
}
```

**Aggregation contract.** `Manager::on_credential_refreshed` returns `Result<Vec<(ResourceKey, RefreshOutcome)>, Error>`. Order matches register insertion order; one tuple per registered resource. Empty vec (wrapped in `Ok`) when no resources registered against the credential. Outer `Err(_)` reserved for setup-time failures only — per-resource errors aggregate into the per-resource `RefreshOutcome::Failed(_)` variant. `RotationOutcome` is derived from this `Vec` at event-emission time (counts of each variant).

**Per-resource isolation invariant** (security amendment B-1 per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md)):

- One resource's `Failed` does NOT poison sibling outcomes — siblings still see `Ok` or `TimedOut` per their own future.
- One resource's `TimedOut` does NOT extend wall-clock for siblings — siblings either complete within their own budget or hit their own timeout.

Validated empirically in spike `parallel_dispatch_isolates_per_resource_errors` ([`spike/.../resource-shape-test/src/lib.rs:537-578`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)) and `parallel_dispatch_isolates_per_resource_latency` ([`spike/.../lib.rs:483-531`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)).

**Event broadcast contract.** Per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md), every dispatched outcome emits **one aggregate event** `ResourceEvent::CredentialRefreshed { credential_id, resources_affected, outcome: RotationOutcome }`. Symmetric `CredentialRevoked` for revocation. **CP1 commits to aggregate-only event broadcast**, with per-resource detail recorded in tracing spans (not events). The per-resource cardinality is fixed at zero for refresh/revoke aggregate events; CP2 §7 may revisit only if operational evidence post-soak ([Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md)) emerges that per-resource events are required — until then, aggregate-only is the contract.

**Revocation health asymmetry — load-bearing.** Refresh emits 1 aggregate event + N tracing spans. Revocation emits 1 aggregate `CredentialRevoked` event + 1 `HealthChanged { healthy: false }` event **per resource where `RevokeOutcome != Ok`** (security amendment B-2 per [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md)). Per-resource `HealthChanged` events fire on the failure path only — successful revocations emit only the aggregate. CP2 §7 finalizes the upper-bound cardinality (the broadcast channel `events.rs` capacity at [`manager.rs:275`](../../../crates/resource/src/manager.rs) is 256 — at N=256 failed revocations the aggregate + N HealthChanged events exhaust the channel; CP2 §7 owns the bounding strategy). The asymmetry itself is the contract: refresh is span-level diagnostic, revocation is event-level health (B-2 mandates per-resource health visibility on revocation failure).

### §3.6 Resolution of Phase 1 🔴-1 (silent revocation drop) and 🔴-4 (drain-abort phase corruption)

**🔴-1 — silent revocation drop.** Resolved by §3.1 (reverse-index write path) + §3.2 (dispatcher implementation). Specifically:

- The `dashmap::DashMap<CredentialId, Vec<...>>` field at [`crates/resource/src/manager.rs:262`](../../../crates/resource/src/manager.rs) gains a write site — `register_inner` populates it for credential-bearing R per §3.1.
- The `on_credential_refreshed` `todo!()` panic at [`manager.rs:1378`](../../../crates/resource/src/manager.rs) is replaced by the parallel-dispatch implementation in §3.2.
- The `on_credential_revoked` `todo!()` panic at [`manager.rs:1400`](../../../crates/resource/src/manager.rs) is replaced by the symmetric revocation dispatcher in §3.2.
- The signature of `on_credential_refreshed` changes: takes a `&(dyn Any + Send + Sync)` scheme parameter (currently no scheme parameter); the per-resource outcome enum is renamed `ReloadOutcome` → `RefreshOutcome` to match the new trait method naming (per Strategy §4.3 + §4.9). The `Result<…, Error>` outer wrapper is preserved (matches the current shape at [`manager.rs:1363`](../../../crates/resource/src/manager.rs)) — per-resource errors aggregate into `RefreshOutcome::Failed(_)` / `RefreshOutcome::TimedOut { … }` variants, and the outer `Err(_)` is reserved for setup-time failures (e.g., reverse-index lookup faults, span construction errors). CP3 §13 enumerates the per-consumer migration impact.

**🔴-4 — drain-abort phase corruption.** Resolved per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md). The `DrainTimeoutPolicy::Abort` branch at [`crates/resource/src/manager.rs:1493-1510`](../../../crates/resource/src/manager.rs) currently calls `set_phase_all(ResourcePhase::Ready)` on line 1507 — corrupts the phase to `Ready` while returning `Err(ShutdownError::DrainTimeout)`. Tech Spec §3.6 commits to wiring `ManagedResource::set_failed(error)` ([`crates/resource/src/runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs), currently `#[expect(dead_code)]`) into the Abort path:

```rust
// Inside crates/resource/src/manager/shutdown.rs (post-split).

// REPLACES manager.rs:1493-1510 DrainTimeoutPolicy::Abort branch.
if let DrainTimeoutPolicy::Abort = self.config.drain_policy {
    let outstanding = self.drain_tracker.0.load(Ordering::SeqCst);
    let err = ShutdownError::DrainTimeout { outstanding };
    self.set_phase_all_failed(err.clone());  // NEW — was set_phase_all(Ready).
    return Err(err);
}
```

`set_phase_all_failed` invokes `ManagedResource::set_failed(err.clone())` per resource — phase becomes `Failed`, `last_error` is recorded. The `#[expect(dead_code)]` annotation on `set_failed` is removed. CP2 §6 records the test invariant.

The fix lands atomically with the `manager.rs` file-split per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md) — both touch the same shutdown path; bundling avoids context-thrash review.

## §4 — Lifecycle

Full lifecycle of a `Resource` registered with `Manager`: `register` → `acquire` → `release` → `drop` → `drain` → `shutdown`. Each subsection records pre-conditions, atomic operations, post-conditions, failure semantics, and cancel-safety. Every claim is anchored to the trait surface from §2 and the runtime surface from §3.

### §4.1 register

**Pre-conditions.** Caller holds an `Arc<Manager>` (or `&Manager`); supplies an `R: Resource` value, `R::Config`, `ScopeLevel`, `TopologyRuntime<R>`, optional resilience, optional recovery gate, and optional `RegisterOptions`. Manager is not in shutdown (no CAS race with `graceful_shutdown` per [`manager.rs:1465-1471`](../../../crates/resource/src/manager.rs)). For credential-bearing resources (`R::Credential != NoCredential`), caller MUST supply `credential_id: Some(_)` per §3.1; for `NoCredential` resources the id is ignored with `tracing::warn!` if `Some`.

**Atomic operations.** Per §3.1 + [Strategy §4.1](2026-04-24-nebula-resource-redesign-strategy.md):

1. `R::Config::validate()` runs first — if it returns `Err`, registration aborts with no side effects.
2. `ManagedResource<R>` is constructed in-process: `arc_swap::ArcSwap<Config>`, `topology` enum, `release_queue` `Arc::clone`, `generation: AtomicU64::new(0)`, `status: ArcSwap<ResourceStatus>` initialised to `Initialising`.
3. **Reverse-index write** runs *before* the registry write per [ADR-0036 line 103](../adr/0036-resource-credential-adoption-auth-retirement.md). For credential-bearing R: `TypedDispatcher` wraps `Arc<ManagedResource<R>>` + `RegisterOptions::credential_rotation_timeout`; pushed under `credential_resources.entry(id)` (DashMap). For `NoCredential` R: skipped silently with warn.
4. **Registry write** appends to `Registry` (the `DashMap<(ResourceKey, ScopeLevel), Arc<dyn AnyManagedResource>>` at [`manager.rs:374`](../../../crates/resource/src/manager.rs)). Same-key replacement is silent today (Strategy §4.5 preserves; CP3 §11 may revisit).
5. `ResourceEvent::Registered { key }` broadcast via `event_tx` (cap 256).

**Post-conditions.** `R::key()` is reachable from any subsequent `acquire_*` call; `on_credential_refresh` / `on_credential_revoke` dispatchers populated for credential-bearing R; status phase = `Initialising` until first acquire creates a runtime instance (or warmup, §5.2).

**Failure semantics.** Returns `Result<(), Error>`. Validation failure → `Err(Error::permanent(_))`. Missing credential id on credential-bearing R → `Err(Error::missing_credential_id(R::key()))` (NEW, §3.1). Shutdown-in-progress → `Err(Error::cancelled(_))` (per existing pattern at [`manager.rs:1465-1471`](../../../crates/resource/src/manager.rs)). All failures leave the registry and reverse-index in their pre-call state — no half-written state observable. The reverse-index write is part of the registration *transaction*: if registration fails after step 3, the dispatcher is dropped (no rollback needed because step 3 happened *before* step 4 only when validation already passed).

**Cancel-safety.** `register` is fully synchronous (no `await` points) — cancellation cannot interleave with the atomic operations above. Construction of `ManagedResource<R>` does not yield. The `event_tx.send(Registered)` is best-effort (lagging subscribers receive `Lagged` on next `recv`).

**Resolves Phase 1 🔴-1 (silent revocation drop).** Reverse-index write at step 3 is the explicit write path Phase 1 found missing ([`02-pain-enumeration.md` §4 row 🔴-1](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md), [`crates/resource/src/manager.rs:262`](../../../crates/resource/src/manager.rs)). Atomic with registry write per [security amendment B-1 / constraint #1](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md).

### §4.2 acquire

**Pre-conditions.** Resource is registered (`Manager::lookup::<R>(scope)` returns `Ok(Arc<ManagedResource<R>>)`). Manager not shut down (`is_accepting()` true). Caller holds an `AcquireOptions` (default acceptable) and a `ResourceContext`. For credential-bearing R, the engine has resolved `<R::Credential as Credential>::Scheme` from the credential store via `CredentialAccessor` ([`crates/resource/src/context.rs`](../../../crates/resource/src/context.rs)).

**Per-topology variants.** Each topology has a distinct atomic-operation chain — five total post-Daemon/EventSource extraction (per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md)):

| Topology  | Method                | What gets returned                                  |
|-----------|-----------------------|-----------------------------------------------------|
| Pooled    | `acquire_pooled`      | `ResourceGuard::guarded(_)` (pool-return on drop)   |
| Resident  | `acquire_resident`    | `ResourceGuard::shared(_)` (Arc-shared lease)       |
| Service   | `acquire_service`     | `ResourceGuard::guarded(_)` (release_token on drop) |
| Transport | `acquire_transport`   | `ResourceGuard::guarded(_)` (close_session on drop) |
| Exclusive | `acquire_exclusive`   | `ResourceGuard::owned(_)` (no return; `reset` on drop) |

**Atomic operations** (common chain):

1. `Manager::lookup::<R>(scope)` — DashMap read; cancel-safe (no `await`).
2. **Recovery gate check** (if `Some(gate)` registered) — early `Err(Error::transient_recovery_gate_open(_))` if gate is open. No await window before first refusal.
3. **Resilience wrap** — if `AcquireResilience` registered, `execute_with_resilience` wraps the acquire body with timeout + retry + circuit breaker per [`crates/resource/src/integration/`](../../../crates/resource/src/integration/).
4. **Topology-specific runtime acquire** — pool semaphore wait, service token request, transport session open, exclusive runtime build. Each is `await`-bounded by the resilience wrap (or unbounded if absent).
5. **Drain-tracker increment** — `drain_tracker.0.fetch_add(1, Relaxed)` *before* `ResourceGuard` construction. Guard's `Drop` decrements + `notify_waiters` on 1→0. This is the contract `wait_for_drain` consumes (§4.5).
6. **Metrics + event emission** — `ResourceOpsMetrics::record_acquire()` + `ResourceEvent::AcquireSuccess { key, duration }` broadcast.

**Post-conditions.** Caller holds a `ResourceGuard<R>` that derefs to `R::Lease`. Drain counter incremented by 1. On `acquire` failure, drain counter unchanged.

**Failure semantics.** Cancel-token signal during step 4 → `Err(Error::cancelled(_))` (drain counter NOT incremented because increment is at step 5). Recovery gate open → `Err(Error::transient_recovery_gate_open(_))`. Resilience-bounded timeout → `Err(Error::timeout(_))`. Topology-specific failure → `Err(R::Error.into())`. **Failure is recorded as `AcquireFailed` event** per [`events.rs:38-44`](../../../crates/resource/src/events.rs); `record_acquire_error()` increments the error counter.

**Cancel-safety.** Each step's `await` site is cancel-safe by design: lookup is sync; resilience wrap respects the `tokio::select!` parent token; topology-specific waits use `tokio::time::timeout` + cancel-token select. **The drain-tracker increment in step 5 happens AFTER the last cancel-aware `await` of the runtime acquire** — there is no window where increment happens but guard never constructs. If cancellation lands between step 5 and `ResourceGuard` construction (in practice, no `await` between them — increment is the last step before guard `new`), the panic-aware `Drop` of the partially-constructed guard recovers the counter. CP3 §11 carries the test invariant.

### §4.3 release

**Pre-conditions.** Caller drops a `ResourceGuard<R>`, or a guard variant fires its `on_release` callback. Manager may be in any phase except final shutdown.

**Two paths** — sync drop vs async ReleaseQueue:

- **Sync drop path.** `ResourceGuard::Drop` runs synchronously at the drop site. For owned guards (Exclusive), `R::reset` is *spawned* via `release_queue.submit` because `reset` is async and `Drop` is sync. For guarded guards (Pool/Service/Transport), the `on_release` closure is invoked synchronously — the closure decides whether to recycle or destroy the lease.
- **Async ReleaseQueue path.** When the on-release closure needs to run async work (close transport session, return pool connection with health probe), it submits a `TaskFactory` to `ReleaseQueue::submit` ([`release_queue.rs:97`](../../../crates/resource/src/release_queue.rs)). Round-robin to N primary workers (default 4); fallback channel on full; rescue spawn on double-full with 30s timeout cap. Tasks bounded to 30s execution ([`release_queue.rs:41`](../../../crates/resource/src/release_queue.rs)).

**Tainted vs healthy.** `ResourceGuard::tainted()` flips the `tainted` flag inside `GuardInner::Guarded { tainted, .. }`. On drop, the on-release closure inspects `tainted`: true → destroy lease (transport: `close_session`, pool: drop without recycle); false → recycle (pool: return to pool, service: `release_token`, transport: pool resume).

**Post-conditions.** `ResourceEvent::Released { key, held, tainted }` broadcast. `ResourceOpsMetrics::record_release()` increments. **Drain-tracker decrements** atomically (Acquire-Release ordering); on 1→0 transition the guard's `Drop` calls `notify_waiters` so `wait_for_drain` wakes up. On tainted-destroy path, `record_destroy()` also increments.

**Failure semantics.** ReleaseQueue full + fallback full + rescue timeout → `dropped_count` increments ([`release_queue.rs:107`](../../../crates/resource/src/release_queue.rs)). The lease leaks if the rescue task drops it before reaching the worker. Operator signal: `dropped_count > 0` is observable but not yet surfaced as a `ResourceEvent` variant (CP3 §11 may revisit; out of CP2 scope per Strategy §5).

**Cancel-safety.** `Drop` is panic-safe via `catch_unwind` around the on-release closure ([`guard.rs:96-99`](../../../crates/resource/src/guard.rs)). Permit return is decoupled from closure execution so a closure panic cannot leak a semaphore slot. Cancel-token cancellation during async release work is acceptable: the worker spawn picks up cancel and exits cleanly per release_queue's cancel-on-drain contract.

### §4.4 drop

**Pre-conditions.** A `ResourceGuard<R>` exits scope without explicit release (function return, panic unwind, `let _ = guard;`).

**`#[must_use]` enforcement.** `ResourceGuard` is annotated `#[must_use = "dropping a ResourceGuard immediately releases the resource"]` at [`guard.rs:31`](../../../crates/resource/src/guard.rs). Compiler emits a warning if the guard is constructed and dropped without binding. **CP2 affirms this annotation as load-bearing**: it is the type-level signal that drop = release.

**Rescue-timeout 30s.** Per [`release_queue.rs:60`](../../../crates/resource/src/release_queue.rs), when both the primary channel and the fallback channel are full at submit time, a short-lived rescue task is spawned that awaits fallback capacity for up to 30s. After 30s the task records the drop and exits. **This is the upper bound on tail-latency for a drop reaching a worker.**

**Atomic operations.** Identical to §4.3 release — drop IS the trigger for the release path. Difference is no caller-visible result; the guard simply ceases to exist.

**Failure semantics.** Best-effort. The rescue-timeout dropped-count metric is the operator signal that the system was overloaded.

**Cancel-safety.** Same as §4.3.

### §4.5 drain

**Pre-conditions.** `Manager::graceful_shutdown(config)` invoked. `shutting_down` CAS flips `false → true` (loser caller gets `Err(ShutdownError::AlreadyShuttingDown)` per [`manager.rs:1465-1471`](../../../crates/resource/src/manager.rs)). No interleaving caller can re-enter the drain logic against half-torn state.

**Atomic operations** — phased per [`manager.rs:1473-1559`](../../../crates/resource/src/manager.rs):

1. **Phase 1 SIGNAL.** `cancel.cancel()` — rejects new acquires (lookup checks token), tells release_queue workers to drain.
2. **`set_phase_all(Draining)`** — every registered `ManagedResource` flips status phase to `Draining` so health probes mid-drain see the correct lifecycle. Uses `Arc<dyn AnyManagedResource>` virtual call into `set_phase_erased`.
3. **Phase 2 DRAIN.** `wait_for_drain(drain_timeout)` blocks on the drain-tracker `Notify`; returns `Ok` when counter hits 0, `Err(DrainTimeoutError { outstanding })` on timeout.
4. **DrainTimeout dispatch** per `DrainTimeoutPolicy`:
   - `Abort` → **fixed in §3.6**: `set_phase_all_failed(ShutdownError::DrainTimeout { outstanding })` per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md). Phase becomes `Failed` (NOT `Ready` — current corruption per Phase 1 🔴-4); `last_error` recorded; `shutting_down` resets to false; returns `Err`.
   - `Force` → log warn, proceed to Phase 3 with `outstanding_after_drain` recorded.
5. **Phase 3 CLEAR.** `set_phase_all(ShuttingDown)` → `registry.clear()`.
6. **Phase 4 AWAIT WORKERS.** `ReleaseQueue::shutdown(handle)` bounded by `release_queue_timeout`. Failure surfaces as `Err(ShutdownError::ReleaseQueueTimeout)`.

**Post-conditions.** On success: registry empty, release_queue workers joined, `ShutdownReport { outstanding_handles_after_drain, registry_cleared: true, release_queue_drained: true }` returned. On Abort failure: registry preserved, all phases `Failed`, `Err(DrainTimeout)` returned.

**Failure semantics.** Three terminal errors per [`manager.rs:147-180`](../../../crates/resource/src/manager.rs): `AlreadyShuttingDown`, `DrainTimeout { outstanding }`, `ReleaseQueueTimeout { timeout }`. **Observability honesty restored** per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md): operators polling `health_check` after `Abort` see `Failed` phase + `last_error: ShutdownError::DrainTimeout` instead of misleading `Ready`.

**Cancel-safety.** Drain is itself cancel-aware (selects on the cancel token) but `graceful_shutdown` is the *driver* of the cancel — calling it concurrently with the cancel-token external trip is benign (the AlreadyShuttingDown CAS dedup wins).

**Resolves Phase 1 🔴-4.** [Phase 1 row 🔴-4](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) — drain-abort path corruption — closed by step 4 Abort branch wiring `set_failed`.

### §4.6 shutdown

**Force vs graceful.** `graceful_shutdown` (§4.5) is the *only* public shutdown entry. Force is a *policy* (`DrainTimeoutPolicy::Force`) not a separate method — it changes whether outstanding handles are tolerated, not whether the drain machinery runs.

**ReleaseQueue drain.** Phase 4 of `graceful_shutdown` (§4.5 step 6) invokes `ReleaseQueue::shutdown(handle)`. Workers received cancel signal in Phase 1; they finish in-flight tasks bounded by `TASK_EXECUTION_TIMEOUT` (30s, [`release_queue.rs:41`](../../../crates/resource/src/release_queue.rs)). `ReleaseQueue::shutdown` joins all worker `JoinHandle`s; `release_queue_timeout` (default per `ShutdownConfig`) bounds the total await.

**Idempotency.** CAS-guarded. Second concurrent `graceful_shutdown` returns `Err(AlreadyShuttingDown)`. After successful return, repeated calls *do* re-enter (CAS succeeds because `shutting_down` was reset to false on Abort or true on success — CP3 §12 may add an explicit terminal-state guard; currently semantically safe because registry is empty so no work to do).

**Post-conditions.** Manager handle still exists; consumers may still hold `Arc<Manager>` for diagnostics, but `register` rejects new resources, `acquire_*` rejects new acquires (cancel token tripped), and `subscribe_events` returns a receiver that will only see future events (none, in practice).

**Failure semantics.** Same as §4.5 step 4 + step 6. No new error variants in §4.6.

**Cancel-safety.** Cancel-driven. The whole machinery is the cancel landing site — there is no caller-side cancel to honour beyond the cancel-token trip itself.

## §5 — Implementation specifics

The mechanism details. Six items: blue-green pool swap (§5.1), credential-bearing `warmup_pool` signature (§5.2), revocation default-hook decision (§5.3), Manager file-split execution (§5.4), resilience policy execution (§5.5), and the CP3 deferrals surfaced from this CP's mechanism work (§5.6). Each of §5.1-§5.5 commits a load-bearing decision; reviewers should scrutinise §5.3 in particular (security implication central). §5.6 records what is *not* in CP2 mechanism scope but must travel with these decisions to CP3.

### §5.1 Blue-green pool swap mechanism

**Resource owns the swap** — Manager dispatches the hook; the resource impl does the swap inside `on_credential_refresh`. This is the canonical pattern from [credential Tech Spec §3.6 lines 961-993](2026-04-24-credential-tech-spec.md), adopted verbatim per [Strategy §4.1](2026-04-24-nebula-resource-redesign-strategy.md). Manager NEVER orchestrates pool recreation; Manager NEVER holds `Scheme` longer than the dispatch call window. This is the key security invariant from [security review constraint #2 (lines 105-108)](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md): "the swap happens *inside* the resource impl, not inside the manager."

**Verbatim cite from credential Tech Spec §3.6 lines 981-993:**

> `pub async fn on_credential_refresh(&self, new_scheme: &PostgresConnectionScheme) -> Result<(), PostgresError> { let new_pool = build_pool_from_scheme(new_scheme).await?; let mut guard = self.inner.write().await; *guard = new_pool; Ok(()) }` — old connections drain naturally as their RAII guards drop; new queries use the new pool (read lock acquires against the new inner after swap).

CP2 does not restate the credential primitives; readers consume §3.6 directly.

**Swap point: `Arc<RwLock<Pool>>`, NOT `ArcSwap`.** The pattern from credential Tech Spec §3.6 uses `Arc<tokio::sync::RwLock<deadpool_postgres::Pool>>`. CP2 commits to this shape for connection-bound resources. Rationale (three reasons):

- **Async-safe write contention.** Pool-rebuild inside `on_credential_refresh` is `async` (`build_pool_from_scheme(new_scheme).await?`) — it must hold the lock across an `.await`. `ArcSwap` does not block writers, but it also does not provide the *exclusion* needed to prevent two concurrent refreshes from racing to rebuild. `tokio::sync::RwLock`'s write guard is `Send` and held across `.await`, so two parallel refresh dispatches against the same resource (which §3.5 isolation protects against externally, but defence-in-depth) serialise on the write guard.
- **Read affordance is identical.** Read-side query path holds `inner.read().await.get_connection().await?` — readers are non-blocking under contention because read locks are shared. Throughput on the hot acquire path is unchanged from `Arc<Pool>` direct-access.
- **Old-pool drain is RAII-natural.** When the write guard publishes the new pool, every caller that had previously taken a `Pool` clone (via `inner.read().await.clone()` or by holding a connection-guard rooted in the old pool) continues to use the old pool. RAII guards drop naturally; the old pool's connection slots reach zero and the pool drops. No explicit drain coordination required from Manager.

**ArcSwap rejected for this surface.** ArcSwap (lock-free atomic swap) is correct for `ResourceStatus` reads (§8.5), where readers want the most recent value with no contention. For pool swap, the writer's *exclusion* requirement is dispositive — a refresh races against itself otherwise. CP3 §10 enumerates the per-topology guidance.

**In-flight acquires during swap.** The acquire path (§4.2 step 4) acquires a *read* lock on `inner` to obtain the live `Pool`, then defers the actual connection `get` to the cloned pool reference. If a swap is in progress (write lock held), new read-lock acquisitions wait. Wait time is bounded by `build_pool_from_scheme(new_scheme).await` — typically sub-second; observability covers this via `nebula_resource.credential_rotation_dispatch_latency_seconds` (§6.2). In-flight queries against the OLD pool are unaffected (their connection-guard is rooted in the pre-swap `Pool` clone). On swap publication, the next acquire sees the new pool; old pool drains as RAII guards drop.

**Budget guidance** (CP3 §11 will detail; surfaced now per security-lead amendment SL-3). The resource-side `build_pool_from_scheme(new_scheme).await` SHOULD complete in significantly less than the Manager dispatch budget (default 30s per CP1 §2.5 Q4) — recommend ≤ 60% of dispatch timeout (≤ 18s under the default) to leave headroom for swap installation (write-guard publication) plus old-pool drop. Rationale: if `build_pool_from_scheme` exhausts the full dispatch budget, Manager's `tokio::time::timeout` fires while the resource-side write guard is still held; the future-drop is RAII-natural so the lock releases (no leak), but observability is poorer — the dispatch surfaces as `TimedOut` rather than `Failed` despite the resource's own rebuild having actually failed inside its allotted time. CP3 §11 specifies the per-`RegisterOptions::pool_rebuild_timeout` knob.

### §5.2 `warmup_pool` credential-bearing signature

**Decision.** `warmup_pool` takes the credential scheme as an explicit parameter; **NOT via `Scheme::default()`** (security amendment B-3 per [`phase-2-security-lead-review.md:76-82`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md), Phase 1 🟡-17 per [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)). The current `R::Auth::default()` call at [`crates/resource/src/manager.rs:1268`](../../../crates/resource/src/manager.rs) is removed.

**Signature** (replaces [`manager.rs:1259-1280`](../../../crates/resource/src/manager.rs)):

```rust
impl Manager {
    /// Warmup pool with a real credential scheme.
    ///
    /// Caller resolves the scheme from the credential store before
    /// invoking warmup. Manager does NOT silently default-fill the scheme
    /// because `Scheme::default()` historically created empty-credential
    /// runtimes (Phase 1 🟡-17). For `NoCredential` resources, see
    /// `warmup_pool_no_credential` below.
    pub async fn warmup_pool<R>(
        &self,
        credential: &<R::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let config = managed.config();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let count = rt.warmup(&managed.resource, &config, credential, ctx).await;
                Ok(count)
            },
            _ => Err(Error::permanent(format!(
                "{}: warmup_pool requires Pool topology, registered as {}",
                R::key(), managed.topology.tag()
            ))),
        }
    }

    /// Warmup pool for a `NoCredential` resource. Compile-time bound
    /// `R::Credential = NoCredential` enforces opt-out at the type level —
    /// no runtime check, no scheme parameter.
    pub async fn warmup_pool_no_credential<R>(
        &self,
        ctx: &ResourceContext,
    ) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled<Credential = nebula_credential::NoCredential>,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        // ... mirrors warmup_pool body, passing &NoScheme.
    }
}
```

Two methods, one per shape, distinguished by trait-bound at compile time. Three reasons over a unified `Option<&Scheme>`:

- **Type-level enforcement of B-3.** A unified `warmup_pool` taking `Option<&<R::Credential as Credential>::Scheme>` would require runtime branching — `None` would fall back to a default. The split makes "no credential" expressible only via the dedicated `_no_credential` variant whose bound prevents calling it on credential-bearing R.
- **No `Default` trait dependency on `Scheme`.** The current `R::Auth: Default` bound at [`manager.rs:1264`](../../../crates/resource/src/manager.rs) goes away. `Scheme` types do not need `Default` per `Credential` trait shape ([`credential/src/contract/credential.rs`](../../../crates/credential/src/contract/credential.rs)).
- **Caller obligation explicit.** The credential-bearing variant requires the caller to resolve `<R::Credential as Credential>::Scheme` first — typically via `CredentialAccessor::resolve(credential_id)` from `ResourceContext`. CP3 §11 specifies the helper if a non-trivial number of consumers want a "warmup with id" convenience.

**Trade-off accepted.** Two methods instead of one. Mitigated by the type-level safety (B-3 unfaultable) and by `RegisterOptions` carrying credential-id metadata for future `warmup_pool_by_id` ergonomics if needed (deferred to CP3 §11).

### §5.3 Revocation default-hook mechanism

**Decision: option (b) — default body returns `Ok(())`; Manager unconditionally flips a per-resource `credential_revoked` atomic post-dispatch. Subsequent `acquire_*` calls fail.**

The hook's *default body* and Manager's *post-dispatch enforcement* are two layers; option (b) wires both. The user-trait method's default body is no-op (matching CP1 §2.1 declaration `async { Ok(()) }`); Manager owns the invariant-enforcement layer that flips the per-resource atomic regardless of whether the hook was overridden.

Three options were considered (per continuation prompt + [Strategy §4.2 lines 250-258](2026-04-24-nebula-resource-redesign-strategy.md)):

- **(a) Default body destroys pool (aggressive).** The default `on_credential_revoke` would tear down the runtime via `destroy()` (consume Runtime) without any user override. **Rejected** — too aggressive for the default contract: many resources have valid reasons to defer destruction (in-flight queries should complete; pool may be shared across credential ids in multi-tenant configs; `NoCredential` resources cannot meaningfully destroy). Forcing destruction by default breaks per-resource invariant authorship — every override must wrap the parent default to suppress destruction, which is the opposite of the additive-override pattern Strategy §4.2 establishes.
- **(b) Default body no-op + Manager unconditional post-dispatch taint-flip. SELECTED.** Default body of `on_credential_revoke` returns `Ok(())`; Manager flips a `credential_revoked: AtomicBool` on `ManagedResource<R>` after every dispatch (success, failure, or timeout). Subsequent `acquire_*` calls check the atomic and return `Err(Error::credential_revoked(_))` per [Strategy §4.2](2026-04-24-nebula-resource-redesign-strategy.md) invariant ("post-invocation, the resource emits no further authenticated traffic on the revoked credential"). In-flight handles complete naturally. Resources MAY override the hook to add stronger semantics (destroy pool synchronously, taint outstanding guards via guard-side `tainted = true`) — overriding tightens, never weakens.
- **(c) Default body no-op; no Manager-side enforcement (override-only invariant).** Default body returns `Ok(())`; Manager makes no post-dispatch change. **Rejected** — a resource with default `on_credential_revoke` body satisfies the trait but VIOLATES the Strategy §4.2 invariant: it continues serving authenticated traffic from the revoked credential because nothing flips its state. The contract is uncatchable by Manager and uncatchable by `tracing::warn!` — silent invariant violation. This is the failure mode security-lead Phase 2 BLOCKED on for Option A.

**Distinction between (b) and (c).** The two diverge only on *who enforces the post-condition*. (c) puts the burden on every overrider; the default body is a structural footgun. (b) puts the burden on Manager, leaving the user's override free to *add* stronger semantics without having to *first* satisfy the base invariant manually.

**Why (b) wins on security grounds.**

- **Default body honours the invariant** (Strategy §4.2 / [ADR-0036 line 102](../adr/0036-resource-credential-adoption-auth-retirement.md)). A resource that does nothing in `on_credential_revoke` (e.g., a stateless API client that only fetches per-request) is still tainted by Manager's enforcement; the next acquire fails. There is no safe-but-silent state where Manager believes revocation succeeded but the resource continues to issue authenticated traffic on the revoked credential.
- **In-flight handles drain naturally.** Outstanding `ResourceGuard<R>` references continue to function (their `Drop` releases against the still-owned runtime). New acquires fail. This matches the soft-revocation semantics from [credential Tech Spec §4.3 lines 1062-1068](2026-04-24-credential-tech-spec.md): existing in-flight resolves continue for grace; new resolves fail.
- **Override is additive, not corrective.** A `PostgresPool` impl can override `on_credential_revoke` to *additionally* destroy the pool synchronously (immediate hard-revocation semantics). It does NOT need to also flip `credential_revoked` itself — Manager handles that layer. The override's job is the resource-specific cleanup; Manager's job is the invariant enforcement.

**Concrete wiring** (CP3 §10 for full code):

```rust
// Inside Manager::dispatch_revoke (§3.2 trampoline path).
async fn dispatch_revoke_with_tainting(
    &self,
    dispatcher: &Arc<dyn ResourceDispatcher>,
    credential_id: &CredentialId,
) -> RevokeOutcome {
    // 1. Run the override (or default no-op).
    let hook_result = dispatcher.dispatch_revoke(credential_id).await;
    // 2. Regardless of override result (Ok / Err / would-be-timeout),
    //    Manager flips the per-resource `credential_revoked` atomic on
    //    `ManagedResource`. This is the invariant-enforcement layer.
    self.set_revoked_for_dispatcher(dispatcher);
    // 3. Convert to outcome (Failed/TimedOut still surface to operators
    //    via RevokeOutcome + HealthChanged events).
    match hook_result { ... }
}
```

The taint-flip is unconditional (runs even when override returns `Err`), preserving the post-condition invariant on the failure path. The override's `Err` is recorded in `RevokeOutcome::Failed` and triggers the `HealthChanged { healthy: false }` event (security amendment B-2). **The atomic flip happens AFTER the await of `dispatch_revoke` — there is no window where the atomic flips before the override could observe its pre-revocation state.**

**Trade-off accepted.** A future revocation override that *intentionally* wants to leave the resource live (e.g., multi-tenant pool where one tenant's credential was revoked but the pool remains valid for other tenants) cannot — the taint flip is unconditional and per-resource (not per-credential within a multi-tenant pool). CP3 §11 may add a per-`RegisterOptions::tainting_policy` knob if a real consumer demands it; CP1/CP2 commits to unconditional tainting because zero in-tree consumers fit the multi-tenant exception today.

### §5.4 Manager file-split execution

Per [Strategy §4.5](2026-04-24-nebula-resource-redesign-strategy.md), the 2101-line `crates/resource/src/manager.rs` splits into submodules **without changing the `Manager` type's public surface**. CP2 commits the cut points:

| Submodule path                                | Contents                                                                 | Internal visibility |
|-----------------------------------------------|--------------------------------------------------------------------------|---------------------|
| `crates/resource/src/manager/mod.rs`          | `Manager` struct, public `register*`, `acquire*`, `subscribe_events`, `is_accepting`, `lookup` | `pub`               |
| `crates/resource/src/manager/options.rs`      | `ManagerConfig`, `RegisterOptions`, `ShutdownConfig`, `DrainTimeoutPolicy`, `ShutdownError`, `ShutdownReport` | `pub` (re-exported) |
| `crates/resource/src/manager/registration.rs` | `register_inner` (§3.1), reverse-index write path, error constructors `missing_credential_id` + `scheme_type_mismatch` | `pub(crate)`        |
| `crates/resource/src/manager/gate.rs`         | `GateAdmission`, `admit_through_gate`, `settle_gate_admission`           | `pub(crate)`        |
| `crates/resource/src/manager/execute.rs`      | `execute_with_resilience`, `validate_pool_config`, `wait_for_drain`      | `pub(crate)`        |
| `crates/resource/src/manager/rotation.rs`     | `ResourceDispatcher` trait, `TypedDispatcher<R>`, `on_credential_refreshed`, `on_credential_revoked`, observability scaffolding | `pub(crate)` (trait `pub` for downstream test-access only via doc(hidden)) |
| `crates/resource/src/manager/shutdown.rs`     | `graceful_shutdown`, `set_phase_all`, `set_phase_all_failed` (§3.6 fix)  | `pub(crate)`        |

**Function-level boundaries deferred to CP3 §9.** CP2 locks the file structure; CP3 enumerates which exact `impl Manager { fn ... }` block lands in which submodule. This split decouples the structural decision (which CP2 must lock for `manager.rs:262` reverse-index write to land in `registration.rs` not `mod.rs`) from the function-arrangement decision (which CP3 owns alongside the per-consumer migration list).

**`pub(crate)` discipline.** `manager/mod.rs` is the only `pub` re-export site for everything except `options.rs` (which carries types already public). `rotation.rs` exposes `ResourceDispatcher` only via `pub` — but it is intentionally an internal seam: doc-hidden re-export under `crate::__internal::ResourceDispatcher` for test access and downstream type-erasure ergonomics. CP3 §10 specifies the doc-hidden path.

**No public API change.** Every existing import at `nebula_resource::*` (5 in-tree consumers) keeps working — `lib.rs` re-exports remain untouched. The split is purely structural; it does not surface as a breaking change in the migration wave.

### §5.5 Resilience policy execution (drain-abort fix)

Per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md) + §3.6 of this Tech Spec. `ManagedResource::set_failed(error)` at [`crates/resource/src/runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs) is currently dead-coded behind `#[expect(dead_code, reason = "callers will land with the recovery-error work")]`. CP2 wires it.

**Wiring site.** The `DrainTimeoutPolicy::Abort` branch in `manager/shutdown.rs` (§5.4 split) replaces:

```rust
// REMOVED — current at manager.rs:1507
self.set_phase_all(crate::state::ResourcePhase::Ready);
```

with:

```rust
// NEW.
self.set_phase_all_failed(ShutdownError::DrainTimeout { outstanding });
```

`set_phase_all_failed` iterates `registry.all_managed()` and calls `managed.set_failed(err.clone())` for each, which:

1. Sets `status` phase to `Failed` (`ResourcePhase::Failed`).
2. Records `last_error: Some(error)` on `ResourceStatus`.
3. Emits a `ResourceEvent::HealthChanged { key, healthy: false }` per [`events.rs:54-60`](../../../crates/resource/src/events.rs).

**`#[expect(dead_code)]` removal.** Annotation lifted from [`runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs). The lint scope was correct ("callers will land with the recovery-error work") — that work is this Tech Spec's CP2 wiring.

**Test invariant** (CP3 §12):

```rust
// `tests/basic_integration.rs` — abort-policy-records-failed-phase.
let report = manager.graceful_shutdown(
    ShutdownConfig::default()
        .with_drain_timeout(Duration::from_millis(50))
        .with_abort_policy()
).await;
assert!(matches!(report, Err(ShutdownError::DrainTimeout { .. })));
let phase = manager.health_check::<MyResource>(&scope).unwrap().phase;
assert_eq!(phase, ResourcePhase::Failed);  // NOT Ready (current corruption).
```

**Resolves Phase 1 🔴-4.** [Phase 1 row 🔴-4](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) — drain-abort phase corruption — closed atomically with §5.4 file-split per [Strategy §4.6](2026-04-24-nebula-resource-redesign-strategy.md).

### §5.6 CP3 deferrals from this CP

CP2 mechanism work surfaced two CP3-track surfaces that are NOT CP2-applicable. Recorded here so they cannot be silently introduced under CP3 DX pressure without their gating constraints.

- **Future `RegisterOptions::tainting_policy` knob (security-lead amendment SL-1; §5.3 line 1250 trade-off).** A per-consumer opt-out from §5.3 option (b)'s unconditional taint flip — for example, a multi-tenant pool that wants to revoke one tenant's credential while continuing to serve others. Two gates required before introduction at CP3 §10 surface review: (1) a real in-tree consumer must surface the exception (synthetic tests do not qualify per §5.3 line 1250 deferral discipline); (2) the knob must include a security-review hook in the CP3 surface review wave that introduces it. CP3 §10 surface review confirms no premature `tainting_policy` knob ships before both gates clear. Until then, unconditional tainting is the secure default and §5.3 option (b) is the only revocation shape.
- **Future `warmup_pool_by_id` ergonomic helper (security-lead amendment SL-2 § follow-up; §5.2 line 1203).** A convenience that resolves the credential scheme via `CredentialAccessor::resolve(credential_id)` internally so consumers do not call the resolver themselves before `warmup_pool`. Gate: the resolution path MUST go through `CredentialAccessor`; bypassing it (e.g., reading scheme bytes from `RegisterOptions` directly) would resurrect the B-3 attack surface the §5.2 split closes. CP3 §11 ergonomics review verifies the helper composes the existing `CredentialAccessor` rather than introducing a parallel resolution path.

These are the only two CP3 deferrals from CP2 mechanism work. SL-3 (resource-side budget guidance) is not deferred; it lands in §5.1 with the CP3 §11 forward-ref.

## §6 — Operational (observability concrete)

Per [Strategy §4.9](2026-04-24-nebula-resource-redesign-strategy.md) + [tech-lead amendment 2 / Phase 3 CP1 E1 invariant](../drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md). Every observability identifier below is **CP-review-gateable** — the names locked in §6.1-§6.4 are what land in the migration PR; review pushback that proposes alternative names lands as an amendment, not a CP3 rename.

### §6.1 Trace span names

Six spans, locked. Each name is the final string used in `tracing::info_span!` / `tracing::error_span!` invocations. No suffixes, no per-resource keys in the name itself (resource keys are *fields*, not name suffixes).

| Span name (literal)                       | Level | Where emitted                                                              | Fields                                                      |
|-------------------------------------------|-------|----------------------------------------------------------------------------|-------------------------------------------------------------|
| `resource.credential_refresh`             | INFO  | `Manager::on_credential_refreshed` outer scope (§3.2 line 793)             | `credential_id`, `resources_affected`                       |
| `resource.credential_refresh.dispatch`    | INFO  | Per-resource future inside `join_all`                                      | `credential_id`, `resource_key`, `timeout_budget_ms`        |
| `resource.credential_revoke`              | WARN  | `Manager::on_credential_revoked` outer scope                               | `credential_id`, `resources_affected`                       |
| `resource.credential_revoke.dispatch`     | WARN  | Per-resource future inside `join_all` (revocation)                         | `credential_id`, `resource_key`, `timeout_budget_ms`        |
| `resource.acquire.{topology}`             | DEBUG | `Manager::acquire_*` (§4.2 step 4)                                         | `resource_key`, `topology`, `scope`                         |
| `resource.shutdown`                       | INFO  | `Manager::graceful_shutdown` outer scope                                   | `drain_timeout_ms`, `policy`, `outstanding_at_drain_start`  |

The `.dispatch` per-resource child span is what satisfies tech-lead Phase 2 amendment 2's "per-resource child span" ask. **Span fields are redacted** per [`PRODUCT_CANON.md §12.5`](../PRODUCT_CANON.md): `credential_id` is the typed ID (no scheme content); `resource_key` is the canonical key; no scheme bytes, no token material in any field.

**Span lifecycle.** Outer span enters at function entry; per-resource child spans enter at the start of each per-resource future (inside `async move { ... }` of the `join_all` map closure). The outer span captures `resources_affected` after the dispatcher list is loaded (§3.2 line 786). Both close on function return / future completion.

`{topology}` for the acquire span is one of `pooled`, `resident`, `service`, `transport`, `exclusive` — no enumeration of the `TopologyTag::Daemon` / `EventSource` per [ADR-0037](../adr/0037-daemon-eventsource-engine-fold.md) extraction.

### §6.2 Counter metrics

Five metrics, locked. Three are NEW in this redesign; two are existing `ResourceOpsMetrics` preserved.

| Metric (literal)                                                  | Type      | Labels                                              | Where emitted                              |
|-------------------------------------------------------------------|-----------|-----------------------------------------------------|--------------------------------------------|
| `nebula_resource.credential_rotation_attempts`                    | Counter   | `outcome` ∈ {`success`, `failed`, `timed_out`}      | Per dispatch in §3.2 trampoline (one inc per resource per refresh event) |
| `nebula_resource.credential_revoke_attempts`                      | Counter   | `outcome` ∈ {`success`, `failed`, `timed_out`}      | Per dispatch in §3.2 trampoline (revocation path) |
| `nebula_resource.credential_rotation_dispatch_latency_seconds`    | Histogram | `outcome` (same set)                                | Per dispatch — wraps `tokio::time::timeout` body (covers Box::pin allocation cost per Q3) |
| `nebula_resource.acquire_total` (existing)                        | Counter   | none currently; CP3 may add `topology`              | `Manager::acquire_*` success path           |
| `nebula_resource.acquire_error_total` (existing)                  | Counter   | none currently                                      | `Manager::acquire_*` failure path           |

Plus existing `release_total`, `create_total`, `destroy_total` from `ResourceOpsMetrics` ([`metrics.rs:36-43`](../../../crates/resource/src/metrics.rs)) — preserved unchanged. Naming constants live in `nebula_metrics::naming::*` ([`metrics.rs:8-11`](../../../crates/resource/src/metrics.rs)) so the migration touches three sites: the `naming.rs` constants, the `ResourceOpsMetrics` struct field list, and the `record_*` methods.

**Histogram bucket discipline** (deferred to CP3 §11). The histogram bucketing is a config-level decision; CP2 commits to the metric *name* and *labels* but not the bucket boundaries. A reasonable default is `[0.001, 0.01, 0.1, 1.0, 10.0, 60.0]` seconds matching common rotation latency distributions.

**Cardinality.** `outcome` label has 3 values; no per-resource_key label on rotation metrics (cardinality bounded — see §6.4). Existing `acquire_total` / `acquire_error_total` are unlabeled today; if CP3 adds `topology` label, that's 5 values × 1 metric = 5 series.

### §6.3 `ResourceEvent` variant additions

Two new variants added to [`crates/resource/src/events.rs`](../../../crates/resource/src/events.rs); existing `HealthChanged` reused per B-2 (security amendment) with revocation-failure semantics — total event-shape delta is two new + one reused, not three new.

```rust
// crates/resource/src/events.rs — additions.

#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    // ... existing variants preserved ...

    /// Aggregate event for one credential refresh dispatch cycle.
    /// Per-resource detail lives in `resource.credential_refresh.dispatch`
    /// tracing spans, not on the event payload (cardinality discipline).
    /// Outcome is the aggregated `RotationOutcome` from §3.5.
    CredentialRefreshed {
        credential_id: CredentialId,
        resources_affected: usize,
        outcome: RotationOutcome,
    },

    /// Aggregate event for one credential revocation dispatch cycle.
    /// Symmetric to `CredentialRefreshed`.
    CredentialRevoked {
        credential_id: CredentialId,
        resources_affected: usize,
        outcome: RotationOutcome,
    },
}
```

**Per-resource `HealthChanged` on revoke failure** (security amendment B-2). The existing `HealthChanged { key, healthy: false }` variant is *reused* — no new variant needed. Per §3.5 contract: aggregate `CredentialRevoked` always fires; per-resource `HealthChanged { healthy: false }` fires *only* for resources where `RevokeOutcome != Ok` (Failed or TimedOut). For successful revocations, the aggregate is sufficient — no per-resource event.

**Revocation health asymmetry.** Refresh emits 1 aggregate `CredentialRefreshed` + per-resource tracing spans (no per-resource events). Revocation emits 1 aggregate `CredentialRevoked` + per-resource `HealthChanged { healthy: false }` events on the failure path. The asymmetry is the contract: refresh failure is recoverable (impl returns to healthy on next refresh attempt); revocation failure means the impl could not enforce the no-further-authenticated-traffic invariant — operators MUST see this per resource.

**`event()::key()` impl.** New variants need to extend `ResourceEvent::key()` at [`events.rs:91-106`](../../../crates/resource/src/events.rs). For `CredentialRefreshed` / `CredentialRevoked` the event has no resource key (it's credential-scoped, not resource-scoped) — the current pattern of `&'static fn key(&self) -> &ResourceKey` is no longer total. CP3 §12 picks: (a) `RegisterOptions::register_with_event_filter` per-consumer filter knob; or (b) crate-level filter trait that consumers implement. CP2 commits the *variants*; CP3 §12 must close the (a)/(b) trade-off **in the same wave that lands the new `ResourceEvent` variants** — they cannot land separately without breaking either the per-consumer filter contract (variants land first, no filter shape exists, every subscriber sees credential-scoped events whether or not they want them) or the broadcast cardinality budget (filter lands first, no variants exist, the filter API has no variants to filter against and ships dead).

### §6.4 Cardinality bounds

**Broadcast queue size: 256** (existing, [`manager.rs:275`](../../../crates/resource/src/manager.rs)). CP2 preserves. Slow subscribers receive `RecvError::Lagged` after the buffer wraps; this is observable and acceptable per Strategy §4.9.

**Tag cardinality.** `AcquireOptions::tags` is reserved-but-unused per [Phase 1 finding 🟠-8](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) and Strategy §5.2. CP2 introduces NO new tag-driven cardinality sources. Existing `outcome` label on rotation metrics caps at 3 values per metric; combined cardinality is small.

**Per-resource event cardinality bound.** For revocation, the per-resource `HealthChanged` events fire on the failure path. Theoretical upper bound: N resources sharing a credential where ALL fail revocation simultaneously emits N events on the broadcast channel. The channel capacity (256) bounds this — at N=256 simultaneous revoke failures, the aggregate `CredentialRevoked` + 256 `HealthChanged` exhausts the channel and the next event is `Lagged` for any subscriber that hasn't drained. **Operational guidance** (CP3 §11 finalises the runbook): if your deployment registers >256 resources against one credential, increase `event_tx` capacity via `ManagerConfig` (currently hardcoded; CP3 may surface as config). For 5 in-tree consumers × ~3 resources per consumer × 1 credential per resource group = ~15 expected, well under 256.

**No new cardinality from this redesign.** Strategy §5.2 defers `AcquireOptions::tags` wiring (#391); CP2 inherits the deferral. Three `outcome` enum values per metric × 5 metrics = 15 series; per-resource_key labels NOT added (would be unbounded with arbitrary registration). Future cardinality additions go through CP3 review.

### §6.5 DoD gate

CP4 ratification requires this checklist to be **demonstrably met** in the migration PR:

- [ ] Six trace spans (§6.1) emitted in the right places, with the right field names. Verifiable via `tracing-subscriber` test harness in CP3 §12 integration tests.
- [ ] Five counter metrics (§6.2) present in the `nebula_metrics::naming` constants list. `ResourceOpsMetrics` struct compiles with new fields. Increments fire in the right paths (verified via `MetricsRegistry` snapshot assertions in CP3 §12).
- [ ] Three `ResourceEvent` variants (§6.3) added; per-resource `HealthChanged { healthy: false }` fires on revocation failure path. Verifiable by subscribing to `event_tx` and asserting variant equality in integration tests.
- [ ] Cardinality bound documented in `crates/resource/docs/observability.md` (CP3 §13 deliverable — new file or extension to existing).
- [ ] No `ResourceEvent::CredentialRefreshed` / `CredentialRevoked` *without* the matching trace span + counter increment in the same code path. Cross-section consistency check at audit time.

Per `feedback_observability_as_completion.md`: **observability is DoD for a hot path, not a follow-up.** CP review hard-checks the gate; this is the section reviewers anchor to.

## §7 — Testing strategy

Per Strategy §6.3 + §4.9 observability gate. Compact subsection bullets; full per-test enumeration is CP3 §12.

### §7.1 Per-topology unit tests

**Coverage target: each of 5 topologies (Pool/Resident/Service/Transport/Exclusive) exercises the full lifecycle (§4) with at least one unit test.**

- **Pooled.** `register_pooled` → `acquire_pooled` (drain-tracker increments) → drop guard (recycle path) → `graceful_shutdown` graceful → registry empty. Existing test shape at [`tests/basic_integration.rs`](../../../crates/resource/tests/basic_integration.rs) covers; extend with the credential-bearing variant from §5.2.
- **Resident.** Single shared `Arc<R::Lease>` across acquires; verify `Released` event fires on every drop with `held: Duration > 0`; phase transitions through `Initialising → Ready → Draining → ShuttingDown` on shutdown.
- **Service.** `acquire_token` returns lease; `release_token` runs on drop; tainted tokens trigger `destroy` not `recycle` (token-lifecycle distinct from connection-lifecycle).
- **Transport.** `open_session` / `close_session` symmetry; verify `keepalive` doesn't fire on tainted handles. Transport currently has 0 Manager-level integration tests (Phase 1 🟠-13); CP3 §12 lands the first.
- **Exclusive.** Owned guard semantics; `reset` runs in `release_queue` on drop (not synchronously); guard's `tainted()` does NOT change `reset` behaviour (Exclusive has no recycle path).

Each test exercises trait reshape (§2.1) end-to-end: `type Credential = NoCredential` for Pool/Resident/Service/Transport/Exclusive *or* a `MockCredential` impl for one credential-bearing variant per topology. Spike validated this composition in `parallel_dispatch_crosses_topology_variants` ([`spike/.../resource-shape-test/src/lib.rs:583-607`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)).

### §7.2 Integration tests

**Extend [`crates/resource/tests/basic_integration.rs`](../../../crates/resource/tests/basic_integration.rs) (4114 lines current) with credential-rotation flows.** New tests:

- **`credential_refresh_drives_per_resource_swap`.** Register 3 resources of mixed topology against one `CredentialId`. Trigger `Manager::on_credential_refreshed(id, scheme)`. Verify each resource's `on_credential_refresh` was called once with the new scheme; verify `ResourceEvent::CredentialRefreshed { resources_affected: 3, outcome: RotationOutcome { ok: 3, .. } }` was broadcast.
- **`credential_revoke_drives_per_resource_taint_default`.** Register 2 resources with default `on_credential_revoke`. Trigger `Manager::on_credential_revoked(id)`. Verify subsequent `acquire_*` returns `Err(Error::credential_revoked(_))` (default-tainting per §5.3 option (b)).
- **`credential_revoke_failure_emits_health_changed`.** One resource overrides `on_credential_revoke` to return `Err`. Verify `ResourceEvent::HealthChanged { key, healthy: false }` is broadcast for that resource (security amendment B-2).
- **`drain_abort_records_failed_phase_not_ready`.** Per §5.5 wire test — verify `DrainTimeoutPolicy::Abort` lands `ResourcePhase::Failed`, not `Ready` (Phase 1 🔴-4 fix).

### §7.3 Property tests

- **Rotation idempotency.** Property: `Manager::on_credential_refreshed(id, scheme); Manager::on_credential_refreshed(id, scheme)` returns the same per-resource outcomes (modulo timing). Generated input: random N (1..32) of registered resources, all default `on_credential_refresh`. Tool: `proptest`.
- **Per-resource isolation invariant.** Property: for any subset S ⊆ registered resources where `dispatch_refresh` returns `Err`, the complement (R \ S) still returns `Ok` outcomes. Generated input: arbitrary `Result<(), MockError>` per resource. Validates security amendment B-1 at scale.
- **Drain-counter monotonicity.** Property: across any sequence of `acquire_pooled` / drop pairs, `drain_tracker.0` returns to 0 at the end. Generated input: arbitrary acquire/drop interleaving across N tasks. Already partially covered by existing tests.

### §7.4 Rotation-dispatch concurrency tests

Spike already validated the dispatch isolation with two integration tests:

- `parallel_dispatch_isolates_per_resource_latency` ([`spike/.../resource-shape-test/src/lib.rs:483-531`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)) — 3 resources, one with 3s sleep, 250ms budget. Wall clock ~270ms (the budget) not 3s. Production carries forward as-is.
- `parallel_dispatch_isolates_per_resource_errors` ([`spike/.../lib.rs:537-578`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/lib.rs)) — 3 resources, one returns `Err`, the rest return `Ok`. Outcomes reflect per-resource truth; sibling dispatches not poisoned.

CP2 commits both spike tests carry forward to production (re-implemented against the production `Manager` shape, not the spike `Manager` shape). New tests:

- **`parallel_dispatch_at_concurrency_cap_default`.** Register 32 resources (the default cap from §3.4); verify `join_all` does not OOM, all 32 outcomes return within ~budget × 1.1 wall-clock.
- **`parallel_dispatch_above_concurrency_cap_does_not_break`.** Register 64 resources (2× cap). The current shape uses unbounded `join_all`; verify it still completes (no `FuturesUnordered` cap yet — Strategy §4.3 deferral). Latency may be higher; correctness preserved.

Security-axis concurrency tests (per security-lead amendment SL-2 — empirical validation of the §5.3 / §3.5 / §3.2 contracts):

- **`revoke_during_inflight_acquire`.** Drive N in-flight `acquire_*` calls (mid-await on the runtime acquire) and call `Manager::on_credential_revoked` against the same `credential_id` while they are still pending. Verify all N in-flight acquires complete normally against the pre-revocation runtime (Strategy §4.2 + credential Tech Spec §4.3 lines 1062-1068 soft-revoke grace), but the next `acquire_*` after revoke completes returns `Err(Error::credential_revoked(_))` per §8.2 line 1505 wire.
- **`concurrent_refresh_plus_revoke`.** From two separate `tokio::spawn` tasks, fire `Manager::on_credential_refreshed(id, scheme)` and `Manager::on_credential_revoked(id)` against the same `credential_id`. Verify revocation is the final observable state regardless of dispatch ordering: `credential_revoked: AtomicBool == true` and subsequent `acquire_*` fails. The refresh outcome is observable via `RotationOutcome` but does NOT clear the revoke taint (revoke is final per Strategy §4.2 invariant).
- **`revoke_during_refresh`.** Fire `on_credential_revoked` while a refresh's blue-green pool swap (per §5.1) is in progress against the same resource. Verify the resource's pool is destroyed (post-revoke `acquire_*` fails) rather than refreshed (`AtomicBool` flip wins over the in-flight rebuild — the revoke's post-dispatch taint flip happens regardless of whether the parallel refresh override completes successfully).

### §7.5 Compile-fail probes

Four probes — three from spike + one new per CP1 Q5 resolution.

- (carried forward) `_wrong_refresh_signature_must_fail` — wrong-signature `on_credential_refresh` override rejected ([`spike/.../resource-shape-test/src/compile_fail.rs:11-95`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).
- (carried forward) `_no_credential_scheme_is_inert_must_fail` — `NoScheme` cannot pretend to be `SecretToken` ([`spike/.../compile_fail.rs:96-115`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).
- (carried forward) `_credential_bound_enforced_must_fail` — non-`Credential` type rejected ([`spike/.../compile_fail.rs:117-150`](../drafts/2026-04-24-nebula-resource-redesign/spike/crates/resource-shape-test/src/compile_fail.rs)).
- **NEW** `_wrong_revoke_signature_must_fail` — wrong-signature `on_credential_revoke` override rejected. Symmetric to probe 1; required per CP1 §2.5 Q5 (the production-relevant gap not in the spike's three probes).

Probe location in production: `crates/resource/tests/compile_fail/` directory (`trybuild` integration). CP3 §12 specifies the harness wiring.

### §7.6 Coverage target

**Per-topology + per-Manager method.**

- **Per-topology**: each of 5 topologies has at least one full-lifecycle integration test (§7.1). Branch coverage on the topology's `Resource` impl methods: ≥ 80%.
- **Per-Manager method** (§3 surface): `register_inner`, `on_credential_refreshed`, `on_credential_revoked`, `graceful_shutdown` (all four `DrainTimeoutPolicy` × `ShutdownConfig` combinations), `warmup_pool`, `warmup_pool_no_credential`, `reload_config`. Branch coverage ≥ 80% per method; 100% for safety-critical paths (revocation default-tainting, abort-policy phase-failed).
- **Aggregate workspace target**: 80% line coverage on `crates/resource/src/` after migration. Tooling: `cargo tarpaulin` or `cargo llvm-cov` (CP3 §13 picks).

CP4 ratification gate: coverage report shows ≥ 80% on `crates/resource/src/manager/`, ≥ 80% on `crates/resource/src/topology/`, 100% on the security-critical paths enumerated in §6.5 DoD checklist.

## §8 — Storage / state

What Manager persists vs runtime-only. Compact subsections.

### §8.1 Persistent state

**None.** Manager is in-process only. No disk persistence, no database. All state is reconstructed at `Manager::new()` / `Manager::with_config()` time. The credential reverse-index, the registry, the drain tracker, the release queue, the cancel token — all in-memory.

This is not new for this redesign. Current Manager has no persistent state ([`manager.rs:247-265`](../../../crates/resource/src/manager.rs) struct fields are all `RwLock` / `DashMap` / `AtomicU64` / `broadcast::Sender` — none persist). CP2 preserves.

**Cross-cascade implication.** Credential persistence lives in `nebula-credential` (`credentials` table per credential Tech Spec §4); resource persistence does not exist. If a resource needs to persist *its* state (e.g., a long-lived workflow execution counter), that's the *consumer's* responsibility, not Manager's.

### §8.2 Runtime state ownership

`Registry` owns `Arc<dyn AnyManagedResource>` ([`crates/resource/src/registry.rs`](../../../crates/resource/src/registry.rs)). Each `ManagedResource<R>` owns:

- `R` (the user's `Resource` impl value).
- `arc_swap::ArcSwap<R::Config>` for hot-reload.
- `TopologyRuntime<R>` (an enum of pool/resident/service/transport/exclusive runtime state).
- `Arc<ReleaseQueue>` (clone of Manager's queue handle).
- `AtomicU64` generation counter.
- `arc_swap::ArcSwap<ResourceStatus>` for lock-free phase reads.
- Optional `AcquireResilience`, `Arc<RecoveryGate>`.

The reverse-index in CP2 owns `Arc<dyn ResourceDispatcher>` (§3.1 field type change), which internally holds `Arc<ManagedResource<R>>` — same ownership graph as today, just with an additional indirection through the dispatcher trampoline.

**Per-resource taint state** (§5.3). New atomic on `ManagedResource<R>`: `credential_revoked: AtomicBool`. Initialised `false`; flipped `true` by `Manager::set_revoked_for_dispatcher` after any `on_credential_revoke` dispatch (regardless of override or default). Read by `acquire_*` paths to early-fail with `Err(Error::credential_revoked(_))`.

### §8.3 Reverse-index lifetime

Lives with Manager. Created in `Manager::with_config` ([`manager.rs:283-296`](../../../crates/resource/src/manager.rs)), populated by `register_inner` (§3.1), cleared on `remove`/`shutdown`.

**`Manager::remove(key)` interaction.** Removing a registered resource MUST also remove its entry from the credential reverse-index. CP2 specifies: `remove` looks up the dispatcher entries that reference the about-to-be-removed `Arc<ManagedResource<R>>` and drops them from the `credential_resources` DashMap. Subsequent `on_credential_refreshed` calls do not see the removed resource. Implementation detail: dispatcher's `Arc<ManagedResource<R>>` weak-ptr would let the dispatcher self-remove on drop, but CP3 §11 picks the simpler eager-remove path (the registry knows the resource key; direct DashMap mutation).

**On `graceful_shutdown`.** Phase 3 CLEAR (§4.5 step 5) drops the registry; the reverse-index DashMap is also cleared at this step. After shutdown, no rotations dispatch to anything.

### §8.4 Generation counter discipline

`AtomicU64::new(0)` per `ManagedResource<R>` at register time ([`manager.rs:366`](../../../crates/resource/src/manager.rs)). Monotonic — only `fetch_add(1, Release)` per `reload_config` ([`manager.rs:1326-1328`](../../../crates/resource/src/manager.rs)). Read at `acquire_*` and guard-construction sites for ABA detection.

**`reload_config` bumps generation, then transitions phase to `Ready`** per existing pattern at [`manager.rs:1325-1335`](../../../crates/resource/src/manager.rs). CP2 preserves. **`on_credential_refresh` does NOT bump generation** — it's an orthogonal concern (config vs credential). Operators distinguishing config-reload from credential-refresh look at events (`ConfigReloaded` vs `CredentialRefreshed`), not generation.

**Memory ordering.** `Release` on increment, `Acquire` on read. Acquired guards capture generation at construction; if the manager subsequently `reload_config`s, the stale-generation guard is operationally identifiable but does NOT auto-invalidate. CP3 §11 specifies the guard-staleness escalation (today's pattern at [`guard.rs`](../../../crates/resource/src/guard.rs) `GuardInner::Guarded { generation }` records but does not enforce; CP2 preserves this conservative shape).

### §8.5 Cell + ArcSwap usage

**Lock-free phase reads.** `arc_swap::ArcSwap<ResourceStatus>` on every `ManagedResource<R>` ([`manager.rs:367`](../../../crates/resource/src/manager.rs)). Readers (health probes, `is_accepting`, drain tracking) load with `arc_swap::ArcSwap::load()` — no locks. Writers (phase transitions) `store(Arc::new(new_status))`.

**ArcSwap vs RwLock decision** (per §5.1 cross-ref). ArcSwap is correct for `ResourceStatus` because:
- Reads are FAR more frequent than writes (every `acquire_*` reads phase; phase changes only on lifecycle boundaries).
- Writers don't need to hold the lock across `await` (phase transition is a single `store`).
- Read-side does not need exclusion (reading "the latest" is sufficient).

**Pool swap uses RwLock instead** (§5.1) because writers DO need to hold across `await` (`build_pool_from_scheme(...).await?` inside the write guard).

**`config: ArcSwap<R::Config>`.** Same rationale as `ResourceStatus` — readers see the most recent config; writers `store(Arc::new(new))` synchronously. Hot-reload uses this shape per [`manager.rs:1318`](../../../crates/resource/src/manager.rs).

**No `RwLock<Pool>` on `ManagedResource<R>`.** The pool swap pattern (§5.1) is INSIDE the user's `Resource` impl (e.g., `PostgresPool::inner: Arc<RwLock<Pool>>`), not on `ManagedResource<R>`. Manager's owning structures stay lock-free; only the user-side runtime owns the swap-coordination lock.

### Open items raised this checkpoint

**CP1 open items (unchanged from CP1 ratification):**

- **§2.4 — TopologyRuntime engine-side landing.** ADR-0037 commits to engine-fold; engine-side module layout, primitive naming (`DaemonRegistry` / `WorkerRuntime` / etc.), `EventSource → TriggerAction` adapter signature is **CP3 §13 deliverable**. Out of CP1 scope.
- **§3.4 — `FuturesUnordered` fan-out at N>32.** [Strategy §4.3](2026-04-24-nebula-resource-redesign-strategy.md) defers; CP1 records the soft cap (32) as configuration; production landing of FuturesUnordered fan-out conditional on operational signal post-soak per [Strategy §6.3](2026-04-24-nebula-resource-redesign-strategy.md).
- **§3.5 — Per-resource event broadcast cardinality bound.** CP1 LOCKED aggregate-only `ResourceEvent::CredentialRefreshed { resources_affected, outcome: RotationOutcome }` for the refresh path and aggregate `CredentialRevoked` + per-resource `HealthChanged { healthy: false }` (only on `RevokeOutcome != Ok`) for the revocation path per security B-2. **Resolved in CP2 §6.4**: per-resource `HealthChanged` events are bounded by the existing `event_tx` capacity of 256; deployments registering >256 resources against one credential need to surface broadcast capacity via `ManagerConfig` (CP3 §11 ergonomics).
- **§2.3 — Idempotency expectation on `on_credential_refresh`.** CP2 §6 was scheduled to finalize whether Manager retries on transient failures; **CP2 leaves Manager retry policy out of scope** (consumers decide; default is no retry) and preserves the implementer-side idempotency expectation from CP1 §2.3.

**CP2 open items (raised this checkpoint):**

- **§5.3 — Multi-tenant pool taint exception.** CP2 commits unconditional taint-flip on revocation (option (b)). A future consumer with multi-tenant pool sharing one resource across many credentials may need a per-`RegisterOptions::tainting_policy` knob to opt out. Trigger to revisit: a real consumer (not synthetic test) that needs the exception. CP3 §11 candidate.
- **§5.4 — Function-level boundaries within submodules.** CP2 locks file structure (which submodule each public method lands in) but defers the per-`impl Manager { fn ... }` block placement to CP3 §9. Reviewer note: cut points within `mod.rs` may surface ergonomics that warrant restructuring; CP3 captures.
- **§6.3 — `ResourceEvent::key()` return type.** New `CredentialRefreshed` / `CredentialRevoked` variants are credential-scoped, not resource-scoped — current `key() -> &ResourceKey` is no longer total. CP3 §12 picks: `Option<&ResourceKey>` (breaking subscriber change) or orthogonal `credential_id() -> Option<&CredentialId>` accessor (additive). CP2 commits the *variants*; defers return-type to CP3.
- **§6.5 — Histogram bucket boundaries.** CP2 commits the histogram metric *name* and *labels* but defers bucket configuration to CP3 §11 (default `[0.001, 0.01, 0.1, 1.0, 10.0, 60.0]` seconds is a candidate; production tuning post-soak per Strategy §6.3).
- **§7.6 — Coverage tooling pick.** CP2 sets the 80% target; CP3 §13 picks `cargo tarpaulin` vs `cargo llvm-cov` based on Windows + macOS coverage support.

### Handoffs requested

**CP1 handoffs (closed at CP1 ratification — preserved as historical record):**

- spec-auditor (CP1): structural audit. CLOSED 2026-04-25 with PASS_WITH_MINOR per CP1 changelog.
- rust-senior (CP1): trait-shape ratification. CLOSED 2026-04-25 with RATIFY_WITH_EDITS per CP1 changelog.
- tech-lead (CP1): ratification → flipped ADR-0036 + ADR-0037 to `accepted`. CLOSED 2026-04-25 per CP1 changelog.

**CP2 handoffs requested (parallel co-decision review, no architect iteration between):**

- **tech-lead**: CP2 ratification of §4 lifecycle + §5 mechanism + §6 observability + §7 testing + §8 storage. Specifically scrutinize: (a) §5.3 revocation default-tainting decision (option (b)) — security implication central, lock vs question; (b) §6.1-§6.3 observability identifier locks (every name is CP-review-gateable per §6 prelude; reviewer pushback on names lands as amendment, not CP3 rename); (c) §5.4 file-split cut points — submodule list locks CP3 §9 function-level work.
- **security-lead**: CP2 ratification of all security-axis content. Specifically scrutinize: (a) §5.3 revocation default-tainting (option (a)/(b)/(c) trade-off — confirm option (b) honours [B-1 / constraint #2 invariant](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)); (b) §5.2 `warmup_pool` credential-bearing signature (B-3 amendment honoured? `Scheme::default()` removed?); (c) §6.3 per-resource `HealthChanged { healthy: false }` on revocation failure (B-2 amendment honoured? cardinality acceptable?); (d) §5.1 `Arc<RwLock<Pool>>` vs `ArcSwap` — Manager NEVER holds scheme longer than dispatch call (constraint #2 invariant)?
- **spec-auditor** (after tech-lead + security-lead converge): CP2 structural audit. Verify cross-section consistency (every § forward ref to CP3/CP4 is real, every code-block-cited-line is in the cited file), forward-reference integrity (no §6.5 DoD claim about a §7 test that doesn't exist in §7), claim-vs-source (every "per Strategy §X" is in Strategy §X; every spike `lib.rs:N-M` line range is in the spike file). Pay specific attention to §5.3 — three options enumerated; verify each rejection rationale derives from the cited source (Strategy §4.2, security review).

## Changelog

- 2026-04-25 CP1 draft — §0 + §1 + §2 + §3 (architect)
- 2026-04-25 CP1 ratified (architect; tech-lead RATIFY_WITH_EDITS + rust-senior RATIFY_WITH_EDITS + spec-auditor PASS_WITH_MINOR). Edits applied: §3.2 `Result<…, Error>` wrapper restored on `on_credential_refreshed`; §3.5 `RotationOutcome` aggregate type defined; §3.5 event broadcast contract LOCKED to aggregate-only refresh + aggregate revoke + per-resource `HealthChanged` on revoke failure (B-2); §3.2 dispatcher lifetime SAFETY comment added; §3.1 NEW error constructors (`Error::missing_credential_id`, `Error::scheme_type_mismatch::<R>()`) called out as required additions. ADR-0037 acceptance gate amended in place to gate on the engine-fold *decision*, not the engine-side *implementation* (which is CP3 §13).
- 2026-04-26 CP2 draft — §4 lifecycle + §5 implementation specifics + §6 operational + §7 testing + §8 storage (architect). Status flipped to `CP2 draft — awaiting tech-lead + security-lead review` per cadence. Key locks: §5.3 revocation default-hook = option (b) Manager-enforced taint flip; §5.1 pool swap = `Arc<RwLock<Pool>>` (NOT `ArcSwap`); §5.2 `warmup_pool` takes credential scheme explicitly + dedicated `warmup_pool_no_credential` for opt-out; §6.1 six trace span names locked; §6.2 five counter metrics locked; §6.3 two new `ResourceEvent` variants + per-resource `HealthChanged` on revoke failure (B-2 honoured); §5.4 seven-submodule file-split cut points locked; §5.5 `ManagedResource::set_failed` wired into `DrainTimeoutPolicy::Abort` path (Phase 1 🔴-4 closed).
- 2026-04-24 CP2 ratified (architect; tech-lead RATIFY_WITH_EDITS + security-lead ENDORSE_WITH_AMENDMENTS). Status flipped to `CP2 ratified — pending CP3 dispatch`. Six bounded edits applied: **E1** §6.3 line 1357 — "Three variants added" → "Two new variants added; existing `HealthChanged` reused per B-2"; **E2** §1.2 — 5-submodule list reconciled to §5.4's 7-submodule list with explicit "extending Strategy §4.5" rationale (registration.rs holds 🔴-1 fix; shutdown.rs holds 🔴-4 fix); **E3** §6.3 line 1391 — `event()::key()` CP3 §12 forward-ref tightened to pin (a) `RegisterOptions::register_with_event_filter` vs (b) crate-level filter trait trade-off and require it close in the same CP3 wave that lands the new variants; **SL-2** §7.4 — three security-axis concurrency tests added (`revoke_during_inflight_acquire`, `concurrent_refresh_plus_revoke`, `revoke_during_refresh`); **SL-3** §5.1 — resource-side `build_pool_from_scheme` budget guidance added (≤ 60% of dispatch timeout) with CP3 §11 forward-ref; **SL-1** new §5.6 — CP3 deferrals subsection capturing future `RegisterOptions::tainting_policy` knob + future `warmup_pool_by_id` helper with their gating constraints. §5 prelude updated five-items → six-items. ADR-0036 + ADR-0037 already at `accepted` per CP1 ratification — CP2 lock is a Tech Spec internal milestone, not an ADR gate. Awaiting orchestrator dispatch of CP3 (§9-§13 interface + ergonomics).

---

**Tech Spec CP1 ratified.** ADR-0036 and ADR-0037 (per its 2026-04-25 amended-in-place gate text) both clear-able to `accepted` on this ratification. CP2 (§4-§8) drafted; awaiting tech-lead + security-lead parallel co-decision review.
