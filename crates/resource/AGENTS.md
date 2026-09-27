# nebula-resource — Agent orientation
> Local guide for `crates/resource/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Engine-owned resource lifecycle (acquire / health-check / hot-reload / scope-bounded release) for pool & SDK-client integrations, handed to actions as a drop-releasing `ResourceGuard`.
**Layer:** Business — depends only downward (root AGENTS.md → Layered Dependency Map).

## Commands

- `cargo nextest run -p nebula-resource --all-features`  ·  doctests: `cargo test -p nebula-resource --doc --all-features`
- `cargo nextest run -p nebula-resource --features rotation` — exercises the credential-rotation fan-out (`tests/resident_rotation_race.rs`, `tests/credential_slot_epoch_fold.rs`); neither this crate nor `nebula-credential` has a `test-util` feature.
- Derive crate: `cargo check -p nebula-resource-macros`; expansion contracts run in the parent and SDK suites below. Root examples are separate named targets, not a `--example resource_*` wildcard.

## Key files

- `src/lib.rs` — crate facade + re-exports
- `src/resource.rs` — `Provider` trait (`Config`/`Instance` assoc types, slot-rotation hooks), `HasCredentialSlots`, `ResourceConfig`, `ResourceMetadata`; `Resource` is the derive macro (slot plumbing only)
- `src/slot.rs` — public, generation-stamped `SlotCell`; retained resource ownership lives in `runtime/resident.rs`, not credential slots
- `src/registry.rs` — type-erased registry, scope-aware lookup, `(key, scope, slot_identity)` row identity
- `src/manager/` — `Manager::register(RegistrationSpec)` funnel, acquire dispatch, shutdown/drain
- `src/topology/contract.rs` — the open `Topology<R>` trait (entry-centric, framework-driven; **slot** = credential axis, **entry** = store axis — see `src/topology/store.rs` module docs). The **framework** owns the acquire loop (`ManagedResource::run_acquire_loop`): fenced `store.checkout()`, stale-entry destroy, cancel-safe wrap, on-release return-or-destroy. A topology supplies only thin R-aware hooks (`create_entry` / `entry_instance` / `into_owned_instance` / `accept` / `prepare` / `on_release` / `pools` / `store_capacity` / `dispatch_credential_hook` / …) and **cannot** reach the revoke fence — never write `store.checkout` / `resource.destroy` / a stale loop / an epoch compare in a `Topology` impl.
- `src/topology/` + `src/runtime/` — `Pooled<R>` / `Resident<R>` / `Bounded<R>` built-in topologies (`Topology<R>` impls; Bounded = runtime concurrency cap, capped/exclusive/unbounded, no warm pool); the framework-owned `InstanceStore<Entry>` is the real idle queue (`ManagedResource.store`)
- `src/release_queue/mod.rs` — `ReleaseQueue` best-effort async drain (canon §11.4); `src/recovery/` — thundering-herd `RecoveryGate`

## Conventions & never-do

- Credentials are declared as `#[credential(key="…")] field: SlotCell<CredentialGuard<C>>`; read via derive-emitted `self.<field>_slot()` (`Option<Arc<…>>`, handle `None`/unbound) — never off the raw cell. No singular `Resource::Credential`; `NoCredential` is gone.
- This crate is NOT a connection driver, retry pipeline, secret holder, or expression evaluator — it owns the lifecycle wrapper only (see Non-goals).
- Async release is best-effort on crash; never assume "release ran" without an explicit checkpoint (canon §11.4).
- For teardown changes, read `src/runtime/teardown.rs` and `src/manager/shutdown.rs` before relying on README hook descriptions. A declared provider hook is not proof of a runtime call site, and queued release is not completed physical destruction.
- `#![forbid(unsafe_code)]` + `#![deny(missing_docs)]` + `#![warn(missing_debug_implementations)]` are active; lifecycle work emits a `ResourceEvent` variant (observability is DoD).

## Industry failure modes — do not repeat

Known defects of peer pools, breakers and workflow engines. A change that reintroduces one needs an explicit reason in review.

- **Local state is not backend health** (Envoy `split_external_local_origin_errors`): only `Transient` / `Exhausted` may trip `RecoveryGate`; `Backpressure` and `Revoked` never do (`manager/gate.rs`).
- **No lockstep expiry or retry** (HikariCP `maxLifetime` attenuation, AWS `buffer_time` jitter): any fleet-wide timer — lifetime, backoff, refresh-ahead — gets jitter.
- **Single-probe recovery** (Resilience4j half-open defaults to 10 calls): keep exactly one prober per gate.
- **No panics on operator input**: `Instant + Duration` goes through `crate::deadline::deadline_after`; config validation returns `Error::permanent`.
- **Nothing unbounded per lease**: no per-acquire task, timer or allocation that outlives the lease; per-row background work stays bounded and observable.
- **No silent staleness for secrets** (Airflow `cache_ttl_seconds`, n8n polling): rotation is pushed to live instances; never add a cache that can serve a rotated-out credential.
- **Limits are per process** (Temporal, Airflow, Dagster, Prefect, Inngest enforce cluster-wide on a server): never document a `Bounded` / `Pooled` cap as a backend-wide budget.
- **A leak detector that only logs does not free capacity** (HikariCP `leakDetectionThreshold`): don't rely on `max_hold_duration` to prevent exhaustion.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Acquire/guard/teardown | [acquire_lifecycle](tests/acquire_lifecycle.rs), [guard_release](tests/guard_release.rs), [recovery_and_shutdown](tests/recovery_and_shutdown.rs), [shutdown_race](tests/shutdown_race.rs). |
| Custom topologies or revoke fencing | [custom_topology_manager](tests/custom_topology_manager.rs), [revoke_recycle_toctou](tests/revoke_recycle_toctou.rs). |
| Credential suspension (same-material blocks) | `src/runtime/admission_tests.rs` (spans, gate tickets), `src/manager/credential_suspension_tests.rs` (row wiring, reopen, install reopen), `src/credential_fanout/suspension_tests.rs` (`--features rotation`: observer scans, `ReauthRequired` hint), [rate_limit](tests/rate_limit.rs) (`Limited` close cause), engine `resource::activation::tests` and `resource_rotation_fanout_wiring`. Invariant I5: `manager` module docs; contract: [credential-rotation](docs/credential-rotation.md) "Same-material blocks". |
| Credential use revision (witnessed floors, readmission) | `src/runtime/admission_tests.rs` (floors, `Readmitted`), `src/manager/credential_suspension_tests.rs` (missed interval, stale reads, install revision), `src/credential_fanout/suspension_tests.rs` (`--features rotation`), engine `resource::activation::tests` (incl. `sqlite_acceptance` over real encrypted SQLite) and `resource_rotation_fanout_wiring`. Invariant I6: `manager` module docs; contract: [credential-rotation](docs/credential-rotation.md) "Use revision". Never close leases on a readmission, and never compare the aggregate revision (a rename). |
| Strict per-acquire credential read (profiles, join-next reads, refusal reasons) | `src/manager/strict_admission_tests.rs` (decision table, every refusal, lock not held across the read, ticket supersession, taint during the read, refresh join timing, background creates), `src/manager/credential_reads_tests.rs` (join-next conformance, burst cost, leader re-election, timeouts, cancel), `src/manager/strict_profile_tests.rs`, engine `resource::activation::tests::sqlite_acceptance` (S1–S5 over real encrypted SQLite), worker `compose` test (strict wiring). Invariant I7: `manager` module docs; contract: [credential-rotation](docs/credential-rotation.md) "Strict per-acquire admission". Never hold `Manager.admission` across the read, never let a caller join a read already in flight when it arrived, and keep store outages out of the recovery gate. |
| Admission generations / lease closing | `src/runtime/admission_tests.rs`, `src/manager/admission_generation_tests.rs`, [rate_limit](tests/rate_limit.rs) (`Limited` waits), [resident_rotation_race](tests/resident_rotation_race.rs) (hand-out refusal), [public_surface_compile_fail](tests/public_surface_compile_fail.rs). Invariants I1–I4: `manager` module docs, "Admission generations". |
| Strict per-attempt credential read (managed attempts, unit pin, `StrictPerAttempt`) | `src/call_strict_tests.rs` (one read per attempt, read after the quota wait, lock not held across the read, taint/closing/cancel/shutdown during the read, read timeout, refresh join, rebinding, pin at the first grant, superseded pin → `Rebinding` with the unit's retry fold, join-next across units, pooled leases), `src/call/strict.rs` tests (pin capture and generations), `src/manager/strict_profile_tests.rs` (profile latch), [managed_logger](tests/managed_logger.rs) (slot-less row on a strict manager reads nothing), engine `resource::activation::tests::sqlite_acceptance` (F1–F6 over real encrypted SQLite). Invariant I7 "Per attempt": `manager` module docs; contract: [credential-rotation](docs/credential-rotation.md) "Strict per-attempt admission". Never hold `Manager.admission` (or any sync mutex) across the attempt's read, never await between the pin and the grant, and never re-pin mid-unit. |
| Rate-limit profile / deprecated `Limited` surface | `src/rate_limit_tests.rs` (profile latching, stable names), [rate_limit](tests/rate_limit.rs) (health snapshot and erased view per profile), README "Rate-limit profiles" table and migration, SDK [public_api_snapshot](../sdk/tests/public_api_snapshot.rs) (`[interim]` tags, `#[deprecated]` on `Limited`; re-bless with `task sdk:api:bless`), SDK [public_perimeter_external_contract](../sdk/tests/public_perimeter_external_contract.rs) (`resource_limited_deprecated`: `wrap` fails under `deny(deprecated)`). Only `InterimPerClosure` is interim; `Limited`, `LimitedError` and `ResourceLimiter::wrap` are deprecated since 0.21.0 until their removal (MIGRATION P10) — in-crate uses carry `#[expect(deprecated)]`; `RateLimitProfile` stays out of the SDK. |
| Managed call facade (`call`: units, attempts, sent state, `PinSlots`) | `src/call_tests.rs` (admission refusals, cancel/grant race, deadline/panic → `MaybeSent`, retry-safety table, unit caps, drain, span/metrics/event), [managed_logger](tests/managed_logger.rs) (slot-less fixture: no credential reads, enqueued vs flushed, shutdown flush), [managed_derived_slots](tests/managed_derived_slots.rs) and the `derive_pinned_slots_are_private` probe (derived `PinSlots`), `src/rate_limit_tests.rs` (weighted booking, `PerAttempt` latch), nebula-action `error_tests` (`From<OpError>`), SDK [public_perimeter_external_contract](../sdk/tests/public_perimeter_external_contract.rs) (`resource_managed_logger`, `managed_no_deref`) and [public_api_snapshot](../sdk/tests/public_api_snapshot.rs). Never add `Deref` to `Managed` or `Unit`; the strict per-attempt credential read and registration live in `call/strict.rs` (row above). |
| Managed row facade and sessions (`ManagedRow`, per-unit checkout, row gate, `SessionProvider`) | `src/call_row_tests.rs` (R1–R14: no connection held while waiting for quota, gate FIFO without backpressure, one read per idle hit / two per create / none slot-less, blocks during the gate wait and the create, lock not held across R1/R2/create, hand-out refusal, cancel at every wait, shutdown, deadline at the gate, retry-safety over lease and row, per-attempt checkout and pin, profile latch, pool saturated by plain leases), `src/call_session_tests.rs` (outcome table, abandoned sessions destroy, connection-bound eviction vs session-bound reuse, cooperative closing, strict reads, nested same-row refusal, metrics and span), `src/guard_tests.rs` (`created`, built slot epoch, row slot freed with the settlement), engine `resource::activation::tests::sqlite_acceptance::session_postgres` (PG1–PG8 on real PostgreSQL, run by the PostgreSQL CI job), SDK [public_perimeter_external_contract](../sdk/tests/public_perimeter_external_contract.rs) (`resource_session`, `managed_row_no_deref`, `session_escape`) and [public_api_snapshot](../sdk/tests/public_api_snapshot.rs) (`managed_row_has_no_deref`). Invariant I7 "Per-unit checkout": `manager` module docs; contract: [credential-rotation](docs/credential-rotation.md) "Sessions and connection-bound pools". Never wait for quota or the row gate while holding a checkout; never hold `Manager.admission` across R1, R2 or the dispatch; free the gate permit only with the topology permit. |
| Streaming units (`call::StreamOperation`, `Streaming`) | `src/call_stream_tests.rs` (item order, error after items, backpressure, drop/cancel/deadline/closing), SDK `tests/resource_http.rs` (streamed HTTP bodies). Streaming stays on the public `Managed`/`Unit` API through the private `Streamed` adapter; never give the unit runtime a streaming variant. |
| Credential rotation | [resident_rotation_race](tests/resident_rotation_race.rs), [credential_slot_epoch_fold](tests/credential_slot_epoch_fold.rs), both with `--features rotation`. |
| Derives | [derive_resource_compile_fail](tests/derive_resource_compile_fail.rs), [resource_config_derive](tests/resource_config_derive.rs), and SDK [derive_external_contract](../sdk/tests/derive_external_contract.rs). |

## See also

- `README.md` — full design, migration recipe (pre-v4 → v4), topology & shared-resource reference
- `docs/topology-reference.md` — topology selection guidance; canon invariants L2-§11.4 / §13.3
